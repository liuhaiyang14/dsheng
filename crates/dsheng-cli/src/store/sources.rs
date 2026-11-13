//! 来源/映射/手工输入/指标定义/来源冲突服务（cli-surface 12–20）。
//!
//! 识别与数值解析全部复用 dsheng-core `import::*`（表头别名 exact 匹配、
//! 值状态机、千分位）；本层只做持久化与视图。
//!
//! 合同缺口（报告主 Agent）：0001 冻结 schema 无字段映射草稿表，
//! `mapping set` 的“原子版本化草稿”以 CLI 侧 `cli_mapping_drafts` 旁表实现
//! （幂等建表；不属冻结 schema 版本）。

use crate::error::{CliError, CliResult};
use chrono::{Datelike, Duration, NaiveDate};
use dsheng_core::analysis::periods::{fiscal_quarter_span, monthly_period_span, CalendarPolicy};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::domain::{DateRange, UnitSignature};
use dsheng_core::contracts::metrics::{
    compute_semantic_hash, Aggregation, ComparisonPolicy, DenominatorPolicy, FormulaNode,
    MetricDefinitionProjection, SemanticType, TargetConstraintScope,
};
use dsheng_core::import::fileio::SheetData;
use dsheng_core::import::headers::{column_letter, normalize_header, HeaderBlock};
use dsheng_core::import::template::{detect_sheet_role_with_names, SheetMatch, SheetRole};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use uuid::Uuid;

/// 工作区指标名全集（metric_key/label/label_aliases，最新版本），供扫描期
/// 转置推移表锚点做后缀剥离后的精确匹配（§4.3 2026-11-04 增量）。
pub fn metric_label_names(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT m.metric_key, m.label, m.label_aliases_json
         FROM metric_definitions m
         WHERE m.workspace_id = ?1 AND m.version =
               (SELECT MAX(version) FROM metric_definitions m2
                WHERE m2.workspace_id = m.workspace_id AND m2.metric_key = m.metric_key)",
    )?;
    let rows: Vec<(String, String, String)> = stmt
        .query_map(params![workspace_id.to_string()], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })?
        .collect::<Result<_, _>>()
        .map_err(CliError::from)?;
    let mut names = Vec::new();
    for (key, label, aliases_raw) in rows {
        names.push(key);
        names.push(label);
        names.extend(serde_json::from_str::<Vec<String>>(&aliases_raw).unwrap_or_default());
    }
    Ok(names)
}

/// CLI 侧旁表（见模块注释）。
pub fn ensure_sidecar_tables(conn: &Connection) -> CliResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS cli_mapping_drafts (
             run_id        TEXT PRIMARY KEY,
             workspace_id  TEXT NOT NULL,
             revision      INTEGER NOT NULL CHECK (revision >= 1),
             draft_json    TEXT NOT NULL,
             updated_at    TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS cli_source_cells (
             workspace_id  TEXT NOT NULL,
             source_id     TEXT NOT NULL,
             sheet         TEXT NOT NULL,
             row_no        INTEGER NOT NULL CHECK (row_no >= 1),
             col_no        INTEGER NOT NULL CHECK (col_no >= 0),
             raw_text      TEXT NOT NULL,
             cell_kind     TEXT NOT NULL CHECK (cell_kind IN
                 ('text','int','number','float_unverified','bool','formula','error')),
             value_state   TEXT NOT NULL CHECK (value_state IN
                 ('valid','missing','pending_input','invalid','conflict',
                  'configuration_required','unsupported_formula','precision_unverified')),
             typed_value   TEXT,
             formula       TEXT,
             merged_range  TEXT,
             PRIMARY KEY (workspace_id, source_id, sheet, row_no, col_no)
         );
         CREATE TABLE IF NOT EXISTS cli_source_sheet_layouts (
             workspace_id  TEXT NOT NULL,
             source_id     TEXT NOT NULL,
             sheet         TEXT NOT NULL,
             role          TEXT NOT NULL,
             layout_json   TEXT NOT NULL,
             PRIMARY KEY (workspace_id, source_id, sheet)
         );
         CREATE TABLE IF NOT EXISTS cli_source_profiles (
             workspace_id TEXT NOT NULL,
             source_id TEXT NOT NULL,
             profile_id TEXT NOT NULL,
             profile_version INTEGER NOT NULL,
             profile_hash TEXT NOT NULL,
             profile_mode TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             PRIMARY KEY (workspace_id, source_id)
         );
         CREATE TABLE IF NOT EXISTS cli_source_mapping_sets (
             workspace_id TEXT NOT NULL,
             directory_label TEXT NOT NULL,
             profile_hash TEXT NOT NULL,
             mapping_json TEXT NOT NULL,
             updated_at TEXT NOT NULL,
             PRIMARY KEY (workspace_id, directory_label, profile_hash)
         );
         CREATE TABLE IF NOT EXISTS cli_two_tables_materializations (
             workspace_id TEXT NOT NULL,
             run_id TEXT NOT NULL,
             revision_no INTEGER NOT NULL CHECK (revision_no >= 1),
             contract_id TEXT NOT NULL CHECK (contract_id = 'two_tables_v1'),
             accounting_ready INTEGER NOT NULL CHECK (accounting_ready IN (0,1)),
             trend_ready INTEGER NOT NULL CHECK (trend_ready IN (0,1)),
             created_at TEXT NOT NULL,
             PRIMARY KEY (workspace_id, run_id, revision_no)
         );",
    )
    .map_err(CliError::from)?;
    Ok(())
}

/// analysis revision 成功后登记固定两表输出已可物化。任务生成只允许引用
/// 同一 workspace/run/revision 的完整登记，避免绕过两表门禁直接发 Provider 请求。
pub fn mark_two_tables_materialized(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision_no: u32,
) -> CliResult<()> {
    ensure_sidecar_tables(conn)?;
    let normalized: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions
             WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision_no as i64],
            |row| row.get(0),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "analysis revision 不存在"))?;
    let normalized: Value = serde_json::from_str(&normalized)
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "normalized result 无效"))?;
    let has_accounting_rows = normalized
        .get("metrics")
        .and_then(Value::as_array)
        .is_some_and(|rows| !rows.is_empty());
    let has_trend_rows = normalized
        .get("series")
        .and_then(Value::as_array)
        .is_some_and(|rows| !rows.is_empty());
    // 分析结果可以先带着缺失期间或待补指标物化；两表页面会将缺项标为
    // pending，只有两张结果表都确实存在时才允许任务生成/外部导出。
    // 不把“至少两个期间桶”当成分析失败条件，单期间数据仍可生成一张
    // 当前期推移表，后续补录后再重算。
    conn.execute(
        "INSERT INTO cli_two_tables_materializations
         (workspace_id, run_id, revision_no, contract_id, accounting_ready, trend_ready, created_at)
         VALUES (?1, ?2, ?3, 'two_tables_v1', ?4, ?5, ?6)
         ON CONFLICT(workspace_id, run_id, revision_no) DO UPDATE SET
           accounting_ready=excluded.accounting_ready, trend_ready=excluded.trend_ready,
           created_at=excluded.created_at",
        params![
            workspace_id.to_string(),
            run_id,
            revision_no as i64,
            if has_accounting_rows { 1 } else { 0 },
            if has_trend_rows { 1 } else { 0 },
            now_iso()
        ],
    )
    .map_err(CliError::from)?;
    Ok(())
}

pub fn require_two_tables_materialized(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision_no: u32,
) -> CliResult<()> {
    ensure_sidecar_tables(conn)?;
    let ready: bool = conn
        .query_row(
            "SELECT accounting_ready = 1 AND trend_ready = 1
             FROM cli_two_tables_materializations
             WHERE workspace_id = ?1 AND run_id = ?2 AND revision_no = ?3
                   AND contract_id = 'two_tables_v1'",
            params![workspace_id.to_string(), run_id, revision_no as i64],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !ready {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "当前分析 revision 尚未生成完整的 two_tables_v1 核算表和推移表",
        ));
    }
    Ok(())
}

/// 旁表单元格字段（raw_text, cell_kind, value_state, typed_value, formula）。
type CellSnapshotFields = (
    String,
    &'static str,
    &'static str,
    Option<String>,
    Option<String>,
);

/// CellValue → 旁表行字段（词法保留；value_state 按 core §1.1 状态机）。
/// 空白格 None（不造记录）。
fn cell_snapshot_fields(
    cell: &dsheng_core::import::fileio::CellValue,
) -> Option<CellSnapshotFields> {
    use dsheng_core::import::fileio::CellValue;
    let state = match cell.value_state() {
        dsheng_core::contracts::domain::ValueState::Valid => "valid",
        dsheng_core::contracts::domain::ValueState::Missing => "missing",
        dsheng_core::contracts::domain::ValueState::PendingInput => "pending_input",
        dsheng_core::contracts::domain::ValueState::Invalid => "invalid",
        dsheng_core::contracts::domain::ValueState::Conflict => "conflict",
        dsheng_core::contracts::domain::ValueState::ConfigurationRequired => {
            "configuration_required"
        }
        dsheng_core::contracts::domain::ValueState::UnsupportedFormula => "unsupported_formula",
        dsheng_core::contracts::domain::ValueState::PrecisionUnverified => "precision_unverified",
    };
    // 空白格不造记录（data-period-import.md §1.4：blank 不发；合并非锚点为 Empty）
    let decimal_of = || {
        cell.exact_decimal()
            .map(dsheng_core::contracts::decimal::to_canonical_string)
    };
    let fields = match cell {
        CellValue::Empty => return None,
        CellValue::Text(t) => {
            if t.trim().is_empty() {
                return None;
            }
            (cell.raw_text(), "text", state, Some(t.clone()), None)
        }
        CellValue::Int(_) => (cell.raw_text(), "int", state, decimal_of(), None),
        CellValue::Number(_) => (cell.raw_text(), "number", state, decimal_of(), None),
        CellValue::FloatUnverified(t) => (t.clone(), "float_unverified", state, None, None),
        CellValue::Bool(b) => (cell.raw_text(), "bool", state, Some(b.to_string()), None),
        CellValue::Formula { source, .. } => {
            (source.clone(), "formula", state, None, Some(source.clone()))
        }
        CellValue::Error(e) => {
            if e.trim().is_empty() {
                return None;
            }
            (cell.raw_text(), "error", state, None, None)
        }
    };
    Some(fields)
}

