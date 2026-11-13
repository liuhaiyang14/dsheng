//! analyze 编排（本地计算，无 Provider）与读侧视图（analysis show / history）。
//!
//! 计算全部复用 dsheng-core：`import::*` 识别/解析、`analysis::engine` 求和与
//! 比较、`analysis::periods` 基期/覆盖、`analysis::targets` 目标政策、
//! `analysis::fingerprint` 输入指纹、`repository::analysis_runs` 不可变 revision。
//! 本层把扫描观察组织成指标行/序列视图，并以 JSON 快照存入 run_revisions
//! （normalized_metrics_json / comparisons_json；形状即 read-views 字段名）。

use crate::error::{CliError, CliResult};
use crate::store::sources::{
    mapping_draft_for_period, register_sources_with_profile,
    FieldMappingEntry,
    ScannedFile, SourceProfileBinding,
};
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
use dsheng_core::import::headers::{parse_period_header, HeaderBlock};
use rusqlite::{params, Connection};
use rust_decimal::Decimal;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
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
    /// 累计范围观察（§2.5 record_kind=cumulative_total；转置推移表 `累计` 行）：
    /// 独立范围汇总，不与月行重复相加（不参与窗口求和）。
    cumulative: bool,
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
    formula: Option<dsheng_core::contracts::metrics::FormulaNode>,
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
                m.unit_signature_json, m.semantic_type, m.formula_json
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
            formula: r.get::<_, Option<String>>(11)?.and_then(|raw| serde_json::from_str(&raw).ok()),
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

pub(super) fn observation_period(text: &str) -> Option<DateRange> {
    dsheng_core::analysis::periods::parse_calendar_date(text)
        .ok()
        .map(|date| DateRange {
            from: date,
            to: date,
        })
        .or_else(|| parse_period_header(text, None))
}

