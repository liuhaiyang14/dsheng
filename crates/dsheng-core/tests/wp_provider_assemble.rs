//! S3 请求组装集成测试（纯逻辑/依赖注入层，非 Provider 调用）。
//!
//! 覆盖：
//! 1. typed 输入投影（generation::input）序列化后通过对应模型输入 schema
//!    （task-input v2 / diagnosis-input v2，Draft 2020-12 + format 断言）；
//! 2. 四种组合的 system/user 组装（冻结正文拼接 + 单遍模板替换）；
//! 3. 单轮 user 文本 8,000 scalar 上限、token 预算、1 MiB 请求体上限。

use chrono::NaiveDate;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::domain::{TaxBasis, UnitDimension, UnitSignature};
use dsheng_core::contracts::provider::{Framework, Workflow};
use dsheng_core::generation::assemble::{
    assemble_messages, assemble_request, check_user_message_scalars,
};
use dsheng_core::generation::input::*;
use dsheng_core::provider::schema::{validate_diagnosis_input, validate_task_input};
use serde_json::{json, Number};

fn unit_signature() -> UnitSignature {
    UnitSignature {
        dimension: UnitDimension::Money,
        base_unit: "元".into(),
        currency: Some("CNY".into()),
        tax_basis: Some(TaxBasis::Exclusive),
    }
}

fn metric_definition() -> MetricDefinitionProjection {
    MetricDefinitionProjection {
        metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
        metric_key: "sales_net".into(),
        version: 1,
        semantic_hash: "a".repeat(64),
        label: "合成销售额".into(),
        semantic_type: SemanticTypeProjection::Flow,
        unit_signature: unit_signature(),
        direction: DirectionProjection::Up,
        target_constraint_scope: TargetConstraintScopeProjection::MonthlyRevenueFloor,
        revenue_semantics: true,
        aggregation: AggregationProjection::SumDisjoint,
        formula: None,
        required_inputs: vec![],
        denominator_policy: DenominatorPolicyProjection::PositiveOnly,
        comparison_policy: ComparisonPolicyProjection {
            positive_base: PositiveBaseProjection::RelativeChange,
            zero_or_negative_base: ZeroOrNegativeBaseProjection::AbsoluteOnly,
        },
        effective_from: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        scope_key: "default".into(),
    }
}

fn candidate(value: &str, conflict: ConflictStatusProjection) -> Option<CandidateProjection> {
    Some(CandidateProjection {
        value: Number::from_string_unchecked(value.to_string()), // arbitrary_precision 保留词法
        unit: "元".into(),
        evidence_ids: vec!["cell:00000000-0000-4000-8000-000000000010:数据:B2".into()],
        comparable: true,
        conflict_status: conflict,
        resolution_id: None,
    })
}

fn metric() -> MetricProjection {
    MetricProjection {
        metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
        label: "合成销售额".into(),
        unit: "{\"base_unit\":\"元\",\"currency\":\"CNY\",\"dimension\":\"money\",\"tax_basis\":\"exclusive\"}".into(),
        direction: DirectionProjection::Up,
        target_constraint_scope: TargetConstraintScopeProjection::MonthlyRevenueFloor,
        explicit_target: candidate("100", ConflictStatusProjection::None),
        annual_allocated_target: candidate("1200", ConflictStatusProjection::None),
        prior_year_actual: candidate("95", ConflictStatusProjection::None),
        target: Some(Number::from_string_unchecked("100".into())),
        actual: Some(Number::from_string_unchecked("100.25".into())),
        delta: Some(Number::from_string_unchecked("0.25".into())),
        mom: None,
        yoy: None,
        evidence_ids: vec!["cell:00000000-0000-4000-8000-000000000010:数据:B2".into()],
    }
}

fn source_record() -> SourceRecordProjection {
    SourceRecordProjection {
        evidence_id: "cell:00000000-0000-4000-8000-000000000010:数据:B2".into(),
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
            mapping: Some(SourceCellMapping {
                metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
                field_key: "actual".into(),
                measure_role: MeasureRoleProjection::Actual,
                period: "2026-09-01/2026-09-30".into(),
                scope_key: "default".into(),
                unit: "元".into(),
            }),
        },
    }
}

fn workspace() -> WorkspaceProjection {
    WorkspaceProjection {
        workspace_id: "00000000-0000-4000-8000-000000000001".into(),
        industry: Some("餐饮".into()),
        cadence: CadenceProjection::Month,
        timezone: "Asia/Shanghai".into(),
        metric_definitions: vec![metric_definition()],
    }
}

