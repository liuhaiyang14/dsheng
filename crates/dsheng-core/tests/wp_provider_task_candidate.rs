//! S7 任务候选本地校验集成测试（纯逻辑层，非 Provider 调用）。
//!
//! 全链路（提取 → schema2 → 候选解析 → 本地语义校验 → taskCandidateView）
//! 覆盖：类别 exact 映射拒整份、重复候选键、引用越界、
//! resolved↔冲突组（INPUT_CONTRACT_INVALID）、月度下限复核
//!（完整月+monthly_revenue_floor+up；未解决冲突不参与；
//! 上调候选需 needs_review；非月度/费用仅显式目标）、
//! C 方案 envelope_locally_reconstructed 视图标记。

use chrono::NaiveDate;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::provider::{ExtractionOutcome, ExtractionStage, Framework};
use dsheng_core::generation::input::*;
use dsheng_core::generation::task_candidate::{
    parse_task_output, validate_candidates, validate_input_metric_candidates, CandidateIdentity,
};
use serde_json::{json, Value};

const METRIC_ID: &str = "00000000-0000-4000-8000-000000000020";
const EVIDENCE: &str = "cell:00000000-0000-4000-8000-000000000010:数据:B2";

fn number(value: &str) -> serde_json::Number {
    serde_json::Number::from_string_unchecked(value.to_string())
}

fn candidate(
    value: &str,
    comparable: bool,
    conflict: ConflictStatusProjection,
) -> Option<CandidateProjection> {
    Some(CandidateProjection {
        value: number(value),
        unit: "元".into(),
        evidence_ids: vec![EVIDENCE.into()],
        comparable,
        conflict_status: conflict,
        resolution_id: None,
    })
}

fn metric(
    scope: TargetConstraintScopeProjection,
    direction: DirectionProjection,
) -> MetricProjection {
    MetricProjection {
        metric_id: METRIC_ID.parse().unwrap(),
        label: "合成销售额".into(),
        unit: "元".into(),
        direction,
        target_constraint_scope: scope,
        explicit_target: candidate("100", true, ConflictStatusProjection::None),
        annual_allocated_target: candidate("110", true, ConflictStatusProjection::None),
        prior_year_actual: candidate("95", true, ConflictStatusProjection::None),
        target: None,
        actual: None,
        delta: None,
        mom: None,
        yoy: None,
        evidence_ids: vec![EVIDENCE.into()],
    }
}

fn task_input(metric: MetricProjection) -> TaskInputProjection {
    TaskInputProjection {
        request: TaskRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: "run-1".into(),
            analysis_revision_no: 1,
            analysis_input_fingerprint: "f".repeat(64),
            prompt_version: TaskPromptVersion::V23,
            workflow: TaskWorkflowMarker::Task,
            schema_version: TaskSchemaVersion::V2,
        },
        workspace: WorkspaceProjection {
            workspace_id: "ws-1".into(),
            industry: None,
            cadence: CadenceProjection::Month,
            timezone: "Asia/Shanghai".into(),
            metric_definitions: vec![],
        },
        metrics: vec![metric],
        source_manifest: vec![],
        source_records: vec![SourceRecordProjection {
            evidence_id: EVIDENCE.into(),
            source_id: "00000000-0000-4000-8000-000000000010".parse().unwrap(),
            file_label: "合成经营表.xlsx".into(),
            sheet: Some("数据".into()),
            cell_or_row: "B2".into(),
            values: SourceCellValues {
                cell_type: CellTypeProjection::Number,
                raw_text: "100.25".into(),
                typed_value: Some(TypedCellValue::Str("100.25".into())),
                formula: None,
                value_state: ValueStateProjection::Valid,
                number_format: Some("0.00".into()),
                merged_range: None,
                mapping: None,
            },
        }],
        manual_inputs: vec![],
        missing_or_anomalous_inputs: vec![],
        source_conflicts: vec![],
        period: TaskPeriodProjection {
            source_from: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            source_to: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            target_from: NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            target_to: NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
            target_grain: CadenceProjection::Month,
            period_policy_version: "pp-1".into(),
        },
        business_review: BusinessReviewProjection {
            summary: "9月合成销售额 100.25 元。".into(),
            prior_task_outcomes: vec![],
        },
        task_skill_context: TaskSkillContextProjection {
            rules_version: TaskPromptVersion::V23,
            required_fields: RequiredFieldsConst,
            category_options: CategoryOptionsConst,
        },
        optional_diagnosis_reference: None,
        user_context: None,
        output_contract: TaskOutputContract {
            schema: TaskOutputSchemaId::TaskOutputV2,
            schema_version: TaskSchemaVersion::V2,
        },
    }
}

