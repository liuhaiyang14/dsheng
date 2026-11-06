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
use dsheng_core::import::headers::{column_letter, HeaderBlock};
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
         );",
    )
    .map_err(CliError::from)?;
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

// ---- 扫描 ----

/// 单个扫描到的 sheet 摘要。
pub struct ScannedSheet {
    pub sheet: Option<String>,
    pub role: &'static str,
    pub header: Option<HeaderBlock>,
    pub match_info: Option<SheetMatch>,
    pub rows: usize,
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
            file_label: label,
            content_hash: sha256_bytes(&bytes),
            kind: ext,
            sheets: sheets
                .into_iter()
                .map(|s| analyze_sheet(s, metric_names))
                .collect(),
        });
    }
    Ok((files, pending))
}

/// 识别一个 sheet 的角色与表头块（core §4.1/§4.3；含转置推移表锚点）。
/// 传入全部行（core 表头搜索内部自限首 50 行；转置锚点的期间数据行
/// 不受 50 行截断，避免长推移表丢行）。
fn analyze_sheet(sheet: SheetData, metric_names: &[String]) -> (SheetData, ScannedSheet) {
    let header_rows: Vec<Vec<String>> = sheet
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.raw_text()).collect())
        .collect();
    let match_info = detect_sheet_role_with_names(&sheet.name, &header_rows, metric_names);
    let role = match match_info.as_ref().map(|m| m.role) {
        Some(r) => r,
        None => SheetRole::GenericTable,
    };
    let header_rows_end = match_info
        .as_ref()
        .and_then(|m| m.header.as_ref())
        .map(|h| h.row + h.row_count - 1)
        .unwrap_or(0);
    let scanned = ScannedSheet {
        sheet: Some(sheet.name.clone()),
        role: role.as_str(),
        header: match_info.as_ref().and_then(|m| m.header.clone()),
        match_info,
        rows: sheet.rows.len().saturating_sub(header_rows_end),
    };
    (sheet, scanned)
}

/// 注册 source_files（同 content_hash 复用 source_id；仅本机 DB，不进导出）。
pub fn register_sources(
    conn: &Connection,
    workspace_id: Uuid,
    files: &[ScannedFile],
) -> CliResult<Vec<Value>> {
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
        let total_rows: usize = file.sheets.iter().map(|(_, s)| s.rows).sum();
        let mapped = file
            .sheets
            .iter()
            .any(|(_, s)| matches!(s.role, "accounting" | "trend" | "task_review" | "task_plan"));
        // 严格 scanFileInfo（additionalProperties:false：file_id/file_label/row_count/
        // sheet_count/mapping_status/note；来源哈希等审计信息不进该视图）
        let note = file
            .sheets
            .iter()
            .map(|(_, s)| {
                format!(
                    "{}:{}({}行)",
                    s.sheet.clone().unwrap_or_default(),
                    s.role,
                    s.rows
                )
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingSetInput {
    pub expected_revision: u32,
    pub mappings: Vec<FieldMappingEntry>,
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
            if !matches!(sheet.role, "accounting" | "trend") {
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
    scan_summary_internal(conn, workspace_id, dir, false)
}

/// 桌面内部扫描视图：附当前策略下确有实绩的自然期间，不扩展冻结 CLI 合同。
pub fn scan_summary_desktop(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
) -> CliResult<Value> {
    scan_summary_internal(conn, workspace_id, dir, true)
}

fn scan_summary_internal(
    conn: &mut Connection,
    workspace_id: Uuid,
    dir: &str,
    desktop_periods: bool,
) -> CliResult<Value> {
    ensure_sidecar_tables(conn)?;
    let metric_names = metric_label_names(conn, workspace_id)?;
    let (files, pending_inputs) = scan_directory(dir, &metric_names)?;
    let registered = register_sources(conn, workspace_id, &files)?;
    let mut total_rows = 0usize;
    let mut min_date: Option<chrono::NaiveDate> = None;
    let mut max_date: Option<chrono::NaiveDate> = None;
    for (view, file) in registered.iter().zip(files.iter()) {
        total_rows += view["row_count"].as_u64().unwrap_or(0) as usize;
        for (sheet_data, sheet) in file.sheets.iter() {
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
        "directory_label": Path::new(dir)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir.to_string()),
        "scanned_at": now_iso(),
        "files": registered,
        "total_rows": total_rows,
        "coverage": {
            "from": from,
            "to": to,
            "granularity": grain,
        },
        "pending_inputs": pending_inputs,
    });
    if desktop_periods {
        let (policy_id, periods) = available_analysis_periods(conn, workspace_id, &files)?;
        summary["available_periods"] = json!(periods);
        summary["available_period_policy_id"] = json!(policy_id);
    }
    Ok(summary)
}