fn task_input() -> TaskInputProjection {
    TaskInputProjection {
        request: TaskRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: "run-1".into(),
            analysis_revision_no: 1,
            analysis_input_fingerprint: "f".repeat(64),
            prompt_version: TaskPromptVersion::V24,
            workflow: TaskWorkflowMarker::Task,
            schema_version: TaskSchemaVersion::V2,
        },
        workspace: workspace(),
        metrics: vec![metric()],
        source_manifest: vec![SourceManifestEntry {
            source_id: "00000000-0000-4000-8000-000000000010".parse().unwrap(),
            file_label: "合成经营表.xlsx".into(),
            sheet: Some("数据".into()),
            selected_range: "A1:B2".into(),
            content_hash: "a".repeat(64),
            selection_hash: "b".repeat(64),
        }],
        source_records: vec![source_record()],
        manual_inputs: vec![],
        missing_or_anomalous_inputs: vec![AnomalyProjection {
            code: "target_missing".into(),
            metric_id: Some("00000000-0000-4000-8000-000000000020".into()),
            message: "目标待确认".into(),
            evidence_ids: vec![],
        }],
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
            rules_version: TaskPromptVersion::V24,
            required_fields: RequiredFieldsConst,
            category_options: CategoryOptionsConst,
        },
        optional_diagnosis_reference: None,
        user_context: Some("优先保住现有客户。".into()),
        output_contract: TaskOutputContract {
            schema: TaskOutputSchemaId::TaskOutputV2,
            schema_version: TaskSchemaVersion::V2,
        },
    }
}

fn diagnosis_input(
    round: u32,
    with_continuation: Option<ContinuationInputProjection>,
) -> DiagnosisInputProjection {
    DiagnosisInputProjection {
        request: DiagnosisRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: "run-1".into(),
            analysis_revision_no: 1,
            analysis_input_fingerprint: "f".repeat(64),
            prompt_version: DiagnosisPromptVersion::V21,
            workflow: DiagnosisWorkflowMarker::Diagnosis,
            report_id: None,
            generation_session_id: "sess-1".into(),
            round_no: round,
            framework: FrameworkProjection::Why5,
            schema_version: DiagnosisSchemaVersion::V3,
            user_confirmed_continue: round > 1,
            context_policy_version: ContextPolicyVersion::V2,
        },
        workspace: workspace(),
        metrics: vec![metric()],
        source_manifest: vec![SourceManifestEntry {
            source_id: "00000000-0000-4000-8000-000000000010".parse().unwrap(),
            file_label: "合成经营表.xlsx".into(),
            sheet: Some("数据".into()),
            selected_range: "A1:B2".into(),
            content_hash: "a".repeat(64),
            selection_hash: "b".repeat(64),
        }],
        source_records: vec![source_record()],
        manual_inputs: vec![ManualInputProjection {
            input_id: "00000000-0000-4000-8000-000000000030".parse().unwrap(),
            metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
            value: Some("12.5".into()),
            unit: Some("元".into()),
            confirmed: true,
            source_note: "人工确认".into(),
            evidence_ids: vec!["manual:00000000-0000-4000-8000-000000000030:r1".into()],
            input_revision: 1,
            measure_role: MeasureRoleProjection::Actual,
            period: Some("2026-09-01/2026-09-30".into()),
            scope_key: "default".into(),
            raw_text: "12.5".into(),
        }],
        missing_or_anomalous_inputs: vec![],
        source_conflicts: vec![SourceConflictProjection {
            conflict_id: "00000000-0000-4000-8000-000000000040".parse().unwrap(),
            metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
            measure_role: MeasureRoleProjection::Actual,
            period: "2026-09-01/2026-09-30".into(),
            scope_key: "default".into(),
            unit: "元".into(),
            candidate_values: vec![
                ConflictCandidateValue {
                    value: Number::from_string_unchecked("100.25".into()),
                    evidence_id: "cell:00000000-0000-4000-8000-000000000010:数据:B2".into(),
                    source_label: "表A".into(),
                },
                ConflictCandidateValue {
                    value: Number::from_string_unchecked("99".into()),
                    evidence_id: "manual:00000000-0000-4000-8000-000000000030:r1".into(),
                    source_label: "手工".into(),
                },
            ],
            resolution_status: ResolutionStatusProjection::Unresolved,
            resolution_id: None,
            selected_evidence_id: None,
            resolution_note: None,
        }],
        period: DiagnosisPeriodProjection {
            source_from: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            source_to: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            period_policy_version: "pp-1".into(),
        },
        scope: ScopeProjection {
            problem_statement: "9月合成销售额为何未达标".into(),
            focus_metric_ids: vec!["00000000-0000-4000-8000-000000000020".into()],
            framework: FrameworkProjection::Why5,
            open_question_ids: vec![],
        },
        user_message: "请分析9月销售额与目标的差异".into(),
        continuation_input: with_continuation,
        session_state: SessionStateProjection {
            accepted_facts: vec![],
            prior_report_summary: None,
            unresolved_questions: vec![],
            pending_clarification: None,
        },
        output_contract: DiagnosisOutputContract {
            schema: DiagnosisTurnSchemaId::Why5,
            schema_version: DiagnosisSchemaVersion::V3,
        },
    }
}

