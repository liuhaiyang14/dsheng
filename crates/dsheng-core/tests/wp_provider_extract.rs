//! S1 C 方案适配层集成测试（纯逻辑层，非 Provider 调用；无网络、无 Mock）。
//!
//! 行为对照基准：`python3 tools/extract_provider_json.py --policy adapt`
//! 对同一组夹具的输出（2026-11-02 实测，见工作包报告）：
//!
//! | 夹具                     | Python stage           | Rust stage（本文件断言） |
//! |--------------------------|------------------------|--------------------------|
//! | f1 SMART 干扰 + 合法根   | contract_matched       | contract_matched         |
//! | f2 中文/代码块包装合法根 | contract_matched       | contract_matched         |
//! | f3 唯一裸任务对象        | wrapped_bare_task(+flag) | wrapped_bare_task(+flag) |
//! | f4 两个裸任务对象        | ambiguous_bare_tasks   | ambiguous_bare_tasks     |
//! | f5 根形状但非法          | root_shape_but_invalid | root_shape_but_invalid   |
//! | f6 无 JSON               | no_json_object         | no_json_object           |
//! | f7 嵌套/字符串花括号     | no_contract_candidate  | no_contract_candidate    |
//!
//! 夹具全部为合成数据。S1 层用谓词注入模拟合同校验
//! （完整 JSON Schema 行为对照在 S2 测试 wp_provider_schema.rs 中进行）。

use dsheng_core::contracts::provider::ExtractionStage;
use dsheng_core::provider::extract::{choose, ExtractPolicy, ExtractionChecks};
use serde_json::{json, Value};

/// 与 Python 参考同构的"根合同"谓词（S1 层近似：完整 schema 见 S2）。
struct ApproxRootCheck;

const TASK_REQUIRED_KEYS: [&str; 20] = [
    "task_key",
    "title",
    "summary",
    "category",
    "metric_id",
    "source_period",
    "target_period",
    "baseline",
    "target_value",
    "unit",
    "owner",
    "planned_start",
    "due_date",
    "steps",
    "resource_need",
    "success_criteria",
    "evidence_ids",
    "assumptions",
    "needs_review",
    "smart_check",
];

impl ExtractionChecks for ApproxRootCheck {
    fn root_contract_passes(&self, obj: &Value) -> bool {
        obj.get("schema_version").and_then(Value::as_str) == Some("2")
            && obj.get("summary").is_some_and(Value::is_string)
            && obj.get("tasks").is_some_and(Value::is_array)
            && obj["tasks"]
                .as_array()
                .map(|t| t.iter().all(task_is_complete))
                .unwrap_or(true)
    }
    fn task_subschema_passes(&self, obj: &Value) -> bool {
        task_is_complete(obj)
    }
}

fn task_is_complete(obj: &Value) -> bool {
    TASK_REQUIRED_KEYS.iter().all(|k| obj.get(*k).is_some())
}

/// 合成完整任务对象（20 字段齐全；数值/日期/证据 ID 均为占位合成值）。
fn full_task() -> Value {
    let evidence = "cell:00000000-0000-4000-8000-000000000010:数据:B2";
    json!({
        "task_key": "candidate-task-1",
        "title": "提升10月合成销售额",
        "summary": "来源记录显示9月合成销售额为100.25；10月目标不低于100。",
        "category": "revenue",
        "metric_id": "00000000-0000-4000-8000-000000000020",
        "source_period": {"from": "2026-09-01", "to": "2026-09-30"},
        "target_period": {"from": "2026-10-01", "to": "2026-10-31"},
        "baseline": {"value": 100.25, "unit": "元"},
        "target_value": 100,
        "unit": "元",
        "owner": null,
        "planned_start": "2026-10-01",
        "due_date": "2026-10-31",
        "steps": [{
            "step_id": "candidate-step-1",
            "order": 1,
            "action": "与负责人确认10月销售改善措施",
            "owner": null,
            "start_date": null,
            "due_date": null,
            "done_criteria": "措施清单获负责人确认",
            "evidence_ids": [evidence],
            "status": "not_started"
        }],
        "resource_need": null,
        "success_criteria": "10月合成销售额不低于100元",
        "evidence_ids": [evidence],
        "assumptions": [],
        "needs_review": ["步骤执行日期待确认"],
        "smart_check": {
            "overall_goal": {
                "goal_text": "将2026年10月合成销售额改善至不低于100",
                "criteria": {
                    "S": {"status": "met", "rationale": "目标指标与数值明确", "evidence_ids": []},
                    "M": {"status": "met", "rationale": "以销售额数值衡量", "evidence_ids": []},
                    "A": {"status": "needs_confirmation", "rationale": "改善幅度依据待确认", "evidence_ids": []},
                    "R": {"status": "met", "rationale": "与经营重点相关", "evidence_ids": []},
                    "T": {"status": "met", "rationale": "目标期间为2026年10月", "evidence_ids": []}
                }
            },
            "subgoals": []
        }
    })
}