/// 落库扫描快照：全部非空格（cli_source_cells）＋每 sheet 布局（layouts）。
/// 幂等（INSERT OR REPLACE）；source_id 由注册视图按 file_label 匹配回填。
pub fn persist_source_snapshot(
    conn: &Connection,
    workspace_id: Uuid,
    registered: &[Value],
    files: &[ScannedFile],
) -> CliResult<()> {
    ensure_sidecar_tables(conn)?;
    for file in files {
        let Some(source_id) = registered
            .iter()
            .find(|v| v["file_label"].as_str() == Some(file.file_label.as_str()))
            .and_then(|v| v["file_id"].as_str())
            .map(str::to_string)
        else {
            continue;
        };
        for (sheet_data, scanned) in file.sheets.iter() {
            // 合并范围 → 左上锚点映射（值仅保留锚点；锚点记录携带完整范围）
            let mut merged_of: std::collections::HashMap<(usize, usize), String> =
                std::collections::HashMap::new();
            for range in &sheet_data.merged_ranges {
                if let Some(((col, row), _)) = parse_a1_pair(range) {
                    merged_of.entry((row, col)).or_insert_with(|| range.clone());
                }
            }
            for (row_idx, row) in sheet_data.rows.iter().enumerate() {
                for (col_idx, cell) in row.iter().enumerate() {
                    let Some((raw_text, cell_kind, value_state, typed_value, formula)) =
                        cell_snapshot_fields(cell)
                    else {
                        continue;
                    };
                    conn.execute(
                        "INSERT OR REPLACE INTO cli_source_cells (workspace_id, source_id, sheet,
                                row_no, col_no, raw_text, cell_kind, value_state, typed_value,
                                formula, merged_range)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                        params![
                            workspace_id.to_string(),
                            source_id,
                            sheet_data.name,
                            row_idx as i64 + 1,
                            col_idx as i64,
                            raw_text,
                            cell_kind,
                            value_state,
                            typed_value,
                            formula,
                            merged_of.get(&(row_idx, col_idx))
                        ],
                    )
                    .map_err(CliError::from)?;
                }
            }
            conn.execute(
                "INSERT OR REPLACE INTO cli_source_sheet_layouts (workspace_id, source_id, sheet,
                        role, layout_json)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    workspace_id.to_string(),
                    source_id,
                    sheet_data.name,
                    scanned.role,
                    serde_json::to_string(&crate::store::analysis::sheet_layout_snapshot(
                        sheet_data, scanned
                    ))
                    .unwrap_or_else(|_| "{}".into())
                ],
            )
            .map_err(CliError::from)?;
        }
    }
    Ok(())
}

/// "B7:B9" → 左上 (0 基 col, row)；解析失败 None。
fn parse_a1_pair(range: &str) -> Option<((usize, usize), (usize, usize))> {
    let (start, end) = range.split_once(':')?;
    let parse = |a1: &str| -> Option<(usize, usize)> {
        let col_letters: String = a1.chars().take_while(char::is_ascii_alphabetic).collect();
        let row_digits: String = a1.chars().skip_while(char::is_ascii_alphabetic).collect();
        if col_letters.is_empty() || row_digits.is_empty() {
            return None;
        }
        let mut col = 0usize;
        for ch in col_letters.chars() {
            col = col * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
        }
        let row: usize = row_digits.parse().ok()?;
        Some((col - 1, row - 1))
    };
    Some((parse(start)?, parse(end)?))
}

fn now_iso() -> String {
    dsheng_core::repository::utc_iso(chrono::Utc::now())
}

fn sha256_bytes(data: &[u8]) -> String {
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// 本次扫描/分析绑定的 source profile 身份。profile 内容 hash 进入来源
/// manifest 与 analysis fingerprint，profile/mapping 变化会自然产生新 revision。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceProfileBinding {
    pub profile_id: String,
    pub version: u32,
    pub hash: String,
    pub mode: String,
    /// 用户选择的原始引用（JSON 路径或注册表 ID）。profile_id 是文件内部
    /// 身份，不能替代这个引用，否则下一次 analyze 无法重新加载本地 JSON。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

impl SourceProfileBinding {
    pub fn generic() -> Self {
        let hash = sha256_bytes(br#"{"profile_id":"generic","version":1}"#);
        Self {
            profile_id: "generic".into(),
            version: 1,
            hash,
            mode: "generic".into(),
            profile_ref: None,
            config: None,
        }
    }

    /// CLI 未传 profile 的历史兼容模式。桌面/UI 明确选择 generic 时仍必须
    /// 经过 source_mapping_set；纯 CLI 旧调用继续使用规范表头以保持脚本兼容。
    pub fn generic_implicit() -> Self {
        let mut profile = Self::generic();
        profile.mode = "generic_implicit".into();
        profile
    }
}

/// 解析 CLI 的 `--source-profile` / `--generic`。profile 参数可以是 profile id，
/// 也可以是本地 JSON 文件；文件内容 hash 作为不可变绑定。未指定时只使用
/// generic profile（绝不套用当前示例企业），以保持旧 CLI/桌面调用兼容。
pub fn resolve_source_profile(
    source_profile: Option<&str>,
    generic: bool,
) -> CliResult<SourceProfileBinding> {
    if source_profile.is_some() && generic {
        return Err(CliError::invalid("--source-profile 与 --generic 互斥"));
    }
    let Some(raw) = source_profile.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(if generic {
            SourceProfileBinding::generic()
        } else {
            SourceProfileBinding::generic_implicit()
        });
    };
    if let Some((profile, bytes)) = builtin_profile(raw) {
        validate_profile_config(&profile)?;
        let id = profile
            .get("profile_id")
            .and_then(Value::as_str)
            .unwrap_or(raw)
            .to_string();
        let version = profile.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
        return Ok(SourceProfileBinding {
            profile_id: id,
            version,
            hash: sha256_bytes(bytes.as_bytes()),
            mode: "profile".into(),
            profile_ref: Some(raw.to_string()),
            config: Some(profile),
        });
    }
    let path = Path::new(raw);
    if path.is_file() {
        let bytes = std::fs::read(path).map_err(|e| {
            CliError::new(CliErrorCode::DataDirectoryUnavailable, e.to_string())
                .with_field("--source-profile")
        })?;
        let profile: Value = serde_json::from_slice(&bytes).map_err(|e| {
            CliError::invalid(format!("source profile JSON 无效: {e}"))
                .with_field("--source-profile")
        })?;
        validate_profile_config(&profile)?;
        let id = profile
            .get("profile_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| {
                CliError::invalid("source profile 缺少 profile_id").with_field("--source-profile")
            })?;
        let version = profile.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
        return Ok(SourceProfileBinding {
            profile_id: id.to_string(),
            version,
            hash: sha256_bytes(&bytes),
            mode: "profile".into(),
            profile_ref: Some(raw.to_string()),
            config: Some(profile),
        });
    }
    if raw == "generic" {
        return Ok(SourceProfileBinding::generic());
    }
    Err(CliError::invalid(format!(
        "未知 source profile：{raw}；请提供已注册 profile JSON 路径，或使用 --generic"
    )))
}

/// 首版提供两个可替换的内置 profile，避免 UI/原型里的 profile ID 只能展示、不能执行。
/// 企业新增类型仍可传本地 JSON；profile 内容 hash 会进入 revision。
fn builtin_profile(id: &str) -> Option<(Value, &'static str)> {
    let bytes = match id {
        "manufacturing_demo" | "manufacturing_erp_v1" => {
            include_str!("../../../../contracts/v1/source-profiles/manufacturing_demo.json")
        }
        "service_demo" => include_str!("../../../../contracts/v1/source-profiles/service_demo.json"),
        "xsd_trade_v1" => include_str!("../../../../contracts/v1/source-profiles/xsd_trade_v1.json"),
        _ => return None,
    };
    let value = serde_json::from_str(bytes).ok()?;
    Some((value, bytes))
}

