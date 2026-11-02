//! 诊断上下文不变量（prompt-context-validation.md §4「发送前跨字段硬校验」、
//! 「澄清如何恢复」，主计划 §8.4；用例 63/64/53 纯逻辑部分）。
//!
//! 覆盖：
//! - pending_clarification 结构与 append/restate 规则（restate 必须绑定
//!   pending.turn_id；孤立确认/否定集合本地拒绝）；
//! - round 规则（round_no=1 固定字段；>1 必有 continuation_input 且
//!   user_confirmed_continue）；
//! - 顶层 user_message 与 continuation_input.user_message 逐字相同；
//! - open_question_ids 一致性（scope == session_state.unresolved_questions
//!   == 当前报告 open 集合；selected ⊆ 该集合；pending 问题不混入）；
//! - 引用域校验（evidence/metric/question/fact 引用只接受本轮发送集合
//!   或当前报告允许的引用域）；
//! - turn 应用状态机：needs_clarification 以新 turn 替换 pending、
//!   off_topic 保留旧 pending、有效 patch 提交后清 pending；
//!   accepted_facts 采纳与轮次推进。
//!
//! 违例 → `CONTEXT_INVALID`（不发 Provider、不自动修补）或
//! `CLARIFICATION_RESTATEMENT_REQUIRED`（不创建 request）。
//! 纯逻辑层：不做 Provider 调用。

use crate::contracts::cli::CliErrorCode;
use crate::generation::input::{
    AcceptedFactProjection, ClarificationQuestionProjection, ContinuationMode,
    DiagnosisInputProjection, FrameworkProjection, PendingClarificationProjection,
};
use crate::generation::report::{apply_report_patch, Report3, ReportPatch, ReportSeed};
use std::collections::BTreeSet;

/// 校验错误：错误码 + 脱敏说明。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("context invalid: {code:?} {detail}")]
pub struct ContextError {
    pub code: CliErrorCode,
    pub detail: String,
}

fn invalid(detail: impl Into<String>) -> ContextError {
    ContextError {
        code: CliErrorCode::ContextInvalid,
        detail: detail.into(),
    }
}

/// 孤立确认/否定集合（§4：trim + 删除末尾中文/英文句号后完全匹配）。
pub const ISOLATED_ANSWERS: &[&str] = &[
    "是", "是的", "对", "对的", "不是", "否", "好的", "确认", "没错", "yes", "no", "ok",
];

/// 孤立短答判定：去首尾空白 + 去末尾中文句号/英文句号后命中集合。
pub fn is_isolated_answer(text: &str) -> bool {
    let trimmed = text.trim().trim_end_matches('。').trim_end_matches('.');
    ISOLATED_ANSWERS.contains(&trimmed)
}

/// 用户文本的孤立短答检查（append/restate 同样适用）。
pub fn check_not_isolated(user_message: &str) -> Result<(), ContextError> {
    if is_isolated_answer(user_message) {
        return Err(ContextError {
            code: CliErrorCode::ClarificationRestatementRequired,
            detail: "孤立确认/否定不能作为补充；需完整重述当前期间/对象与事实".into(),
        });
    }
    Ok(())
}

/// 本轮发送集合中的引用域（evidence/metric/question/fact）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceDomains {
    pub evidence_ids: BTreeSet<String>,
    pub metric_ids: BTreeSet<String>,
    pub question_ids: BTreeSet<String>,
    pub fact_ids: BTreeSet<String>,
}

impl ReferenceDomains {
    /// 从诊断输入投影构造：evidence 域 = source_records + manual_inputs +
    /// metrics 的 evidence；metric 域 = metrics；fact 域 = accepted_facts。
    pub fn from_input(input: &DiagnosisInputProjection) -> Self {
        let mut domains = Self::default();
        for record in &input.source_records {
            domains.evidence_ids.insert(record.evidence_id.clone());
        }
        for manual in &input.manual_inputs {
            for id in &manual.evidence_ids {
                domains.evidence_ids.insert(id.clone());
            }
        }
        for metric in &input.metrics {
            domains.metric_ids.insert(metric.metric_id.to_string());
            for id in &metric.evidence_ids {
                domains.evidence_ids.insert(id.clone());
            }
        }
        for fact in &input.session_state.accepted_facts {
            domains.fact_ids.insert(fact.fact_id.clone());
        }
        domains
    }
}

