//! 证据投影类型（data-period-import.md §1.4：source_records / manual_inputs /
//! source_conflicts 最小结构）。字段集与 required/nullable 语义按合同逐字段冻结。

use crate::contracts::domain::{DateRange, MeasureRole, ValueState};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 单元格类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellType {
    Text,
    Number,
    Formula,
    Boolean,
    Error,
    Blank,
}

/// 单元格 mapping 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingMeasureRole {
    Actual,
    Plan,
    AnnualMonthPlan,
    Metadata,
}

/// 一个 source_record 的 mapping（封闭对象，全字段 required、可 null）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CellMapping {
    pub metric_id: Option<uuid::Uuid>,
    /// §4.2 列出的一个标准字段。
    pub field_key: String,
    pub measure_role: MappingMeasureRole,
    /// 已解析闭区间或 null。
    pub period: Option<DateRange>,
    pub scope_key: Option<String>,
    /// 规范 unitSignature JSON 字符串或 null。
    pub unit: Option<String>,
}

/// source_record values 封闭对象。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRecordValues {
    pub cell_type: CellType,
    /// 原文（不裁剪）。
    pub raw_text: String,
    /// 数值为规范十进制字符串；日期为解析后 ISO 日期字符串；boolean 布尔；其余 null。
    pub typed_value: Option<serde_json::Value>,
    /// 本格公式原文或 null；永不执行。
    pub formula: Option<String>,
    pub value_state: ValueState,
    pub number_format: Option<String>,
    pub merged_range: Option<String>,
    pub mapping: Option<CellMapping>,
}

/// source_record 顶层对象（一格一记录）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceRecord {
    /// `cell:<source UUID>:<sheet-or-csv>:<A1>`
    pub evidence_id: String,
    pub source_id: uuid::Uuid,
    /// 用户确认的非空显示标签；不含绝对路径。
    pub file_label: String,
    /// sheet 名或 null（CSV）。
    pub sheet: Option<String>,
    /// 单一 A1 坐标。
    pub cell_or_row: String,
    pub values: SourceRecordValues,
}

/// A1 坐标校验（列 A–XFD 大写、行 1–1048576 无前导零）。
pub fn validate_a1(cell: &str) -> Result<(), &'static str> {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = PATTERN
        .get_or_init(|| regex::Regex::new(r"^[A-Z]{1,3}(?:[1-9][0-9]{0,6})$").expect("静态正则"));
    if !re.is_match(cell) {
        return Err("invalid_a1");
    }
    // 列 ≤ XFD
    let col: String = cell
        .chars()
        .take_while(|c| c.is_ascii_uppercase())
        .collect();
    let mut n: u32 = 0;
    for c in col.chars() {
        n = n * 26 + (c as u32 - 'A' as u32 + 1);
    }
    if n > 16384 {
        return Err("invalid_a1_column");
    }
    let row: u64 = cell[col.len()..].parse().map_err(|_| "invalid_a1_row")?;
    if !(1..=1_048_576).contains(&row) {
        return Err("invalid_a1_row");
    }
    Ok(())
}

/// 构造 evidence_id。
pub fn cell_evidence_id(source_id: uuid::Uuid, sheet_or_csv: &str, a1: &str) -> String {
    format!("cell:{source_id}:{sheet_or_csv}:{a1}")
}

/// manual_inputs 每项（封闭对象，全字段 required）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualInputRecord {
    pub input_id: uuid::Uuid,
    pub metric_id: uuid::Uuid,
    /// 规范十进制字符串或 null。
    pub value: Option<String>,
    /// 规范 unitSignature JSON 字符串或 null。
    pub unit: Option<String>,
    pub confirmed: bool,
    /// 首尾清理后非空。
    pub source_note: String,
    /// 恰有一个虚拟 ID：`manual:<input_id>:r<input_revision>`。
    pub evidence_ids: Vec<String>,
    pub input_revision: u32,
    pub measure_role: MeasureRole,
    /// 闭区间或 null（null 仅允许经确认的永久定义配置）。
    pub period: Option<DateRange>,
    pub scope_key: String,
    pub raw_text: String,
}

/// manual evidence 虚拟 ID。
pub fn manual_evidence_id(input_id: uuid::Uuid, revision: u32) -> String {
    format!("manual:{input_id}:r{revision}")
}

/// manual 虚拟 source_id。
pub fn manual_source_id(input_id: uuid::Uuid) -> String {
    format!("manual:{input_id}")
}

