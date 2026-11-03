//! analyze 编排（本地计算，无 Provider）与读侧视图（analysis show / history）。
//!
//! 计算全部复用 dsheng-core：`import::*` 识别/解析、`analysis::engine` 求和与
//! 比较、`analysis::periods` 基期/覆盖、`analysis::targets` 目标政策、
//! `analysis::fingerprint` 输入指纹、`repository::analysis_runs` 不可变 revision。
//! 本层把扫描观察组织成指标行/序列视图，并以 JSON 快照存入 run_revisions
//! （normalized_metrics_json / comparisons_json；形状即 read-views 字段名）。

use crate::error::{CliError, CliResult};
use crate::store::sources::{scan_directory, ScannedFile};
use chrono::NaiveDate;
use dsheng_core::analysis::fingerprint::{
    AnalysisRevisionInput, BaseRef, ManualInputSnapshot, MetricDefSnapshot, SourceSelection,
};
use dsheng_core::analysis::periods::{mom_base, yoy_base, CalendarPolicy};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::decimal::to_canonical_string;
use dsheng_core::contracts::domain::Granularity;
use dsheng_core::contracts::domain::{Cadence, DateRange, MeasureRole};
use dsheng_core::import::fileio::CellValue;
use dsheng_core::import::headers::parse_period_header;
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use uuid::Uuid;

/// 观察记录（规范化中间态；evidence 定位单元格/手工输入）。
#[derive(Debug, Clone)]
struct Obs {
    metric_id: String,
    unit: String,
    role: MeasureRole,
    value: Decimal,
    period: DateRange,
    evidence_id: String,
    source_label: String,
    scope: String,
}

#[derive(Debug, Clone)]
struct ConflictDraft {
    metric_id: String,
    role: MeasureRole,
    period: DateRange,
    scope_key: String,
    unit: String,
    candidates: Vec<(Decimal, String, String)>, // value, evidence_id, source_label
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalyzeExtraInput {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub target_period: Option<serde_json::Value>,
    /// 用户显式决策：分析窗口内 flow 指标的空实绩格按 0 计（映射合同
    /// coverage.empty_days_are_zero=true）。必须同时给出 `coverage_confirmed_by`；
    /// 缺省 false：缺失≠0（§1.1）。
    #[serde(default)]
    pub empty_as_zero: bool,
    /// 覆盖确认人（映射 coverage.confirmed_by；给出即 coverage.status=complete）。
    #[serde(default)]
    pub coverage_confirmed_by: Option<String>,
}

/// 本次 analyze 的覆盖决策（data-period-import.md §2 coverage）。
#[derive(Debug, Clone, Default)]
struct CoverageDecision {
    confirmed_by: Option<String>,
    empty_days_are_zero: bool,
}

impl CoverageDecision {
    /// 校验 analyze 选项：empty_as_zero 须有非空 coverage_confirmed_by。
    fn from_extra(extra: Option<&AnalyzeExtraInput>) -> CliResult<Self> {
        let Some(extra) = extra else {
            return Ok(Self::default());
        };
        let confirmed_by = extra
            .coverage_confirmed_by
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if extra.empty_as_zero && confirmed_by.is_none() {
            return Err(CliError::new(
                CliErrorCode::DataValidationFailed,
                "empty_as_zero=true 须同时给出非空 coverage_confirmed_by（显式确认后空值才按 0 计；默认缺失≠0）",
            )
            .with_field("coverage_confirmed_by"));
        }
        Ok(Self {
            empty_days_are_zero: extra.empty_as_zero,
            confirmed_by,
        })
    }

    /// 空格补 0 仅在显式确认后启用。
    fn zero_fill(&self) -> Option<&str> {
        if self.empty_days_are_zero {
            self.confirmed_by.as_deref()
        } else {
            None
        }
    }