/// 发送前跨字段硬校验（§4）。`report_open_question_ids` 为当前报告的
/// open 问题 ID 集合（无报告时为空，由调用方从持久化报告读取）。
pub fn validate_diagnosis_context(
    input: &DiagnosisInputProjection,
    report_open_question_ids: &[String],
) -> Result<(), ContextError> {
    // 1. framework 一致性：request == scope == output_contract
    let request_framework = input.request.framework;
    if input.scope.framework != request_framework {
        return Err(invalid("scope.framework 与 request.framework 不一致"));
    }
    let contract_framework = match input.output_contract.schema {
        crate::generation::input::DiagnosisTurnSchemaId::Why5 => FrameworkProjection::Why5,
        crate::generation::input::DiagnosisTurnSchemaId::W5 => FrameworkProjection::W5,
        crate::generation::input::DiagnosisTurnSchemaId::W5h2 => FrameworkProjection::W5h2,
    };
    if contract_framework != request_framework {
        return Err(invalid(
            "output_contract.schema 与 request.framework 不一致",
        ));
    }

    // 2. round 规则
    let round = input.request.round_no;
    if round == 1 {
        if input.request.report_id.is_some() {
            return Err(invalid("首轮 report_id 必须为 null"));
        }
        if input.request.user_confirmed_continue {
            return Err(invalid("首轮 user_confirmed_continue 必须为 false"));
        }
        if input.continuation_input.is_some() {
            return Err(invalid("首轮 continuation_input 必须为 null"));
        }
    } else {
        if input.continuation_input.is_none() {
            return Err(invalid(format!(
                "round_no={round} 必须有 continuation_input（即使还没有有效报告）"
            )));
        }
        if !input.request.user_confirmed_continue {
            return Err(invalid(format!(
                "round_no={round} 必须 user_confirmed_continue=true"
            )));
        }
    }

    // 3. 顶层 user_message 与 continuation_input.user_message 逐字相同
    if let Some(cont) = &input.continuation_input {
        if input.user_message != cont.user_message {
            return Err(invalid(
                "顶层 user_message 须与 continuation_input.user_message 逐字相同",
            ));
        }
        // 4. mode 规则
        match cont.mode {
            ContinuationMode::Restate => {
                let pending = input
                    .session_state
                    .pending_clarification
                    .as_ref()
                    .ok_or_else(|| invalid("restate 必须存在 pending_clarification"))?;
                if cont.answer_to_turn_id.as_deref() != Some(pending.turn_id.as_str()) {
                    return Err(invalid(
                        "restate 的 answer_to_turn_id 必须精确等于 pending.turn_id",
                    ));
                }
            }
            ContinuationMode::Append => {
                if cont.answer_to_turn_id.is_some() {
                    return Err(invalid("append 的 answer_to_turn_id 必须为 null"));
                }
            }
        }
        // selected_open_question_ids 只能取 open 集合子集
        for id in &cont.selected_open_question_ids {
            if !report_open_question_ids.contains(id) {
                return Err(invalid(format!(
                    "selected_open_question_ids 含非 open 问题 {id}（跨 session ID 不接受）"
                )));
            }
        }
    }

    // 5. 孤立确认/否定：append/restate 同样拒绝（不创建 request）
    check_not_isolated(&input.user_message)?;

    // 6. open_question_ids 一致性：scope == unresolved == 报告 open
    let open_set: BTreeSet<String> = report_open_question_ids.iter().cloned().collect();
    let scope_set: BTreeSet<String> = input.scope.open_question_ids.iter().cloned().collect();
    let unresolved_set: BTreeSet<String> = input
        .session_state
        .unresolved_questions
        .iter()
        .map(|q| q.question_id.clone())
        .collect();
    if scope_set != open_set {
        return Err(invalid(
            "scope.open_question_ids 必须等于当前报告 open 问题集合（无报告时为空）",
        ));
    }
    if unresolved_set != open_set {
        return Err(invalid(
            "session_state.unresolved_questions 必须等于当前报告 open 问题集合",
        ));
    }
    // 无报告时两边都必须为空
    if input.request.report_id.is_none() && !open_set.is_empty() {
        return Err(invalid("无报告时 open 问题集合必须为空"));
    }

    // 7. pending 问题 ID 不伪装成报告问题
    if let Some(pending) = &input.session_state.pending_clarification {
        for q in &pending.clarification_questions {
            if open_set.contains(&q.question_id) {
                return Err(invalid(format!(
                    "pending 问题 {} 不得混入报告 open_question_ids",
                    q.question_id
                )));
            }
        }
    }

    // 8. 引用域校验
    let domains = ReferenceDomains::from_input(input);
    for metric_id in &input.scope.focus_metric_ids {
        if !domains.metric_ids.contains(metric_id) {
            return Err(invalid(format!(
                "focus metric {metric_id} 不在本轮发送集合"
            )));
        }
    }
    for fact in &input.session_state.accepted_facts {
        for id in &fact.evidence_ids {
            if !domains.evidence_ids.contains(id) {
                return Err(invalid(format!(
                    "accepted_fact {} 引用 evidence {id} 超出本轮发送集合",
                    fact.fact_id
                )));
            }
        }
        for id in &fact.open_question_ids {
            if !open_set.contains(id) {
                return Err(invalid(format!(
                    "accepted_fact {} 引用 open 问题 {id} 不在当前集合",
                    fact.fact_id
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// turn 应用状态机（澄清恢复）
// ---------------------------------------------------------------------------

/// 模型单轮输出的本地投影（只取状态机需要的字段；完整对象先经
/// turn schema3 校验）。
#[derive(Debug, Clone, PartialEq)]
pub struct TurnOutcome {
    pub turn_disposition: TurnDisposition,
    pub report_patch: Option<ReportPatch>,
    /// 本轮模型 clarification_questions（needs_clarification 时进入 pending）。
    pub clarification_questions: Vec<ClarificationQuestionProjection>,
    pub redirect_message: Option<String>,
    pub stop_reason: Option<String>,
    /// 本轮模型 accepted_facts（on_topic/partially_relevant 时采纳）。
    pub accepted_facts: Vec<AcceptedFactProjection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDisposition {
    OnTopic,
    PartiallyRelevant,
    OffTopicRedirect,
    NeedsClarification,
}

/// 诊断会话状态（context_policy_version=2 的本地重建状态）。
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosisSessionState {
    pub session_id: String,
    pub report_id: Option<String>,
    pub round_no: u32,
    pub pending: Option<PendingClarificationProjection>,
    pub accepted_facts: Vec<AcceptedFactProjection>,
    pub report: Option<Report3>,
}

impl DiagnosisSessionState {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            report_id: None,
            round_no: 1,
            pending: None,
            accepted_facts: vec![],
            report: None,
        }
    }

    /// 应用一轮模型输出（turn schema3 已通过后的状态推进）。
    ///
    /// - needs_clarification：不建 patch、不加报告 revision；以**新 turn**
    ///   替换 pending（保存本轮用户原文与模型澄清问题）；
    /// - off_topic_redirect：保留旧 pending，不采纳事实；
    /// - on_topic / partially_relevant：应用 patch（首个有效 patch 建
    ///   revision 1），采纳本轮 accepted_facts（绑定本轮 turn_id/round），
    ///   有效 patch 提交后清 pending。
    pub fn apply_turn(
        &mut self,
        turn: TurnOutcome,
        turn_id: &str,
        seed: &ReportSeed,
    ) -> Result<AppliedTurn, ContextError> {
        match turn.turn_disposition {
            TurnDisposition::NeedsClarification => {
                self.pending = Some(PendingClarificationProjection {
                    turn_id: turn_id.to_string(),
                    user_message: seed.current_user_message.clone().unwrap_or_default(),
                    clarification_questions: turn.clarification_questions,
                    redirect_message: turn.redirect_message,
                    stop_reason: turn.stop_reason,
                });
                self.round_no += 1;
                Ok(AppliedTurn::Clarification)
            }
            TurnDisposition::OffTopicRedirect => {
                // 跑题保留旧 pending；无 patch、无 revision
                self.round_no += 1;
                Ok(AppliedTurn::Redirect)
            }
            TurnDisposition::OnTopic | TurnDisposition::PartiallyRelevant => {
                let patch = turn
                    .report_patch
                    .ok_or_else(|| invalid("on_topic/partially_relevant 必须携带 report_patch"))?;
                let report =
                    apply_report_patch(self.report.as_ref(), &patch, seed).map_err(|e| {
                        ContextError {
                            code: e.code,
                            detail: e.detail,
                        }
                    })?;
                let created = self.report.is_none();
                self.report = Some(report);
                if created {
                    self.report_id = Some(seed.report_id.clone());
                }
                // 采纳本轮事实（绑定来源 turn/round）
                for mut fact in turn.accepted_facts {
                    fact.source_turn_id = turn_id.to_string();
                    fact.source_turn_no = self.round_no;
                    // 针对旧 pending 的 accepted_fact 允许空 open_question_ids
                    self.accepted_facts.push(fact);
                }
                // 重复 fact_id 拒绝（不按相似内容去重）
                let mut seen = BTreeSet::new();
                for fact in &self.accepted_facts {
                    if !seen.insert(fact.fact_id.clone()) {
                        return Err(invalid(format!("重复 fact_id {}", fact.fact_id)));
                    }
                }
                // 本轮有效 patch 提交后清 pending
                self.pending = None;
                self.round_no += 1;
                Ok(AppliedTurn::Patched { created })
            }
        }
    }

    /// 用户本地撤销 pending：保留 turn 历史（不删除 accepted_facts/报告），
    /// 仅清除待澄清指针。
    pub fn withdraw_pending(&mut self) {
        self.pending = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppliedTurn {
    /// 首个有效 patch 建立报告。
    Patched {
        created: bool,
    },
    Clarification,
    Redirect,
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试（上下文不变量，非 Provider 调用）。
    use super::*;
    use crate::generation::input::*;

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
            metrics: vec![MetricProjection {
                metric_id: "00000000-0000-4000-8000-000000000020".parse().unwrap(),
                label: "合成销售额".into(),
                unit: "元".into(),
                direction: DirectionProjection::Up,
                target_constraint_scope: TargetConstraintScopeProjection::None,
                explicit_target: None,
                annual_allocated_target: None,
                prior_year_actual: None,
                target: None,
                actual: None,
                delta: None,
                mom: None,
                yoy: None,
                evidence_ids: vec![],
            }],
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
                problem_statement: "销售额下降".into(),
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

    fn report_seed() -> crate::generation::report::ReportSeed {
        crate::generation::report::ReportSeed {
            report_id: "rep-1".into(),
            generation_session_id: "sess-1".into(),
            workspace_id: "ws-1".into(),
            run_id: "run-1".into(),
            framework: FrameworkProjection::Why5,
            scope: crate::generation::report::ReportScope {
                problem_statement: "销售额下降".into(),
                focus_metric_ids: vec![],
            },
            period: crate::generation::report::ReportPeriod {
                from: Some("2026-09-01".into()),
                to: Some("2026-09-30".into()),
                period_policy_version: "pp-1".into(),
            },
            source_manifest: vec![],
            analysis_revision_no: 1,
            analysis_input_fingerprint: "c".repeat(64),
            now: "2026-11-02T00:00:00Z".into(),
            current_user_message: Some("9月销售额下降".into()),
        }
    }

    fn minimal_patch() -> crate::generation::report::ReportPatch {
        crate::generation::report::ReportPatch {
            summary_update: Some("首轮摘要".into()),
            findings_upsert: vec![],
            facts_and_hypotheses_upsert: vec![],
            evidence_refs_append: vec![],
            clarification_questions_upsert: vec![],
            warnings_upsert: vec![],
            stop_reason: None,
            can_continue: true,
        }
    }

    #[test]
    fn isolated_answer_rules() {
        for text in [
            "是",
            "是的。",
            "对.",
            " 对 ",
            "不是",
            "否",
            "好的",
            "确认",
            "没错",
            "yes",
            "no",
            "ok",
        ] {
            assert!(is_isolated_answer(text), "{text} 应判定为孤立短答");
        }
        for text in [
            "9月销售额下降10%",
            "确认9月下旬连续三天暴雨导致客流下降",
            "对手门店在隔壁开业",
            "",
        ] {
            assert!(!is_isolated_answer(text), "{text} 不是孤立短答");
        }
        let err = check_not_isolated("好的。").unwrap_err();
        assert_eq!(err.code, CliErrorCode::ClarificationRestatementRequired);
    }

    fn base_input(round: u32) -> DiagnosisInputProjection {
        let mut input = minimal_diagnosis_input();
        input.request.round_no = round;
        input
    }

    #[test]
    fn round1_fixed_fields() {
        let mut input = base_input(1);
        assert!(validate_diagnosis_context(&input, &[]).is_ok());
        // report_id 非空拒绝
        input.request.report_id = Some("rep-1".into());
        let err = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert_eq!(err.code, CliErrorCode::ContextInvalid);
        assert!(err.detail.contains("report_id"));
        // user_confirmed_continue=true 拒绝
        let mut input2 = base_input(1);
        input2.request.user_confirmed_continue = true;
        assert!(validate_diagnosis_context(&input2, &[]).is_err());
        // continuation_input 非空拒绝
        let mut input3 = base_input(1);
        input3.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: "补充".into(),
        });
        assert!(validate_diagnosis_context(&input3, &[]).is_err());
    }

    #[test]
    fn round2_requires_continuation_and_confirm() {
        let mut input = base_input(2);
        let err = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert!(err.detail.contains("continuation_input"));
        input.request.user_confirmed_continue = true;
        let err2 = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert!(err2.detail.contains("continuation_input"));
        input.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: "9月下旬连续三天暴雨，客流明显下降".into(),
        });
        input.user_message = "9月下旬连续三天暴雨，客流明显下降".into();
        assert!(validate_diagnosis_context(&input, &[]).is_ok());
        // user_message 与 continuation 不一致拒绝
        input.user_message = "不同文本".into();
        assert!(validate_diagnosis_context(&input, &[]).is_err());
        input.user_message = "9月下旬连续三天暴雨，客流明显下降".into();
        // 孤立短答拒绝（append 同样适用）
        input.user_message = "是的".into();
        input.continuation_input.as_mut().unwrap().user_message = "是的".into();
        let err3 = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert_eq!(err3.code, CliErrorCode::ClarificationRestatementRequired);
    }

    #[test]
    fn restate_binds_pending_turn() {
        let mut input = base_input(2);
        input.request.user_confirmed_continue = true;
        input.user_message = "完整重述：9月合成销售额环比下降10%，期间为2026-09-01至09-30".into();
        input.session_state.pending_clarification = Some(PendingClarificationProjection {
            turn_id: "turn-7".into(),
            user_message: "9月销售额下降".into(),
            clarification_questions: vec![],
            redirect_message: None,
            stop_reason: None,
        });
        input.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Restate,
            answer_to_turn_id: None, // 缺绑定拒绝
            selected_open_question_ids: vec![],
            user_message: input.user_message.clone(),
        });
        let err = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert!(err.detail.contains("answer_to_turn_id"));
        // 绑定错误 turn 拒绝
        input.continuation_input.as_mut().unwrap().answer_to_turn_id = Some("turn-1".into());
        assert!(validate_diagnosis_context(&input, &[]).is_err());
        // 正确绑定通过
        input.continuation_input.as_mut().unwrap().answer_to_turn_id = Some("turn-7".into());
        assert!(validate_diagnosis_context(&input, &[]).is_ok());
        // 无 pending 的 restate 拒绝
        input.session_state.pending_clarification = None;
        assert!(validate_diagnosis_context(&input, &[]).is_err());
    }

    #[test]
    fn open_question_consistency() {
        // 有报告的续问场景（round=2 基线）
        let mut input = base_input(2);
        input.request.user_confirmed_continue = true;
        input.request.report_id = Some("rep-1".into());
        input.user_message = "9月下旬连续三天暴雨，客流明显下降".into();
        input.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: input.user_message.clone(),
        });
        let report_open = vec!["q1".to_string(), "q2".to_string()];
        let as_unresolved = |ids: &[String]| {
            ids.iter()
                .map(|id| UnresolvedQuestionProjection {
                    question_id: id.clone(),
                    question: format!("问题 {id}"),
                    status: OpenOnly::Open,
                    answer_summary: None,
                })
                .collect::<Vec<_>>()
        };
        // scope 空 ≠ 报告 open
        let err = validate_diagnosis_context(&input, &report_open).unwrap_err();
        assert!(err.detail.contains("open_question_ids"));
        input.scope.open_question_ids = report_open.clone();
        // unresolved 空 ≠ open
        let err2 = validate_diagnosis_context(&input, &report_open).unwrap_err();
        assert!(
            err2.detail.contains("unresolved_questions"),
            "实际: {}",
            err2.detail
        );
        input.session_state.unresolved_questions = as_unresolved(&report_open);
        assert!(validate_diagnosis_context(&input, &report_open).is_ok());
        // selected 引用非 open 问题拒绝
        input
            .continuation_input
            .as_mut()
            .unwrap()
            .selected_open_question_ids = vec!["q9".into()];
        let err3 = validate_diagnosis_context(&input, &report_open).unwrap_err();
        assert!(err3.detail.contains("q9"));
        input
            .continuation_input
            .as_mut()
            .unwrap()
            .selected_open_question_ids = vec![];
        // pending 问题混入报告 open 集合拒绝
        let mut bad_report_open = report_open.clone();
        bad_report_open.push("pq1".into());
        input.scope.open_question_ids = bad_report_open.clone();
        input.session_state.unresolved_questions = as_unresolved(&bad_report_open);
        input.session_state.pending_clarification = Some(PendingClarificationProjection {
            turn_id: "t".into(),
            user_message: "m".into(),
            clarification_questions: vec![ClarificationQuestionProjection {
                question_id: "pq1".into(),
                question: "q".into(),
                why_needed: "w".into(),
                evidence_ids: vec![],
            }],
            redirect_message: None,
            stop_reason: None,
        });
        let err4 = validate_diagnosis_context(&input, &bad_report_open).unwrap_err();
        assert!(err4.detail.contains("pending"), "实际: {}", err4.detail);
        // 无报告时 open 集合必须为空（round=2 无报告：prior report 仍 null）
        let mut no_report = base_input(2);
        no_report.request.user_confirmed_continue = true;
        no_report.user_message = "补充说明".into();
        no_report.continuation_input = Some(ContinuationInputProjection {
            mode: ContinuationMode::Append,
            answer_to_turn_id: None,
            selected_open_question_ids: vec![],
            user_message: "补充说明".into(),
        });
        no_report.scope.open_question_ids = vec!["q1".into()];
        no_report.session_state.unresolved_questions = as_unresolved(&["q1".into()]);
        let err5 = validate_diagnosis_context(&no_report, &["q1".into()]).unwrap_err();
        assert!(err5.detail.contains("无报告"), "实际: {}", err5.detail);
    }

    #[test]
    fn reference_domain_checks() {
        let mut input = base_input(1);
        input.scope.focus_metric_ids = vec!["00000000-0000-4000-8000-000000000099".into()];
        let err = validate_diagnosis_context(&input, &[]).unwrap_err();
        assert!(err.detail.contains("focus metric"));
        // accepted_fact 引用越界 evidence
        let mut input2 = base_input(1);
        input2.session_state.accepted_facts = vec![AcceptedFactProjection {
            fact_id: "f1".into(),
            source_turn_id: "t1".into(),
            source_turn_no: 1,
            user_quote: "quote".into(),
            normalized_statement: "事实".into(),
            open_question_ids: vec![],
            evidence_ids: vec!["cell:unknown".into()],
            fact_status: UserReportOnly::UserReport,
            confidence: ConfidenceProjection::High,
        }];
        let err2 = validate_diagnosis_context(&input2, &[]).unwrap_err();
        assert!(err2.detail.contains("超出本轮发送集合"));
    }

    #[test]
    fn session_state_machine_pending_transitions() {
        let mut session = DiagnosisSessionState::new("sess-1");
        let seed = report_seed();
        // 首轮 needs_clarification：不建报告，pending=新 turn，round 递增，
        // prior report 仍为 null
        let turn = TurnOutcome {
            turn_disposition: TurnDisposition::NeedsClarification,
            report_patch: None,
            clarification_questions: vec![ClarificationQuestionProjection {
                question_id: "pq1".into(),
                question: "指哪个口径？".into(),
                why_needed: "锁定范围".into(),
                evidence_ids: vec![],
            }],
            redirect_message: None,
            stop_reason: None,
            accepted_facts: vec![],
        };
        let applied = session.apply_turn(turn, "turn-1", &seed).unwrap();
        assert_eq!(applied, AppliedTurn::Clarification);
        assert_eq!(session.round_no, 2);
        assert!(session.report.is_none());
        assert_eq!(session.pending.as_ref().unwrap().turn_id, "turn-1");
        assert_eq!(
            session
                .pending
                .as_ref()
                .unwrap()
                .clarification_questions
                .len(),
            1
        );

        // 跑题：保留旧 pending
        let off_topic = TurnOutcome {
            turn_disposition: TurnDisposition::OffTopicRedirect,
            report_patch: None,
            clarification_questions: vec![],
            redirect_message: Some("请回到当前诊断主题".into()),
            stop_reason: None,
            accepted_facts: vec![],
        };
        session.apply_turn(off_topic, "turn-2", &seed).unwrap();
        assert_eq!(
            session.pending.as_ref().unwrap().turn_id,
            "turn-1",
            "跑题保留旧 pending"
        );

        // 有效 patch：建报告 revision 1，清 pending，采纳事实
        let patch_turn = TurnOutcome {
            turn_disposition: TurnDisposition::OnTopic,
            report_patch: Some(minimal_patch()),
            clarification_questions: vec![],
            redirect_message: None,
            stop_reason: None,
            accepted_facts: vec![AcceptedFactProjection {
                fact_id: "fact-1".into(),
                source_turn_id: "old".into(),
                source_turn_no: 0,
                user_quote: "9月销售额下降10%".into(),
                normalized_statement: "2026-09 合成销售额环比下降 10%".into(),
                open_question_ids: vec![],
                evidence_ids: vec![],
                fact_status: UserReportOnly::UserReport,
                confidence: ConfidenceProjection::High,
            }],
        };
        let applied3 = session.apply_turn(patch_turn, "turn-3", &seed).unwrap();
        assert!(matches!(applied3, AppliedTurn::Patched { created: true }));
        assert!(session.pending.is_none(), "有效 patch 提交后清 pending");
        assert_eq!(session.report.as_ref().unwrap().revision_no, 1);
        assert_eq!(session.report_id.as_deref(), Some("rep-1"));
        assert_eq!(session.accepted_facts[0].source_turn_id, "turn-3");
        assert_eq!(session.accepted_facts[0].source_turn_no, 3);

        // 再次 needs_clarification：以新 turn 替换 pending
        let clarify_again = TurnOutcome {
            turn_disposition: TurnDisposition::NeedsClarification,
            report_patch: None,
            clarification_questions: vec![],
            redirect_message: None,
            stop_reason: None,
            accepted_facts: vec![],
        };
        session.apply_turn(clarify_again, "turn-4", &seed).unwrap();
        assert_eq!(session.pending.as_ref().unwrap().turn_id, "turn-4");
        assert_eq!(
            session.report.as_ref().unwrap().revision_no,
            1,
            "澄清不加 revision"
        );

        // 用户撤销 pending：本地动作，报告/事实不动
        session.withdraw_pending();
        assert!(session.pending.is_none());
        assert_eq!(session.accepted_facts.len(), 1);
    }
}