/// source_conflicts 候选值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictCandidate {
    /// number（模型边界）/ 规范字符串（本地）。
    pub value: serde_json::Value,
    pub evidence_id: String,
    pub source_label: String,
}

/// source_conflicts 投影对象（§1.4 表，全字段 required）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceConflictRecord {
    pub conflict_id: uuid::Uuid,
    pub metric_id: uuid::Uuid,
    pub measure_role: MeasureRole,
    /// `YYYY-MM-DD/YYYY-MM-DD`
    pub period: String,
    pub scope_key: String,
    /// 与该 metric 规范 unit 字符串完全相等。
    pub unit: String,
    pub candidate_values: Vec<ConflictCandidate>,
    pub resolution_status: ResolutionStatus,
    pub resolution_id: Option<uuid::Uuid>,
    pub selected_evidence_id: Option<String>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Unresolved,
    Resolved,
}

impl SourceConflictRecord {
    /// 跨字段校验（§1.4 表尾段）。
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.candidate_values.len() < 2 {
            return Err("candidate_values_min_2");
        }
        let mut ids = std::collections::BTreeSet::new();
        for c in &self.candidate_values {
            if c.evidence_id.is_empty() || c.source_label.is_empty() {
                return Err("candidate_fields_required");
            }
            if !ids.insert(c.evidence_id.as_str()) {
                return Err("candidate_evidence_duplicate");
            }
        }
        match self.resolution_status {
            ResolutionStatus::Resolved => {
                if self.resolution_id.is_none() {
                    return Err("resolved_requires_resolution_id");
                }
                let sel = self
                    .selected_evidence_id
                    .as_deref()
                    .ok_or("resolved_requires_selected_evidence")?;
                if !ids.contains(sel) {
                    return Err("selected_not_in_candidates");
                }
                if self
                    .resolution_note
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err("resolved_requires_note");
                }
            }
            ResolutionStatus::Unresolved => {
                if self.resolution_id.is_some()
                    || self.selected_evidence_id.is_some()
                    || self.resolution_note.is_some()
                {
                    return Err("unresolved_requires_null_fields");
                }
            }
        }
        // period 形状
        let parts: Vec<&str> = self.period.split('/').collect();
        if parts.len() != 2 {
            return Err("period_format");
        }
        let from = NaiveDate::parse_from_str(parts[0], "%Y-%m-%d").map_err(|_| "period_date")?;
        let to = NaiveDate::parse_from_str(parts[1], "%Y-%m-%d").map_err(|_| "period_date")?;
        if from > to {
            return Err("period_order");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_validation() {
        assert!(validate_a1("A1").is_ok());
        assert!(validate_a1("XFD1048576").is_ok());
        assert!(validate_a1("a1").is_err(), "小写非法");
        assert!(validate_a1("A01").is_err(), "前导零非法");
        assert!(validate_a1("XFE1").is_err(), "列越界");
        assert!(validate_a1("A1048577").is_err(), "行越界");
    }

    #[test]
    fn conflict_cross_checks() {
        let base = SourceConflictRecord {
            conflict_id: uuid::Uuid::new_v4(),
            metric_id: uuid::Uuid::new_v4(),
            measure_role: MeasureRole::Actual,
            period: "2026-10-01/2026-10-31".into(),
            scope_key: "全公司".into(),
            unit: "{\"base_unit\":\"元\"}".into(),
            candidate_values: vec![
                ConflictCandidate {
                    value: serde_json::json!(100),
                    evidence_id: "cell:x:核算表:C7".into(),
                    source_label: "核算表".into(),
                },
                ConflictCandidate {
                    value: serde_json::json!(110),
                    evidence_id: "cell:y:核算表:C8".into(),
                    source_label: "副本".into(),
                },
            ],
            resolution_status: ResolutionStatus::Unresolved,
            resolution_id: None,
            selected_evidence_id: None,
            resolution_note: None,
        };
        assert!(base.validate().is_ok());
        let mut bad = base.clone();
        bad.selected_evidence_id = Some("cell:y:核算表:C8".into());
        assert!(bad.validate().is_err(), "unresolved 必须全 null");
        let mut resolved = base;
        resolved.resolution_status = ResolutionStatus::Resolved;
        assert!(
            resolved.validate().is_err(),
            "resolved 缺 resolution_id/note"
        );
    }
}