fn validate_profile_config(profile: &Value) -> CliResult<()> {
    fn reject_unknown_keys(
        object: &serde_json::Map<String, Value>,
        path: &str,
        allowed: &[&str],
    ) -> CliResult<()> {
        if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
            return Err(CliError::invalid(format!(
                "source profile {path} 含未定义字段：{key}"
            )));
        }
        Ok(())
    }
    let object = |name: &str| {
        profile
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| CliError::invalid(format!("source profile 缺少或无效 {name}")))
    };
    let nonempty = |name: &str| {
        profile
            .get(name)
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| CliError::invalid(format!("source profile 缺少或无效 {name}")))
    };
    if profile.get("schema_version").and_then(Value::as_str) != Some("1")
        || profile.get("output_contract_id").and_then(Value::as_str) != Some("two_tables_v1")
    {
        return Err(CliError::invalid("source profile schema_version/output_contract_id 无效"));
    }
    reject_unknown_keys(
        profile
            .as_object()
            .ok_or_else(|| CliError::invalid("source profile 根对象无效"))?,
        "root",
        &[
            "schema_version", "profile_id", "version", "enterprise_type", "display_name",
            "source_template_version", "template", "output_contract_id", "file_roles",
            "field_aliases", "row_rules", "normalization", "dedupe", "record_mappings", "notes",
        ],
    )?;
    let profile_id = nonempty("profile_id")?;
    if !profile_id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        || !profile_id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(CliError::invalid("source profile profile_id 格式无效"));
    }
    for name in ["version", "source_template_version"] {
        if profile.get(name).and_then(Value::as_u64).is_none_or(|v| v == 0) {
            return Err(CliError::invalid(format!("source profile {name} 必须是正整数")));
        }
    }
    for name in ["enterprise_type", "display_name"] {
        nonempty(name)?;
    }
    let template = object("template")?;
    reject_unknown_keys(template, "template", &["files", "sheet_name", "columns"])?;
    let strings = |obj: &serde_json::Map<String, Value>, name: &str, min: usize| -> CliResult<Vec<String>> {
        let values = obj
            .get(name)
            .and_then(Value::as_array)
            .ok_or_else(|| CliError::invalid(format!("source profile template.{name} 无效")))?;
        if values.len() < min {
            return Err(CliError::invalid(format!("source profile template.{name} 数量不足")));
        }
        let mut out = Vec::with_capacity(values.len());
        for value in values {
            let item = value
                .as_str()
                .filter(|v| !v.trim().is_empty())
                .ok_or_else(|| CliError::invalid(format!("source profile template.{name} 必须是非空字符串")))?;
            if out.iter().any(|seen| seen == item) {
                return Err(CliError::invalid(format!("source profile template.{name} 不能重复")));
            }
            out.push(item.to_string());
        }
        Ok(out)
    };
    strings(template, "files", 1)?;
    if template.get("sheet_name").and_then(Value::as_str).is_none_or(|v| v.trim().is_empty()) {
        return Err(CliError::invalid("source profile template.sheet_name 无效"));
    }
    strings(template, "columns", 2)?;
    let aliases = profile
        .get("field_aliases")
        .and_then(Value::as_object)
        .ok_or_else(|| CliError::invalid("source profile 缺少 field_aliases"))?;
    if aliases.is_empty() {
        return Err(CliError::invalid("source profile field_aliases 不能为空"));
    }
    let mut alias_owner = BTreeMap::<String, String>::new();
    for (field, values) in aliases {
        let values = values
            .as_array()
            .filter(|values| !values.is_empty())
            .ok_or_else(|| CliError::invalid(format!("source profile field_aliases.{field} 无效")))?;
        let mut local = BTreeSet::new();
        for alias in values.iter().filter_map(Value::as_str) {
            if alias.trim().is_empty() || !local.insert(normalize_header(alias)) {
                return Err(CliError::invalid(format!("source profile field_aliases.{field} 含空值或重复别名")));
            }
            let key = normalize_header(alias);
            if let Some(previous) = alias_owner.insert(key.clone(), field.clone()) {
                if previous != *field {
                    return Err(CliError::invalid(format!("source profile 跨字段重复别名：{key}")));
                }
            }
        }
    }
    let roles = profile
        .get("file_roles")
        .and_then(Value::as_array)
        .filter(|roles| !roles.is_empty())
        .ok_or_else(|| CliError::invalid("source profile 缺少或无效 file_roles"))?;
    let mut role_ids = BTreeSet::new();
    for role in roles {
        let role = role.as_object().ok_or_else(|| CliError::invalid("source profile file_role 无效"))?;
        reject_unknown_keys(
            role,
            "file_roles[]",
            &["role_id", "required", "file_matchers", "sheet_matchers", "field_map"],
        )?;
        let id = role.get("role_id").and_then(Value::as_str).unwrap_or_default();
        if id.is_empty() || !role_ids.insert(id.to_string())
            || !id.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            || !id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(CliError::invalid(format!("source profile role_id 重复或为空：{id}")));
        }
        if role.get("required").and_then(Value::as_bool).is_none()
            || role.get("file_matchers").and_then(Value::as_array).is_none()
            || role.get("sheet_matchers").and_then(Value::as_array).is_none()
            || role.get("field_map").and_then(Value::as_object).is_none()
        {
            return Err(CliError::invalid(format!("source profile role {id} 缺少 required/file_matchers/sheet_matchers/field_map")));
        }
    }
    let row_rules = object("row_rules")?;
    reject_unknown_keys(
        row_rules,
        "row_rules",
        &["header_rows_max", "detail_key_fields", "date_field", "amount_fields"],
    )?;
    if row_rules.get("header_rows_max").and_then(Value::as_u64).is_none_or(|v| !(1..=20).contains(&v))
        || row_rules.get("date_field").and_then(Value::as_str).is_none_or(|v| v.trim().is_empty())
        || row_rules.get("detail_key_fields").and_then(Value::as_array).is_none_or(|v| v.is_empty())
        || row_rules.get("amount_fields").and_then(Value::as_array).is_none_or(|v| v.is_empty())
    {
        return Err(CliError::invalid("source profile row_rules 无效"));
    }
    let normalization = object("normalization")?;
    reject_unknown_keys(
        normalization,
        "normalization",
        &["currency", "tax_basis", "timezone", "status_map"],
    )?;
    let currency = normalization.get("currency").and_then(Value::as_str).unwrap_or_default();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_uppercase())
        || normalization.get("timezone").and_then(Value::as_str).is_none_or(|v| v.trim().is_empty())
        || normalization.get("status_map").and_then(Value::as_object).is_none()
        || !matches!(normalization.get("tax_basis").and_then(Value::as_str), Some("inclusive" | "exclusive" | "not_applicable" | "confirm_at_runtime"))
    {
        return Err(CliError::invalid("source profile normalization 无效"));
    }
    let dedupe = object("dedupe")?;
    reject_unknown_keys(
        dedupe,
        "dedupe",
        &["same_content", "different_content", "detail_and_total"],
    )?;
    if !matches!(dedupe.get("same_content").and_then(Value::as_str), Some("dedupe" | "conflict"))
        || !matches!(dedupe.get("different_content").and_then(Value::as_str), Some("conflict" | "latest_revision" | "manual_review"))
        || !matches!(dedupe.get("detail_and_total").and_then(Value::as_str), Some("select_authoritative" | "manual_review" | "reject"))
    {
        return Err(CliError::invalid("source profile dedupe 无效"));
    }
    if let Some(record_mappings) = profile.get("record_mappings") {
        for mapping in record_mappings
            .as_array()
            .ok_or_else(|| CliError::invalid("source profile record_mappings 无效"))?
        {
            reject_unknown_keys(
                mapping
                    .as_object()
                    .ok_or_else(|| CliError::invalid("source profile record_mapping 无效"))?,
                "record_mappings[]",
                &[
                    "mapping_id", "file_matchers", "sheet_matchers", "metric_key", "value_fields",
                    "date_fields", "status_field", "accepted_statuses", "requires_confirmation",
                    "period_kind", "scope_field",
                ],
            )?;
        }
    }
    Ok(())
}

fn profile_aliases(profile: &SourceProfileBinding) -> Option<BTreeMap<String, Vec<String>>> {
    let mut aliases: BTreeMap<String, Vec<String>> = profile
        .config
        .as_ref()
        .and_then(|config| config.get("field_aliases"))
        .and_then(Value::as_object)
        .map(|obj| {
            obj.iter()
                .filter_map(|(field, aliases)| {
                    Some((field.clone(), aliases.as_array()?.iter().filter_map(Value::as_str).map(str::to_string).collect()))
                })
                .collect()
        })?;
    if let Some(config) = profile.config.as_ref() {
        if let Some(roles) = config.get("file_roles").and_then(Value::as_array) {
            for role in roles {
                if let Some(field_map) = role.get("field_map").and_then(Value::as_object) {
                    for (canonical, source_name) in field_map {
                        if let Some(source_name) = source_name.as_str() {
                            let values = aliases.entry(canonical.clone()).or_default();
                            if !values.iter().any(|value| normalize_header(value) == normalize_header(source_name)) {
                                values.push(source_name.to_string());
                            }
                        }
                    }
                }
            }
        }
        if let Some(max_rows) = config
            .get("row_rules")
            .and_then(|rules| rules.get("header_rows_max"))
            .and_then(Value::as_u64)
        {
            aliases.insert("__header_rows_max".into(), vec![max_rows.to_string()]);
        }
    }
    Some(aliases)
}

pub fn profile_aliases_for_binding(
    profile: &SourceProfileBinding,
) -> Option<BTreeMap<String, Vec<String>>> {
    profile_aliases(profile)
}

// ---- 扫描 ----

/// 单个扫描到的 sheet 摘要。
pub struct ScannedSheet {
    pub sheet: Option<String>,
    pub role: &'static str,
    pub header: Option<HeaderBlock>,
    pub match_info: Option<SheetMatch>,
    pub rows: usize,
    /// 生成结果工作表不可作为 source 输入时的机器原因。
    pub blocked_reason: Option<&'static str>,
    /// profile file_roles 命中的动态角色；固定 role 字段继续用于旧视图兼容。
    pub profile_role_id: Option<String>,
}

pub struct ScannedFile {
    pub file_label: String,
    pub content_hash: String,
    pub kind: String,
    pub sheets: Vec<(SheetData, ScannedSheet)>,
}

/// 读目录下全部支持文件（xlsx/xls/csv；其他扩展名跳过并记 pending）。
/// `metric_names` 为工作区指标名全集（转置推移表锚点用；空 = 不启用转置识别）。
pub fn scan_directory(
    dir: &str,
    metric_names: &[String],
) -> CliResult<(Vec<ScannedFile>, Vec<String>)> {
    scan_directory_with_profile(dir, metric_names, None)
}

pub fn scan_directory_with_profile(
    dir: &str,
    metric_names: &[String],
    aliases: Option<&BTreeMap<String, Vec<String>>>,
) -> CliResult<(Vec<ScannedFile>, Vec<String>)> {
    scan_directory_with_profile_config(dir, metric_names, aliases, None)
}

