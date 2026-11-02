//! S6 诊断上下文不变量集成测试（纯逻辑层，非 Provider 调用）。
//!
//! 用例 63/64/53 的纯逻辑部分：
//! - 63 澄清恢复：首轮 needs_clarification → 孤立短答本地拒绝 →
//!   restate 绑定 pending.turn_id → 有效 patch 采纳并清 pending；
//! - 64 更正矛盾：同一 item fact_status 翻转拒绝 patch（更正须另建报告）；
//! - 53 警告证据：requirement 不可变、resolved 只读、解除需证据+note。

use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::generation::context::{
    check_not_isolated, is_isolated_answer, validate_diagnosis_context, AppliedTurn,
    DiagnosisSessionState, TurnDisposition, TurnOutcome,
};
use dsheng_core::generation::input::*;
use dsheng_core::generation::report::{
    apply_report_patch, FactItem3, ReportPatch, ReportSeed, Warning3,
};

fn session_input(round: u32) -> DiagnosisInputProjection {
    let mut input = minimal_diagnosis_input();
    input.request.round_no = round;
    if round > 1 {
        input.request.user_confirmed_continue = true;
        input.request.report_id = None;
        input.user_message = "9月下旬连续三天暴雨，客流明显下降".into();
        input.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: input.user_message.clone(),
        });
    }
    input
}