/// 模型任务输出（通过 schema2 的根对象）。
fn model_task_output(target: Value, needs_review: Vec<&str>) -> Value {
    json!({
        "schema_version": "2",
        "summary": "基于来源记录生成任务",
        "tasks": [{
            "task_key": "candidate-task-1",
            "title": "提升10月合成销售额",
            "summary": "摘要",
            "category": "revenue",
            "metric_id": METRIC_ID,
            "source_period": {"from": "2026-09-01", "to": "2026-09-30"},
            "target_period": {"from": "2026-10-01", "to": "2026-10-31"},
            "baseline": {"value": 100.25, "unit": "元"},
            "target_value": target,
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
                "evidence_ids": [EVIDENCE],
                "status": "not_started"
            }],
            "resource_need": null,
            "success_criteria": "不低于目标",
            "evidence_ids": [EVIDENCE],
            "assumptions": [],
            "needs_review": needs_review,
            "smart_check": {
                "overall_goal": {
                    "goal_text": "目标",
                    "criteria": {
                        "S": {"status": "met", "rationale": "r", "evidence_ids": [EVIDENCE]},
                        "M": {"status": "met", "rationale": "r", "evidence_ids": []},
                        "A": {"status": "needs_confirmation", "rationale": "r", "evidence_ids": []},
                        "R": {"status": "met", "rationale": "r", "evidence_ids": []},
                        "T": {"status": "met", "rationale": "r", "evidence_ids": []}
                    }
                },
                "subgoals": []
            }
        }]
    })
}

fn extract_matched(root: Value) -> ExtractionOutcome {
    ExtractionOutcome {
        stage: ExtractionStage::ContractMatched,
        object: Some(root),
        envelope_locally_reconstructed: false,
    }
}

fn identity() -> CandidateIdentity {
    CandidateIdentity {
        candidate_generation_id: "00000000-0000-4000-8000-0000000000aa".parse().unwrap(),
        candidate_revision_no: 1,
        run_id: "run-1".into(),
        analysis_revision_no: 1,
    }
}

#[test]
fn happy_path_view_and_canonical_numbers() {
    let input = task_input(metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    ));
    let root = model_task_output(json!(100), vec!["步骤执行日期待确认"]);
    let set = parse_task_output(&extract_matched(root)).unwrap();
    let view = validate_candidates(&set, &input, &identity()).unwrap();
    assert!(!view.envelope_locally_reconstructed);
    assert_eq!(
        serde_json::to_value(view.state).unwrap(),
        json!("candidate")
    );
    assert_eq!(view.tasks.len(), 1);
    assert_eq!(view.tasks[0].task_key, "candidate-task-1");
    assert_eq!(view.tasks[0].target_value.as_deref(), Some("100"));
    assert_eq!(view.tasks[0].steps[0].step_id, "candidate-step-1");
    // 尾随零规范化（100.2500 → 100.25）
    let root2 = model_task_output(json!(100.2500), vec!["x"]);
    let set2 = parse_task_output(&extract_matched(root2)).unwrap();
    assert_eq!(set2.tasks[0].baseline_value.as_deref(), Some("100.25"));
}