fn fixture_smart_plus_root() -> String {
    let smart = {
        let mut obj = serde_json::Map::new();
        for (k, v) in [("S", "x"), ("M", "y"), ("A", "z"), ("R", "w"), ("T", "v")] {
            obj.insert(k.into(), json!(v.repeat(200)));
        }
        serde_json::to_string(&Value::Object(obj)).unwrap()
    };
    let root = r#"{"schema_version":"2","summary":"ok","tasks":[]}"#;
    format!("分析：{smart}\n最终：{root}\n")
}

#[test]
fn f1_smart_plus_root_contract_matched() {
    let out = choose(
        &fixture_smart_plus_root(),
        &ApproxRootCheck,
        ExtractPolicy::Adapt,
    );
    assert_eq!(out.stage, ExtractionStage::ContractMatched);
    assert!(!out.envelope_locally_reconstructed);
    assert_eq!(out.object.unwrap()["summary"], json!("ok"));
}

#[test]
fn f2_chinese_wrapper_root_contract_matched() {
    let root = json!({"schema_version": "2", "summary": "s", "tasks": [full_task()]});
    let text = format!(
        "模型输出如下：\n```json\n{}\n```\n请查收。",
        serde_json::to_string(&root).unwrap()
    );
    let out = choose(&text, &ApproxRootCheck, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::ContractMatched);
    assert_eq!(
        out.object.unwrap()["tasks"].as_array().map(Vec::len),
        Some(1)
    );
}

#[test]
fn f3_single_bare_task_wrapped() {
    let text = format!("结果：{}", serde_json::to_string(&full_task()).unwrap());
    let out = choose(&text, &ApproxRootCheck, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::WrappedBareTask);
    assert!(out.envelope_locally_reconstructed, "必须持久化标记");
    let obj = out.object.unwrap();
    assert_eq!(obj["schema_version"], json!("2"));
    assert_eq!(
        obj["summary"],
        json!("来源记录显示9月合成销售额为100.25；10月目标不低于100。")
    );
    // 包装后必须重新通过根合同（Python: contract_pass）
    assert!(ApproxRootCheck.root_contract_passes(&obj));
    // 数值词法不经 f64 往返
    assert_eq!(obj["tasks"][0]["baseline"]["value"].to_string(), "100.25");
}

#[test]
fn f4_two_bare_tasks_ambiguous() {
    let mut second = full_task();
    second["task_key"] = json!("k2");
    let text = format!(
        "{} {}",
        serde_json::to_string(&full_task()).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    let out = choose(&text, &ApproxRootCheck, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::AmbiguousBareTasks);
    assert!(
        !out.envelope_locally_reconstructed,
        "多个候选拒绝，不得猜测包装"
    );
}

#[test]
fn f5_root_shape_but_invalid() {
    let bad = r#"{"schema_version":"3","summary":"s","tasks":[]}"#;
    let text = format!("最终结果：{bad}");
    let out = choose(&text, &ApproxRootCheck, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::RootShapeButInvalid);
    assert!(!out.envelope_locally_reconstructed);
    // 根形状判定只用 schema_version/tasks 强标记；非法根不得被包装成合同对象
    assert_eq!(out.object.unwrap()["schema_version"], json!("3"));
}

#[test]
fn f6_no_json_object() {
    let out = choose(
        "抱歉，本次没有生成结构化输出。",
        &ApproxRootCheck,
        ExtractPolicy::Adapt,
    );
    assert_eq!(out.stage, ExtractionStage::NoJsonObject);
    assert!(out.object.is_none());
}

#[test]
fn f7_nested_braces_no_contract_candidate() {
    let text = r#"说明 {"outer": "含}花括号{文本", "inner": {"x": 1}} 完成"#;
    let out = choose(text, &ApproxRootCheck, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::NoContractCandidate);
}

#[test]
fn extraction_outcome_mapping() {
    let out = choose(
        &format!("结果：{}", serde_json::to_string(&full_task()).unwrap()),
        &ApproxRootCheck,
        ExtractPolicy::Adapt,
    );
    let outcome = out.to_extraction_outcome();
    assert_eq!(outcome.stage, ExtractionStage::WrappedBareTask);
    assert!(outcome.envelope_locally_reconstructed);
    assert!(outcome.object.is_some());
}