fn minimal_diagnosis_input() -> DiagnosisInputProjection {
    DiagnosisInputProjection {
        request: DiagnosisRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: "run-1".into(),
            analysis_revision_no: 1,
            analysis_input_fingerprint: "f".repeat(64),
            prompt_version: DiagnosisPromptVersion::V20,
            workflow: DiagnosisWorkflowMarker::Diagnosis,
            report_id: None,
            generation_session_id: "sess-1".into(),
            round_no: 1,
            framework: FrameworkProjection::Why5,
            schema_version: DiagnosisSchemaVersion::V3,
            user_confirmed_continue: false,
            context_policy_version: ContextPolicyVersion::V2,
        },
        workspace: WorkspaceProjection {
            workspace_id: "ws-1".into(),
            industry: None,
            cadence: CadenceProjection::Month,
            timezone: "Asia/Shanghai".into(),
            metric_definitions: vec![],
        },
        metrics: vec![],
        source_manifest: vec![],
        source_records: vec![],
        manual_inputs: vec![],
        missing_or_anomalous_inputs: vec![],
        source_conflicts: vec![],
        period: DiagnosisPeriodProjection {
            source_from: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
            source_to: chrono::NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            period_policy_version: "pp-1".into(),
        },
        scope: ScopeProjection {
            problem_statement: "9月合成销售额为何未达标".into(),
            focus_metric_ids: vec![],
            framework: FrameworkProjection::Why5,
            open_question_ids: vec![],
        },
        user_message: "请分析9月销售额与目标的差异".into(),
        continuation_input: None,
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

fn report_seed(user_message: &str) -> ReportSeed {
    ReportSeed {
        report_id: "rep-1".into(),
        generation_session_id: "sess-1".into(),
        workspace_id: "ws-1".into(),
        run_id: "run-1".into(),
        framework: FrameworkProjection::Why5,
        scope: dsheng_core::generation::report::ReportScope {
            problem_statement: "9月合成销售额为何未达标".into(),
            focus_metric_ids: vec![],
        },
        period: dsheng_core::generation::report::ReportPeriod {
            from: Some("2026-09-01".into()),
            to: Some("2026-09-30".into()),
            period_policy_version: "pp-1".into(),
        },
        source_manifest: vec![],
        analysis_revision_no: 1,
        analysis_input_fingerprint: "c".repeat(64),
        now: "2026-11-02T00:00:00Z".into(),
        current_user_message: Some(user_message.into()),
    }
}

fn base_patch() -> ReportPatch {
    ReportPatch {
        summary_update: Some("首轮摘要".into()),
        findings_upsert: vec![dsheng_core::generation::report::Finding3::Why5(
            dsheng_core::generation::report::FindingWhy5 {
                finding_id: "f1".into(),
                level: 1,
                question: "为什么销售额下降？".into(),
                answer: "客流减少".into(),
                fact_status: "fact".into(),
                evidence_ids: vec![],
                reasoning_summary: "来源记录支持".into(),
                confidence: "high".into(),
                validation_question: None,
                status: "candidate".into(),
            },
        )],
        facts_and_hypotheses_upsert: vec![FactItem3 {
            item_id: "item-1".into(),
            statement: "9月合成销售额环比下降".into(),
            fact_status: "user_report".into(),
            evidence_ids: vec![],
            user_quote: Some("9月销售额下降".into()),
            confidence: "medium".into(),
            reasoning_summary: "用户补充".into(),
        }],
        evidence_refs_append: vec![],
        clarification_questions_upsert: vec![],
        warnings_upsert: vec![Warning3 {
            warning_id: "w1".into(),
            message: "目标口径未确认".into(),
            status: "active".into(),
            resolution_requirement: "user_confirmation".into(),
            evidence_ids: vec![],
            fact_ids: vec![],
            resolution_note: None,
        }],
        stop_reason: None,
        can_continue: true,
    }
}

#[test]
fn case63_clarification_restatement_recovery() {
    let mut session = DiagnosisSessionState::new("sess-1");

    // 首轮输入通过校验；模型 needs_clarification（范围过宽）
    let round1 = session_input(1);
    assert!(validate_diagnosis_context(&round1, &[]).is_ok());
    let clarify = TurnOutcome {
        turn_disposition: TurnDisposition::NeedsClarification,
        report_patch: None,
        clarification_questions: vec![ClarificationQuestionProjection {
            question_id: "pq1".into(),
            question: "指哪个口径的销售额？期间是哪几天？".into(),
            why_needed: "锁定诊断范围".into(),
            evidence_ids: vec![],
        }],
        redirect_message: None,
        stop_reason: None,
        accepted_facts: vec![],
    };
    session
        .apply_turn(
            clarify,
            "turn-1",
            &report_seed("请分析9月销售额与目标的差异"),
        )
        .unwrap();
    assert_eq!(session.round_no, 2);
    assert!(session.report.is_none(), "澄清轮不建报告");
    assert!(session.pending.is_some());

    // 孤立确认被本地拒绝，不创建 request
    assert_eq!(
        check_not_isolated("是的。").unwrap_err().code,
        CliErrorCode::ClarificationRestatementRequired
    );
    let mut isolated = session_input(2);
    isolated.user_message = "对".into();
    isolated.continuation_input.as_mut().unwrap().user_message = "对".into();
    let err = validate_diagnosis_context(&isolated, &[]).unwrap_err();
    assert_eq!(err.code, CliErrorCode::ClarificationRestatementRequired);

    // restate 必须绑定 pending.turn_id 且完整重述
    let mut restate = session_input(2);
    restate.continuation_input = Some(ContinuationInputProjection {
        mode: ContinuationMode::Restate,
        answer_to_turn_id: Some("turn-1".into()),
        selected_open_question_ids: vec![],
        user_message: "完整重述：2026-09-01至09-30的净销售额口径，环比下降10%".into(),
    });
    restate.user_message = restate
        .continuation_input
        .as_ref()
        .unwrap()
        .user_message
        .clone();
    restate.session_state.pending_clarification = session.pending.clone();
    assert!(validate_diagnosis_context(&restate, &[]).is_ok());
    // 绑错 turn 拒绝
    let mut wrong = restate.clone();
    wrong.continuation_input.as_mut().unwrap().answer_to_turn_id = Some("turn-9".into());
    assert_eq!(
        validate_diagnosis_context(&wrong, &[]).unwrap_err().code,
        CliErrorCode::ContextInvalid
    );

    // 完整重述后的有效 patch：建报告 revision 1，采纳事实并清 pending
    let turn2 = TurnOutcome {
        turn_disposition: TurnDisposition::OnTopic,
        report_patch: Some(base_patch()),
        clarification_questions: vec![],
        redirect_message: None,
        stop_reason: None,
        accepted_facts: vec![AcceptedFactProjection {
            fact_id: "fact-1".into(),
            source_turn_id: "placeholder".into(),
            source_turn_no: 0,
            user_quote: "完整重述：2026-09-01至09-30的净销售额口径，环比下降10%".into(),
            normalized_statement: "2026-09 净销售额环比下降 10%".into(),
            open_question_ids: vec![],
            evidence_ids: vec![],
            fact_status: UserReportOnly::UserReport,
            confidence: ConfidenceProjection::High,
        }],
    };
    let applied = session
        .apply_turn(
            turn2,
            "turn-2",
            &report_seed("完整重述：2026-09-01至09-30的净销售额口径，环比下降10%"),
        )
        .unwrap();
    assert!(matches!(applied, AppliedTurn::Patched { created: true }));
    assert!(session.pending.is_none(), "有效 patch 提交后清 pending");
    assert_eq!(session.report.as_ref().unwrap().revision_no, 1);
    assert_eq!(session.accepted_facts.len(), 1);
    assert_eq!(session.accepted_facts[0].source_turn_id, "turn-2");
}

#[test]
fn case64_deterministic_fact_conflict_rejects_patch() {
    let seed = report_seed("补充");
    let r1 = apply_report_patch(None, &base_patch(), &seed).unwrap();
    // 同一 item 从 user_report 改判 fact（升级）拒绝
    let mut p = base_patch();
    p.facts_and_hypotheses_upsert[0].fact_status = "fact".into();
    let err = apply_report_patch(Some(&r1), &p, &seed).unwrap_err();
    assert_eq!(err.code, CliErrorCode::SemanticValidationFailed);
    assert!(err.detail.contains("更正须另建报告"));
    // 跑题/澄清轮不覆盖旧事实：needs_clarification 状态机不产生 patch
    let mut session = DiagnosisSessionState::new("sess-1");
    session.report = Some(r1.clone());
    session.report_id = Some("rep-1".into());
    session.round_no = 2;
    let off = TurnOutcome {
        turn_disposition: TurnDisposition::OffTopicRedirect,
        report_patch: None,
        clarification_questions: vec![],
        redirect_message: Some("请回到当前诊断主题".into()),
        stop_reason: None,
        accepted_facts: vec![],
    };
    session.apply_turn(off, "turn-2", &seed).unwrap();
    assert_eq!(
        session.report.as_ref().unwrap().revision_no,
        1,
        "跑题不加 revision"
    );
}

#[test]
fn case53_warning_evidence_contract() {
    let seed = report_seed("补充");
    let r1 = apply_report_patch(None, &base_patch(), &seed).unwrap();
    // requirement 不可变
    let mut p = base_patch();
    p.warnings_upsert[0].resolution_requirement = "source_record".into();
    assert!(apply_report_patch(Some(&r1), &p, &seed).is_err());
    // 解除：user_confirmation 型需用户确认语义 + 非空 note + 引用
    let mut resolve = base_patch();
    resolve.warnings_upsert[0].status = "resolved".into();
    resolve.warnings_upsert[0].resolution_note =
        Some("用户确认采用已批准的缓冲库存政策口径".into());
    resolve.warnings_upsert[0].fact_ids = vec!["item-1".into()];
    let r2 = apply_report_patch(Some(&r1), &resolve, &seed).unwrap();
    assert_eq!(r2.warnings[0].status, "resolved");
    // resolved 只读不可重开；复发新 ID
    let mut reopen = resolve.clone();
    reopen.warnings_upsert[0].status = "active".into();
    reopen.warnings_upsert[0].resolution_note = None;
    assert!(apply_report_patch(Some(&r2), &reopen, &seed).is_err());
    // 合并结果通过 schema3
    assert!(dsheng_core::provider::schema::validate_diagnosis_report(
        &serde_json::to_value(&r2).unwrap()
    )
    .is_ok());
}

#[test]
fn isolated_answer_set_matches_contract() {
    // 合同集合：{是,是的,对,对的,不是,否,好的,确认,没错,yes,no,ok}
    for t in [
        "是", "是的", "对", "对的", "不是", "否", "好的", "确认", "没错", "yes", "no", "ok",
    ] {
        assert!(is_isolated_answer(t));
        assert!(is_isolated_answer(&format!("{t}。")));
        assert!(is_isolated_answer(&format!(" {t} ")));
        assert!(is_isolated_answer(&format!("{t}.")));
    }
    // 合同口径：trim 后仅去末尾句号再精确比较；句号前的空白
    // （"ok ."）不属于合同处理的孤立短答形式
    assert!(!is_isolated_answer(" ok . "));
    // 大小写敏感：OK 不在集合（与冻结文本一致）
    assert!(!is_isolated_answer("OK"));
    // 其他简短数值/事实允许
    for t in ["下降10%", "三天暴雨", "确认采用新口径后下降10%"] {
        assert!(!is_isolated_answer(t));
    }
}