#[test]
fn wrapped_bare_task_sets_envelope_flag_in_view() {
    // C 方案唯一裸任务 → 本地包装 → 视图必须带 envelope_locally_reconstructed
    let mut task = model_task_output(json!(100), vec!["候选上调待采纳"])["tasks"][0].clone();
    task["target_value"] = json!(110);
    let wrapped = ExtractionOutcome {
        stage: ExtractionStage::WrappedBareTask,
        object: Some(
            json!({"schema_version": "2", "summary": task["summary"].clone(), "tasks": [task]}),
        ),
        envelope_locally_reconstructed: true,
    };
    let input = task_input(metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    ));
    let set = parse_task_output(&wrapped).unwrap();
    let view = validate_candidates(&set, &input, &identity()).unwrap();
    assert!(view.envelope_locally_reconstructed, "视图必须暴露包装标记");
}

#[test]
fn unknown_category_rejects_whole_set() {
    let mut root = model_task_output(json!(100), vec![]);
    root["tasks"][0]["category"] = json!("增长");
    let set = parse_task_output(&extract_matched(root)).unwrap_err();
    assert_eq!(set.code, CliErrorCode::TaskCategoryUnrecognized);
}

#[test]
fn duplicate_keys_rejected() {
    let mut root = model_task_output(json!(100), vec![]);
    let first = root["tasks"][0].clone();
    root["tasks"].as_array_mut().unwrap().push(first);
    let input = task_input(metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    ));
    let set = parse_task_output(&extract_matched(root)).unwrap();
    let err = validate_candidates(&set, &input, &identity()).unwrap_err();
    assert_eq!(err.code, CliErrorCode::DuplicateCandidateKey);
    // step_id 重复
    let mut root2 = model_task_output(json!(100), vec![]);
    let dup_step = root2["tasks"][0]["steps"][0].clone();
    root2["tasks"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(dup_step);
    let set2 = parse_task_output(&extract_matched(root2)).unwrap();
    assert_eq!(
        validate_candidates(&set2, &input, &identity())
            .unwrap_err()
            .code,
        CliErrorCode::DuplicateCandidateKey
    );
}

#[test]
fn evidence_and_metric_reference_domain() {
    let input = task_input(metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    ));
    // 引用未知 evidence
    let mut root = model_task_output(json!(100), vec![]);
    root["tasks"][0]["evidence_ids"] = json!(["cell:unknown"]);
    let set = parse_task_output(&extract_matched(root)).unwrap();
    let err = validate_candidates(&set, &input, &identity()).unwrap_err();
    assert_eq!(err.code, CliErrorCode::SemanticValidationFailed);
    assert!(err.detail.contains("获准集合"));
    // 引用输入外指标
    let mut root2 = model_task_output(json!(100), vec![]);
    root2["tasks"][0]["metric_id"] = json!("00000000-0000-4000-8000-000000000099");
    let set2 = parse_task_output(&extract_matched(root2)).unwrap();
    assert_eq!(
        validate_candidates(&set2, &input, &identity())
            .unwrap_err()
            .code,
        CliErrorCode::TaskCategoryMetricMismatch
    );
    // other 类可无指标
    let mut root3 = model_task_output(json!(100), vec![]);
    root3["tasks"][0]["category"] = json!("other");
    root3["tasks"][0]["metric_id"] = Value::Null;
    // other 无指标 → 仅显式路径按无指标处理（目标须 null）
    root3["tasks"][0]["target_value"] = Value::Null;
    let set3 = parse_task_output(&extract_matched(root3)).unwrap();
    assert!(validate_candidates(&set3, &input, &identity()).is_ok());
}

