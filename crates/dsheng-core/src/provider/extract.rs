//! C 方案响应适配层（prompt-context-validation.md §6、主计划 §8.2）。
//!
//! 行为基准：`tools/extract_provider_json.py --policy adapt` 的 `choose()`
//! （2026-11-02 用户决策的 option_c_hybrid_adopted 提取策略）：
//!
//! 1. 响应文本中每个 `'{'` 位置起尝试解码一个顶层 JSON 对象
//!    （等价 Python `json.JSONDecoder().raw_decode` 的起点扫描）；
//! 2. 阶段序（严格优先）：
//!    `contract_matched` → `root_shape_but_invalid`（根形状判定只看
//!    `schema_version`/`tasks` 两个强标记）→ adapt 策略下
//!    `wrapped_bare_task`（唯一通过任务子 schema 的裸对象本地包装根信封，
//!    `envelope_locally_reconstructed=true`）→ `ambiguous_bare_tasks`
//!    （多个裸任务候选拒绝）→ `no_contract_candidate` → `no_json_object`。
//!
//! 与 Python 参考实现的有意差异（不改变阶段判定，仅影响失败阶段的
//! "最大候选"展示对象）：
//! - "largest" 度量使用紧凑序列化的 UTF-8 字节数，而不是
//!   `len(json.dumps(...))`（Python 默认带分隔符空格且 ensure_ascii 转义）；
//!   近似平局时两者可能选出不同候选，但 `contract_matched`/
//!   `root_shape_but_invalid` 的强标记计数与 `wrapped_bare_task` 的唯一性
//!   判定与 Python 完全一致。
//! - Python `json` 接受 `NaN`/`Infinity` 字面量，serde_json 拒绝；
//!   此类候选在两边都无法通过 dsheng 输出合同，最终同样整份拒绝。
//!
//! 本模块只做结构提取，不猜业务值：包装根信封只重建客户端自有字段
//! （`schema_version="2"`、`summary` 取任务对象自带 summary 字符串、
//! 无则空串），`envelope_locally_reconstructed` 必须由调用方在所有存储
//! 与验收记录中持久化（见 [`crate::contracts::provider::ExtractionOutcome`]）。

use crate::contracts::provider::{ExtractionOutcome, ExtractionStage};
use serde_json::Value;

/// 根形状强标记（"summary" 单独出现不算：任务对象自带 summary 字段）。
const ROOT_STRONG_KEYS: [&str; 2] = ["schema_version", "tasks"];

/// 提取策略：strict 拒绝裸任务对象；adapt 允许唯一裸任务对象的本地包装。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractPolicy {
    Strict,
    Adapt,
}

/// 提取所需的两个合同校验谓词（由 S2 JSON Schema 校验层提供；
/// 纯逻辑测试可用闭包注入，见 tests/wp_provider_extract.rs）。
pub trait ExtractionChecks {
    /// 候选对象是否通过完整任务输出合同（schema2 根对象）。
    fn root_contract_passes(&self, obj: &Value) -> bool;
    /// 候选对象是否通过任务子 schema（`#/$defs/task`，裸任务对象）。
    fn task_subschema_passes(&self, obj: &Value) -> bool;
}

/// 一个可解码的顶层 JSON 对象候选（字节区间为左闭右开）。
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub start: usize,
    pub end: usize,
    pub object: Value,
}

/// 扫描响应文本中每个 `'{'` 起可解码的顶层 JSON 对象。
///
/// 等价 Python 参考：`re.finditer(r"\{", text)` + `raw_decode(text[start..])`。
/// 平衡花括号扫描跳过字符串字面量内部的 `{`/`}` 与转义；解析失败
/// （含尾随垃圾被平衡区间截断后仍非法）的起点直接跳过。serde_json 的
/// `arbitrary_precision`（workspace 已启用）保证 Number 保留原始词法，
/// 包装裸任务对象时数值词法不经过 f64 往返。
pub fn scan_candidates(text: &str) -> Vec<Candidate> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    for start in 0..bytes.len() {
        if bytes[start] != b'{' {
            continue;
        }
        let Some(end) = balanced_object_end(bytes, start) else {
            continue;
        };
        if let Ok(object) = serde_json::from_str::<Value>(&text[start..end]) {
            // 只有解码出 JSON 对象才构成候选（起点是 '{'，必然是对象）
            if object.is_object() {
                out.push(Candidate { start, end, object });
            }
        }
    }
    out
}

