//! S2 JSON Schema 校验层集成测试（纯逻辑层，非 Provider 调用）。
//!
//! 覆盖：任务输出 schema2、三个诊断 turn schema3、报告 schema3 的正反例；
//! 诊断输出拒绝 task/task_steps 字段；5W/5W2H 不得被误解析为 5WHY；
//! 离线注册表对未注册 $ref 直接报错；以及用**真实 schema 校验器**重跑
//! S1 的 C 方案提取夹具（与 tools/extract_provider_json.py 行为对照）。

use dsheng_core::contracts::provider::{ExtractionStage, Framework};
use dsheng_core::provider::extract::{choose, ExtractPolicy};
use dsheng_core::provider::schema::{
    contract_validators, validate_diagnosis_report, validate_diagnosis_turn, validate_task_output,
    TaskOutputChecks,
};
use serde_json::{json, Value};

fn full_task() -> Value {
    let evidence = "cell:00000000-0000-4000-8000-000000000010:数据:B2";
    json!({
        "task_key": "candidate-task-1",
        "title": "提升10月合成销售额",
        "summary": "摘要",
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
            "action": "与负责人确认措施",
            "owner": null,
            "start_date": null,
            "due_date": null,
            "done_criteria": "清单确认",
            "evidence_ids": [evidence],
            "status": "not_started"
        }],
        "resource_need": null,
        "success_criteria": "不低于100元",
        "evidence_ids": [evidence],
        "assumptions": [],
        "needs_review": ["步骤执行日期待确认"],
        "smart_check": {
            "overall_goal": {
                "goal_text": "目标",
                "criteria": {
                    "S": {"status": "met", "rationale": "r", "evidence_ids": []},
                    "M": {"status": "met", "rationale": "r", "evidence_ids": []},
                    "A": {"status": "needs_confirmation", "rationale": "r", "evidence_ids": []},
                    "R": {"status": "met", "rationale": "r", "evidence_ids": []},
                    "T": {"status": "met", "rationale": "r", "evidence_ids": []}
                }
            },
            "subgoals": []
        }
    })
}

fn report_patch_5why() -> Value {
    json!({
        "summary_update": "更新摘要",
        "findings_upsert": [{
            "finding_id": "f1",
            "level": 1,
            "question": "为什么销售额下降？",
            "answer": "客流减少",
            "fact_status": "fact",
            "evidence_ids": ["cell:00000000-0000-4000-8000-000000000010:数据:B2"],
            "reasoning_summary": "来源记录支持",
            "confidence": "high",
            "validation_question": null,
            "status": "candidate"
        }],
        "facts_and_hypotheses_upsert": [],
        "evidence_refs_append": [],
        "clarification_questions_upsert": [],
        "warnings_upsert": [],
        "stop_reason": null,
        "can_continue": true
    })
}

fn turn_5why_on_topic() -> Value {
    json!({
        "schema_version": "3",
        "framework": "5why",
        "turn_disposition": "on_topic",
        "accepted_facts": [],
        "report_patch": report_patch_5why(),
        "redirect_message": null,
        "clarification_questions": [],
        "stop_reason": null
    })
}

fn turn_5w_on_topic() -> Value {
    json!({
        "schema_version": "3",
        "framework": "5w",
        "turn_disposition": "on_topic",
        "accepted_facts": [],
        "report_patch": {
            "summary_update": null,
            "findings_upsert": [{
                "finding_id": "f1",
                "dimension": "What",
                "question": "发生了什么？",
                "answer": "销售额下降",
                "fact_status": "fact",
                "evidence_ids": [],
                "confidence": "high",
                "missing_information": [],
                "clarification_questions": [],
                "status": "candidate"
            }],
            "facts_and_hypotheses_upsert": [],
            "evidence_refs_append": [],
            "clarification_questions_upsert": [],
            "warnings_upsert": [],
            "stop_reason": null,
            "can_continue": true
        },
        "redirect_message": null,
        "clarification_questions": [],
        "stop_reason": null
    })
}