pub fn scan_directory_with_profile_config(
    dir: &str,
    metric_names: &[String],
    aliases: Option<&BTreeMap<String, Vec<String>>>,
    profile: Option<&SourceProfileBinding>,
) -> CliResult<(Vec<ScannedFile>, Vec<String>)> {
    let root = Path::new(dir);
    if !root.is_dir() {
        return Err(
            CliError::new(CliErrorCode::DataValidationFailed, "--dir 不是有效目录")
                .with_field("--dir"),
        );
    }
    let mut files = Vec::new();
    let mut pending = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(root)
        .map_err(|e| CliError::new(CliErrorCode::DataDirectoryUnavailable, e.to_string()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let label = entry.file_name().to_string_lossy().to_string();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .unwrap_or_default();
        let sheets = match ext.as_str() {
            "xlsx" | "xls" => dsheng_core::import::fileio::read_workbook(&path)
                .map_err(|e| CliError::new(CliErrorCode::DataValidationFailed, e.to_string()))?,
            "csv" => vec![dsheng_core::import::fileio::read_csv(&path)
                .map_err(|e| CliError::new(CliErrorCode::DataValidationFailed, e.to_string()))?],
            other => {
                pending.push(format!("unsupported_file_kind:{label}({other})"));
                continue;
            }
        };
        let bytes = std::fs::read(&path)
            .map_err(|e| CliError::new(CliErrorCode::DataDirectoryUnavailable, e.to_string()))?;
        files.push(ScannedFile {
            file_label: label.clone(),
            content_hash: sha256_bytes(&bytes),
            kind: ext,
            sheets: sheets
                .into_iter()
                .map(|s| {
                    let (sheet, mut scanned) = analyze_sheet_with_aliases(s, metric_names, aliases);
                    if let Some(profile) = profile {
                        scanned.profile_role_id = profile_role_for_sheet(profile, &label, scanned.sheet.as_deref());
                    }
                    (sheet, scanned)
                })
                .collect(),
        });
    }
    if let Some(profile) = profile.filter(|p| p.mode == "profile") {
        if let Some(roles) = profile
            .config
            .as_ref()
            .and_then(|config| config.get("file_roles"))
            .and_then(Value::as_array)
        {
            for role in roles.iter().filter(|role| role.get("required").and_then(Value::as_bool) == Some(true)) {
                let Some(role_id) = role.get("role_id").and_then(Value::as_str) else { continue };
                let found = files.iter().any(|file| file.sheets.iter().any(|(_, sheet)| {
                    sheet.profile_role_id.as_deref() == Some(role_id)
                        && sheet.blocked_reason.is_none()
                }));
                if !found {
                    pending.push(format!("required_profile_role_missing:{role_id}"));
                }
            }
        }
    }
    Ok((files, pending))
}

/// 识别一个 sheet 的角色与表头块（core §4.1/§4.3；含转置推移表锚点）。
/// 传入全部行（core 表头搜索内部自限首 50 行；转置锚点的期间数据行
/// 不受 50 行截断，避免长推移表丢行）。
fn analyze_sheet_with_aliases(
    sheet: SheetData,
    metric_names: &[String],
    aliases: Option<&BTreeMap<String, Vec<String>>>,
) -> (SheetData, ScannedSheet) {
    let header_rows: Vec<Vec<String>> = sheet
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.raw_text()).collect())
        .collect();
    let match_info = detect_sheet_role_with_names(&sheet.name, &header_rows, metric_names);
    let profile_header = if match_info.is_none() {
        aliases.and_then(|a| profile_header_block(&header_rows, a))
    } else {
        None
    };
    let generated = dsheng_core::import::is_generated_output_sheet_name(&sheet.name);
    let role = match match_info.as_ref().map(|m| m.role) {
        Some(r) => r,
        None => SheetRole::GenericTable,
    };
    let header_rows_end = profile_header
        .as_ref()
        .or_else(|| match_info.as_ref().and_then(|m| m.header.as_ref()))
        .map(|h| h.row + h.row_count - 1)
        .unwrap_or(0);
    let scanned = ScannedSheet {
        sheet: Some(sheet.name.clone()),
        role: if generated {
            "generated_output_input_not_allowed"
        } else {
            role.as_str()
        },
        header: profile_header.clone().or_else(|| match_info.as_ref().and_then(|m| m.header.clone())),
        match_info,
        rows: sheet.rows.len().saturating_sub(header_rows_end),
        blocked_reason: generated.then_some("generated_output_input_not_allowed"),
        profile_role_id: None,
    };
    (sheet, scanned)
}

fn profile_header_block(
    rows: &[Vec<String>],
    aliases: &BTreeMap<String, Vec<String>>,
) -> Option<HeaderBlock> {
    let known = [
        "period", "metric_label", "unit", "planned", "actual", "annual_month_plan",
        "owner", "due_date", "category", "source", "amount", "business_date",
    ];
    let mut alias_to_field: BTreeMap<String, String> = BTreeMap::new();
    for field in known {
        alias_to_field.insert(normalize_header(field), field.to_string());
        if let Some(items) = aliases.get(field) {
            for alias in items {
                alias_to_field.insert(normalize_header(alias), field.to_string());
            }
        }
    }
    for (field, items) in aliases {
        if field.starts_with("__") {
            continue;
        }
        for alias in items {
            alias_to_field.insert(normalize_header(alias), field.clone());
        }
    }
    let header_rows_max = aliases
        .get("__header_rows_max")
        .and_then(|values| values.first())
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(20);
    for (row_idx, row) in rows.iter().take(header_rows_max).enumerate() {
        let columns: Vec<(usize, String)> = row
            .iter()
            .enumerate()
            .filter_map(|(column, raw)| {
                let field = alias_to_field.get(&normalize_header(raw))?;
                Some((column, field.clone()))
            })
            .collect();
        if columns.len() >= 2 {
            return Some(HeaderBlock {
                row: row_idx + 1,
                row_count: 1,
                columns,
                core_score: 0,
            });
        }
    }
    None
}

fn profile_role_for_sheet(
    profile: &SourceProfileBinding,
    file_label: &str,
    sheet_name: Option<&str>,
) -> Option<String> {
    let roles = profile.config.as_ref()?.get("file_roles")?.as_array()?;
    roles.iter().find_map(|role| {
        let file_matchers = role.get("file_matchers")?.as_array()?;
        let sheet_matchers = role.get("sheet_matchers")?.as_array()?;
        let file_ok = file_matchers.is_empty() || file_matchers.iter().filter_map(Value::as_str).any(|m| file_label.to_lowercase().contains(&m.to_lowercase()));
        let sheet = sheet_name.unwrap_or_default();
        let sheet_ok = sheet_matchers.is_empty() || sheet_matchers.iter().filter_map(Value::as_str).any(|m| sheet.to_lowercase().contains(&m.to_lowercase()));
        if file_ok && sheet_ok {
            role.get("role_id").and_then(Value::as_str).map(str::to_string)
        } else {
            None
        }
    })
}

/// 注册 source_files（同 content_hash 复用 source_id；仅本机 DB，不进导出）。
pub fn register_sources(
    conn: &Connection,
    workspace_id: Uuid,
    files: &[ScannedFile],
) -> CliResult<Vec<Value>> {
    register_sources_with_profile(conn, workspace_id, files, &SourceProfileBinding::generic())
}