/// 从 `start`（指向 `'{'`）扫描到与之平衡的 `'}'` 后一位；无平衡区间返回 None。
fn balanced_object_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, &b) in bytes[start..].iter().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(start + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// `choose()` 结果：合同阶段 + 候选键摘要（诊断用，不进入业务存储）。
#[derive(Debug, Clone, PartialEq)]
pub struct ChooseOutcome {
    pub stage: ExtractionStage,
    pub object: Option<Value>,
    pub envelope_locally_reconstructed: bool,
    /// 每个候选的顶层键排序摘要（截取前 12 个候选，对齐 Python 工具输出）。
    pub candidate_top_level_keys: Vec<Vec<String>>,
}

impl ChooseOutcome {
    pub fn to_extraction_outcome(self) -> ExtractionOutcome {
        ExtractionOutcome {
            stage: self.stage,
            object: self.object,
            envelope_locally_reconstructed: self.envelope_locally_reconstructed,
        }
    }
}

/// C 方案合同优先选择规则（tools/extract_provider_json.py `choose()` 的移植）。
pub fn choose(text: &str, checks: &dyn ExtractionChecks, policy: ExtractPolicy) -> ChooseOutcome {
    let cands = scan_candidates(text);
    let keys = candidate_key_summary(&cands);
    if cands.is_empty() {
        return ChooseOutcome {
            stage: ExtractionStage::NoJsonObject,
            object: None,
            envelope_locally_reconstructed: false,
            candidate_top_level_keys: keys,
        };
    }

    // 1. 严格优先：任一候选通过完整输出合同即采用（按文档顺序取第一个）。
    for cand in &cands {
        if checks.root_contract_passes(&cand.object) {
            return ChooseOutcome {
                stage: ExtractionStage::ContractMatched,
                object: Some(cand.object.clone()),
                envelope_locally_reconstructed: false,
                candidate_top_level_keys: keys,
            };
        }
    }

    // 2. 根形状判定：只看 schema_version/tasks 两个强标记；
    //    强标记数最多的候选进入 root_shape_but_invalid（平局取首个，同 Python max）。
    let strong: Vec<&Candidate> = cands
        .iter()
        .filter(|c| strong_root_marker_count(&c.object) > 0)
        .collect();
    if !strong.is_empty() {
        let best = strong
            .iter()
            .max_by_key(|c| strong_root_marker_count(&c.object))
            .expect("strong 非空");
        return ChooseOutcome {
            stage: ExtractionStage::RootShapeButInvalid,
            object: Some(best.object.clone()),
            envelope_locally_reconstructed: false,
            candidate_top_level_keys: keys,
        };
    }

    // 3. adapt 策略：唯一通过任务子 schema 的裸对象本地包装根信封。
    if policy == ExtractPolicy::Adapt {
        let bare: Vec<&Candidate> = cands
            .iter()
            .filter(|c| checks.task_subschema_passes(&c.object))
            .collect();
        if bare.len() == 1 {
            let wrapped = wrap_bare_task(&bare[0].object);
            return ChooseOutcome {
                stage: ExtractionStage::WrappedBareTask,
                object: Some(wrapped),
                envelope_locally_reconstructed: true,
                candidate_top_level_keys: keys,
            };
        }
        if bare.len() > 1 {
            let largest = largest_candidate(bare.iter().copied());
            return ChooseOutcome {
                stage: ExtractionStage::AmbiguousBareTasks,
                object: Some(largest.object.clone()),
                envelope_locally_reconstructed: false,
                candidate_top_level_keys: keys,
            };
        }
    }

    // 4. 无任何合同线索：报告最大候选（仅诊断用途）。
    let largest = largest_candidate(cands.iter());
    ChooseOutcome {
        stage: ExtractionStage::NoContractCandidate,
        object: Some(largest.object.clone()),
        envelope_locally_reconstructed: false,
        candidate_top_level_keys: keys,
    }
}

/// 包装裸任务对象：只重建客户端自有信封字段，不发明业务值。
/// `summary` 回退为该任务对象自带的 summary 字符串（无则空串）。
fn wrap_bare_task(task: &Value) -> Value {
    let summary = task
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or_default();
    serde_json::json!({
        "schema_version": "2",
        "summary": summary,
        "tasks": [task],
    })
}

/// 根形状强标记命中数（0 表示非根形状候选）。
fn strong_root_marker_count(obj: &Value) -> usize {
    ROOT_STRONG_KEYS
        .iter()
        .filter(|k| obj.get(**k).is_some())
        .count()
}

/// 最大候选度量：紧凑序列化 UTF-8 字节数；平局取首个。
/// 与 Python `len(json.dumps(...))` 的差异见模块文档。
fn largest_candidate<'a>(cands: impl Iterator<Item = &'a Candidate>) -> &'a Candidate {
    cands
        .max_by_key(|c| {
            serde_json::to_string(&c.object)
                .map(|s| s.len())
                .unwrap_or(0)
        })
        .expect("候选列表非空")
}