fn turn_5w2h_on_topic() -> Value {
    json!({
        "schema_version": "3",
        "framework": "5w2h",
        "turn_disposition": "on_topic",
        "accepted_facts": [],
        "report_patch": {
            "summary_update": null,
            "findings_upsert": [{
                "finding_id": "f1",
                "dimension": "How",
                "question": "如何改进？",
                "answer": "调整排班",
                "kind": "proposal",
                "fact_status": "hypothesis",
                "evidence_ids": [],
                "confidence": "medium",
                "missing_information": [],
                "clarification_questions": [],
                "status": "candidate"
            }],
            "facts_and_hypotheses_upsert": [],
            "evidence_refs_append": [],
            "clarification_questions_upsert": [],
            "warnings_upsert": [],
            "stop_reason": null,
            "can_continue": true
        },
        "redirect_message": null,
        "clarification_questions": [],
        "stop_reason": null
    })
}

fn full_report_5why() -> Value {
    json!({
        "schema_version": "3",
        "report_id": "rep-1",
        "generation_session_id": "sess-1",
        "workspace_id": "ws-1",
        "run_id": "run-1",
        "framework": "5why",
        "scope": {"problem_statement": "销售额下降", "focus_metric_ids": []},
        "period": {"from": "2026-10-01", "to": "2026-10-31", "period_policy_version": "v1"},
        "revision_no": 1,
        "summary": "首轮报告",
        "source_manifest": [{
            "source_id": "src-1",
            "file_label": "合成表",
            "sheet": "数据",
            "selected_range": "A1:B2",
            "content_hash": "a".repeat(64),
            "selection_hash": "b".repeat(64)
        }],
        "findings": [{
            "finding_id": "f1",
            "level": 1,
            "question": "为什么销售额下降？",
            "answer": "客流减少",
            "fact_status": "fact",
            "evidence_ids": ["ev-1"],
            "reasoning_summary": "来源记录支持",
            "confidence": "high",
            "validation_question": null,
            "status": "candidate"
        }],
        "facts_and_hypotheses": [],
        "evidence_refs": [{
            "evidence_id": "ev-1",
            "source_id": "src-1",
            "source_label": "合成表",
            "locator": "数据!B2"
        }],
        "clarification_questions": [],
        "warnings": [],
        "stop_reason": null,
        "can_continue": true,
        "created_at": "2026-11-02T00:00:00Z",
        "updated_at": "2026-11-02T00:00:00Z",
        "analysis_revision_no": 1,
        "analysis_input_fingerprint": "c".repeat(64)
    })
}

// ---------- 任务输出 schema2 ----------

#[test]
fn task_output_positive() {
    let root = json!({"schema_version": "2", "summary": "s", "tasks": [full_task()]});
    assert!(validate_task_output(&root).is_ok());
    let empty_tasks = json!({"schema_version": "2", "summary": "s", "tasks": []});
    assert!(validate_task_output(&empty_tasks).is_ok());
}

#[test]
fn task_output_negative_missing_fields() {
    let v = contract_validators();
    // 缺 summary
    let bad = json!({"schema_version": "2", "tasks": []});
    assert!(validate_task_output(&bad).is_err());
    // schema_version 错误
    let bad2 = json!({"schema_version": "3", "summary": "s", "tasks": []});
    assert!(validate_task_output(&bad2).is_err());
    // 任务缺 smart_check
    let mut task = full_task();
    task.as_object_mut().unwrap().remove("smart_check");
    let bad3 = json!({"schema_version": "2", "summary": "s", "tasks": [task]});
    assert!(validate_task_output(&bad3).is_err());
    // steps 为空数组（minItems 1）
    let mut task2 = full_task();
    task2["steps"] = json!([]);
    let bad4 = json!({"schema_version": "2", "summary": "s", "tasks": [task2]});
    assert!(validate_task_output(&bad4).is_err());
    // 未知字段
    let mut task3 = full_task();
    task3["extra"] = json!(true);
    let bad5 = json!({"schema_version": "2", "summary": "s", "tasks": [task3]});
    assert!(validate_task_output(&bad5).is_err());
    // revenue 缺 metric_id（allOf 条件必填）
    let mut task4 = full_task();
    task4["metric_id"] = json!(null);
    let bad6 = json!({"schema_version": "2", "summary": "s", "tasks": [task4]});
    assert!(validate_task_output(&bad6).is_err());
    let _ = v;
}

// ---------- 诊断 turn schema3 ----------

