//! analysis_input_fingerprint 规范化（task-lifecycle.md §2.1）。
//!
//! canonicalization_version 固定 `dsheng-canonical-json-v1`：
//! - 对象键按 Unicode 码点升序；数组按各合同固定顺序（来源按
//!   source_id/sheet/range、指标按 metric_key/scope、比较按 type/base_ref，
//!   由 [`AnalysisRevisionInput::to_fingerprint_json`] 强制排序）；
//! - 数值统一为无多余前导零/末尾零的十进制字符串，`-0` 为 0；
//! - 日期 ISO date，时间 UTC ISO-8601（调用方已存字符串）；
//! - UTF-8、无无意义空白、不做会改变业务原文的 Unicode 归一化；
//! - 缺字段与 null 不等价，字段齐全后再 hash。
//!
//! 不包含绝对路径、API Key 或可恢复凭据的派生值。

use crate::contracts::decimal::to_canonical_string;
use crate::contracts::domain::DateRange;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 规范化版本标识（0001 冻结列默认值）。
pub const CANONICALIZATION_VERSION: &str = "dsheng-canonical-json-v1";

/// dsheng-canonical-json-v1 序列化。
///
/// 数字按词法（不经 f64，serde_json arbitrary_precision）解析为 Decimal 后
/// 输出规范十进制字符串；字符串按 JSON 转义、不做 Unicode 归一化。
pub fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort(); // String 字节序 = UTF-8 码点序
            let mut out = String::from("{");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).expect("key"));
                out.push(':');
                out.push_str(&canonical_json(&map[*k]));
            }
            out.push('}');
            out
        }
        serde_json::Value::Array(items) => {
            let mut out = String::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&canonical_json(item));
            }
            out.push(']');
            out
        }
        serde_json::Value::Number(n) => canonical_number(n),
        other => serde_json::to_string(other).expect("scalar"),
    }
}

fn canonical_number(n: &serde_json::Number) -> String {
    // arbitrary_precision 保留原始词法；不经 f64
    let lexeme = n.to_string();
    if let Ok(d) = Decimal::from_str_exact(&lexeme) {
        return to_canonical_string(d); // -0 → 0、去尾零
    }
    // 指数词法（1e3/1E+3）按科学计数解析（仍不经 f64）
    if let Ok(d) = Decimal::from_scientific(&lexeme) {
        return to_canonical_string(d);
    }
    lexeme
}

/// SHA-256 指纹（hex 小写）。
pub fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 来源选择快照（按 source_id/sheet/range 排序后进指纹）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSelection {
    pub source_id: String,
    pub sheet: Option<String>,
    pub range: String,
    pub content_hash: String,
}

/// 指标定义快照（按 metric_key/scope 排序后进指纹）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDefSnapshot {
    pub metric_key: String,
    pub version: u32,
    pub scope: String,
    pub semantic_hash: String,
}

/// 手工输入快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManualInputSnapshot {
    pub input_id: String,
    pub metric_id: String,
    pub input_revision: u32,
    /// 规范十进制字符串或 null。
    pub value: Option<String>,
}

/// 已确认来源冲突选择快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConflictResolutionSnapshot {
    pub conflict_id: String,
    pub selected_evidence_id: String,
}

/// 比较基期引用（按 run_id/revision_no 排序）。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct BaseRef {
    pub run_id: String,
    pub revision_no: u32,
}

/// 一份分析 revision 的完整输入快照（task-lifecycle §2.1 固定结构）。
#[derive(Debug, Clone)]
pub struct AnalysisRevisionInput {
    pub sources: Vec<SourceSelection>,
    pub mapping_version: String,
    pub mappings: serde_json::Value,
    pub metric_definitions: Vec<MetricDefSnapshot>,
    pub manual_inputs: Vec<ManualInputSnapshot>,
    pub conflict_resolutions: Vec<ConflictResolutionSnapshot>,
    /// 周期策略/时区/业务范围快照。
    pub period_policy: serde_json::Value,
    pub scope: String,
    pub current_period: DateRange,
    pub target_period: Option<DateRange>,
    /// 规范化指标（含规范十进制字符串数值）。
    pub normalized_metrics: serde_json::Value,
    /// 比较值（mom/yoy；含 base_run_id/base_revision_no）。
    pub comparisons: serde_json::Value,
    pub comparison_base_refs: Vec<BaseRef>,
}