/// 注册 source_files 并持久化本次选择的 profile manifest。
pub fn register_sources_with_profile(
    conn: &Connection,
    workspace_id: Uuid,
    files: &[ScannedFile],
    profile: &SourceProfileBinding,
) -> CliResult<Vec<Value>> {
    ensure_sidecar_tables(conn)?;
    let mut views = Vec::new();
    for file in files {
        let existing: Option<String> = conn
            .query_row(
                "SELECT source_id FROM source_files WHERE workspace_id = ?1 AND content_hash = ?2",
                params![workspace_id.to_string(), file.content_hash],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(CliError::from)?;
        let source_id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = now_iso();
        conn.execute(
            "INSERT INTO source_files (source_id, workspace_id, file_label, path_ref, kind,
                    content_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(source_id) DO UPDATE SET
                file_label = excluded.file_label, kind = excluded.kind,
                content_hash = excluded.content_hash, updated_at = excluded.updated_at",
            params![
                source_id,
                workspace_id.to_string(),
                file.file_label,
                file.file_label,
                file.kind,
                file.content_hash,
                now
            ],
        )
        .map_err(CliError::from)?;
        conn.execute(
            "INSERT INTO cli_source_profiles
             (workspace_id, source_id, profile_id, profile_version, profile_hash, profile_mode, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(workspace_id, source_id) DO UPDATE SET
               profile_id=excluded.profile_id, profile_version=excluded.profile_version,
               profile_hash=excluded.profile_hash, profile_mode=excluded.profile_mode,
               updated_at=excluded.updated_at",
            params![workspace_id.to_string(), source_id, profile.profile_id, profile.version, profile.hash, profile.mode, now_iso()],
        )
        .map_err(CliError::from)?;
        let total_rows: usize = file.sheets.iter().map(|(_, s)| s.rows).sum();
        let mapped = file.sheets.iter().any(|(_, s)| {
            matches!(s.role, "accounting" | "trend" | "task_review" | "task_plan")
                || (profile.mode != "generic"
                    && s.role == "generic_table"
                    && s.header.is_some()
                    && s.blocked_reason.is_none())
        });
        // 严格 scanFileInfo（additionalProperties:false：file_id/file_label/row_count/
        // sheet_count/mapping_status/note；来源哈希等审计信息不进该视图）
        let note = file
            .sheets
            .iter()
            .map(|(_, s)| {
                if let Some(reason) = s.blocked_reason {
                    format!(
                        "{}:{}{}({}行)",
                        s.sheet.clone().unwrap_or_default(),
                        s.profile_role_id.as_deref().map(|id| format!("/profile:{id}")).unwrap_or_default(),
                        reason,
                        s.rows
                    )
                } else {
                    format!(
                        "{}:{}{}({}行)",
                        s.sheet.clone().unwrap_or_default(),
                        s.role,
                        s.profile_role_id.as_deref().map(|id| format!("/profile:{id}")).unwrap_or_default(),
                        s.rows
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("；");
        views.push(json!({
            "file_id": source_id,
            "file_label": file.file_label,
            "row_count": total_rows,
            "sheet_count": file.sheets.len(),
            "mapping_status": if mapped { "mapped" } else { "pending_review" },
            "note": note,
        }));
    }
    Ok(views)
}

// ---- mapping（fieldMappingView + 版本化草稿） ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldMappingEntry {
    pub tool_field: String,
    pub source_label: String,
    pub recognized: String,
    pub usage: String,
    /// 手工 generic 映射的稳定定位信息；旧 mapping JSON 缺省时仍兼容。
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub sheet: Option<String>,
    #[serde(default)]
    pub column: Option<String>,
    #[serde(default)]
    pub header_signature: Option<String>,
    #[serde(default)]
    pub target_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingSetInput {
    pub expected_revision: u32,
    pub mappings: Vec<FieldMappingEntry>,
}

/// generic source 在首次分析前的明确映射确认。它与 run 级 mapping draft 分开，
/// 因为首次分析尚未创建 run；确认绑定目录和 profile hash，目录或 profile 变化会重新要求确认。
pub fn source_mapping_confirmed(
    conn: &Connection,
    workspace_id: Uuid,
    directory_label: &str,
    profile_hash: &str,
) -> CliResult<Option<Vec<FieldMappingEntry>>> {
    ensure_sidecar_tables(conn)?;
    let raw: Option<String> = conn
        .query_row(
            "SELECT mapping_json FROM cli_source_mapping_sets
             WHERE workspace_id = ?1 AND directory_label = ?2 AND profile_hash = ?3",
            params![workspace_id.to_string(), directory_label, profile_hash],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    raw.map(|value| {
        serde_json::from_str(&value)
            .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "来源映射确认 JSON 损坏"))
    })
    .transpose()
}

pub fn source_mapping_set(
    conn: &Connection,
    workspace_id: Uuid,
    directory_label: &str,
    profile_hash: &str,
    mappings: Vec<FieldMappingEntry>,
) -> CliResult<Value> {
    if directory_label.trim().is_empty() || profile_hash.trim().is_empty() {
        return Err(CliError::invalid("directory_label/profile_hash 不能为空"));
    }
    ensure_sidecar_tables(conn)?;
    let mapping_json = serde_json::to_string(&mappings).map_err(|e| CliError::invalid(e.to_string()))?;
    conn.execute(
        "INSERT INTO cli_source_mapping_sets
         (workspace_id, directory_label, profile_hash, mapping_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(workspace_id, directory_label, profile_hash) DO UPDATE SET
           mapping_json=excluded.mapping_json, updated_at=excluded.updated_at",
        params![workspace_id.to_string(), directory_label, profile_hash, mapping_json, now_iso()],
    )
    .map_err(CliError::from)?;
    Ok(json!({
        "workspace_id": workspace_id,
        "directory_label": directory_label,
        "profile_hash": profile_hash,
        "confirmed": true,
        "mapping_count": mappings.len(),
    }))
}

/// 根据期间 key 读取最近一次 mapping 草稿；mapping set 后 analyze 才会消费。
pub fn mapping_draft_for_period(
    conn: &Connection,
    workspace_id: Uuid,
    period_key: &str,
) -> CliResult<Option<(u32, Vec<FieldMappingEntry>)>> {
    let run_id: Option<String> = conn
        .query_row(
            "SELECT run_id FROM analysis_runs WHERE workspace_id = ?1 AND period_key = ?2",
            params![workspace_id.to_string(), period_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let Some(run_id) = run_id else {
        return Ok(None);
    };
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT revision, draft_json FROM cli_mapping_drafts WHERE run_id = ?1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let Some((revision, raw)) = row else {
        return Ok(None);
    };
    let mappings = serde_json::from_str::<Vec<FieldMappingEntry>>(&raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "草稿 JSON 损坏"))?;
    Ok(Some((revision as u32, mappings)))
}

/// 由表头块生成默认映射视图（recognized=列字母；usage=规范字段）。
pub fn default_mapping_from_header(header: &HeaderBlock) -> Vec<Value> {
    header
        .columns
        .iter()
        .map(|(col, field)| {
            json!({
                "tool_field": field,
                "source_label": format!("{}列", column_letter(*col)),
                "recognized": column_letter(*col),
                "usage": format!("规范字段 {field}（exact 别名命中）"),
            })
        })
        .collect()
}

/// mapping show：草稿优先，否则 run 当前 revision 的 mapping 快照。
pub fn mapping_show(conn: &Connection, workspace_id: Uuid, run_id: &str) -> CliResult<Value> {
    ensure_sidecar_tables(conn)?;
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let draft: Option<(i64, String)> = conn
        .query_row(
            "SELECT revision, draft_json FROM cli_mapping_drafts WHERE run_id = ?1",
            params![run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let (current, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    if let Some((revision, draft_json)) = draft {
        let entries: Value = serde_json::from_str(&draft_json)
            .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "草稿 JSON 损坏"))?;
        return Ok(json!({
            "run_id": run_id,
            "revision": revision,
            "current_run_revision": current,
            "pending_draft": true,
            "analysis_input_fingerprint": fingerprint,
            "mappings": entries,
        }));
    }
    let snapshot: Option<String> = conn
        .query_row(
            "SELECT mapping_snapshot_json FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, current as i64],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    Ok(json!({
        "run_id": run_id,
        "revision": current,
        "current_run_revision": current,
        "pending_draft": false,
        "analysis_input_fingerprint": fingerprint,
        "mappings": snapshot
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .unwrap_or_else(|| json!([])),
    }))
}

/// mapping set：原子版本化草稿（expected_revision 校验；revision+1）。
pub fn mapping_set(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    input: MappingSetInput,
) -> CliResult<Value> {
    ensure_sidecar_tables(conn)?;
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let current_draft: Option<i64> = conn
        .query_row(
            "SELECT revision FROM cli_mapping_drafts WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let (run_current, _) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let base = current_draft.unwrap_or(run_current as i64);
    if input.expected_revision != base as u32 {
        return Err(CliError::new(
            CliErrorCode::RevisionConflict,
            format!(
                "expected mapping revision {}，当前 {base}",
                input.expected_revision
            ),
        ));
    }
    if input.mappings.is_empty() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "mappings 不能为空",
        ));
    }
    let serialized = serde_json::to_string(&input.mappings).expect("json");
    conn.execute(
        "INSERT INTO cli_mapping_drafts (run_id, workspace_id, revision, draft_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(run_id) DO UPDATE SET
            revision = excluded.revision, draft_json = excluded.draft_json,
            updated_at = excluded.updated_at",
        params![
            run_id,
            workspace_id.to_string(),
            base + 1,
            serialized,
            now_iso()
        ],
    )
    .map_err(CliError::from)?;
    Ok(json!({
        "run_id": run_id,
        "revision": base + 1,
        "pending_draft": true,
        "analysis_recalculation_required": true,
    }))
}

// ---- manual inputs ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualInputSet {
    pub metric_id: String,
    pub measure_role: String,
    pub scope_key: String,
    pub value: Option<String>,
    #[serde(default)]
    pub unit: Option<String>,
    pub confirmed: bool,
    pub source_note: String,
    #[serde(default)]
    pub raw_text: Option<String>,
    pub expected_revision: u32,
}

pub fn manual_input_list(
    conn: &Connection,
    workspace_id: Uuid,
    period_key: &str,
) -> CliResult<Value> {
    let mut stmt = conn.prepare(
        "SELECT input_id, metric_id, period_key, measure_role, scope_key, value, unit, confirmed,
                source_note, input_revision, raw_text
         FROM manual_inputs WHERE workspace_id = ?1 AND period_key = ?2 ORDER BY metric_id",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string(), period_key], |r| {
        let value: Option<String> = r.get(5)?;
        let value_json = match &value {
            Some(v) => match dsheng_core::contracts::decimal::canonical_decimal(v) {
                Ok(d) => json!(dsheng_core::contracts::decimal::to_canonical_string(d)),
                Err(_) => json!(v.as_str()),
            },
            None => json!(null),
        };
        Ok(json!({
            "input_id": r.get::<_, String>(0)?,
            "workspace_id": workspace_id.to_string(),
            "metric_id": r.get::<_, String>(1)?,
            "period_key": r.get::<_, String>(2)?,
            "measure_role": r.get::<_, String>(3)?,
            "scope_key": r.get::<_, String>(4)?,
            "value": value_json,
            "unit": r.get::<_, Option<String>>(6)?,
            "confirmed": r.get::<_, i64>(7)? == 1,
            "source_note": r.get::<_, String>(8)?,
            "input_revision": r.get::<_, i64>(9)?,
            "raw_text": r.get::<_, Option<String>>(10)?,
            "evidence_id": format!("manual:{}:r{}", r.get::<_, String>(0)?, r.get::<_, i64>(9)?),
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(json!({ "period_key": period_key, "inputs": out }))
}

/// set：upsert（workspace+period_key+metric_id 唯一）；无隐式重分析。
pub fn manual_input_set(
    conn: &mut Connection,
    workspace_id: Uuid,
    period_key: &str,
    input: ManualInputSet,
) -> CliResult<Value> {
    if Uuid::parse_str(&input.metric_id).is_err() {
        return Err(CliError::invalid("metric_id 必须是 UUID"));
    }
    if !matches!(
        input.measure_role.as_str(),
        "actual" | "plan" | "annual_month_plan"
    ) {
        return Err(CliError::invalid(
            "measure_role 必须是 actual/plan/annual_month_plan",
        ));
    }
    if input.source_note.trim().is_empty() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "source_note 必填非空",
        ));
    }
    if input.scope_key.trim().is_empty() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "scope_key 必填非空",
        ));
    }
    if let Some(v) = &input.value {
        dsheng_core::contracts::decimal::parse_input_decimal(v).map_err(|e| {
            CliError::new(
                CliErrorCode::DataValidationFailed,
                format!("value 非规范 decimal: {e}"),
            )
        })?;
    }
    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let metric_registered: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM metric_definitions
             WHERE metric_id = ?1 AND workspace_id = ?2",
            params![input.metric_id, workspace_id.to_string()],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    if !metric_registered {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "metric_id 未在工作区注册",
        ));
    }
    let current: Option<i64> = tx
        .query_row(
            "SELECT input_revision FROM manual_inputs
             WHERE workspace_id = ?1 AND period_key = ?2 AND metric_id = ?3",
            params![workspace_id.to_string(), period_key, input.metric_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let next_rev = match current {
        None => {
            if input.expected_revision != 0 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    "新手工输入 expected_revision 应为 0",
                ));
            }
            1
        }
        Some(cur) => {
            if input.expected_revision != cur as u32 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    format!("expected_revision {}，当前 {cur}", input.expected_revision),
                ));
            }
            cur + 1
        }
    };
    let input_id = Uuid::new_v4().to_string();
    let now = now_iso();
    let raw_text = input
        .raw_text
        .or_else(|| input.value.clone())
        .unwrap_or_default();
    tx.execute(
        "INSERT INTO manual_inputs (input_id, workspace_id, metric_id, period_key, measure_role,
                scope_key, value, unit, confirmed, source_note, input_revision, raw_text,
                created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
         ON CONFLICT(workspace_id, period_key, metric_id) DO UPDATE SET
            measure_role = excluded.measure_role, scope_key = excluded.scope_key,
            value = excluded.value, unit = excluded.unit, confirmed = excluded.confirmed,
            source_note = excluded.source_note, input_revision = excluded.input_revision,
            raw_text = excluded.raw_text, updated_at = excluded.updated_at",
        params![
            input_id,
            workspace_id.to_string(),
            input.metric_id,
            period_key,
            input.measure_role,
            input.scope_key,
            input.value,
            input.unit,
            input.confirmed as i64,
            input.source_note.trim(),
            next_rev,
            raw_text,
            now
        ],
    )
    .map_err(CliError::from)?;
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    Ok(json!({
        "input_id": input_id,
        "metric_id": input.metric_id,
        "period_key": period_key,
        "input_revision": next_rev,
        "reanalysis_required": true,
    }))
}