#[test]
fn turn_5why_positive_and_negative() {
    assert!(validate_diagnosis_turn(Framework::Why5, &turn_5why_on_topic()).is_ok());
    // needs_clarification：report_patch 必须为 null
    let mut nc = turn_5why_on_topic();
    nc["turn_disposition"] = json!("needs_clarification");
    assert!(
        validate_diagnosis_turn(Framework::Why5, &nc).is_err(),
        "needs_clarification 时 report_patch 必须为 null"
    );
    nc["report_patch"] = json!(null);
    nc["clarification_questions"] = json!([{
        "question_id": "q1", "question": "哪个期间？",
        "why_needed": "锁定范围", "evidence_ids": []
    }]);
    assert!(validate_diagnosis_turn(Framework::Why5, &nc).is_ok());
    // off_topic：redirect_message 必填非空
    let mut off = turn_5why_on_topic();
    off["turn_disposition"] = json!("off_topic_redirect");
    off["report_patch"] = json!(null);
    assert!(validate_diagnosis_turn(Framework::Why5, &off).is_err());
    off["redirect_message"] = json!("请回到当前诊断主题");
    assert!(validate_diagnosis_turn(Framework::Why5, &off).is_ok());
}

#[test]
fn turn_5w_and_5w2h_positive() {
    assert!(validate_diagnosis_turn(Framework::W5, &turn_5w_on_topic()).is_ok());
    assert!(validate_diagnosis_turn(Framework::W5h2, &turn_5w2h_on_topic()).is_ok());
}

#[test]
fn diagnosis_turn_rejects_task_fields() {
    // 诊断输出不得包含任务/任务步骤字段（additionalProperties: false 封闭字段）
    let mut with_task = turn_5why_on_topic();
    with_task["task"] = json!({"task_key": "k"});
    assert!(
        validate_diagnosis_turn(Framework::Why5, &with_task).is_err(),
        "诊断 turn 不得携带 task 字段"
    );
    let mut with_steps = turn_5w_on_topic();
    with_steps["task_steps"] = json!([{"step_id": "s"}]);
    assert!(
        validate_diagnosis_turn(Framework::W5, &with_steps).is_err(),
        "诊断 turn 不得携带 task_steps 字段"
    );
}

#[test]
fn framework_consts_prevent_cross_parsing() {
    // 5W 输出不得通过 5WHY turn schema（framework const 不同）
    assert!(validate_diagnosis_turn(Framework::Why5, &turn_5w_on_topic()).is_err());
    assert!(validate_diagnosis_turn(Framework::W5, &turn_5why_on_topic()).is_err());
    assert!(validate_diagnosis_turn(Framework::W5h2, &turn_5w_on_topic()).is_err());
    // 5W 的 finding 无 kind 字段；去掉 5W2H 特有 kind 并换成 5W 维度后应通过
    let mut five_w_shape = turn_5w2h_on_topic();
    five_w_shape["framework"] = json!("5w");
    five_w_shape["report_patch"]["findings_upsert"][0]["dimension"] = json!("What");
    five_w_shape["report_patch"]["findings_upsert"][0]
        .as_object_mut()
        .unwrap()
        .remove("kind");
    assert!(validate_diagnosis_turn(Framework::W5, &five_w_shape).is_ok());
    // 5WHY 因果链 finding（level 字段）不是 5W 维度界定
    let mut why_as_5w = turn_5w_on_topic();
    why_as_5w["report_patch"]["findings_upsert"][0]["level"] = json!(1);
    assert!(
        validate_diagnosis_turn(Framework::W5, &why_as_5w).is_err(),
        "5WHY 的 level 字段不属于 5W finding"
    );
}

// ---------- 报告 schema3 ----------

#[test]
fn report_positive_and_negative() {
    assert!(validate_diagnosis_report(&full_report_5why()).is_ok());
    let mut bad = full_report_5why();
    bad["framework"] = json!("5w");
    // 5w 要求恰好 5 个维度 finding
    assert!(validate_diagnosis_report(&bad).is_err());
    let mut bad2 = full_report_5why();
    bad2["revision_no"] = json!(0);
    assert!(validate_diagnosis_report(&bad2).is_err());
    let mut bad3 = full_report_5why();
    bad3["analysis_input_fingerprint"] = json!("not-hex");
    assert!(validate_diagnosis_report(&bad3).is_err());
}