#[test]
fn monthly_floor_full_month_revenue_up() {
    let input = task_input(metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    ));
    // 下限 = max(合法显式100, 年度当月分解110, 去年同月95) = 110
    // 模型输出 100（低于下限）→ 拒绝
    let low = parse_task_output(&extract_matched(model_task_output(
        json!(100),
        vec!["其他"],
    )))
    .unwrap();
    let err = validate_candidates(&low, &input, &identity()).unwrap_err();
    assert_eq!(err.code, CliErrorCode::SemanticValidationFailed);
    assert!(
        err.detail.contains("月度下限复核失败"),
        "实际: {}",
        err.detail
    );

    // 输出上调值 110 但 needs_review 为空 → 拒绝（候选上调待采纳）
    let up_no_review =
        parse_task_output(&extract_matched(model_task_output(json!(110), vec![]))).unwrap();
    let err2 = validate_candidates(&up_no_review, &input, &identity()).unwrap_err();
    assert!(
        err2.detail.contains("候选上调待采纳"),
        "实际: {}",
        err2.detail
    );

    // 输出 110 + needs_review → 通过；视图 target=110（原输入 explicit=100 不改写）
    let up = parse_task_output(&extract_matched(model_task_output(
        json!(110),
        vec!["目标由年度当月分解110上调，候选上调待采纳"],
    )))
    .unwrap();
    let view = validate_candidates(&up, &input, &identity()).unwrap();
    assert_eq!(view.tasks[0].target_value.as_deref(), Some("110"));
    assert_eq!(
        input.metrics[0]
            .explicit_target
            .as_ref()
            .unwrap()
            .value
            .to_string(),
        "100",
        "原目标输入不被改写"
    );

    // 年度当月分解带未解决冲突 → 不参与，期望 = max(显式100, 去年95) = 100
    let mut m = metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    );
    m.annual_allocated_target = candidate("110", true, ConflictStatusProjection::Unresolved);
    let input2 = task_input(m);
    let ok = parse_task_output(&extract_matched(model_task_output(
        json!(100),
        vec!["其他"],
    )))
    .unwrap();
    assert!(validate_candidates(&ok, &input2, &identity()).is_ok());

    // 不可比候选同样不参与（prior 不可比）
    let mut m2 = metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    );
    m2.annual_allocated_target = candidate("110", false, ConflictStatusProjection::None);
    m2.prior_year_actual = candidate("95", false, ConflictStatusProjection::None);
    let input3 = task_input(m2);
    let ok2 = parse_task_output(&extract_matched(model_task_output(
        json!(100),
        vec!["其他"],
    )))
    .unwrap();
    assert!(validate_candidates(&ok2, &input3, &identity()).is_ok());
    // 但若所有候选不可比 → 无依据 → 目标必须 null
    let mut m3 = metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    );
    m3.explicit_target = candidate("100", false, ConflictStatusProjection::None);
    m3.annual_allocated_target = candidate("110", false, ConflictStatusProjection::None);
    m3.prior_year_actual = candidate("95", false, ConflictStatusProjection::None);
    let input4 = task_input(m3);
    let with_target =
        parse_task_output(&extract_matched(model_task_output(json!(100), vec!["x"]))).unwrap();
    let err3 = validate_candidates(&with_target, &input4, &identity()).unwrap_err();
    assert!(err3.detail.contains("必须为 null"), "实际: {}", err3.detail);
}

#[test]
fn monthly_floor_not_applied_outside_full_month_or_scope() {
    // 非完整月（10-02 起）→ 不套下限，仅显式目标
    let input = task_input(metric(
        TargetConstraintScopeProjection::MonthlyRevenueFloor,
        DirectionProjection::Up,
    ));
    let mut root = model_task_output(json!(100), vec![]);
    root["tasks"][0]["target_period"] = json!({"from": "2026-10-02", "to": "2026-10-31"});
    let set = parse_task_output(&extract_matched(root)).unwrap();
    assert!(validate_candidates(&set, &input, &identity()).is_ok());
    // 非完整月却输出年度分解值 110 → 拒绝（不得套用下限推高）
    let mut root2 = model_task_output(json!(110), vec![]);
    root2["tasks"][0]["target_period"] = json!({"from": "2026-10-02", "to": "2026-10-31"});
    let set2 = parse_task_output(&extract_matched(root2)).unwrap();
    assert!(validate_candidates(&set2, &input, &identity()).is_err());

    // 费用（direction=down + explicit_target_only）：仅显式目标，不推降幅
    let cost = metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Down,
    );
    let input2 = task_input(cost);
    let ok = parse_task_output(&extract_matched(model_task_output(
        json!(100),
        vec!["其他"],
    )))
    .unwrap();
    assert!(validate_candidates(&ok, &input2, &identity()).is_ok());
    let mut m = metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Down,
    );
    // 费用也不得拿去年/年度数字推低（95 < 100 也不采用）
    m.prior_year_actual = candidate("80", true, ConflictStatusProjection::None);
    let input3 = task_input(m);
    let low = parse_task_output(&extract_matched(model_task_output(json!(80), vec!["x"]))).unwrap();
    let err = validate_candidates(&low, &input3, &identity()).unwrap_err();
    assert!(err.detail.contains("不得推算"), "实际: {}", err.detail);
}