// ---- metric definitions ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinitionSet {
    pub metric_key: String,
    pub label: String,
    /// 显示别名（行名识别用）。缺省=继承上一版本别名；显式给 [] 表示清空。
    #[serde(default)]
    pub label_aliases: Option<Vec<String>>,
    pub semantic_type: String,
    pub unit_signature: Value,
    pub direction: String,
    pub target_constraint_scope: String,
    #[serde(default)]
    pub revenue_semantics: bool,
    pub aggregation: String,
    #[serde(default)]
    pub formula: Option<Value>,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    #[serde(default = "default_denominator")]
    pub denominator_policy: String,
    pub comparison_policy: Value,
    pub effective_from: String,
    #[serde(default)]
    pub labor_treatment: Option<String>,
    #[serde(default)]
    pub scope_key: Option<String>,
    pub expected_revision: u32,
}

fn default_denominator() -> String {
    "positive_only".into()
}

fn validation_error(text: impl Into<String>) -> CliError {
    CliError::new(CliErrorCode::DataValidationFailed, text.into())
}

/// metric set：新版本（同 metric_key 递增 version；新 metric_id）。
pub fn metric_set(
    conn: &mut Connection,
    workspace_id: Uuid,
    input: MetricDefinitionSet,
) -> CliResult<Value> {
    for (field, value, allowed) in [
        (
            "semantic_type",
            &input.semantic_type,
            &["flow", "stock", "ratio", "configured_derived"][..],
        ),
        (
            "direction",
            &input.direction,
            &["up", "down", "neutral"][..],
        ),
        (
            "target_constraint_scope",
            &input.target_constraint_scope,
            &["monthly_revenue_floor", "explicit_target_only", "none"][..],
        ),
        (
            "aggregation",
            &input.aggregation,
            &[
                "sum_disjoint",
                "configured_recompute",
                "recompute_after_aggregation",
                "ratio_of_aggregated_inputs",
                "opening_boundary",
                "closing_balance_or_rollforward",
                "closing_boundary",
                "sum_item_closing_values",
            ][..],
        ),
    ] {
        if !allowed.contains(&value.as_str()) {
            return Err(CliError::new(
                CliErrorCode::DataValidationFailed,
                format!("{field} 取值不在合同枚举内"),
            )
            .with_field(field));
        }
    }
    if input.target_constraint_scope == "monthly_revenue_floor"
        && !(input.revenue_semantics && input.direction == "up")
    {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "monthly_revenue_floor 仅限 revenue_semantics=true 且 direction=up",
        ));
    }
    let unit: UnitSignature = serde_json::from_value(input.unit_signature.clone())
        .map_err(|e| validation_error(format!("unit_signature 非法: {e}")))?;
    unit.validate().map_err(validation_error)?;
    let formula_node: Option<FormulaNode> = match &input.formula {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| validation_error(format!("formula AST 非法: {e}")))?,
        ),
    };
    let comparison: ComparisonPolicy = serde_json::from_value(input.comparison_policy.clone())
        .map_err(|_| {
            validation_error(
                "comparison_policy 固定 positive_base=relative_change、zero_or_negative_base=absolute_only",
            )
        })?;
    let _ = comparison;
    chrono::NaiveDate::parse_from_str(&input.effective_from, "%Y-%m-%d")
        .map_err(|_| validation_error("effective_from 必须是 YYYY-MM-DD"))?;

    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let current_version: Option<i64> = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM metric_definitions
             WHERE workspace_id = ?1 AND metric_key = ?2",
            params![workspace_id.to_string(), input.metric_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let next_version = match current_version {
        None => {
            if input.expected_revision != 0 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    "新指标定义 expected_revision 应为 0",
                ));
            }
            1
        }
        Some(cur) => {
            if input.expected_revision != cur as u32 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    format!("expected_revision {}，当前 {cur}", input.expected_revision),
                ));
            }
            cur + 1
        }
    };
    // 别名继承：未提供 label_aliases 时沿用上一版（指标行名识别连续性）
    let effective_aliases: Vec<String> = match &input.label_aliases {
        Some(list) => list.clone(),
        None => {
            let prev: Option<String> = tx
                .query_row(
                    "SELECT label_aliases_json FROM metric_definitions
                     WHERE workspace_id = ?1 AND metric_key = ?2
                     ORDER BY version DESC LIMIT 1",
                    params![workspace_id.to_string(), input.metric_key],
                    |r| r.get(0),
                )
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })
                .map_err(CliError::from)?;
            prev.and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
                .unwrap_or_default()
        }
    };

    // 定义身份（§1.4）：同 metric_key 升版本保留同一 stable_metric_id（首版行 id）
    let inherited_stable_id: Option<String> = tx
        .query_row(
            "SELECT stable_metric_id FROM metric_definitions
             WHERE workspace_id = ?1 AND metric_key = ?2
             ORDER BY version DESC LIMIT 1",
            params![workspace_id.to_string(), input.metric_key],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    if (next_version == 1) != inherited_stable_id.is_none() {
        return Err(CliError::new(
            CliErrorCode::LocalSaveFailed,
            "stable_metric_id 身份读取与版本推进不一致（库状态异常）",
        ));
    }
    // 公式引用：AST 结构 + 引用键（自身/required_inputs/已注册 key）
    if let Some(node) = &formula_node {
        let mut known: std::collections::HashSet<String> =
            input.required_inputs.iter().cloned().collect();
        known.insert(input.metric_key.clone());
        {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT metric_key FROM metric_definitions WHERE workspace_id = ?1",
            )?;
            let keys: Vec<String> = stmt
                .query_map(params![workspace_id.to_string()], |r| r.get(0))?
                .collect::<Result<_, _>>()
                .map_err(CliError::from)?;
            for k in keys {
                known.insert(k);
            }
        }
        let refs: std::collections::HashSet<String> = {
            let mut raw = Vec::new();
            node.collect_refs(&mut raw);
            raw.into_iter().map(|s| s.to_string()).collect()
        };
        let known_refs: std::collections::HashSet<&str> =
            known.iter().map(|s| s.as_str()).collect();
        node.validate(&known_refs)
            .map_err(|e| validation_error(format!("formula 非法: {e}")))?;
        for r in &refs {
            if !known.contains(r) {
                return Err(CliError::new(
                    CliErrorCode::DataValidationFailed,
                    format!("formula 引用未注册指标 {r}"),
                ));
            }
        }
    }
    // semantic hash（core 材料；依赖 hash 取本地最新版本）
    let mut dependencies = BTreeMap::new();
    {
        let mut stmt = tx.prepare(
            "SELECT m.metric_key, m.semantic_hash FROM metric_definitions m
             WHERE m.workspace_id = ?1 AND m.version =
                   (SELECT MAX(version) FROM metric_definitions m2
                    WHERE m2.workspace_id = m.workspace_id AND m2.metric_key = m.metric_key)",
        )?;
        let rows: Vec<(String, String)> = stmt
            .query_map(params![workspace_id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?
            .collect::<Result<_, _>>()
            .map_err(CliError::from)?;
        for (k, h) in rows {
            dependencies.insert(k, h);
        }
    }
    let projection = MetricDefinitionProjection {
        metric_id: Uuid::new_v4(),
        metric_key: input.metric_key.clone(),
        version: next_version as u32,
        semantic_hash: String::new(),
        label: input.label.clone(),
        semantic_type: serde_json::from_value(json!(&input.semantic_type))
            .map_err(|e| validation_error(e.to_string()))?,
        unit_signature: unit,
        direction: serde_json::from_value(json!(&input.direction))
            .map_err(|e| validation_error(e.to_string()))?,
        target_constraint_scope: serde_json::from_value(json!(&input.target_constraint_scope))
            .map_err(|e| validation_error(e.to_string()))?,
        revenue_semantics: input.revenue_semantics,
        aggregation: serde_json::from_value(json!(&input.aggregation))
            .map_err(|e| validation_error(e.to_string()))?,
        formula: formula_node,
        required_inputs: input.required_inputs.clone(),
        denominator_policy: serde_json::from_value(json!("positive_only"))
            .map_err(|e| validation_error(e.to_string()))?,
        comparison_policy: ComparisonPolicy {
            positive_base: serde_json::from_value(json!("relative_change"))
                .map_err(|e| validation_error(e.to_string()))?,
            zero_or_negative_base: serde_json::from_value(json!("absolute_only"))
                .map_err(|e| validation_error(e.to_string()))?,
        },
        effective_from: chrono::NaiveDate::parse_from_str(&input.effective_from, "%Y-%m-%d")
            .map_err(|_| validation_error("effective_from 非法"))?,
        scope_key: input.scope_key.clone().unwrap_or_else(|| "全公司".into()),
    };
    let _ = DenominatorPolicy::PositiveOnly;
    let _ = SemanticType::Flow;
    let _ = TargetConstraintScope::None;
    let _ = Aggregation::SumDisjoint;
    let semantic_hash = compute_semantic_hash(
        &projection,
        input.labor_treatment.as_deref().and_then(parse_labor),
        &dependencies,
    );
    let metric_id = Uuid::new_v4().to_string();
    let stable_metric_id = inherited_stable_id.unwrap_or_else(|| metric_id.clone());
    tx.execute(
        "INSERT INTO metric_definitions (metric_id, workspace_id, metric_key, version,
                stable_metric_id, semantic_hash, label, label_aliases_json, semantic_type,
                unit_signature_json, direction, target_constraint_scope, revenue_semantics,
                aggregation, formula_json, required_inputs_json, denominator_policy,
                comparison_policy_json, labor_treatment, effective_from, effective_to, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?20, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 'positive_only', ?16, ?17, ?18, NULL, ?19)",
        params![
            metric_id,
            workspace_id.to_string(),
            input.metric_key,
            next_version,
            stable_metric_id,
            semantic_hash,
            input.label,
            input.semantic_type,
            serde_json::to_string(&input.unit_signature).unwrap(),
            input.direction,
            input.target_constraint_scope,
            input.revenue_semantics as i64,
            input.aggregation,
            input
                .formula
                .as_ref()
                .map(|f| serde_json::to_string(f).unwrap()),
            serde_json::to_string(&input.required_inputs).unwrap(),
            serde_json::to_string(&input.comparison_policy).unwrap(),
            input.labor_treatment,
            input.effective_from,
            now_iso(),
            serde_json::to_string(&effective_aliases).unwrap(),
        ],
    )
    .map_err(CliError::from)?;
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    metric_view(conn, workspace_id, &metric_id)
}