pub(super) fn cell_value(cell: Option<&CellValue>) -> Option<Decimal> {
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

fn period_str(range: &DateRange) -> String {
    format!(
        "{}/{}",
        range.from.format("%Y-%m-%d"),
        range.to.format("%Y-%m-%d")
    )
}

/// 扫描布局快照（cli_source_sheet_layouts.layout_json；生成载荷相关行/列
/// 裁剪的输入）。字段推导与 extract_observations 同源：表头块/标签列/期间列/
/// 推移表期间列/转置布局。无表头且非转置 → `{}`（该 sheet 仅在证据锚点
/// 指向时入选）。
pub fn sheet_layout_snapshot(
    sheet_data: &dsheng_core::import::fileio::SheetData,
    scanned: &crate::store::sources::ScannedSheet,
) -> Value {
    if let Some(layout) = scanned
        .match_info
        .as_ref()
        .and_then(|m| m.transposed.as_ref())
    {
        return json!({
            "transposed": {
                "header_row": layout.header_row,
                "metric_columns": layout
                    .metric_columns
                    .iter()
                    .map(|(c, n)| json!([c, n]))
                    .collect::<Vec<_>>(),
                "period_rows": layout
                    .period_rows
                    .iter()
                    .map(|(r, p)| json!([r, period_str(p)]))
                    .collect::<Vec<_>>(),
                "cumulative_rows": layout
                    .cumulative_rows
                    .iter()
                    .map(|(r, p)| json!([r, period_str(p)]))
                    .collect::<Vec<_>>(),
            }
        });
    }
    let Some(header) = &scanned.header else {
        return json!({});
    };
    let col_of = |field: &str| {
        header
            .columns
            .iter()
            .find(|(_, f)| *f == field)
            .map(|(c, _)| *c)
    };
    let period_col = col_of("period");
    // 推移表：无 period 列时，表头块行内可解析为期间的未映射列即期间列
    //（与 extract_observations 相同推导；§4.1/§4.3）。
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
    let mut columns = serde_json::Map::new();
    for (col, field) in &header.columns {
        columns.insert(field.to_string(), json!(col));
    }
    json!({
        "header": {
            "row": header.row,
            "row_count": header.row_count,
            "columns": columns,
        },
        "label_col": col_of("metric_label").or_else(|| col_of("task_title")),
        "period_col": period_col,
        "trend_period_cols": trend_period_cols
            .iter()
            .map(|(c, p)| json!([c, period_str(p)]))
            .collect::<Vec<_>>(),
    })
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

fn mapping_column_index(raw: &str) -> Option<usize> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(n) = text.parse::<usize>() {
        return n.checked_sub(1);
    }
    let mut col = 0usize;
    for ch in text.chars() {
        if !ch.is_ascii_alphabetic() {
            return None;
        }
        col = col * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    col.checked_sub(1)
}

fn generic_header_signature(
    sheet: &dsheng_core::import::fileio::SheetData,
    header: Option<&HeaderBlock>,
) -> String {
    let (start, count) = header
        .map(|h| (h.row.saturating_sub(1), h.row_count))
        .unwrap_or((0, 1));
    let mut parts = Vec::new();
    for row in sheet.rows.iter().skip(start).take(count) {
        parts.push(
            row.iter()
                .map(|c| c.raw_text())
                .collect::<Vec<_>>()
                .join("\u{1f}"),
        );
    }
    let raw = parts.join("\u{1e}");
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(raw.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn mapping_targets_for_sheet(
    entries: &[FieldMappingEntry],
    file: &ScannedFile,
    sheet: &crate::store::sources::ScannedSheet,
    data: &dsheng_core::import::fileio::SheetData,
) -> Vec<(usize, String, String)> {
    let signature = generic_header_signature(data, sheet.header.as_ref());
    entries
        .iter()
        .filter_map(|entry| {
            let source_match = entry
                .source_id
                .as_deref()
                .or(entry.source.as_deref())
                .map(|s| s == file.file_label)
                .unwrap_or(true);
            let sheet_match = entry
                .sheet
                .as_deref()
                .map(|s| Some(s) == sheet.sheet.as_deref())
                .unwrap_or(true);
            if !source_match || !sheet_match {
                return None;
            }
            if let Some(expected) = entry.header_signature.as_deref() {
                let expected = expected.trim();
                if !expected.is_empty() && expected != signature {
                    return None;
                }
            }
            let col = entry
                .column
                .as_deref()
                .or(Some(entry.recognized.as_str()))
                .and_then(mapping_column_index)?;
            let role = entry
                .target_role
                .as_deref()
                .unwrap_or(&entry.tool_field)
                .trim()
                .to_string();
            if role.is_empty() {
                return None;
            }
            Some((col, role, entry.source_label.clone()))
        })
        .collect()
}

/// 按 source profile 的业务明细映射抽取实际值。业务导出文件通常没有
/// “科目/实际值”这种规范表头，而是一个文件对应一个或多个指标；映射只描述
/// 文件/工作表/日期/金额列，数值聚合仍由本地 Decimal 引擎完成。
fn extract_profile_records(
    profile: &SourceProfileBinding,
    file: &ScannedFile,
    sheet: &crate::store::sources::ScannedSheet,
    data: &dsheng_core::import::fileio::SheetData,
    defs: &[MetricDefRow],
    _window: DateRange,
    scope: &str,
    obs: &mut Vec<Obs>,
    pending_reasons: &mut BTreeMap<String, Vec<String>>,
    mapping_rows: &mut Vec<Value>,
) {
    let Some(config) = profile.config.as_ref() else { return };
    let Some(mappings) = config.get("record_mappings").and_then(Value::as_array) else { return };
    let file_lower = file.file_label.to_lowercase();
    let sheet_lower = sheet.sheet.as_deref().unwrap_or_default().to_lowercase();
    for mapping in mappings {
        let file_match = mapping.get("file_matchers").and_then(Value::as_array)
            .map(|xs| xs.is_empty() || xs.iter().filter_map(Value::as_str).any(|m| file_lower.contains(&m.to_lowercase())))
            .unwrap_or(false);
        let sheet_match = mapping.get("sheet_matchers").and_then(Value::as_array)
            .map(|xs| xs.is_empty() || xs.iter().filter_map(Value::as_str).any(|m| sheet_lower.contains(&m.to_lowercase())))
            .unwrap_or(false);
        if !file_match || !sheet_match { continue; }
        let Some(metric_key) = mapping.get("metric_key").and_then(Value::as_str) else { continue; };
        let Some(def) = defs.iter().find(|d| d.metric_key == metric_key) else { continue; };
        let value_fields: Vec<String> = mapping.get("value_fields").and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let date_fields: Vec<String> = mapping.get("date_fields").and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let detail_key_fields: Vec<String> = config
            .get("row_rules")
            .and_then(|rules| rules.get("detail_key_fields"))
            .and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let same_content_dedupe = config
            .get("dedupe")
            .and_then(|dedupe| dedupe.get("same_content"))
            .and_then(Value::as_str)
            == Some("dedupe");
        if value_fields.is_empty() || date_fields.is_empty() { continue; }
        let max_header = config.get("row_rules").and_then(|r| r.get("header_rows_max")).and_then(Value::as_u64).unwrap_or(20) as usize;
        let norm = |v: &str| dsheng_core::import::headers::normalize_header(v);
        let mut header_row = None;
        let mut field_cols = BTreeMap::<String, usize>::new();
        for (ri, row) in data.rows.iter().take(max_header).enumerate() {
            let mut found = BTreeMap::new();
            for (ci, cell) in row.iter().enumerate() {
                let raw = norm(&cell.raw_text());
                for field in value_fields.iter().chain(date_fields.iter()).chain(detail_key_fields.iter()) {
                    if raw == norm(field) { found.insert(field.clone(), ci); }
                }
                if let Some(status) = mapping.get("status_field").and_then(Value::as_str) {
                    if raw == norm(status) { found.insert(status.to_string(), ci); }
                }
                if let Some(scope_field) = mapping.get("scope_field").and_then(Value::as_str) {
                    if raw == norm(scope_field) { found.insert(scope_field.to_string(), ci); }
                }
            }
            if found.len() >= 2 { header_row = Some(ri); field_cols = found; break; }
        }
        let Some(header_row) = header_row else { continue; };
        let date_kind = mapping.get("period_kind").and_then(Value::as_str).unwrap_or("date");
        let status_col = mapping.get("status_field").and_then(Value::as_str).and_then(|f| field_cols.get(f)).copied();
        let accepted: Vec<String> = mapping.get("accepted_statuses").and_then(Value::as_array)
            .map(|xs| xs.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let requires_confirmation = mapping.get("requires_confirmation").and_then(Value::as_bool) == Some(true);
        let scope_col = mapping.get("scope_field").and_then(Value::as_str).and_then(|f| field_cols.get(f)).copied();
        let mut count = 0usize;
        let mut seen_detail_content = BTreeMap::<String, String>::new();
        for (row_idx, row) in data.rows.iter().enumerate().skip(header_row + 1) {
            if let Some(col) = status_col {
                let status = row.get(col).map(|c| c.raw_text()).unwrap_or_default();
                if !accepted.is_empty() {
                    if status.trim().is_empty() {
                        pending_reasons.entry(metric_key.to_string()).or_default().push("status_missing_or_unconfirmed".into());
                        continue;
                    }
                    if !accepted.iter().any(|v| v == status.trim()) {
                        pending_reasons.entry(metric_key.to_string()).or_default().push("status_invalid_or_unconfirmed".into());
                        continue;
                    }
                }
            }
            let value = value_fields.iter().filter_map(|field| field_cols.get(field).and_then(|c| row.get(*c)).and_then(|c| cell_value(Some(c)))).next();
            let Some(value) = value else { continue; };
            let Some(period) = (if date_kind == "year_month" {
                let year = date_fields.first().and_then(|f| field_cols.get(f)).and_then(|c| row.get(*c)).and_then(|c| c.raw_text().trim().parse::<i32>().ok());
                let month = date_fields.get(1).and_then(|f| field_cols.get(f)).and_then(|c| row.get(*c)).and_then(|c| c.raw_text().trim().parse::<u32>().ok());
                year.zip(month).and_then(|(y,m)| chrono::NaiveDate::from_ymd_opt(y,m,1)).map(|d| DateRange { from: d, to: d })
            } else {
                date_fields.iter().filter_map(|f| field_cols.get(f).and_then(|c| row.get(*c)).and_then(|c| profile_observation_period(&c.raw_text()))).next()
            }) else {
                pending_reasons.entry(metric_key.to_string()).or_default().push("period_missing_or_invalid".into());
                continue;
            };
            let evidence_id = format!("cell:{}:{}:{}{}", file.file_label, sheet.sheet.clone().unwrap_or_default(), dsheng_core::import::headers::column_letter(value_fields.iter().find_map(|f| field_cols.get(f)).copied().unwrap_or(0)), row_idx + 1);
            let detail_key = detail_key_fields
                .iter()
                .filter_map(|field| field_cols.get(field).and_then(|col| row.get(*col)).map(|cell| cell.raw_text().trim().to_string()))
                .collect::<Vec<_>>()
                .join("\u{1f}");
            let content_key = format!(
                "{}|{}|{}",
                value_fields
                    .iter()
                    .filter_map(|field| field_cols.get(field).and_then(|col| row.get(*col)).map(|cell| cell.raw_text().trim().to_string()))
                    .collect::<Vec<_>>()
                    .join("\u{1f}"),
                date_fields
                    .iter()
                    .filter_map(|field| field_cols.get(field).and_then(|col| row.get(*col)).map(|cell| cell.raw_text().trim().to_string()))
                    .collect::<Vec<_>>()
                    .join("\u{1f}"),
                status_col
                    .and_then(|col| row.get(col))
                    .map(|cell| cell.raw_text().trim().to_string())
                    .unwrap_or_default(),
            );
            if !detail_key.is_empty() {
                if same_content_dedupe
                    && seen_detail_content
                        .get(&detail_key)
                        .is_some_and(|previous| previous == &content_key)
                {
                    continue;
                }
                seen_detail_content
                    .entry(detail_key.clone())
                    .or_insert(content_key);
            }
            if requires_confirmation {
                // 费用分类/人工处理尚未确认时，保留映射证据但隔离金额，
                // 防止待确认费用进入附加值和经营收益公式。
                pending_reasons.entry(metric_key.to_string()).or_default().push("classification_confirmation_required".into());
                mapping_rows.push(json!({"mapping_id": mapping.get("mapping_id"), "metric_key": metric_key, "source_id": file.file_label, "sheet": sheet.sheet, "evidence_id": evidence_id}));
                continue;
            }
            let base_scope = scope_col.and_then(|c| row.get(c)).map(|c| c.raw_text()).filter(|v| !v.trim().is_empty()).unwrap_or_else(|| scope.to_string());
            // 同一业务明细映射下的多行是可加总的明细，不应被通用
            // “同键不同值→冲突”规则误判为重复来源；把映射/行号纳入内部
            // scope，仍保留原 scope 文本作为前缀供审计查看。
            let row_scope = format!(
                "{}|{}|{}",
                base_scope,
                mapping.get("mapping_id").and_then(Value::as_str).unwrap_or("record"),
                if detail_key.is_empty() { row_idx.to_string() } else { detail_key },
            );
            obs.push(Obs { metric_id: def.metric_id.clone(), unit: unit_display(&def.unit_json), role: MeasureRole::Actual, value, period, evidence_id: evidence_id.clone(), source_label: file.file_label.clone(), scope: row_scope, cumulative: false });
            mapping_rows.push(json!({"mapping_id": mapping.get("mapping_id"), "metric_key": metric_key, "source_id": file.file_label, "sheet": sheet.sheet, "evidence_id": evidence_id}));
            count += 1;
        }
        if requires_confirmation {
            pending_reasons.entry(metric_key.to_string()).or_default().push("classification_confirmation_required".into());
        }
        if count == 0 {
            pending_reasons.entry(metric_key.to_string()).or_default().push("source_value_missing".into());
        }
    }
}

fn profile_observation_period(raw: &str) -> Option<DateRange> {
    observation_period(raw).or_else(|| {
        let date = raw.trim().split([' ', 'T']).next()?;
        NaiveDate::parse_from_str(date, "%Y-%m-%d").ok().map(|d| DateRange { from: d, to: d })
    })
}

/// 提取 generic_table 的已确认手工映射。每个映射都保留 source/sheet/column/
/// header signature/target role，并在表头变化时失效，不会静默把新列当旧列。
fn extract_generic_sheet(
    file: &ScannedFile,
    sheet: &crate::store::sources::ScannedSheet,
    data: &dsheng_core::import::fileio::SheetData,
    mappings: &[(usize, String, String)],
    defs: &[MetricDefRow],
    window: DateRange,
    _grain: Granularity,
    scope: &str,
    coverage: &CoverageDecision,
    profile: &SourceProfileBinding,
    obs: &mut Vec<Obs>,
    zero_filled: &mut Vec<String>,
    pending_reasons: &mut BTreeMap<String, Vec<String>>,
) {
    let header_row = sheet.header.as_ref().map(|h| h.row).unwrap_or(1);
    let header_count = sheet.header.as_ref().map(|h| h.row_count).unwrap_or(1);
    let label_col = mappings
        .iter()
        .find(|(_, role, _)| matches!(role.as_str(), "metric_label" | "metric_id" | "label"))
        .map(|(c, _, _)| *c);
    let Some(label_col) = label_col else {
        return;
    };
    let key_cols: Vec<usize> = profile
        .config
        .as_ref()
        .and_then(|config| config.get("row_rules"))
        .and_then(|rules| rules.get("detail_key_fields"))
        .and_then(Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|field| mappings.iter().find(|(_, role, _)| role == field).map(|(col, _, _)| *col))
                .collect()
        })
        .unwrap_or_default();
    let dedupe_same_content = profile
        .config
        .as_ref()
        .and_then(|config| config.get("dedupe"))
        .and_then(|dedupe| dedupe.get("same_content"))
        .and_then(Value::as_str)
        == Some("dedupe");
    let mut seen_detail_keys = std::collections::BTreeSet::new();
    let period_col = mappings
        .iter()
        .find(|(_, role, _)| {
            role == "period"
                || role == "date"
                || profile
                    .config
                    .as_ref()
                    .and_then(|config| config.get("row_rules"))
                    .and_then(|rules| rules.get("date_field"))
                    .and_then(Value::as_str)
                    == Some(role.as_str())
        })
        .map(|(c, _, _)| *c);
    let amount_fields: Vec<String> = profile
        .config
        .as_ref()
        .and_then(|config| config.get("row_rules"))
        .and_then(|rules| rules.get("amount_fields"))
        .and_then(Value::as_array)
        .map(|fields| fields.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let status_col = mappings
        .iter()
        .find(|(_, role, _)| role == "status" || role == "task_status")
        .map(|(col, _, _)| *col);
    let status_map = profile
        .config
        .as_ref()
        .and_then(|config| config.get("normalization"))
        .and_then(|normalization| normalization.get("status_map"))
        .and_then(Value::as_object);
    for (row_idx, row) in data.rows.iter().enumerate().skip(header_row) {
        let label = row.get(label_col).map(|c| c.raw_text()).unwrap_or_default();
        let Some(def) = match_metric(defs, &label) else {
            continue;
        };
        if let (Some(status_col), Some(status_map)) = (status_col, status_map) {
            let raw_status = row.get(status_col).map(|cell| cell.raw_text()).unwrap_or_default();
            let Some(normalized_status) = status_map.get(raw_status.trim()).and_then(Value::as_str) else {
                pending_reasons.entry(def.metric_key.clone()).or_default().push("status_missing_or_unconfirmed".into());
                continue;
            };
            if normalized_status != "valid" {
                pending_reasons.entry(def.metric_key.clone()).or_default().push("status_invalid_or_unconfirmed".into());
                continue;
            }
        }
        if dedupe_same_content && !key_cols.is_empty() {
            let key = key_cols
                .iter()
                .map(|col| row.get(*col).map(|cell| cell.raw_text()).unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\u{1f}");
            if !key.trim_matches('\u{1f}').is_empty() && !seen_detail_keys.insert(key) {
                continue;
            }
        }
        let period = if let Some(period_col) = period_col {
            let Some(period) = row
                .get(period_col)
                .and_then(|c| observation_period(&c.raw_text()))
            else {
                // 无日期或非法日期不能猜作当前分析窗口；该行不形成观察，
                // 后续指标状态会保留 actual_missing/pending。
                continue;
            };
            period
        } else {
            // 没有期间列时，未提供可验证的日期映射，不能把数字绑定到
            // 当前窗口（尤其不能把月度汇总拆成日/周观察）。
            continue;
        };
        for (col, role, _) in mappings.iter().filter(|(_, role, _)| {
            matches!(role.as_str(), "actual" | "planned" | "plan" | "annual_month_plan")
                || amount_fields.iter().any(|field| field == role)
        }) {
            let measure_role = match role.as_str() {
                "planned" | "plan" => MeasureRole::Plan,
                "annual_month_plan" => MeasureRole::AnnualMonthPlan,
                _ => MeasureRole::Actual,
            };
            let period = if period_col.is_none() {
                let header_text = data
                    .rows
                    .iter()
                    .skip(header_row.saturating_sub(1))
                    .take(header_count)
                    .find_map(|r| r.get(*col).filter(|c| !c.raw_text().trim().is_empty()))
                    .map(|c| c.raw_text());
                let Some(period) = header_text.and_then(|t| parse_period_header(&t, None)) else {
                    continue;
                };
                period
            } else {
                period
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
                None if zero_fill_applies(coverage, measure_role, def, cell, period, window) => {
                    zero_filled.push(evidence_id.clone());
                    (Decimal::ZERO, zero_fill_label(&file.file_label, coverage))
                }
                None => continue,
            };
            obs.push(Obs {
                metric_id: def.metric_id.clone(),
                unit: unit_display(&def.unit_json),
                role: measure_role,
                value,
                period,
                evidence_id,
                source_label,
                scope: scope.to_string(),
                cumulative: false,
            });
        }
    }
}

/// 提取观察（核算表/推移表/手工输入）＋冲突草稿（同键不同值）。
#[allow(clippy::too_many_arguments)]
fn extract_observations(
    conn: &Connection,
    workspace_id: Uuid,
    defs: &[MetricDefRow],
    files: &[ScannedFile],
    window: DateRange,
    grain: Granularity,
    period_key: &str,
    scope: &str,
    coverage: &CoverageDecision,
    mapping_entries: &[FieldMappingEntry],
    profile: &SourceProfileBinding,
) -> CliResult<(Extracted, BTreeMap<String, Vec<String>>)> {
    let mut obs: Vec<Obs> = Vec::new();
    let mut pending_metrics: Vec<String> = Vec::new();
    let mut mapping_rows: Vec<Value> = Vec::new();
    let mut sources: Vec<SourceSelection> = Vec::new();
    let mut zero_filled: Vec<String> = Vec::new();
    let mut pending_reasons: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // 保留用户确认的完整定位信息，mapping 变化会纳入 analysis fingerprint。
    for entry in mapping_entries {
        mapping_rows.push(serde_json::to_value(entry).unwrap_or_else(|_| json!({})));
    }
    for file in files {
        for (sheet_data, sheet) in file.sheets.iter() {
            if sheet.role == "generated_output_input_not_allowed" {
                // 核算表/推移表/差异分析/图表是输出层，绝不回灌观察。
                continue;
            }
            // 生成结果在任何 profile 映射之前就被拒绝；否则一个 profile
            // 恰好匹配“核算表/推移表”字段时会绕过扫描层的禁止回灌规则。
            extract_profile_records(profile, file, sheet, sheet_data, defs, window, scope, &mut obs, &mut pending_reasons, &mut mapping_rows);
            if sheet.role == "generic_table" {
                if profile.mode == "profile" && sheet.profile_role_id.is_none() {
                    // 已选择企业 profile 时，未命中任何 file/sheet role 的表只保留
                    // 在扫描证据中，不得混入 normalized observations。
                    continue;
                }
                let mut mapped =
                    mapping_targets_for_sheet(mapping_entries, file, sheet, sheet_data);
                // profile/generic source with a recognized header can use the stable
                // standard field names directly. Explicit manual mapping still wins
                // and is required when the header is unknown or ambiguous.
                if mapped.is_empty() {
                    if let Some(header) = sheet.header.as_ref() {
                        mapped = header
                            .columns
                            .iter()
                            .map(|(column, field)| {
                                (*column, (*field).to_string(), (*field).to_string())
                            })
                            .collect();
                        let header_signature = generic_header_signature(sheet_data, Some(header));
                        for (column, role, source_label) in &mapped {
                            mapping_rows.push(json!({
                                "tool_field": role,
                                "source_label": source_label,
                                "recognized": dsheng_core::import::headers::column_letter(*column),
                                "usage": role,
                                "source_id": file.file_label,
                                "source": file.file_label,
                                "sheet": sheet.sheet,
                                "column": dsheng_core::import::headers::column_letter(*column),
                                "header_signature": header_signature,
                                "target_role": role,
                            }));
                        }
                    }
                }
                if !mapped.is_empty() {
                    extract_generic_sheet(
                        file,
                        sheet,
                        sheet_data,
                        &mapped,
                        defs,
                        window,
                        grain,
                        scope,
                        coverage,
                        profile,
                        &mut obs,
                        &mut zero_filled,
                        &mut pending_reasons,
                    );
                    let width = sheet_data.rows.iter().map(|r| r.len()).max().unwrap_or(1);
                    sources.push(SourceSelection {
                        source_id: file.file_label.clone(),
                        sheet: sheet.sheet.clone(),
                        range: format!(
                            "A1:{}{}",
                            dsheng_core::import::headers::column_letter(width.saturating_sub(1)),
                            sheet_data.rows.len()
                        ),
                        content_hash: file.content_hash.clone(),
                        source_profile_id: profile.profile_id.clone(),
                        source_profile_version: profile.version,
                        source_profile_hash: profile.hash.clone(),
                    });
                }
                continue;
            }
            if !matches!(sheet.role, "accounting" | "trend") {
                continue;
            }
            // 转置推移表（§4.3 2026-11-04 增量）：期间为行×指标为列，
            // 无标准表头块；与标准 trend 互斥（core 识别期已定）。
            if let Some(layout) = sheet
                .match_info
                .as_ref()
                .and_then(|m| m.transposed.as_ref())
            {
                let width = sheet_data.rows.iter().map(|r| r.len()).max().unwrap_or(1);
                sources.push(SourceSelection {
                    source_id: file.file_label.clone(), // source_id 在注册后回填
                    sheet: sheet.sheet.clone(),
                    range: format!(
                        "A{}:{}",
                        layout.header_row + 1,
                        dsheng_core::import::headers::column_letter(width)
                    ),
                    content_hash: file.content_hash.clone(),
                    source_profile_id: profile.profile_id.clone(),
                    source_profile_version: profile.version,
                    source_profile_hash: profile.hash.clone(),
                });
                let header_texts = sheet_data.rows.get(layout.header_row - 1);
                for (col, _) in &layout.metric_columns {
                    mapping_rows.push(json!({
                        "tool_field": "metric_label",
                        "source_label": header_texts
                            .and_then(|r| r.get(*col))
                            .map(|c| c.raw_text())
                            .unwrap_or_default(),
                        "recognized": dsheng_core::import::headers::column_letter(*col),
                        "usage": "转置推移表指标列（单位后缀剥离后按目录精确匹配）",
                        "source_id": file.file_label,
                        "source": file.file_label,
                        "sheet": sheet.sheet,
                        "column": dsheng_core::import::headers::column_letter(*col),
                        "header_signature": generic_header_signature(sheet_data, None),
                        "target_role": "metric_label",
                    }));
                }
                // 每个 (期间, 指标列) 一个 actual 观察；空=missing；真 0/负数保留
                let mut rows_iter: Vec<(usize, DateRange, bool)> = layout
                    .period_rows
                    .iter()
                    .map(|(r, p)| (*r, *p, false))
                    .chain(layout.cumulative_rows.iter().map(|(r, p)| (*r, *p, true)))
                    .collect();
                rows_iter.sort_by_key(|(r, _, _)| *r);
                for (row_idx, period, cumulative) in rows_iter {
                    for (col, name) in &layout.metric_columns {
                        let Some(def) = match_metric(defs, name) else {
                            let note = format!("未注册指标列（不参与计算）: {name}");
                            if !pending_metrics.contains(&note) {
                                pending_metrics.push(note);
                            }
                            continue;
                        };
                        let evidence_id = format!(
                            "cell:{}:{}:{}{}",
                            file.file_label,
                            sheet.sheet.clone().unwrap_or_default(),
                            dsheng_core::import::headers::column_letter(*col),
                            row_idx + 1
                        );
                        let cell = sheet_data.rows.get(row_idx).and_then(|r| r.get(*col));
                        // 累计行空格不补 0：未报告的累计不是 0（§2.5）
                        let (value, source_label) = match cell_value(cell) {
                            Some(v) => (v, file.file_label.clone()),
                            None if !cumulative
                                && zero_fill_applies(
                                    coverage,
                                    MeasureRole::Actual,
                                    def,
                                    cell,
                                    period,
                                    window,
                                ) =>
                            {
                                zero_filled.push(evidence_id.clone());
                                (Decimal::ZERO, zero_fill_label(&file.file_label, coverage))
                            }
                            // 空格=missing（缺失≠0）；占位/无效文本同样不产生观察
                            None => continue,
                        };
                        obs.push(Obs {
                            metric_id: def.metric_id.clone(),
                            unit: unit_display(&def.unit_json),
                            role: MeasureRole::Actual,
                            value,
                            period,
                            evidence_id,
                            source_label,
                            scope: scope.to_string(),
                            cumulative,
                        });
                    }
                }
                continue;
            }
            let Some(header) = &sheet.header else {
                continue;
            };
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
                source_profile_id: profile.profile_id.clone(),
                source_profile_version: profile.version,
                source_profile_hash: profile.hash.clone(),
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
                    "source_id": file.file_label,
                    "source": file.file_label,
                    "sheet": sheet.sheet,
                    "column": dsheng_core::import::headers::column_letter(col),
                    "header_signature": generic_header_signature(sheet_data, Some(header)),
                    "target_role": field,
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
                        // 显式日期非法/缺失不借用当前窗口；月度观察保持整月归属。
                        let Some(period) = observation_period(&text) else {
                            continue;
                        };
                        period
                    } else if let Some((_, period)) =
                        trend_period_cols.iter().find(|(c, _)| *c == col)
                    {
                        *period
                    } else {
                        // 没有可验证期间列或期间表头时，不能把月/周/季
                        // 汇总猜成当前分析窗口；原始值保留在 snapshot。
                        continue;
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
                        cumulative: false,
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
                            cumulative: false,
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
             WHERE mi.workspace_id = ?1 AND mi.period_key = ?2 AND mi.confirmed = 1",
        )?;
        let rows: Vec<(String, String, String, String, Option<String>, i64)> = stmt
            .query_map(params![workspace_id.to_string(), period_key], |r| {
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
                cumulative: false,
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
    let different_content_policy = profile
        .config
        .as_ref()
        .and_then(|config| config.get("dedupe"))
        .and_then(|dedupe| dedupe.get("different_content"))
        .and_then(Value::as_str)
        .unwrap_or("manual_review");
    let detail_total_policy = profile
        .config
        .as_ref()
        .and_then(|config| config.get("dedupe"))
        .and_then(|dedupe| dedupe.get("detail_and_total"))
        .and_then(Value::as_str)
        .unwrap_or("manual_review");
    for (_, group) in merged {
        let distinct: Vec<Decimal> = {
            let mut v: Vec<Decimal> = group.iter().map(|o| o.value).collect();
            v.sort();
            v.dedup();
            v
        };
        if distinct.len() <= 1 && !(group.len() > 1 && detail_total_policy == "reject") {
            final_obs.push(group.into_iter().next().expect("非空组"));
        } else if distinct.len() > 1
            && (different_content_policy == "latest_revision"
                || detail_total_policy == "select_authoritative")
        {
            // profile 明确声明 latest_revision 时，按稳定来源标签/证据排序选最后一条；
            // 这样选择仍可复现，且不会把“自动覆盖”写死在两表公式中。
            let mut ordered = group;
            ordered.sort_by(|a, b| {
                a.source_label
                    .cmp(&b.source_label)
                    .then_with(|| a.evidence_id.cmp(&b.evidence_id))
            });
            if let Some(latest) = ordered.pop() {
                final_obs.push(latest);
            }
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
        (final_obs, conflicts, sources, mapping_rows, pending_metrics.len(), zero_filled),
        pending_reasons,
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
                // 只汇总完整落入窗口的观察；月总值不能拆成/复制成某一天。
                && o.period.from >= range.from
                && o.period.to <= range.to
                // 累计是独立范围观察（§2.5）：不与月行一起进入窗口求和
                && !o.cumulative
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

fn evaluate_metric_formula(
    key: &str,
    defs: &[MetricDefRow],
    direct: &BTreeMap<String, Option<Decimal>>,
    memo: &mut BTreeMap<String, Option<Decimal>>,
    visiting: &mut BTreeSet<String>,
) -> Option<Decimal> {
    if let Some(value) = memo.get(key) { return *value; }
    if !visiting.insert(key.to_string()) { return None; }
    let value = if let Some(def) = defs.iter().find(|d| d.metric_key == key) {
        if let Some(formula) = &def.formula {
            fn eval_node(
                node: &dsheng_core::contracts::metrics::FormulaNode,
                defs: &[MetricDefRow], direct: &BTreeMap<String, Option<Decimal>>,
                memo: &mut BTreeMap<String, Option<Decimal>>, visiting: &mut BTreeSet<String>,
            ) -> Option<Decimal> {
                use dsheng_core::contracts::metrics::FormulaNode;
                match node {
                    FormulaNode::Ref { metric_key } => evaluate_metric_formula(metric_key, defs, direct, memo, visiting),
                    FormulaNode::Const { value } => rust_decimal::Decimal::from_str_exact(value).ok(),
                    FormulaNode::Add { args } => Some(eval_node(&args[0], defs, direct, memo, visiting)? + eval_node(&args[1], defs, direct, memo, visiting)?),
                    FormulaNode::Sub { args } => Some(eval_node(&args[0], defs, direct, memo, visiting)? - eval_node(&args[1], defs, direct, memo, visiting)?),
                    FormulaNode::Mul { args } => Some(eval_node(&args[0], defs, direct, memo, visiting)? * eval_node(&args[1], defs, direct, memo, visiting)?),
                    FormulaNode::Div { args } => dsheng_core::analysis::engine::positive_div(eval_node(&args[0], defs, direct, memo, visiting)?, eval_node(&args[1], defs, direct, memo, visiting)?).ok(),
                }
            }
            eval_node(formula, defs, direct, memo, visiting)
        } else {
            direct.get(key).copied().flatten()
        }
    } else { None };
    visiting.remove(key);
    memo.insert(key.to_string(), value);
    value
}

fn profile_requires_confirmation(profile: &SourceProfileBinding, metric_key: &str) -> bool {
    profile.config.as_ref()
        .and_then(|c| c.get("record_mappings"))
        .and_then(Value::as_array)
        .map(|xs| xs.iter().any(|m| m.get("metric_key").and_then(Value::as_str) == Some(metric_key) && m.get("requires_confirmation").and_then(Value::as_bool) == Some(true)))
        .unwrap_or(false)
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
    recalculate: bool,
    extra: Option<AnalyzeExtraInput>,
) -> CliResult<Value> {
    run_analyze_with_profile(
        conn,
        workspace_id,
        dir,
        from,
        to,
        grain_auto,
        grain_explicit,
        policy_explicit,
        recalculate,
        extra,
        SourceProfileBinding::generic_implicit(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_analyze_with_profile(
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
    profile: SourceProfileBinding,
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
    let (week_start, fiscal, tz, cadence, month_start): (u32, u32, String, String, u32) = conn
        .query_row(
            "SELECT week_start_day, fiscal_year_start_month, timezone_id, cadence, month_start_day
             FROM period_policies WHERE workspace_id = ?1 AND version = ?2",
            params![workspace_id.to_string(), policy_version],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
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
        month_start_day: month_start,
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

    // 扫描（注册来源）＋观察提取；指标名全集供转置推移表锚点匹配（§4.3 增量）
    crate::store::sources::ensure_sidecar_tables(conn)?;
    let metric_names = crate::store::sources::metric_label_names(conn, workspace_id)?;
    let aliases = crate::store::sources::profile_aliases_for_binding(&profile);
    let (files, pending_files) = crate::store::sources::scan_directory_with_profile_config(
        dir,
        &metric_names,
        aliases.as_ref(),
        Some(&profile),
    )?;
    let registered = register_sources_with_profile(conn, workspace_id, &files, &profile)?;
    // 扫描快照落旁表（非空格＋布局）：生成载荷按相关行/列裁剪的确定性输入
    crate::store::sources::persist_source_snapshot(conn, workspace_id, &registered, &files)?;
    let defs = load_metric_defs(conn, workspace_id)?;
    let period_key = dsheng_core::repository::analysis_runs::build_period_key(
        &policy_version,
        grain,
        window,
        &scope,
    );
    let generic_mapping_required = profile.mode == "generic"
        && files.iter().any(|file| {
            file.sheets.iter().any(|(_, sheet)| {
                sheet.role == "generic_table" && sheet.header.is_some() && sheet.blocked_reason.is_none()
            })
        });
    let confirmed_source_mapping = crate::store::sources::source_mapping_confirmed(
        conn,
        workspace_id,
        dir,
        &profile.hash,
    )?;
    if generic_mapping_required && confirmed_source_mapping.is_none() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "通用 source 的字段映射尚未确认；请先在数据源页面确认映射后再分析",
        ));
    }
    let mapping_entries = if let Some(entries) = confirmed_source_mapping {
        entries
    } else {
        mapping_draft_for_period(conn, workspace_id, &period_key)?
            .map(|(_, entries)| entries)
            .unwrap_or_default()
    };
    let ((mut obs, mut conflicts, mut sources, mapping_rows, mut pending_count, zero_filled), mut pending_reasons) =
        extract_observations(
            conn,
            workspace_id,
            &defs,
            &files,
            window,
            grain,
            &period_key,
            &scope,
            &coverage,
            &mapping_entries,
            &profile,
        )?;
    let tax_basis_pending = profile
        .config
        .as_ref()
        .and_then(|config| config.get("normalization"))
        .and_then(|normalization| normalization.get("tax_basis"))
        .and_then(Value::as_str)
        == Some("confirm_at_runtime");
    if tax_basis_pending {
        // 税口径尚未由企业确认时，原始记录仍保留在 source snapshot，但
        // 未确认金额不得进入 valid observations、派生公式或两表数值。
        obs.clear();
        conflicts.clear();
        for def in &defs {
            pending_reasons
                .entry(def.metric_key.clone())
                .or_default()
                .push("configuration_required_tax_basis".into());
        }
        pending_count += 1;
    }
    if !pending_files.is_empty() {
        for pending in &pending_files {
            for def in &defs {
                pending_reasons
                    .entry(def.metric_key.clone())
                    .or_default()
                    .push(pending.clone());
            }
        }
        pending_count += pending_files.len();
    }
    // 来源范围绑定本次实际扫描的目录，而不只绑定命中核算/推移适配器的 sheet。
    // generic CSV 等仍可能为生成提供相关行；空 sheet 没有持久化单元格，不入选。
    for file in &files {
        let source_id = registered
            .iter()
            .find(|v| v["file_label"].as_str() == Some(file.file_label.as_str()))
            .and_then(|v| v["file_id"].as_str())
            .ok_or_else(|| {
                CliError::new(CliErrorCode::DataValidationFailed, "扫描来源未完成注册")
            })?;
        for (sheet_data, sheet) in &file.sheets {
            if sheet.blocked_reason.is_some() {
                continue;
            }
            if sources
                .iter()
                .any(|s| s.source_id == file.file_label && s.sheet == sheet.sheet)
            {
                continue;
            }
            let has_cells: bool = conn.query_row(
                "SELECT EXISTS (SELECT 1 FROM cli_source_cells
                 WHERE workspace_id = ?1 AND source_id = ?2 AND sheet = ?3)",
                params![workspace_id.to_string(), source_id, sheet_data.name],
                |r| r.get(0),
            )?;
            if !has_cells {
                continue;
            }
            let width = sheet_data
                .rows
                .iter()
                .map(|row| row.len())
                .max()
                .unwrap_or(1);
            sources.push(SourceSelection {
                source_id: file.file_label.clone(),
                sheet: sheet.sheet.clone(),
                range: format!(
                    "A1:{}{}",
                    dsheng_core::import::headers::column_letter(width.saturating_sub(1)),
                    sheet_data.rows.len()
                ),
                content_hash: file.content_hash.clone(),
                source_profile_id: profile.profile_id.clone(),
                source_profile_version: profile.version,
                source_profile_hash: profile.hash.clone(),
            });
        }
    }
    // 注册视图字段为 file_id；历史版本的空 source_id 不在此重写。
    for sel in sources.iter_mut() {
        if let Some(view) = registered
            .iter()
            .find(|v| v["file_label"].as_str() == Some(sel.source_id.as_str()))
        {
            sel.source_id = view["file_id"].as_str().unwrap_or_default().to_string();
        }
    }

    // 指标行/序列
    let mut metrics_rows = Vec::new();
    // 内部 revision 元数据：与当期 actual 使用同一观察集合，不依赖同比/环比可用性。
    // 不投影到冻结的 metricRowView；桌面诊断通过只读 helper 单独取证据。
    let mut diagnosis_evidence_by_metric = serde_json::Map::new();
    let mut series_rows = Vec::new();
    let mut comparisons_obj = serde_json::Map::new();
    let mut base_refs_used: Vec<BaseRef> = Vec::new();
    let direct_actuals: BTreeMap<String, Option<Decimal>> = defs.iter().map(|def| {
        let values = obs_in(&obs, &def.metric_id, MeasureRole::Actual, window);
        let value = flow_sum_window(&values);
        let value = if profile_requires_confirmation(&profile, &def.metric_key) { None } else { value };
        (def.metric_key.clone(), value)
    }).collect();
    let mut formula_actuals = BTreeMap::new();
    // 目录将单位时间附加值定义为需确认的派生指标；没有企业确认的
    // formula/有效工时输入时，不能把来源中同名的一列当成最终结果。
    let configuration_required = |def: &MetricDefRow| {
        def.metric_key == "value_added_per_hour"
            && def.formula.is_none()
    };
    let mut series_period_keys: BTreeSet<String> = obs
        .iter()
        .filter(|o| o.period.from >= window.from && o.period.to <= window.to)
        .map(|o| o.period.to_period_string())
        .collect();
    if series_period_keys.is_empty() {
        series_period_keys.insert(window.to_period_string());
    }
    for def in &defs {
        let relevant: Vec<&Obs> = obs
            .iter()
            .filter(|o| {
                o.metric_id == def.metric_id
                    && o.period.from >= window.from
                    && o.period.to <= window.to
            })
            .collect();
        // 固定两表必须保留指标行；没有观察时呈现 null/待补状态，
        // 不能通过省略该行把“关键指标缺失”伪装成已完成。
        if relevant.is_empty() {
            pending_count += 1;
        }
        let actual_observations = obs_in(&obs, &def.metric_id, MeasureRole::Actual, window);
        let actual = if configuration_required(def) {
            None
        } else if def.formula.is_some() {
            evaluate_metric_formula(&def.metric_key, &defs, &direct_actuals, &mut formula_actuals, &mut BTreeSet::new())
        } else {
            flow_sum_window(&actual_observations)
        };
        let mut evidence_ids: Vec<String> = actual_observations
            .iter()
            .map(|o| o.evidence_id.clone())
            .collect();
        evidence_ids.sort();
        evidence_ids.dedup();
        diagnosis_evidence_by_metric.insert(def.metric_id.clone(), json!(evidence_ids));
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
                "上一完整周期".to_string(),
            ),
            (
                "yoy",
                yoy_base(window, grain, &policy),
                "上一年同期".to_string(),
            ),
        ] {
            let Ok(base_period) = base_result else {
                continue;
            };
            let base_observations =
                obs_in(&obs, &def.metric_id, MeasureRole::Actual, base_period.range);
            let base_value = flow_sum_window(&base_observations);
            let mut comparison_evidence: Vec<String> = actual_observations
                .iter()
                .chain(base_observations.iter())
                .map(|observation| observation.evidence_id.clone())
                .collect();
            comparison_evidence.sort();
            comparison_evidence.dedup();
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
                comparison_evidence,
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
        let mut pending_for_metric = pending_reasons.get(&def.metric_key).cloned().unwrap_or_default();
        if configuration_required(def) {
            pending_for_metric.push("configuration_required".into());
        }
        if actual.is_none() { pending_for_metric.push("actual_missing".into()); }
        let observed_period_keys: BTreeSet<String> = relevant
            .iter()
            .map(|o| o.period.to_period_string())
            .collect();
        if !series_period_keys
            .iter()
            .all(|key| observed_period_keys.contains(key))
        {
            pending_for_metric.push("period_bucket_missing".into());
        }
        if target.is_none() { pending_for_metric.push("target_missing".into()); }
        pending_for_metric.sort();
        pending_for_metric.dedup();
        if !pending_for_metric.is_empty() { pending_count += 1; }
        metrics_rows.push(json!({
            "metric_id": def.metric_id,
            "metric_key": def.metric_key,
            "metric_label": def.label,
            "unit": unit_display(&def.unit_json),
            "direction": def.direction,
            "target_value": target.map(to_canonical_string),
            "actual_value": actual.map(to_canonical_string),
            "delta": delta.map(to_canonical_string),
            "status": if pending_for_metric.is_empty() { "ready" } else { "pending" },
            "pending_reasons": pending_for_metric,
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
        // 期间桶按所有 source 指定的窗口内期间对齐；某个指标没有该桶时
        // 保留 null，确保“缺失≠0”且推移表不会把断点隐藏掉。
        for key in &series_period_keys {
            buckets.entry(key.clone()).or_insert((None, None));
        }
        if def.formula.is_some() && buckets.values().all(|(a, _)| a.is_none()) {
            if let Some(value) = actual {
                buckets.insert(window.to_period_string(), (Some(value), None));
            }
        }
        if !buckets.is_empty() || relevant.is_empty() {
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
        "diagnosis_evidence_by_metric": diagnosis_evidence_by_metric,
        "series": series_rows,
        "pending_inputs": pending_count,
        "scope": scope,
        "source_profile": {
            "profile_id": profile.profile_id,
            "version": profile.version,
            "hash": profile.hash,
            "mode": profile.mode,
            "profile_ref": profile.profile_ref,
            "config": profile.config,
        },
    });
    let comparisons = Value::Object(comparisons_obj);

    // 手工输入快照
    let manual_snapshots: Vec<ManualInputSnapshot> = {
        let mut stmt = conn.prepare(
            "SELECT input_id, metric_id, input_revision, value FROM manual_inputs
             WHERE workspace_id = ?1 AND period_key = ?2",
        )?;
        let rows: Vec<(String, String, i64, Option<String>)> = stmt
            .query_map(params![workspace_id.to_string(), period_key], |r| {
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
    let mapping_value = json!(mapping_rows);
    let mapping_version = dsheng_core::analysis::fingerprint::sha256_hex(
        &dsheng_core::analysis::fingerprint::canonical_json(&mapping_value),
    );
    let input = AnalysisRevisionInput {
        sources,
        mapping_version,
        mappings: mapping_value,
        metric_definitions: defs
            .iter()
            .map(|d| MetricDefSnapshot {
                metric_key: d.metric_key.clone(),
                version: d.version as u32,
                scope: d.scope.clone(),
                semantic_hash: d.semantic_hash.clone(),
                semantic_type: Some(d.semantic_type.clone()),
                unit_signature: serde_json::from_str(&d.unit_json).ok(),
            })
            .collect(),
        manual_inputs: manual_snapshots,
        conflict_resolutions: vec![],
        period_policy: json!({
            "version": policy_version,
            "week_start_day": week_start,
            "month_start_day": month_start,
            "fiscal_year_start_month": fiscal,
            "timezone_id": tz,
            "source_profile": {
                "profile_id": profile.profile_id,
                "version": profile.version,
                "hash": profile.hash,
                "mode": profile.mode,
                "profile_ref": profile.profile_ref,
                "config": profile.config,
            },
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
    // mapping 草稿已被本次 revision 消费；后续 mapping show 读取不可变快照。
    conn.execute(
        "DELETE FROM cli_mapping_drafts WHERE run_id = ?1",
        params![upsert.run_id],
    )?;
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
    crate::store::sources::mark_two_tables_materialized(
        conn,
        workspace_id,
        &run_id,
        upsert.revision_no,
    )?;
    analysis_run_view(conn, workspace_id, &run_id, upsert.revision_no)
}

/// 诊断问题可引用的本地证据（指定不可变分析 revision；不读取当前来源或指标定义）。
/// 新快照使用当期 actual 的内部证据元数据；旧版仅复用已有 comparison 证据。
/// 旧版缺少证据时返回空集合，由界面提示重新执行本地分析，不补造定位。
pub fn diagnosis_evidence_ids(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: u32,
    metric_id: Option<&str>,
) -> CliResult<Vec<String>> {
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let raw: String = conn.query_row(
        "SELECT normalized_metrics_json FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
        params![run_id, revision], |r| r.get(0),
    ).map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在"),
        other => CliError::from(other),
    })?;
    let snapshot: Value = serde_json::from_str(&raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    let metrics = snapshot
        .get("metrics")
        .and_then(Value::as_array)
        .ok_or_else(|| CliError::new(CliErrorCode::LocalSaveFailed, "分析快照缺少指标行"))?;
    if let Some(id) = metric_id {
        if !metrics
            .iter()
            .any(|row| row.get("metric_id").and_then(Value::as_str) == Some(id))
        {
            return Err(CliError::new(
                CliErrorCode::InputContractInvalid,
                "指标不属于所选分析 revision",
            )
            .with_field("metric_id"));
        }
    }
    let metadata = snapshot.get("diagnosis_evidence_by_metric");
    let mut evidence_ids = Vec::new();
    for row in metrics {
        let Some(id) = row.get("metric_id").and_then(Value::as_str) else {
            continue;
        };
        if metric_id.is_some_and(|selected| selected != id) {
            continue;
        }
        let lists: Vec<&Value> = if let Some(metadata) = metadata {
            metadata.get(id).into_iter().collect()
        } else {
            [
                row.pointer("/mom/evidence_ids"),
                row.pointer("/yoy/evidence_ids"),
            ]
            .into_iter()
            .flatten()
            .collect()
        };
        for list in lists {
            if let Some(items) = list.as_array() {
                evidence_ids.extend(
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .map(str::to_string),
                );
            }
        }
    }
    evidence_ids.sort();
    evidence_ids.dedup();
    Ok(evidence_ids)
}

/// analysisRunView（从 run + revision 快照渲染）。
pub fn analysis_run_view(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: u32,
) -> CliResult<Value> {
    let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let (normalized_raw, fingerprint): (String, String) = conn
        .query_row(
            "SELECT normalized_metrics_json, analysis_input_fingerprint
             FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| {
            CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在")
        })?;
    let normalized: Value = serde_json::from_str(&normalized_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    let two_tables_ready: bool = conn
        .query_row(
            "SELECT accounting_ready = 1 AND trend_ready = 1
             FROM cli_two_tables_materializations
             WHERE workspace_id = ?1 AND run_id = ?2 AND revision_no = ?3
                   AND contract_id = 'two_tables_v1'",
            params![workspace_id.to_string(), run_id, revision as i64],
            |row| row.get(0),
        )
        .unwrap_or(false);
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
        "two_tables_ready": two_tables_ready,
        "source_profile": normalized.get("source_profile").cloned().unwrap_or(json!(null)),
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
        let policy_version = period_key
            .split('|')
            .find_map(|part| part.strip_prefix("pv="))
            .and_then(|version| version.parse::<u32>().ok())
            .unwrap_or(1);
        out.push(json!({
            "run_id": run_id,
            "workspace_id": ws,
            "period": period,
            "period_key": period_key,
            "cadence": grain,
            "policy_version": policy_version,
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
    let (current, _current_fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let revision = revision.unwrap_or(current);
    let (normalized_raw, captured_at, fingerprint): (String, String, String) = conn
        .query_row(
            "SELECT normalized_metrics_json, created_at, analysis_input_fingerprint FROM run_revisions
             WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在"))?;
    let normalized: Value = serde_json::from_str(&normalized_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    let two_tables_ready: bool = conn
        .query_row(
            "SELECT accounting_ready = 1 AND trend_ready = 1
             FROM cli_two_tables_materializations
             WHERE workspace_id = ?1 AND run_id = ?2 AND revision_no = ?3
                   AND contract_id = 'two_tables_v1'",
            params![workspace_id.to_string(), run_id, revision as i64],
            |row| row.get(0),
        )
        .unwrap_or(false);
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
    let task_plan_period =
        dsheng_core::repository::task_create::saved_task_periods(conn, run_id, revision)
            .ok()
            .map(|(_, plan)| plan);
    Ok(json!({
        "run_id": run_id,
        "analysis_revision_no": revision,
        "analysis_input_fingerprint": fingerprint,
        "task_plan_period": task_plan_period,
        "task_snapshot_no": snapshot_no,
        "captured_at": captured_at,
        "metrics": normalized.get("metrics").cloned().unwrap_or(json!([])),
        "series": normalized.get("series").cloned().unwrap_or(json!([])),
        "grain": normalized.get("grain").cloned().unwrap_or(json!("month")),
        "period": normalized.get("period").cloned().unwrap_or(json!({"from": null, "to": null})),
        "tasks": tasks,
        "two_tables_ready": two_tables_ready,
        "source_profile": normalized.get("source_profile").cloned().unwrap_or(json!(null)),
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