    /// coverage 快照（合同字段 + 补 0 的证据坐标审计）。
    fn snapshot(&self, window: DateRange, grain: Granularity, zero_filled: &[String]) -> Value {
        let coverage = dsheng_core::contracts::domain::Coverage {
            from: window.from,
            to: window.to,
            granularity: grain,
            status: if self.confirmed_by.is_some() {
                dsheng_core::contracts::domain::CoverageStatus::Complete
            } else {
                dsheng_core::contracts::domain::CoverageStatus::Unknown
            },
            confirmed_by: self.confirmed_by.clone(),
            empty_days_are_zero: self.empty_days_are_zero,
        };
        let mut value = serde_json::to_value(coverage).expect("coverage json");
        let mut cells = zero_filled.to_vec();
        cells.sort();
        value["zero_filled_evidence_ids"] = json!(cells);
        value
    }
}

fn parse_date(text: &str, flag: &str) -> CliResult<NaiveDate> {
    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .map_err(|_| CliError::invalid(format!("{flag} 必须是 YYYY-MM-DD")).with_field(flag))
}

fn parse_direction(raw: &str) -> dsheng_core::contracts::domain::Direction {
    serde_json::from_value(json!(raw)).unwrap_or(dsheng_core::contracts::domain::Direction::Neutral)
}

fn cadence_of_grain(g: &str) -> Cadence {
    serde_json::from_value(json!(g)).unwrap_or(Cadence::Month)
}

/// 从 DB 读当前指标定义（最新版本；按 metric_key 去重）。
///
/// `metric_id` 装载 stable_metric_id（§1.4 定义身份：版本变化保留身份）：
/// 观察归属、冲突草稿、normalized metrics 投影全部以稳定身份为键；
/// 具体版本行的 metric_id/semantic_hash 仍冻结在 run_revisions 快照。
struct MetricDefRow {
    metric_id: String,
    metric_key: String,
    version: i64,
    semantic_hash: String,
    label: String,
    label_aliases: Vec<String>,
    semantic_type: String,
    direction: String,
    target_constraint_scope: String,
    revenue_semantics: bool,
    unit_json: String,
    scope: String,
}

fn load_metric_defs(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<MetricDefRow>> {
    let mut stmt = conn.prepare(
        "SELECT m.stable_metric_id, m.metric_key, m.version, m.semantic_hash, m.label,
                m.label_aliases_json, m.direction, m.target_constraint_scope, m.revenue_semantics,
                m.unit_signature_json, m.semantic_type
         FROM metric_definitions m
         WHERE m.workspace_id = ?1 AND m.version =
               (SELECT MAX(version) FROM metric_definitions m2
                WHERE m2.workspace_id = m.workspace_id AND m2.metric_key = m.metric_key)
         ORDER BY m.metric_key",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
        Ok(MetricDefRow {
            metric_id: r.get(0)?,
            metric_key: r.get(1)?,
            version: r.get(2)?,
            semantic_hash: r.get(3)?,
            label: r.get(4)?,
            label_aliases: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
            direction: r.get(6)?,
            target_constraint_scope: r.get(7)?,
            revenue_semantics: r.get::<_, i64>(8)? == 1,
            unit_json: r.get(9)?,
            semantic_type: r.get(10)?,
            scope: "全公司".into(),
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn match_metric<'a>(defs: &'a [MetricDefRow], label: &str) -> Option<&'a MetricDefRow> {
    let normalized = label.trim();
    defs.iter().find(|d| {
        d.metric_key == normalized
            || d.label == normalized
            || d.label_aliases.iter().any(|a| a == normalized)
    })
}

fn unit_display(unit_json: &str) -> String {
    serde_json::from_str::<Value>(unit_json)
        .ok()
        .and_then(|v| {
            v.get("base_unit")
                .and_then(|b| b.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| unit_json.to_string())
}

fn cell_value(cell: Option<&CellValue>) -> Option<Decimal> {
    let cell = cell?;
    match cell {
        // 精确整数与 XLSX 数值 XML 原始字面量（§1.1；超输入精度为 invalid → None）
        CellValue::Int(_) | CellValue::Number(_) => cell.exact_decimal(),
        CellValue::Text(t) => dsheng_core::import::value::parse_amount_text(t.trim()),
        CellValue::FloatUnverified(text) => {
            // 词法保留的 f64 文本：仅当无小数精度损失（整数值）才采用，
            // 否则 precision_unverified 不进入经营计算（§1.1）。
            if text.contains('.') || text.contains('e') || text.contains('E') {
                None
            } else {
                Decimal::from_str_exact(text).ok()
            }
        }
        _ => None,
    }
}

/// 空白单元格（§1.1 missing 且无 missing_token：无值或仅空白文本）。
/// `待补`、`NA/N/A/-`、`—` 等占位不是空白。
fn cell_is_blank(cell: Option<&CellValue>) -> bool {
    match cell {
        None | Some(CellValue::Empty) => true,
        Some(CellValue::Text(t)) => t.trim().is_empty(),
        Some(_) => false,
    }
}

/// 显式覆盖确认后的空格补 0（§2 coverage.empty_days_are_zero）：仅 actual、
/// flow 指标、观察期间落在分析窗口内的空白格；目标/年度分解的空格是“未设定”
/// 而非 0（§1.2 explicit_target_only），stock/ratio 空格不可当 0（需期末/底数）。
fn zero_fill_applies(
    coverage: &CoverageDecision,
    role: MeasureRole,
    def: &MetricDefRow,
    cell: Option<&CellValue>,
    period: DateRange,
    window: DateRange,
) -> bool {
    coverage.zero_fill().is_some()
        && role == MeasureRole::Actual
        && def.semantic_type == "flow"
        && cell_is_blank(cell)
        && period.from >= window.from
        && period.to <= window.to
}

fn zero_fill_label(file_label: &str, coverage: &CoverageDecision) -> String {
    format!(
        "{file_label}（空格按0计·覆盖确认：{}）",
        coverage.zero_fill().unwrap_or_default()
    )
}

/// 观察提取结果：观察、冲突草稿、来源选择、映射行、待确认指标数、补 0 证据坐标。
type Extracted = (
    Vec<Obs>,
    Vec<ConflictDraft>,
    Vec<SourceSelection>,
    Vec<Value>,
    usize,
    Vec<String>,
);

/// 提取观察（核算表/推移表/手工输入）＋冲突草稿（同键不同值）。
fn extract_observations(
    conn: &Connection,
    workspace_id: Uuid,
    defs: &[MetricDefRow],
    files: &[ScannedFile],
    window: DateRange,
    scope: &str,
    coverage: &CoverageDecision,
) -> CliResult<Extracted> {
    let mut obs: Vec<Obs> = Vec::new();
    let mut pending_metrics: Vec<String> = Vec::new();
    let mut mapping_rows: Vec<Value> = Vec::new();
    let mut sources: Vec<SourceSelection> = Vec::new();
    let mut zero_filled: Vec<String> = Vec::new();
    for file in files {
        for (sheet_data, sheet) in file.sheets.iter() {
            let Some(header) = &sheet.header else {
                continue;
            };
            if !matches!(sheet.role, "accounting" | "trend") {
                continue;
            }
            sources.push(SourceSelection {
                source_id: file.file_label.clone(), // source_id 在注册后回填
                sheet: sheet.sheet.clone(),
                range: format!(
                    "A{}:{}",
                    header.row + 1,
                    dsheng_core::import::headers::column_letter(
                        sheet_data.rows.iter().map(|r| r.len()).max().unwrap_or(1)
                    )
                ),
                content_hash: file.content_hash.clone(),
            });
            let col_of = |field: &str| {
                header
                    .columns
                    .iter()
                    .find(|(_, f)| *f == field)
                    .map(|(c, _)| *c)
            };
            let label_col = col_of("metric_label").or_else(|| col_of("task_title"));
            let Some(label_col) = label_col else {
                continue;
            };
            let period_col = col_of("period");
            // 推移表：无 period 列时，表头块行内可解析为期间的未映射列即期间列
            //（数据行里的日期文本不作期间锚点，§4.1/§4.3）。
            let trend_period_cols: Vec<(usize, DateRange)> = if period_col.is_none() {
                let width = sheet_data.rows.iter().map(|r| r.len()).max().unwrap_or(0);
                let header_rows: Vec<&Vec<CellValue>> = (header.row.saturating_sub(1)
                    ..header.row.saturating_sub(1) + header.row_count)
                    .filter_map(|r| sheet_data.rows.get(r))
                    .collect();
                (0..width)
                    .filter_map(|c| {
                        if header.columns.iter().any(|(hc, _)| *hc == c) {
                            return None;
                        }
                        // 合并表头：列内首个非空表头（左上标签投影）
                        let text = header_rows
                            .iter()
                            .find_map(|r| r.get(c).filter(|v| !v.raw_text().trim().is_empty()))
                            .map(|cell| cell.raw_text())?;
                        let range = parse_period_header(&text, None)?;
                        Some((c, range))
                    })
                    .collect()
            } else {
                Vec::new()
            };
            for (field, role) in [
                ("actual", MeasureRole::Actual),
                ("planned", MeasureRole::Plan),
                ("annual_month_plan", MeasureRole::AnnualMonthPlan),
            ] {
                let Some(col) = col_of(field) else { continue };
                mapping_rows.push(json!({
                    "tool_field": field,
                    "source_label": sheet_data
                        .rows
                        .get(header.row.saturating_sub(1))
                        .and_then(|r| r.get(col))
                        .map(|c| c.raw_text())
                        .unwrap_or_default(),
                    "recognized": dsheng_core::import::headers::column_letter(col),
                    "usage": format!("{}列（{}）", dsheng_core::import::headers::column_letter(col), field),
                }));
                for (row_idx, row) in sheet_data.rows.iter().enumerate().skip(header.row) {
                    let Some(label) = row.get(label_col).map(|c| c.raw_text()) else {
                        continue;
                    };
                    if label.trim().is_empty() {
                        continue;
                    }
                    let Some(def) = match_metric(defs, &label) else {
                        let note = format!("未注册指标行（不参与计算）: {label}");
                        if !pending_metrics.contains(&note) {
                            pending_metrics.push(note);
                        }
                        continue;
                    };
                    let period = if let Some(pc) = period_col {
                        let text = row.get(pc).map(|c| c.raw_text()).unwrap_or_default();
                        match dsheng_core::analysis::periods::parse_calendar_date(&text) {
                            Ok(d) => DateRange::new(d, d).unwrap_or(window),
                            Err(_) => window,
                        }
                    } else {
                        trend_period_cols
                            .iter()
                            .find(|(c, _)| *c == col)
                            .map(|(_, r)| *r)
                            .unwrap_or(window)
                    };
                    let evidence_id = format!(
                        "cell:{}:{}:{}{}",
                        file.file_label,
                        sheet.sheet.clone().unwrap_or_default(),
                        dsheng_core::import::headers::column_letter(col),
                        row_idx + 1
                    );
                    let cell = row.get(col);
                    let (value, source_label) = match cell_value(cell) {
                        Some(v) => (v, file.file_label.clone()),
                        None if zero_fill_applies(coverage, role, def, cell, period, window) => {
                            zero_filled.push(evidence_id.clone());
                            (Decimal::ZERO, zero_fill_label(&file.file_label, coverage))
                        }
                        // 空格=missing（缺失≠0）；占位/无效文本同样不产生观察
                        None => continue,
                    };
                    obs.push(Obs {
                        metric_id: def.metric_id.clone(),
                        unit: unit_display(&def.unit_json),
                        role,
                        value,
                        period,
                        evidence_id,
                        source_label,
                        scope: scope.to_string(),
                    });
                }
            }
            // 推移表期间列的值（无 actual 字段列时；列头是期间）
            if period_col.is_none() {
                for (col, period) in &trend_period_cols {
                    for (row_idx, row) in sheet_data.rows.iter().enumerate().skip(header.row) {
                        let Some(label) = row.get(label_col).map(|c| c.raw_text()) else {
                            continue;
                        };
                        let Some(def) = match_metric(defs, &label) else {
                            continue;
                        };
                        let evidence_id = format!(
                            "cell:{}:{}:{}{}",
                            file.file_label,
                            sheet.sheet.clone().unwrap_or_default(),
                            dsheng_core::import::headers::column_letter(*col),
                            row_idx + 1
                        );
                        let cell = row.get(*col);
                        let (value, source_label) = match cell_value(cell) {
                            Some(v) => (v, file.file_label.clone()),
                            None if zero_fill_applies(
                                coverage,
                                MeasureRole::Actual,
                                def,
                                cell,
                                *period,
                                window,
                            ) =>
                            {
                                zero_filled.push(evidence_id.clone());
                                (Decimal::ZERO, zero_fill_label(&file.file_label, coverage))
                            }
                            None => continue,
                        };
                        obs.push(Obs {
                            metric_id: def.metric_id.clone(),
                            unit: unit_display(&def.unit_json),
                            role: MeasureRole::Actual,
                            value,
                            period: *period,
                            evidence_id,
                            source_label,
                            scope: scope.to_string(),
                        });
                    }
                }
            }
        }
    }
    // 手工输入（confirmed 的 actual/plan；period_key 匹配窗口）。
    // manual_inputs.metric_id 指向写入时的具体版本行；按 stable_metric_id
    // 归并到当前定义身份（指标升版本后旧输入仍参与计算，§1.4）。
    {
        let mut stmt = conn.prepare(
            "SELECT mi.input_id, md.stable_metric_id, mi.measure_role, mi.scope_key,
                    mi.value, mi.input_revision
             FROM manual_inputs mi
             JOIN metric_definitions md ON md.metric_id = mi.metric_id
             WHERE mi.workspace_id = ?1 AND mi.confirmed = 1",
        )?;
        let rows: Vec<(String, String, String, String, Option<String>, i64)> = stmt
            .query_map(params![workspace_id.to_string()], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })?
            .collect::<Result<_, _>>()
            .map_err(CliError::from)?;
        for (input_id, metric_id, role, scope_key, value, revision) in rows {
            let Some(def) = defs.iter().find(|d| d.metric_id == metric_id) else {
                continue;
            };
            let Some(raw) = value else { continue };
            let Ok(value) = dsheng_core::contracts::decimal::canonical_decimal(&raw) else {
                continue;
            };
            let role = match role.as_str() {
                "actual" => MeasureRole::Actual,
                "plan" => MeasureRole::Plan,
                _ => MeasureRole::AnnualMonthPlan,
            };
            obs.push(Obs {
                metric_id: def.metric_id.clone(),
                unit: unit_display(&def.unit_json),
                role,
                value,
                period: window,
                evidence_id: format!("manual:{input_id}:r{revision}"),
                source_label: format!("用户手工输入（第{revision}版）"),
                scope: scope_key,
            });
        }
    }
    // 同 metric+role+period+scope 多值：同值合并证据；不同值 → 冲突草稿
    let mut merged: BTreeMap<String, Vec<Obs>> = BTreeMap::new();
    for o in obs.drain(..) {
        let key = format!(
            "{}|{:?}|{}|{}|{}",
            o.metric_id,
            o.role,
            o.period.to_period_string(),
            o.scope,
            o.unit
        );
        merged.entry(key).or_default().push(o);
    }
    let mut final_obs = Vec::new();
    let mut conflicts = Vec::new();
    for (_, group) in merged {
        let distinct: Vec<Decimal> = {
            let mut v: Vec<Decimal> = group.iter().map(|o| o.value).collect();
            v.sort();
            v.dedup();
            v
        };
        if distinct.len() <= 1 {
            final_obs.push(group.into_iter().next().expect("非空组"));
        } else {
            conflicts.push(ConflictDraft {
                metric_id: group[0].metric_id.clone(),
                role: group[0].role,
                period: group[0].period,
                scope_key: group[0].scope.clone(),
                unit: group[0].unit.clone(),
                candidates: group
                    .iter()
                    .map(|o| (o.value, o.evidence_id.clone(), o.source_label.clone()))
                    .collect(),
            });
        }
    }
    Ok((
        final_obs,
        conflicts,
        sources,
        mapping_rows,
        pending_metrics.len(),
        zero_filled,
    ))
}

fn obs_in<'a>(
    obs: &'a [Obs],
    metric_id: &str,
    role: MeasureRole,
    range: DateRange,
) -> Vec<&'a Obs> {
    obs.iter()
        .filter(|o| {
            o.metric_id == metric_id
                && matches!(
                    (o.role, role),
                    (MeasureRole::Actual, MeasureRole::Actual)
                        | (MeasureRole::Plan, MeasureRole::Plan)
                        | (MeasureRole::AnnualMonthPlan, MeasureRole::AnnualMonthPlan)
                )
                // 期间相交（观察期 ⊆ 窗口或覆盖窗口）
                && o.period.from <= range.to
                && o.period.to >= range.from
        })
        .collect()
}

fn flow_sum_window(obs: &[&Obs]) -> Option<Decimal> {
    if obs.is_empty() {
        return None;
    }
    // 单条完整窗口观察直接采用（期间总值）；多条 disjoint 明细求和（core）。
    if obs.len() == 1 {
        return Some(obs[0].value);
    }
    let values: Vec<Decimal> = obs.iter().map(|o| o.value).collect();
    dsheng_core::analysis::engine::flow_sum(&values, true).ok()
}

fn comparison_view(
    comparison_type: &str,
    label: String,
    base: DateRange,
    base_value: Option<Decimal>,
    current: Option<Decimal>,
    evidence_ids: Vec<String>,
    base_refs: Vec<Value>,
) -> Value {
    let (availability, reason, absolute, rate) = match (current, base_value) {
        (Some(c), Some(b)) => {
            let outcome = dsheng_core::analysis::engine::compute_comparison(c, b);
            let availability = if outcome.reason.is_none() {
                "available"
            } else if outcome.reason.is_some_and(|r| {
                matches!(
                    r,
                    dsheng_core::contracts::domain::ComparisonReason::ZeroBase
                        | dsheng_core::contracts::domain::ComparisonReason::NegativeBase
                )
            }) {
                "partial"
            } else {
                "unavailable"
            };
            (
                availability,
                outcome.reason.map(|r| {
                    serde_json::to_value(r)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string()
                }),
                Some(outcome.absolute_change),
                outcome.change_rate,
            )
        }
        _ => (
            "unavailable",
            Some(if base_value.is_none() {
                "missing_base".into()
            } else {
                "missing_current".into()
            }),
            None,
            None,
        ),
    };
    json!({
        "comparison_type": comparison_type,
        "label": label,
        "base_from": base.from.format("%Y-%m-%d").to_string(),
        "base_to": base.to.format("%Y-%m-%d").to_string(),
        "base_value": base_value.map(to_canonical_string),
        "absolute_change": absolute.map(to_canonical_string),
        "change_rate": rate.map(to_canonical_string),
        "availability_status": availability,
        "reason": reason,
        "base_refs": base_refs,
        "evidence_ids": evidence_ids,
    })
}

/// run 的 period 从 period_key 解析（build_period_key 固定格式）。
fn period_of_key(key: &str) -> Option<DateRange> {
    let from = key.split("from=").nth(1)?.split('|').next()?;
    let to = key.split("to=").nth(1)?.split('|').next()?;
    let f = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
    let t = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
    DateRange::new(f, t).ok()
}

fn base_refs_for(conn: &Connection, workspace_id: Uuid, base: DateRange) -> CliResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT run_id, current_revision_no, period_key FROM analysis_runs WHERE workspace_id = ?1",
    )?;
    let rows: Vec<(String, i64, String)> = stmt
        .query_map(params![workspace_id.to_string()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()
        .map_err(CliError::from)?;
    let mut out = Vec::new();
    for (run_id, revision, key) in rows {
        if let Some(range) = period_of_key(&key) {
            if range.from == base.from && range.to == base.to {
                out.push(json!({"run_id": run_id, "revision_no": revision}));
            }
        }
    }
    Ok(out)
}

/// analyze 主入口（cli-surface 21：本地计算；保留历史快照）。
#[allow(clippy::too_many_arguments)]
pub fn run_analyze(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
    from: &str,
    to: &str,
    grain_auto: bool,
    grain_explicit: Option<String>,
    policy_explicit: Option<String>,
    _recalculate: bool,
    extra: Option<AnalyzeExtraInput>,
) -> CliResult<Value> {
    let f = parse_date(from, "--from")?;
    let t = parse_date(to, "--to")?;
    let window =
        DateRange::new(f, t).map_err(|e| CliError::new(CliErrorCode::DataValidationFailed, e))?;
    // 覆盖决策先校验（不合法时不扫描、不落库）
    let coverage = CoverageDecision::from_extra(extra.as_ref())?;
    // 策略：跨界且未显式选择 → POLICY_SELECTION_REQUIRED（core 规则）
    let (policy_version, _override) =
        dsheng_core::repository::analysis_runs::resolve_policy_for_range(
            conn,
            &workspace_id.to_string(),
            window,
            policy_explicit.as_deref(),
        )
        .map_err(CliError::from)?;
    let (week_start, fiscal, tz, cadence): (u32, u32, String, String) = conn
        .query_row(
            "SELECT week_start_day, fiscal_year_start_month, timezone_id, cadence
             FROM period_policies WHERE workspace_id = ?1 AND version = ?2",
            params![workspace_id.to_string(), policy_version],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::ContextInvalid, "策略版本不存在"))?;
    let cadence = cadence_of_grain(&cadence);
    let grain_name = if grain_auto {
        serde_json::to_value(cadence)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string()
    } else {
        grain_explicit.unwrap_or_else(|| "month".into())
    };
    let grain: Granularity = serde_json::from_value(json!(grain_name))
        .map_err(|_| CliError::invalid("grain 取值非法"))?;
    let policy = CalendarPolicy {
        week_start_day: week_start,
        fiscal_year_start_month: fiscal,
        timezone_id: tz.clone(),
    };
    let scope = extra
        .as_ref()
        .and_then(|e| e.scope.clone())
        .unwrap_or_else(|| "全公司".into());
    let target_period: Option<DateRange> = extra
        .as_ref()
        .and_then(|e| e.target_period.clone())
        .and_then(|v| {
            let from = v.get("from")?.as_str()?;
            let to = v.get("to")?.as_str()?;
            let f = NaiveDate::parse_from_str(from, "%Y-%m-%d").ok()?;
            let t = NaiveDate::parse_from_str(to, "%Y-%m-%d").ok()?;
            DateRange::new(f, t).ok()
        });

    // 扫描（注册来源）＋观察提取
    crate::store::sources::ensure_sidecar_tables(conn)?;
    let (files, _pending_files) = scan_directory(dir)?;
    let registered = crate::store::sources::register_sources(conn, workspace_id, &files)?;
    let defs = load_metric_defs(conn, workspace_id)?;
    let (obs, conflicts, mut sources, mapping_rows, pending_count, zero_filled) =
        extract_observations(conn, workspace_id, &defs, &files, window, &scope, &coverage)?;
    // source_id 回填（注册视图含真实 source_id）
    for sel in sources.iter_mut() {
        if let Some(view) = registered
            .iter()
            .find(|v| v["file_label"].as_str() == Some(sel.source_id.as_str()))
        {
            sel.source_id = view["source_id"].as_str().unwrap_or_default().to_string();
        }
    }

    // 指标行/序列
    let mut metrics_rows = Vec::new();
    let mut series_rows = Vec::new();
    let mut comparisons_obj = serde_json::Map::new();
    let mut base_refs_used: Vec<BaseRef> = Vec::new();
    for def in &defs {
        let relevant: Vec<&Obs> = obs
            .iter()
            .filter(|o| o.metric_id == def.metric_id)
            .collect();
        if relevant.is_empty() {
            continue;
        }
        let actual = flow_sum_window(&obs_in(&obs, &def.metric_id, MeasureRole::Actual, window));
        let explicit = flow_sum_window(&obs_in(&obs, &def.metric_id, MeasureRole::Plan, window));
        let annual = flow_sum_window(&obs_in(
            &obs,
            &def.metric_id,
            MeasureRole::AnnualMonthPlan,
            window,
        ));
        // 目标政策（core targets）
        let target = if def.target_constraint_scope == "monthly_revenue_floor"
            && def.revenue_semantics
            && def.direction == "up"
        {
            let yoy_range = yoy_base(window, grain, &policy).ok().map(|b| b.range);
            let prior_year = yoy_range.and_then(|r| {
                flow_sum_window(&obs_in(&obs, &def.metric_id, MeasureRole::Actual, r))
            });
            let outcome = dsheng_core::analysis::targets::monthly_revenue_floor(
                &dsheng_core::analysis::targets::MonthlyFloorInput {
                    explicit_target: explicit,
                    comparable_annual_month_plan: annual,
                    comparable_prior_year_actual: prior_year,
                    incomparable_values: vec![],
                },
                dsheng_core::contracts::metrics::TargetConstraintScope::MonthlyRevenueFloor,
                dsheng_core::analysis::targets::monthly_revenue_floor_eligible(
                    def.revenue_semantics,
                    parse_direction(&def.direction),
                    Granularity::Month,
                ) && grain == Granularity::Month,
            );
            outcome.target
        } else {
            dsheng_core::analysis::targets::explicit_target_only(explicit).ok()
        };
        let delta = match (actual, target) {
            (Some(a), Some(t)) => Some(a - t),
            _ => None,
        };
        // mom/yoy
        let mut mom = None;
        let mut yoy = None;
        for (ctype, base_result, label) in [
            (
                "mom",
                mom_base(window, grain, &policy),
                "上一完整自然月".to_string(),
            ),
            (
                "yoy",
                yoy_base(window, grain, &policy),
                "上一年同月".to_string(),
            ),
        ] {
            let Ok(base_period) = base_result else {
                continue;
            };
            let base_value = flow_sum_window(&obs_in(
                &obs,
                &def.metric_id,
                MeasureRole::Actual,
                base_period.range,
            ));
            let refs = base_refs_for(conn, workspace_id, base_period.range)?;
            let custom_label = match ctype {
                "yoy" if grain == Granularity::Week => {
                    dsheng_core::analysis::periods::FIFTY_TWO_WEEK_LABEL.to_string()
                }
                _ => label,
            };
            let view = comparison_view(
                ctype,
                custom_label,
                base_period.range,
                base_value,
                actual,
                relevant
                    .iter()
                    .filter(|o| o.role == MeasureRole::Actual)
                    .map(|o| o.evidence_id.clone())
                    .collect(),
                refs.clone(),
            );
            if ctype == "mom" {
                mom = Some(view.clone());
            } else {
                yoy = Some(view.clone());
            }
            comparisons_obj.insert(format!("{}_{}", def.metric_key, ctype), view);
            for r in refs {
                let Some(run_id) = r["run_id"].as_str() else {
                    continue;
                };
                let Some(rev) = r["revision_no"].as_u64() else {
                    continue;
                };
                let b = BaseRef {
                    run_id: run_id.to_string(),
                    revision_no: rev as u32,
                };
                if !base_refs_used.contains(&b) {
                    base_refs_used.push(b);
                }
            }
        }
        metrics_rows.push(json!({
            "metric_id": def.metric_id,
            "metric_key": def.metric_key,
            "metric_label": def.label,
            "unit": unit_display(&def.unit_json),
            "direction": def.direction,
            "target_value": target.map(to_canonical_string),
            "actual_value": actual.map(to_canonical_string),
            "delta": delta.map(to_canonical_string),
            "mom": mom,
            "yoy": yoy,
        }));
        // 序列：全部期间桶（升序），actual/plan 对齐
        let mut buckets: BTreeMap<String, (Option<Decimal>, Option<Decimal>)> = BTreeMap::new();
        for o in relevant.iter() {
            let key = o.period.to_period_string();
            let slot = buckets.entry(key).or_insert((None, None));
            match o.role {
                MeasureRole::Actual => {
                    slot.0 = Some(match slot.0 {
                        Some(v) => v + o.value,
                        None => o.value,
                    })
                }
                MeasureRole::Plan => {
                    slot.1 = Some(match slot.1 {
                        Some(v) => v + o.value,
                        None => o.value,
                    })
                }
                MeasureRole::AnnualMonthPlan => {}
            }
        }
        if !buckets.is_empty() {
            series_rows.push(json!({
                "metric_key": def.metric_key,
                "metric_label": def.label,
                "unit": unit_display(&def.unit_json),
                "buckets": buckets.keys().cloned().collect::<Vec<_>>(),
                "actual": buckets.values().map(|(a, _)| a.map(to_canonical_string)).collect::<Vec<_>>(),
                "plan": buckets.values().map(|(_, p)| p.map(to_canonical_string)).collect::<Vec<_>>(),
                "delta": Value::Null,
                "composition": Value::Null,
            }));
        }
    }
    let normalized = json!({
        "grain": grain_name,
        "period": {"from": window.from.format("%Y-%m-%d").to_string(), "to": window.to.format("%Y-%m-%d").to_string()},
        "metrics": metrics_rows,
        "series": series_rows,
        "pending_inputs": pending_count,
        "scope": scope,
    });
    let comparisons = Value::Object(comparisons_obj);

    // 手工输入快照
    let manual_snapshots: Vec<ManualInputSnapshot> = {
        let mut stmt = conn.prepare(
            "SELECT input_id, metric_id, input_revision, value FROM manual_inputs
             WHERE workspace_id = ?1",
        )?;
        let rows: Vec<(String, String, i64, Option<String>)> = stmt
            .query_map(params![workspace_id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })?
            .collect::<Result<_, _>>()
            .map_err(CliError::from)?;
        rows.into_iter()
            .map(
                |(input_id, metric_id, revision, value)| ManualInputSnapshot {
                    input_id,
                    metric_id,
                    input_revision: revision as u32,
                    value,
                },
            )
            .collect()
    };
    let input = AnalysisRevisionInput {
        sources,
        mapping_version: "auto-v1".to_string(),
        mappings: json!(mapping_rows),
        metric_definitions: defs
            .iter()
            .map(|d| MetricDefSnapshot {
                metric_key: d.metric_key.clone(),
                version: d.version as u32,
                scope: d.scope.clone(),
                semantic_hash: d.semantic_hash.clone(),
            })
            .collect(),
        manual_inputs: manual_snapshots,
        conflict_resolutions: vec![],
        period_policy: json!({
            "version": policy_version,
            "week_start_day": week_start,
            "fiscal_year_start_month": fiscal,
            "timezone_id": tz,
            // §2 coverage 快照（默认 unknown/empty_days_are_zero=false：缺失≠0）
            "coverage": coverage.snapshot(window, grain, &zero_filled),
        }),
        scope,
        current_period: window,
        target_period,
        normalized_metrics: normalized.clone(),
        comparisons: comparisons.clone(),
        comparison_base_refs: base_refs_used,
    };
    let period_key = dsheng_core::repository::analysis_runs::build_period_key(
        &policy_version,
        grain,
        window,
        &input.scope,
    );
    let upsert = dsheng_core::repository::analysis_runs::upsert_analysis_revision(
        conn,
        &workspace_id.to_string(),
        &period_key,
        &dsheng_core::repository::analysis_runs::RevisionPayload {
            input,
            action_period_snapshot: target_period,
        },
        chrono::Utc::now(),
    )
    .map_err(CliError::from)?;
    // 冲突落库（run 建立后）
    for draft in conflicts {
        conn.execute(
            "INSERT OR IGNORE INTO source_conflicts (conflict_id, workspace_id, run_id, metric_id,
                    measure_role, period_from, period_to, scope_key, unit, candidate_values_json,
                    resolution_status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'unresolved', ?11)",
            params![
                Uuid::new_v4().to_string(),
                workspace_id.to_string(),
                upsert.run_id,
                draft.metric_id,
                serde_json::to_value(draft.role).unwrap().as_str().unwrap(),
                draft.period.from.format("%Y-%m-%d").to_string(),
                draft.period.to.format("%Y-%m-%d").to_string(),
                draft.scope_key,
                draft.unit,
                serde_json::to_string(
                    &draft
                        .candidates
                        .iter()
                        .map(|(v, e, s)| json!({
                            "value": v.to_string(),
                            "evidence_id": e,
                            "source_label": s,
                        }))
                        .collect::<Vec<_>>()
                )
                .unwrap(),
                dsheng_core::repository::utc_iso(chrono::Utc::now())
            ],
        )
        .map_err(CliError::from)?;
    }
    let run_id = upsert.run_id.clone();
    analysis_run_view(conn, workspace_id, &run_id, upsert.revision_no)
}

/// analysisRunView（从 run + revision 快照渲染）。
pub fn analysis_run_view(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: u32,
) -> CliResult<Value> {
    let (current, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let normalized_raw: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| r.get(0),
        )
        .map_err(|_| {
            CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在")
        })?;
    let normalized: Value = serde_json::from_str(&normalized_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    let period = normalized
        .get("period")
        .cloned()
        .unwrap_or_else(|| json!({"from": null, "to": null}));
    Ok(json!({
        "workspace_id": workspace_id.to_string(),
        "run_id": run_id,
        "period": period,
        "current_revision_no": current,
        "analysis_input_fingerprint": fingerprint,
        "grain": normalized.get("grain").cloned().unwrap_or(json!("month")),
        "metrics": normalized.get("metrics").cloned().unwrap_or(json!([])),
        "series": normalized.get("series").cloned().unwrap_or(json!([])),
        "pending_inputs": normalized.get("pending_inputs").cloned().unwrap_or(json!(0)),
    }))
}

// ---- history 读侧 ----

pub fn history_list(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT r.run_id, r.workspace_id, r.period_key, r.current_revision_no, r.updated_at,
                (SELECT normalized_metrics_json FROM run_revisions rr
                 WHERE rr.run_id = r.run_id AND rr.revision_no = r.current_revision_no),
                (SELECT COUNT(*) FROM task_period_links l
                 WHERE l.run_id = r.run_id AND l.analysis_revision_no = r.current_revision_no),
                (SELECT COUNT(*) FROM run_task_snapshots s WHERE s.run_id = r.run_id),
                (SELECT COUNT(*) FROM run_task_snapshots s WHERE s.run_id = r.run_id) AS chart_count
         FROM analysis_runs r WHERE r.workspace_id = ?1 ORDER BY r.updated_at DESC",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, i64>(7)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (run_id, ws, period_key, current, updated, normalized_raw, task_count, snapshot_count) =
            row?;
        let normalized: Value = normalized_raw
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_else(|| json!({}));
        let metric_count = normalized
            .get("metrics")
            .and_then(|m| m.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let period = period_of_key(&period_key)
            .map(|p| {
                json!({
                    "from": p.from.format("%Y-%m-%d").to_string(),
                    "to": p.to.format("%Y-%m-%d").to_string(),
                })
            })
            .unwrap_or_else(|| json!({"from": null, "to": null}));
        let grain = normalized.get("grain").cloned().unwrap_or(json!("month"));
        out.push(json!({
            "run_id": run_id,
            "workspace_id": ws,
            "period": period,
            "period_key": period_key,
            "cadence": grain,
            "policy_version": 1,
            "current_revision_no": current,
            "updated_at": updated,
            "summary": {
                "metric_count": metric_count,
                "task_count": task_count,
                "chart_count": 0,
            },
            "task_snapshot_no": snapshot_count,
        }));
    }
    Ok(out)
}

pub fn history_revisions(conn: &Connection, workspace_id: Uuid, run_id: &str) -> CliResult<Value> {
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let nums = dsheng_core::repository::analysis_runs::revision_numbers(conn, run_id)
        .map_err(CliError::from)?;
    let mut items = Vec::new();
    for n in nums {
        let created: String = conn
            .query_row(
                "SELECT created_at FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
                params![run_id, n as i64],
                |r| r.get(0),
            )
            .map_err(CliError::from)?;
        items.push(json!({"revision_no": n, "created_at": created}));
    }
    Ok(json!({"run_id": run_id, "revisions": items}))
}

/// historyDetailView（revision 快照 + 任务快照）。
pub fn history_detail(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: Option<u32>,
) -> CliResult<Value> {
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let (current, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let revision = revision.unwrap_or(current);
    let (normalized_raw, captured_at): (String, String) = conn
        .query_row(
            "SELECT normalized_metrics_json, created_at FROM run_revisions
             WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在"))?;
    let normalized: Value = serde_json::from_str(&normalized_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    // 任务快照（该 revision 最新一版）
    let snapshot: Option<(i64, String)> = conn
        .query_row(
            "SELECT task_snapshot_no, task_snapshot_json FROM run_task_snapshots
             WHERE run_id = ?1 AND analysis_revision_no = ?2
             ORDER BY task_snapshot_no DESC LIMIT 1",
            params![run_id, revision as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let (snapshot_no, tasks) = match snapshot {
        Some((no, raw)) => (
            no,
            serde_json::from_str::<Value>(&raw)
                .ok()
                .and_then(|v| v.get("tasks").cloned())
                .unwrap_or_else(|| json!([])),
        ),
        None => (0, json!([])),
    };
    let _ = fingerprint;
    Ok(json!({
        "run_id": run_id,
        "analysis_revision_no": revision,
        "task_snapshot_no": snapshot_no,
        "captured_at": captured_at,
        "metrics": normalized.get("metrics").cloned().unwrap_or(json!([])),
        "series": normalized.get("series").cloned().unwrap_or(json!([])),
        "grain": normalized.get("grain").cloned().unwrap_or(json!("month")),
        "period": normalized.get("period").cloned().unwrap_or(json!({"from": null, "to": null})),
        "tasks": tasks,
    }))
}

/// history compare：revision 的比较快照按类型过滤。
pub fn history_compare(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: Option<u32>,
    compare_type: &str,
) -> CliResult<Value> {
    let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let revision = revision.unwrap_or(current);
    let comparisons_raw: String = conn
        .query_row(
            "SELECT comparisons_json FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| r.get(0),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在"))?;
    let comparisons: Value = serde_json::from_str(&comparisons_raw).unwrap_or_else(|_| json!({}));
    let mut items = Vec::new();
    if let Some(map) = comparisons.as_object() {
        for (key, value) in map {
            if key.ends_with(compare_type) {
                items.push(value.clone());
            }
        }
    }
    Ok(json!({
        "run_id": run_id,
        "analysis_revision_no": revision,
        "comparison_type": compare_type,
        "comparisons": items,
    }))
}