fn candidate_key_summary(cands: &[Candidate]) -> Vec<Vec<String>> {
    cands
        .iter()
        .take(12)
        .map(|c| {
            let mut keys: Vec<String> = c
                .object
                .as_object()
                .map(|m| m.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            keys
        })
        .collect()
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试（结构提取层，非 Provider 调用）。
    use super::*;

    /// 与 Python selftest 的 AnyTask 等价：接受任何带 task_key 的字典。
    struct AnyTaskCheck;

    impl ExtractionChecks for AnyTaskCheck {
        fn root_contract_passes(&self, obj: &Value) -> bool {
            obj.get("schema_version").and_then(Value::as_str) == Some("2")
                && obj.get("tasks").is_some_and(Value::is_array)
                && obj.get("summary").is_some_and(Value::is_string)
        }
        fn task_subschema_passes(&self, obj: &Value) -> bool {
            obj.get("task_key").is_some()
        }
    }

    /// 永不通过的校验（等价 Python selftest 中 validator=None 的行为子集：
    /// 无根合同校验、无任务子校验）。
    struct NoCheck;

    impl ExtractionChecks for NoCheck {
        fn root_contract_passes(&self, _obj: &Value) -> bool {
            false
        }
        fn task_subschema_passes(&self, _obj: &Value) -> bool {
            false
        }
    }

    #[test]
    fn scan_finds_nested_and_string_braces() {
        let text = r#"前缀 {"a": "包含}花括号{的字符串", "b": {"c": 1}} 尾缀 {"d": 2}"#;
        let cands = scan_candidates(text);
        // 三个 '{' 起点：外层对象、内层 {"c":1}、第二个对象；
        // 字符串内部的 { 起点无法形成平衡对象。
        assert_eq!(cands.len(), 3, "候选数：{cands:?}");
        assert_eq!(cands[0].object["b"]["c"], serde_json::json!(1));
        assert_eq!(cands[1].object["c"], serde_json::json!(1));
        assert_eq!(cands[2].object["d"], serde_json::json!(2));
    }

    #[test]
    fn scan_preserves_number_lexeme() {
        // 纯逻辑：任意精度 Number 保留词法（不经 f64 往返）。
        // 尾随小数零原样保留；指数形式保留指数（serde 规范化为 e+，
        // 十进制契约层另行拒绝指数形式）。
        let text = r#"{"v": 100.2500}"#;
        let cands = scan_candidates(text);
        assert_eq!(cands[0].object["v"].to_string(), "100.2500");
        let text2 = r#"{"v": 1e3}"#;
        let cands2 = scan_candidates(text2);
        assert_eq!(cands2[0].object["v"].to_string(), "1e+3");
        let text3 = r#"{"v": 123456789012345678901234567890.123456789}"#;
        let cands3 = scan_candidates(text3);
        assert_eq!(
            cands3[0].object["v"].to_string(),
            "123456789012345678901234567890.123456789"
        );
    }

    #[test]
    fn selftest_contract_first_over_larger_smart_block() {
        // 对齐 Python --selftest（validator=None）：SMART 大块干扰下
        // 根形状候选必须胜出（合同优先，不取最大块）
        let smart = {
            let mut obj = serde_json::Map::new();
            for (k, v) in [("S", "x"), ("M", "y"), ("A", "z"), ("R", "w"), ("T", "v")] {
                obj.insert(k.into(), serde_json::json!(v.repeat(200)));
            }
            serde_json::to_string(&Value::Object(obj)).unwrap()
        };
        let root = r#"{"schema_version":"2","summary":"ok","tasks":[]}"#;
        let text = format!("分析：{smart}\n最终：{root}");
        let out = choose(&text, &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.stage, ExtractionStage::RootShapeButInvalid);
        let obj = out.object.expect("根形状候选作为报告对象");
        assert!(
            obj.get("schema_version").is_some() && obj.get("tasks").is_some(),
            "强标记双键候选胜出，而不是更大的 SMART 块"
        );

        // 有根合同校验时，通过合同的根对象直接 contract_matched
        let out2 = choose(&text, &AnyTaskCheck, ExtractPolicy::Adapt);
        assert_eq!(out2.stage, ExtractionStage::ContractMatched);
        assert_eq!(out2.object.unwrap()["summary"], serde_json::json!("ok"));
    }

    #[test]
    fn selftest_no_contract_candidate() {
        let bare = r#"{"task_key":"k","title":"t","steps":[]}"#;
        let out = choose(&format!("结果：{bare}"), &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.stage, ExtractionStage::NoContractCandidate);
        assert_eq!(out.object, Some(serde_json::from_str(bare).unwrap()));
    }

    #[test]
    fn selftest_no_json_object() {
        let out = choose("no json here", &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.stage, ExtractionStage::NoJsonObject);
        assert!(out.object.is_none());
    }

    #[test]
    fn selftest_wrapped_bare_task() {
        let bare = r#"{"task_key":"k","title":"t","summary":"自带摘要","steps":[]}"#;
        let out = choose(
            &format!("结果：{bare}"),
            &AnyTaskCheck,
            ExtractPolicy::Adapt,
        );
        assert_eq!(out.stage, ExtractionStage::WrappedBareTask);
        assert!(out.envelope_locally_reconstructed);
        let obj = out.object.unwrap();
        assert_eq!(obj["schema_version"], serde_json::json!("2"));
        assert_eq!(obj["summary"], serde_json::json!("自带摘要"));
        assert_eq!(obj["tasks"].as_array().map(Vec::len), Some(1));
        assert_eq!(obj["tasks"][0]["task_key"], serde_json::json!("k"));
        // strict 策略不包装裸任务对象
        let strict = choose(
            &format!("结果：{bare}"),
            &AnyTaskCheck,
            ExtractPolicy::Strict,
        );
        assert_eq!(strict.stage, ExtractionStage::NoContractCandidate);
        assert!(!strict.envelope_locally_reconstructed);
    }

    #[test]
    fn wrap_bare_task_without_summary_uses_empty_string() {
        let bare = r#"{"task_key":"k","summary":123}"#;
        let out = choose(bare, &AnyTaskCheck, ExtractPolicy::Adapt);
        assert_eq!(out.stage, ExtractionStage::WrappedBareTask);
        assert_eq!(out.object.unwrap()["summary"], serde_json::json!(""));
    }

    #[test]
    fn selftest_ambiguous_bare_tasks() {
        let two = r#"{"task_key":"k","title":"t","steps":[]}"#;
        let two = format!("{two} {}", r#"{"task_key":"k2","title":"t2","steps":[]}"#);
        let out = choose(&two, &AnyTaskCheck, ExtractPolicy::Adapt);
        assert_eq!(out.stage, ExtractionStage::AmbiguousBareTasks);
        assert!(!out.envelope_locally_reconstructed);
        // 多个裸任务候选时拒绝：不得猜测包装（对象仅作诊断报告）
        assert!(out.object.is_some());
    }

    #[test]
    fn contract_matched_takes_first_passing_candidate() {
        let text = "说明 {\"task_key\":\"x\"} 结果 {\"schema_version\":\"2\",\"summary\":\"s\",\"tasks\":[]}";
        let out = choose(text, &AnyTaskCheck, ExtractPolicy::Adapt);
        assert_eq!(out.stage, ExtractionStage::ContractMatched);
        assert!(!out.envelope_locally_reconstructed);
        assert_eq!(out.object.unwrap()["summary"], serde_json::json!("s"));
    }

    #[test]
    fn chinese_wrapper_and_nested_text() {
        let inner = r#"{"schema_version":"2","summary":"中文摘要","tasks":[{"task_key":"a"}]}"#;
        let text = format!("模型返回如下内容：\n```json\n{inner}\n```\n以上是全部输出。");
        let out = choose(&text, &AnyTaskCheck, ExtractPolicy::Adapt);
        assert_eq!(out.stage, ExtractionStage::ContractMatched);
        assert_eq!(
            out.object.unwrap()["summary"],
            serde_json::json!("中文摘要")
        );
    }

    #[test]
    fn truncated_object_is_skipped() {
        let text = "{\"a\": {\"b\": 1";
        assert!(scan_candidates(text).is_empty());
        let out = choose(text, &NoCheck, ExtractPolicy::Adapt);
        assert_eq!(out.stage, ExtractionStage::NoJsonObject);
    }

    #[test]
    fn summary_alone_is_not_root_marker() {
        // "summary" 单独出现不算根形状证据（任务对象自带 summary）
        let text = r#"{"summary":"任务自带摘要","task_key":"k"}"#;
        let out = choose(text, &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.stage, ExtractionStage::NoContractCandidate);
    }

    #[test]
    fn root_marker_count_prefers_two_key_candidate() {
        let text = format!(
            "{{\"schema_version\":\"2\"}} {}",
            r#"{"schema_version":"bad","tasks":[]}"#
        );
        let out = choose(&text, &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.stage, ExtractionStage::RootShapeButInvalid);
        // 双强标记候选胜过单强标记候选
        assert!(out.object.unwrap().get("tasks").is_some());
    }

    #[test]
    fn candidate_keys_summary_sorted_and_capped() {
        let mut text = String::new();
        for i in 0..15 {
            text.push_str(&format!("{{\"b{i}\":1,\"a{i}\":2}} "));
        }
        let out = choose(&text, &NoCheck, ExtractPolicy::Strict);
        assert_eq!(out.candidate_top_level_keys.len(), 12, "截取前 12 个");
        assert_eq!(
            out.candidate_top_level_keys[0],
            vec!["a0".to_string(), "b0".to_string()]
        );
    }
}