fn parse_labor(text: &str) -> Option<dsheng_core::contracts::metrics::LaborTreatment> {
    match text {
        "included_in_expense" => {
            Some(dsheng_core::contracts::metrics::LaborTreatment::IncludedInExpense)
        }
        "separate_from_expense" => {
            Some(dsheng_core::contracts::metrics::LaborTreatment::SeparateFromExpense)
        }
        _ => None,
    }
}

pub fn metric_view(conn: &Connection, workspace_id: Uuid, metric_id: &str) -> CliResult<Value> {
    conn.query_row(
        "SELECT metric_id, stable_metric_id, metric_key, version, semantic_hash, label,
                semantic_type, unit_signature_json, direction, target_constraint_scope,
                revenue_semantics, aggregation, formula_json, required_inputs_json,
                denominator_policy, comparison_policy_json, labor_treatment, effective_from,
                effective_to, created_at
         FROM metric_definitions WHERE metric_id = ?1 AND workspace_id = ?2",
        params![metric_id, workspace_id.to_string()],
        |r| {
            Ok(json!({
                "metric_id": r.get::<_, String>(0)?,
                "stable_metric_id": r.get::<_, String>(1)?,
                "workspace_id": workspace_id.to_string(),
                "metric_key": r.get::<_, String>(2)?,
                "version": r.get::<_, i64>(3)?,
                "semantic_hash": r.get::<_, String>(4)?,
                "label": r.get::<_, String>(5)?,
                "semantic_type": r.get::<_, String>(6)?,
                "unit_signature": serde_json::from_str::<Value>(&r.get::<_, String>(7)?).unwrap_or(json!(null)),
                "direction": r.get::<_, String>(8)?,
                "target_constraint_scope": r.get::<_, String>(9)?,
                "revenue_semantics": r.get::<_, i64>(10)? == 1,
                "aggregation": r.get::<_, String>(11)?,
                "formula": r.get::<_, Option<String>>(12)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
                "required_inputs": serde_json::from_str::<Value>(&r.get::<_, String>(13)?).unwrap_or(json!([])),
                "denominator_policy": r.get::<_, String>(14)?,
                "comparison_policy": serde_json::from_str::<Value>(&r.get::<_, String>(15)?).unwrap_or(json!({})),
                "labor_treatment": r.get::<_, Option<String>>(16)?,
                "effective_from": r.get::<_, String>(17)?,
                "effective_to": r.get::<_, Option<String>>(18)?,
                "created_at": r.get::<_, String>(19)?,
            }))
        },
    )
    .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "metric 定义不存在"))
}

pub fn metric_list(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT metric_id FROM metric_definitions WHERE workspace_id = ?1
         ORDER BY metric_key, version",
    )?;
    let ids: Vec<String> = stmt
        .query_map(params![workspace_id.to_string()], |r| r.get(0))?
        .collect::<Result<_, _>>()
        .map_err(CliError::from)?;
    ids.iter()
        .map(|id| metric_view(conn, workspace_id, id))
        .collect()
}

// ---- source conflicts ----

pub fn conflict_list(conn: &Connection, workspace_id: Uuid, run_id: &str) -> CliResult<Value> {
    let mut stmt = conn.prepare(
        "SELECT conflict_id, metric_id, measure_role, period_from, period_to, scope_key, unit,
                candidate_values_json, resolution_status, resolution_id, selected_evidence_id,
                resolution_note
         FROM source_conflicts WHERE workspace_id = ?1 AND run_id = ?2 ORDER BY created_at",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string(), run_id], |r| {
        Ok(json!({
            "conflict_id": r.get::<_, String>(0)?,
            "metric_id": r.get::<_, String>(1)?,
            "measure_role": r.get::<_, String>(2)?,
            "period": format!("{}/{}", r.get::<_, String>(3)?, r.get::<_, String>(4)?),
            "scope_key": r.get::<_, String>(5)?,
            "unit": r.get::<_, String>(6)?,
            "candidate_values": serde_json::from_str::<Value>(&r.get::<_, String>(7)?).unwrap_or(json!([])),
            "resolution_status": r.get::<_, String>(8)?,
            "resolution_id": r.get::<_, Option<String>>(9)?,
            "selected_evidence_id": r.get::<_, Option<String>>(10)?,
            "resolution_note": r.get::<_, Option<String>>(11)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(json!({ "run_id": run_id, "conflicts": out }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictResolveInput {
    pub conflict_id: String,
    pub selected_evidence_id: String,
    pub resolution_note: String,
    pub expected_revision: u32,
}

/// resolve：选中证据 + 理由（原候选不覆盖；expected_revision=run 当前 revision）。
pub fn conflict_resolve(
    conn: &mut Connection,
    workspace_id: Uuid,
    run_id: &str,
    input: ConflictResolveInput,
) -> CliResult<Value> {
    if input.resolution_note.trim().is_empty() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "resolution_note 必填非空",
        ));
    }
    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let row: Option<(String, String, String, i64)> = tx
        .query_row(
            "SELECT c.workspace_id, c.resolution_status, c.candidate_values_json,
                    (SELECT current_revision_no FROM analysis_runs a WHERE a.run_id = c.run_id)
             FROM source_conflicts c WHERE c.conflict_id = ?1 AND c.run_id = ?2",
            params![input.conflict_id, run_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let Some((owner_ws, status, candidates_raw, run_current)) = row else {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "conflict 不存在或不属于该 run",
        ));
    };
    if owner_ws != workspace_id.to_string() {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "conflict 不属于所选 workspace",
        ));
    }
    if status != "unresolved" {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "冲突已解决；改选择创建新分析 revision",
        ));
    }
    if input.expected_revision != run_current as u32 {
        return Err(CliError::new(
            CliErrorCode::RevisionConflict,
            format!("expected_revision 须等于 run 当前 revision {run_current}"),
        ));
    }
    let candidates: Value = serde_json::from_str(&candidates_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "候选 JSON 损坏"))?;
    let evidence_ok = candidates
        .as_array()
        .map(|arr| {
            arr.iter().any(|c| {
                c.get("evidence_id").and_then(|e| e.as_str()) == Some(&input.selected_evidence_id)
            })
        })
        .unwrap_or(false);
    if !evidence_ok {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "selected_evidence_id 必须是候选组内证据",
        ));
    }
    let resolution_id = Uuid::new_v4().to_string();
    tx.execute(
        "UPDATE source_conflicts SET resolution_status = 'resolved', resolution_id = ?2,
                selected_evidence_id = ?3, resolution_note = ?4, resolved_at = ?5
         WHERE conflict_id = ?1",
        params![
            input.conflict_id,
            resolution_id,
            input.selected_evidence_id,
            input.resolution_note.trim(),
            now_iso()
        ],
    )
    .map_err(CliError::from)?;
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    Ok(json!({
        "conflict_id": input.conflict_id,
        "resolution_id": resolution_id,
        "selected_evidence_id": input.selected_evidence_id,
        "analysis_recalculation_required": true,
    }))
}

/// 当前桌面分析选择口径；不改变 CLI 历史回算的显式范围能力。
struct AnalysisCalendar {
    policy_id: String,
    version: String,
    grain: String,
    calendar: CalendarPolicy,
}

fn current_analysis_calendar(conn: &Connection, workspace_id: Uuid) -> CliResult<AnalysisCalendar> {
    conn.query_row(
        "SELECT p.period_policy_id, p.version, p.cadence, p.week_start_day, p.fiscal_year_start_month, p.timezone_id, p.month_start_day
         FROM workspaces w JOIN period_policies p ON p.period_policy_id = w.current_period_policy_id
         WHERE w.workspace_id = ?1",
        params![workspace_id.to_string()],
        |r| Ok(AnalysisCalendar {
            policy_id: r.get(0)?, version: r.get::<_, i64>(1)?.to_string(), grain: r.get(2)?,
            calendar: CalendarPolicy {
                week_start_day: r.get(3)?, month_start_day: r.get(6)?, fiscal_year_start_month: r.get(4)?, timezone_id: r.get(5)?,
            },
        }),
    ).map_err(CliError::from)
}

fn analysis_bucket(date: NaiveDate, grain: &str, policy: &CalendarPolicy) -> CliResult<DateRange> {
    match grain {
        "day" => Ok(DateRange {
            from: date,
            to: date,
        }),
        "month" => Ok(monthly_period_span(date, policy.month_start_day)),
        "quarter" => Ok(fiscal_quarter_span(date, policy.fiscal_year_start_month).2),
        "week" => {
            let offset = (date.weekday().number_from_monday() + 7 - policy.week_start_day) % 7;
            let from = date - Duration::days(offset as i64);
            Ok(DateRange {
                from,
                to: from + Duration::days(6),
            })
        }
        _ => Err(CliError::invalid("当前系统分析粒度不支持期间选择")),
    }
}

/// 桌面只允许系统策略粒度下的一个自然桶；月度文件仅覆盖数日也选完整月份。
pub fn validate_desktop_analysis_period(
    conn: &Connection,
    workspace_id: Uuid,
    from: &str,
    to: &str,
    requested_grain: &str,
) -> CliResult<String> {
    let current = current_analysis_calendar(conn, workspace_id)?;
    if requested_grain != "auto" && requested_grain != current.grain {
        return Err(CliError::invalid("分析粒度须使用系统设置中的当前粒度"));
    }
    let from = dsheng_core::analysis::periods::parse_calendar_date(from)
        .map_err(|_| CliError::invalid("分析期间开始日期无效"))?;
    let to = dsheng_core::analysis::periods::parse_calendar_date(to)
        .map_err(|_| CliError::invalid("分析期间结束日期无效"))?;
    let bucket = analysis_bucket(from, &current.grain, &current.calendar)?;
    if from != bucket.from || to != bucket.to {
        return Err(CliError::invalid(
            "请选择系统粒度对应的一个完整期间，不支持自定义日期范围",
        ));
    }
    // 返回该策略版本用于本次显式分析，避免历史日期自动选回旧日历口径。
    Ok(current.version)
}