#[test]
fn input_candidate_resolution_correspondence() {
    // resolved 候选无对应冲突组 → INPUT_CONTRACT_INVALID
    let mut m = metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    );
    m.explicit_target = candidate("100", true, ConflictStatusProjection::Resolved);
    m.explicit_target.as_mut().unwrap().resolution_id =
        Some("00000000-0000-4000-8000-0000000000cc".into());
    let input = task_input(m);
    let err = validate_input_metric_candidates(&input).unwrap_err();
    assert_eq!(err.code, CliErrorCode::InputContractInvalid);
    assert!(err.detail.contains("不对应"));

    // 加入对应已解决冲突组后通过
    let mut input2 = input.clone();
    input2.source_conflicts = vec![SourceConflictProjection {
        conflict_id: "00000000-0000-4000-8000-0000000000bb".parse().unwrap(),
        metric_id: METRIC_ID.parse().unwrap(),
        measure_role: MeasureRoleProjection::Actual,
        period: "2026-09-01/2026-09-30".into(),
        scope_key: "default".into(),
        unit: "元".into(),
        candidate_values: vec![
            ConflictCandidateValue {
                value: number("100"),
                evidence_id: EVIDENCE.into(),
                source_label: "表A".into(),
            },
            ConflictCandidateValue {
                value: number("99"),
                evidence_id: "manual:00000000-0000-4000-8000-000000000030:r1".into(),
                source_label: "手工".into(),
            },
        ],
        resolution_status: ResolutionStatusProjection::Resolved,
        resolution_id: Some("00000000-0000-4000-8000-0000000000cc".parse().unwrap()),
        selected_evidence_id: Some(EVIDENCE.into()),
        resolution_note: Some("选择表A".into()),
    }];
    assert!(validate_input_metric_candidates(&input2).is_ok());
    // 非 resolved 携带 resolution_id → 拒绝
    let mut m2 = metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    );
    m2.explicit_target.as_mut().unwrap().resolution_id = Some("x".into());
    let input3 = task_input(m2);
    assert!(validate_input_metric_candidates(&input3).is_err());
}

#[test]
fn view_matches_read_views_shape() {
    let input = task_input(metric(
        TargetConstraintScopeProjection::ExplicitTargetOnly,
        DirectionProjection::Up,
    ));
    let set = parse_task_output(&extract_matched(model_task_output(
        json!(100),
        vec!["待确认"],
    )))
    .unwrap();
    let view = validate_candidates(&set, &input, &identity()).unwrap();
    let value = serde_json::to_value(&view).unwrap();
    // 顶层字段精确匹配（additionalProperties=false）
    let mut keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "analysis_revision_no",
            "candidate_generation_id",
            "candidate_revision_no",
            "envelope_locally_reconstructed",
            "run_id",
            "state",
            "tasks",
        ]
    );
    let task = &value["tasks"][0];
    let mut task_keys: Vec<&str> = task
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    task_keys.sort_unstable();
    assert_eq!(
        task_keys,
        vec![
            "category",
            "owner",
            "steps",
            "summary",
            "target_value",
            "task_key",
            "title"
        ]
    );
    let mut step_keys: Vec<&str> = task["steps"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    step_keys.sort_unstable();
    assert_eq!(step_keys, vec!["action", "step_id"]);
    // 结构化往返（视图可序列化）
    let round: dsheng_core::generation::task_candidate::TaskCandidateView =
        serde_json::from_value(value.clone()).unwrap();
    assert_eq!(round, view);
    // Framework 导入不为未使用告警（保持与 input 投影一致的框架枚举来源）
    let _ = Framework::Why5;
}