impl AnalysisRevisionInput {
    /// 构造指纹 JSON：合同固定顺序的数组在此排序，调用方无法乱序。
    pub fn to_fingerprint_json(&self) -> serde_json::Value {
        let mut sources: Vec<&SourceSelection> = self.sources.iter().collect();
        sources.sort_by(|a, b| {
            (&a.source_id, &a.sheet, &a.range).cmp(&(&b.source_id, &b.sheet, &b.range))
        });
        let mut defs: Vec<&MetricDefSnapshot> = self.metric_definitions.iter().collect();
        defs.sort_by(|a, b| (&a.metric_key, &a.scope).cmp(&(&b.metric_key, &b.scope)));
        let mut base_refs: Vec<&BaseRef> = self.comparison_base_refs.iter().collect();
        base_refs.sort();

        serde_json::json!({
            "comparison_base_refs": base_refs.iter().map(|r| serde_json::json!({
                "revision_no": r.revision_no,
                "run_id": r.run_id,
            })).collect::<Vec<_>>(),
            "comparisons": self.comparisons,
            "conflict_resolutions": self.conflict_resolutions.iter().map(|c| serde_json::json!({
                "conflict_id": c.conflict_id,
                "selected_evidence_id": c.selected_evidence_id,
            })).collect::<Vec<_>>(),
            "current_period": self.current_period.to_period_string(),
            "manual_inputs": self.manual_inputs.iter().map(|m| serde_json::json!({
                "input_id": m.input_id,
                "input_revision": m.input_revision,
                "metric_id": m.metric_id,
                "value": m.value,
            })).collect::<Vec<_>>(),
            "mapping_version": self.mapping_version,
            "mappings": self.mappings,
            "metric_definitions": defs.iter().map(|d| serde_json::json!({
                "metric_key": d.metric_key,
                "scope": d.scope,
                "semantic_hash": d.semantic_hash,
                "version": d.version,
            })).collect::<Vec<_>>(),
            "normalized_metrics": self.normalized_metrics,
            "period_policy": self.period_policy,
            "scope": self.scope,
            "sources": sources.iter().map(|s| serde_json::json!({
                "content_hash": s.content_hash,
                "range": s.range,
                "sheet": s.sheet,
                "source_id": s.source_id,
            })).collect::<Vec<_>>(),
            "target_period": self.target_period.as_ref().map(|p| p.to_period_string()),
        })
    }

    /// analysis_input_fingerprint（SHA-256 over dsheng-canonical-json-v1）。
    pub fn fingerprint(&self) -> String {
        sha256_hex(&canonical_json(&self.to_fingerprint_json()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sorted_and_numbers_canonical() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"b":1e3,"a":{"y":-0,"x":[1.50,2]}}"#).unwrap();
        assert_eq!(
            canonical_json(&v),
            "{\"a\":{\"x\":[1.5,2],\"y\":0},\"b\":1000}"
        );
    }

    #[test]
    fn no_unicode_normalization() {
        // NFC 与 NFD 形式的 “é” 必须产生不同输出（不归一化业务原文）
        let nfc = serde_json::json!({"k": "é"});
        let nfd = serde_json::json!({"k": "é"});
        assert_ne!(canonical_json(&nfc), canonical_json(&nfd));
    }

    #[test]
    fn missing_field_not_null() {
        let with_field = serde_json::json!({"a": 1, "b": null});
        let without = serde_json::json!({"a": 1});
        assert_ne!(canonical_json(&with_field), canonical_json(&without));
    }
}