/// 提取真正带实绩的源期间，不把日期最小/最大值之间的空白日补成可用数据。
/// 月汇总保留为月区间，不能拆成每日；无明确日期、累计行和空值不造选项。
fn observed_source_periods(files: &[ScannedFile]) -> BTreeSet<(NaiveDate, NaiveDate)> {
    use dsheng_core::import::headers::parse_period_header;
    let mut periods = BTreeSet::new();
    for file in files {
        for (data, sheet) in &file.sheets {
            // 企业 source profile 通常把原始表识别为 generic_table；只要有
            // 可解析表头，就同样参与可选期间计算。生成层 sheet 已在扫描时
            // 标记为 generated_output_input_not_allowed，因此不会被回灌。
            if !matches!(sheet.role, "accounting" | "trend" | "generic_table") {
                continue;
            }
            if let Some(layout) = sheet
                .match_info
                .as_ref()
                .and_then(|m| m.transposed.as_ref())
            {
                for (row, period) in &layout.period_rows {
                    if layout.metric_columns.iter().any(|(col, _)| {
                        data.rows
                            .get(*row)
                            .and_then(|r| r.get(*col))
                            .and_then(|c| crate::store::analysis::cell_value(Some(c)))
                            .is_some()
                    }) {
                        periods.insert((period.from, period.to));
                    }
                }
                continue;
            }
            let Some(header) = &sheet.header else {
                continue;
            };
            let col_of = |field: &str| {
                header
                    .columns
                    .iter()
                    .find(|(_, f)| *f == field)
                    .map(|(c, _)| *c)
            };
            let Some(label_col) = col_of("metric_label") else {
                continue;
            };
            let rows = || {
                data.rows
                    .iter()
                    .skip(header.row + header.row_count - 1)
                    .filter(|row| {
                        row.get(label_col)
                            .is_some_and(|cell| !cell.raw_text().trim().is_empty())
                    })
            };
            if let Some(period_col) = col_of("period") {
                let Some(actual_col) = col_of("actual") else {
                    continue;
                };
                for row in rows() {
                    if row
                        .get(actual_col)
                        .and_then(|c| crate::store::analysis::cell_value(Some(c)))
                        .is_none()
                    {
                        continue;
                    }
                    if let Some(period) = row.get(period_col).and_then(|cell| {
                        crate::store::analysis::observation_period(&cell.raw_text())
                    }) {
                        periods.insert((period.from, period.to));
                    }
                }
            } else {
                let width = data.rows.iter().map(Vec::len).max().unwrap_or(0);
                for col in 0..width {
                    if header.columns.iter().any(|(mapped, _)| *mapped == col) {
                        continue;
                    }
                    let text = data
                        .rows
                        .iter()
                        .skip(header.row.saturating_sub(1))
                        .take(header.row_count)
                        .find_map(|row| {
                            row.get(col)
                                .filter(|cell| !cell.raw_text().trim().is_empty())
                        })
                        .map(|cell| cell.raw_text());
                    let Some(period) = text.and_then(|text| parse_period_header(&text, None))
                    else {
                        continue;
                    };
                    if rows().any(|row| {
                        row.get(col)
                            .and_then(|cell| crate::store::analysis::cell_value(Some(cell)))
                            .is_some()
                    }) {
                        periods.insert((period.from, period.to));
                    }
                }
            }
        }
    }
    periods
}

fn available_analysis_periods(
    conn: &Connection,
    workspace_id: Uuid,
    files: &[ScannedFile],
) -> CliResult<(String, Vec<Value>)> {
    let current = current_analysis_calendar(conn, workspace_id)?;
    let mut buckets = BTreeSet::new();
    for (from, to) in observed_source_periods(files) {
        let bucket = analysis_bucket(from, &current.grain, &current.calendar)?;
        // 只能向同一或更粗桶归并。跨桶总值不能拆分、复制到多个桶。
        if from >= bucket.from && to <= bucket.to {
            buckets.insert((bucket.from, bucket.to));
        }
    }
    let periods = buckets
        .into_iter()
        .map(|(from, to)| {
            json!({
                "from": from.format("%Y-%m-%d").to_string(),
                "to": to.format("%Y-%m-%d").to_string(), "grain": current.grain,
            })
        })
        .collect();
    Ok((current.policy_id, periods))
}

/// scan 视图：目录摘要（read-views.scanSummaryView）。冻结 CLI 形状保持不变。
pub fn scan_summary(conn: &mut Connection, workspace_id: Uuid, dir: &str) -> CliResult<Value> {
    scan_summary_internal(
        conn,
        workspace_id,
        dir,
        false,
        &SourceProfileBinding::generic(),
    )
}

pub fn scan_summary_with_profile(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
    profile: &SourceProfileBinding,
) -> CliResult<Value> {
    scan_summary_internal(conn, workspace_id, dir, false, profile)
}

/// 桌面内部扫描视图：附当前策略下确有实绩的自然期间，不扩展冻结 CLI 合同。
pub fn scan_summary_desktop(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
) -> CliResult<Value> {
    scan_summary_internal(
        conn,
        workspace_id,
        dir,
        true,
        &SourceProfileBinding::generic(),
    )
}

pub fn scan_summary_desktop_with_profile(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
    profile: &SourceProfileBinding,
) -> CliResult<Value> {
    scan_summary_internal(conn, workspace_id, dir, true, profile)
}

fn scan_summary_internal(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
    desktop_periods: bool,
    profile: &SourceProfileBinding,
) -> CliResult<Value> {
    ensure_sidecar_tables(conn)?;
    let metric_names = metric_label_names(conn, workspace_id)?;
    let aliases = profile_aliases(profile);
    let (files, mut pending_inputs) = scan_directory_with_profile_config(
        dir,
        &metric_names,
        aliases.as_ref(),
        Some(profile),
    )?;
    for file in &files {
        for (_, sheet) in &file.sheets {
            if let Some(reason) = sheet.blocked_reason {
                pending_inputs.push(format!(
                    "{reason}:{}:{}",
                    file.file_label,
                    sheet.sheet.clone().unwrap_or_default()
                ));
            }
        }
    }
    let mut registered = register_sources_with_profile(conn, workspace_id, &files, profile)?;
    let mapping_confirmed = profile.mode != "generic"
        || source_mapping_confirmed(conn, workspace_id, dir, &profile.hash)?.is_some();
    if mapping_confirmed {
        for (view, file) in registered.iter_mut().zip(files.iter()) {
            if file.sheets.iter().any(|(_, sheet)| {
                sheet.role == "generic_table" && sheet.header.is_some() && sheet.blocked_reason.is_none()
            }) {
                view["mapping_status"] = json!("mapped");
            }
        }
    }
    let mut total_rows = 0usize;
    let mut min_date: Option<chrono::NaiveDate> = None;
    let mut max_date: Option<chrono::NaiveDate> = None;
    for (view, file) in registered.iter().zip(files.iter()) {
        total_rows += view["row_count"].as_u64().unwrap_or(0) as usize;
        for (sheet_data, sheet) in file.sheets.iter() {
            if sheet.blocked_reason.is_some() {
                continue;
            }
            let Some(header) = &sheet.header else {
                continue;
            };
            for (col, field) in &header.columns {
                if *field != "period" {
                    continue;
                }
                for row in sheet_data
                    .rows
                    .iter()
                    .skip(header.row + header.row_count - 1)
                {
                    if let Some(text) = row.get(*col).map(|c| c.raw_text()) {
                        if let Ok(d) = dsheng_core::analysis::periods::parse_calendar_date(&text) {
                            min_date = Some(match min_date {
                                Some(m) if m < d => m,
                                _ => d,
                            });
                            max_date = Some(match max_date {
                                Some(m) if m > d => m,
                                _ => d,
                            });
                        }
                    }
                }
            }
        }
    }
    let grain: String = conn
        .query_row(
            "SELECT p.cadence FROM workspaces w
             JOIN period_policies p ON p.period_policy_id = w.current_period_policy_id
             WHERE w.workspace_id = ?1",
            params![workspace_id.to_string()],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "month".into());
    let from = min_date.map(|d| d.format("%Y-%m-%d").to_string());
    let to = max_date.map(|d| d.format("%Y-%m-%d").to_string());
    let mut summary = json!({
        "workspace_id": workspace_id.to_string(),
        "directory_label": dir,
        "scanned_at": now_iso(),
        "files": registered,
        "total_rows": total_rows,
        "coverage": {
            "from": from,
            "to": to,
            "granularity": grain,
        },
        "pending_inputs": pending_inputs,
        "source_profile": {
            "profile_id": profile.profile_id,
            "version": profile.version,
            "hash": profile.hash,
            "mode": profile.mode,
            "profile_ref": profile.profile_ref,
        },
    });
    if desktop_periods {
        let (policy_id, periods) = available_analysis_periods(conn, workspace_id, &files)?;
        summary["available_periods"] = json!(periods);
        summary["available_period_policy_id"] = json!(policy_id);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dsheng_core::import::fileio::{CellValue, SheetData};

    #[test]
    fn generated_output_sheet_is_blocked_at_scan() {
        let sheet = SheetData {
            name: "核算表".into(),
            rows: vec![vec![CellValue::Text("科目".into())]],
            merged_ranges: vec![],
        };
        let (_, scanned) = analyze_sheet_with_aliases(sheet, &[], None);
        assert_eq!(scanned.role, "generated_output_input_not_allowed");
        assert_eq!(
            scanned.blocked_reason,
            Some("generated_output_input_not_allowed")
        );
    }

    #[test]
    fn source_profile_selection_is_explicit_and_hashed() {
        assert_eq!(resolve_source_profile(None, false).unwrap().mode, "generic_implicit");
        assert_eq!(resolve_source_profile(None, true).unwrap().mode, "generic");
        assert!(resolve_source_profile(Some("manufacturing_erp_v1"), true).is_err());
        let built_in = resolve_source_profile(Some("manufacturing_demo"), false).unwrap();
        assert_eq!(built_in.mode, "profile");
        assert_eq!(built_in.profile_ref.as_deref(), Some("manufacturing_demo"));
        assert!(resolve_source_profile(Some("manufacturing_erp_v1"), false).is_ok());
    }
}