#[test]
fn typed_task_input_passes_schema() {
    let value = serde_json::to_value(task_input()).unwrap();
    if let Err(errors) = validate_task_input(&value) {
        panic!(
            "任务输入投影未通过 schema: {errors:?}\n{}",
            serde_json::to_string_pretty(&value).unwrap()
        );
    }
    assert_eq!(value["request"]["prompt_version"], "2.4");
    assert_eq!(value["task_skill_context"]["rules_version"], "2.4");
    // 数值词法保留（不经 f64）
    assert_eq!(value["metrics"][0]["actual"].to_string(), "100.25");
}

#[test]
fn typed_diagnosis_input_passes_schema_round1_and_continuation() {
    let round1 = serde_json::to_value(diagnosis_input(1, None)).unwrap();
    if let Err(errors) = validate_diagnosis_input(&round1) {
        panic!("诊断输入（首轮）未通过 schema: {errors:?}");
    }

    assert_eq!(round1["request"]["prompt_version"], "2.1");

    let mut round2 = diagnosis_input(
        2,
        Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: "补充：9月下旬连续三天暴雨，客流明显下降".into(),
        }),
    );
    round2.request.report_id = Some("rep-1".into());
    round2.session_state.pending_clarification = Some(PendingClarificationProjection {
        turn_id: "turn-1".into(),
        user_message: "9月销售额下降".into(),
        clarification_questions: vec![ClarificationQuestionProjection {
            question_id: "q1".into(),
            question: "指哪个口径的销售额？".into(),
            why_needed: "锁定范围".into(),
            evidence_ids: vec![],
        }],
        redirect_message: None,
        stop_reason: None,
    });
    round2.session_state.accepted_facts = vec![AcceptedFactProjection {
        fact_id: "fact-1".into(),
        source_turn_id: "turn-1".into(),
        source_turn_no: 1,
        user_quote: "9月销售额下降".into(),
        normalized_statement: "2026年9月合成销售额环比下降".into(),
        open_question_ids: vec![],
        evidence_ids: vec![],
        fact_status: UserReportOnly::UserReport,
        confidence: ConfidenceProjection::Medium,
    }];
    round2.user_message = "补充：9月下旬连续三天暴雨，客流明显下降".into();
    let value2 = serde_json::to_value(round2).unwrap();
    if let Err(errors) = validate_diagnosis_input(&value2) {
        panic!("诊断输入（续问）未通过 schema: {errors:?}");
    }
}

#[test]
fn typed_projection_negative_cases() {
    // 错误的 prompt_version 常量 → schema 拒绝
    let mut bad = task_input();
    bad.request.prompt_version = TaskPromptVersion::V24; // 唯一合法值，改为 JSON 篡改
    let mut value = serde_json::to_value(bad).unwrap();
    value["request"]["prompt_version"] = json!("2.3");
    assert!(validate_task_input(&value).is_err());
    let mut old_diagnosis = serde_json::to_value(diagnosis_input(1, None)).unwrap();
    old_diagnosis["request"]["prompt_version"] = json!("2.0");
    assert!(validate_diagnosis_input(&old_diagnosis).is_err());
    // user_context 超 8000 字符
    let mut bad2 = task_input();
    bad2.user_context = Some("长".repeat(8_001));
    let value2 = serde_json::to_value(bad2).unwrap();
    assert!(validate_task_input(&value2).is_err());
    // 诊断 round_no=2 但无 continuation_input 由 S6 上下文层硬校验
    // （schema 层不约束，见合同"发送前跨字段硬校验"）
}