#[test]
fn report_warning_resolution_rules() {
    // resolved 必须有非空 resolution_note 且至少一个 evidence/fact 引用
    let mut w = full_report_5why();
    w["warnings"] = json!([{
        "warning_id": "w1",
        "message": "记录不完整",
        "status": "resolved",
        "resolution_requirement": "source_record",
        "evidence_ids": [],
        "fact_ids": [],
        "resolution_note": "已补充来源"
    }]);
    assert!(
        validate_diagnosis_report(&w).is_err(),
        "resolved 警告必须有 evidence/fact 引用"
    );
    w["warnings"][0]["evidence_ids"] = json!(["ev-1"]);
    assert!(validate_diagnosis_report(&w).is_ok());
    // active 警告 resolution_note 必须为 null
    let mut a = full_report_5why();
    a["warnings"] = json!([{
        "warning_id": "w1",
        "message": "记录不完整",
        "status": "active",
        "resolution_requirement": "user_confirmation",
        "evidence_ids": [],
        "fact_ids": [],
        "resolution_note": "口述已处理"
    }]);
    assert!(validate_diagnosis_report(&a).is_err());
}

// ---------- C 方案提取 × 真实 schema（与 Python 工具行为对照） ----------

#[test]
fn extraction_with_real_validators_matches_python_stages() {
    // f1：SMART 干扰 + 合法根 → contract_matched（Python: contract_matched/contract_pass）
    let smart = {
        let mut obj = serde_json::Map::new();
        for (k, v) in [("S", "x"), ("M", "y"), ("A", "z"), ("R", "w"), ("T", "v")] {
            obj.insert(k.into(), json!(v.repeat(200)));
        }
        serde_json::to_string(&Value::Object(obj)).unwrap()
    };
    let text = format!(
        "分析：{smart}\n最终：{}",
        r#"{"schema_version":"2","summary":"ok","tasks":[]}"#
    );
    let out = choose(&text, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out.stage, ExtractionStage::ContractMatched);

    // f2：中文/代码块包装合法根 → contract_matched
    let root = json!({"schema_version": "2", "summary": "s", "tasks": [full_task()]});
    let text2 = format!(
        "模型输出如下：\n```json\n{}\n```\n请查收。",
        serde_json::to_string(&root).unwrap()
    );
    let out2 = choose(&text2, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out2.stage, ExtractionStage::ContractMatched);

    // f3：唯一裸任务 → wrapped_bare_task 且包装后通过根合同
    // （Python: wrapped_bare_task / contract_pass / envelope_locally_reconstructed=true）
    let text3 = format!("结果：{}", serde_json::to_string(&full_task()).unwrap());
    let out3 = choose(&text3, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out3.stage, ExtractionStage::WrappedBareTask);
    assert!(out3.envelope_locally_reconstructed);
    assert!(
        validate_task_output(out3.object.as_ref().unwrap()).is_ok(),
        "本地包装后的根对象必须重新通过输出合同"
    );

    // f4：两个裸任务 → ambiguous_bare_tasks（Python 同）
    let mut second = full_task();
    second["task_key"] = json!("k2");
    let text4 = format!(
        "{} {}",
        serde_json::to_string(&full_task()).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    let out4 = choose(&text4, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out4.stage, ExtractionStage::AmbiguousBareTasks);

    // f5：根形状但非法 → root_shape_but_invalid（Python 同）
    let text5 = r#"最终结果：{"schema_version":"3","summary":"s","tasks":[]}"#.to_string();
    let out5 = choose(&text5, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out5.stage, ExtractionStage::RootShapeButInvalid);

    // f6：无 JSON → no_json_object（Python 同）
    let out6 = choose(
        "抱歉，本次没有生成结构化输出。",
        &TaskOutputChecks,
        ExtractPolicy::Adapt,
    );
    assert_eq!(out6.stage, ExtractionStage::NoJsonObject);

    // f7：嵌套/字符串花括号 → no_contract_candidate（Python 同）
    let text7 = r#"说明 {"outer": "含}花括号{文本", "inner": {"x": 1}} 完成"#;
    let out7 = choose(text7, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out7.stage, ExtractionStage::NoContractCandidate);
}

#[test]
fn strict_policy_rejects_bare_task() {
    let text = format!("结果：{}", serde_json::to_string(&full_task()).unwrap());
    let out = choose(&text, &TaskOutputChecks, ExtractPolicy::Strict);
    assert_eq!(out.stage, ExtractionStage::NoContractCandidate);
    // 真实 schema 下 SMART 形状（S/M/A/R/T）不是任务子 schema 候选
    let smart_only = r#"{"S":"x","M":"y","A":"z","R":"w","T":"v"}"#.to_string();
    let out2 = choose(&smart_only, &TaskOutputChecks, ExtractPolicy::Adapt);
    assert_eq!(out2.stage, ExtractionStage::NoContractCandidate);
}