#[test]
fn four_combination_message_assembly() {
    let payload = serde_json::to_string(&task_input()).unwrap();
    let task = assemble_messages(Workflow::Task, None, &payload).unwrap();
    let task_manifest = dsheng_core::contracts::prompts::assembly_manifest(Workflow::Task, None);
    let expected_system = task_manifest
        .system_files
        .iter()
        .map(|f| dsheng_core::contracts::prompts::resource(f).unwrap())
        .collect::<String>();
    assert_eq!(task.system, expected_system, "system 为文件序列直接拼接");
    assert!(task.system.contains("供用户查看的思考文字使用简体中文"));
    assert!(task.system.contains("不得混入最终 JSON 正文"));
    assert!(task.user.contains(&payload));
    assert!(task.user.contains("\"$defs\""));

    for f in [Framework::Why5, Framework::W5, Framework::W5h2] {
        let d = assemble_messages(Workflow::Diagnosis, Some(f), &payload).unwrap();
        let manifest =
            dsheng_core::contracts::prompts::assembly_manifest(Workflow::Diagnosis, Some(f));
        let expected = manifest
            .system_files
            .iter()
            .map(|x| dsheng_core::contracts::prompts::resource(x).unwrap())
            .collect::<String>();
        assert_eq!(d.system, expected, "诊断 system = system.md + 单个框架文件");
        assert!(d.system.contains("供用户查看的思考文字使用简体中文"));
        assert!(d
            .system
            .contains("JSON 字段名、枚举值、ID、证据引用、公式、原文引用"));
        // 只拼接所选框架，不混拼
        let frameworks_in_system = [
            d.system.contains("5WHY 框架") || d.system.contains("5WHY"),
            d.system.contains("5W 框架") || d.system.contains("5W2H"),
        ];
        assert!(!frameworks_in_system.iter().all(|b| *b));
    }
}

#[test]
fn limits_rejected_with_correct_codes() {
    let budget = dsheng_core::contracts::settings::DEFAULT_BUDGET;
    let payload = serde_json::to_string(&task_input()).unwrap();

    // 用户单轮文本 8,001 scalars → USER_MESSAGE_TOO_LARGE
    let over = "好".repeat(8_001);
    let err =
        assemble_request(Workflow::Task, None, "glm-5.3", &payload, &over, &budget).unwrap_err();
    assert_eq!(err.code, CliErrorCode::UserMessageTooLarge);

    // 巨大 payload → CONTEXT_BUDGET_EXCEEDED
    let big = format!("{}{}", payload, "x".repeat(30_000));
    let err2 = assemble_request(Workflow::Task, None, "glm-5.3", &big, "", &budget).unwrap_err();
    assert_eq!(err2.code, CliErrorCode::ContextBudgetExceeded);

    // 正常路径全通过（完整 payload+schema 约 25.5KB，默认 32k/8k 预算不够，
    // 用合同允许的更大窗口——真实部署按 Provider 配置设定）
    let big_window = dsheng_core::contracts::settings::Budget {
        context_window_tokens: 65_536,
        output_reserve_tokens: 8_192,
    };
    let ok = assemble_request(
        Workflow::Task,
        None,
        "glm-5.3",
        &payload,
        "优先保住现有客户。",
        &big_window,
    )
    .unwrap();
    assert_eq!(ok.request.messages.len(), 2);
    assert!(ok.request_bytes <= 1_048_576);
    assert_eq!(
        ok.conservative_input_tokens,
        ok.request.messages[0].content.len() as u64
            + ok.request.messages[1].content.len() as u64
            + 256
    );
}

#[test]
fn scalar_check_is_char_count() {
    // 纯逻辑：按 Unicode scalar（char）计数而非 UTF-16 单元。
    // 5,000 个 emoji = 5,000 scalars = 10,000 UTF-16 单元：scalar 计数通过，
    // UTF-16 计数会误拒。
    let text = "😀".repeat(5_000);
    assert_eq!(text.chars().count(), 5_000);
    assert!(check_user_message_scalars(&text).is_ok());
    let over = "😀".repeat(8_001); // 8,001 scalars
    assert_eq!(
        check_user_message_scalars(&over).unwrap_err().code,
        CliErrorCode::UserMessageTooLarge
    );
}
