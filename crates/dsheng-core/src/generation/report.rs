//! schema3 诊断报告 typed 结构与 turn 级合并 patch 语义
//! （prompt-context-validation.md §4–§5、主计划 §8.4）。
//!
//! - 第一个有效 patch 建立报告 revision 1；之后每个有效 patch +1；
//!   澄清轮 patch=null 不建报告也不加 revision。
//! - upsert 语义：findings/facts/questions 按 ID 替换或插入；
//!   evidence_refs 追加去重（按 evidence_id）。
//! - warning 合同：`resolution_requirement` 首建后不可改；同一
//!   warning_id 的 message（风险主题）不可改；`resolved` 只读不可重开；
//!   active→resolved 需非空 resolution_note 且至少一个 evidence/fact
//!   引用；复发必须新 warning_id。
//! - 合并结果必须重新通过 diagnosis-report schema3（测试断言）。
//!
//! 纯逻辑层：不做 Provider 调用。

use crate::contracts::cli::CliErrorCode;
use crate::generation::input::FrameworkProjection;
use serde::{Deserialize, Serialize};

/// 合并错误：错误码 + 脱敏说明。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("report merge error: {code:?} {detail}")]
pub struct MergeError {
    pub code: CliErrorCode,
    pub detail: String,
}

fn err(detail: impl Into<String>) -> MergeError {
    MergeError {
        code: CliErrorCode::SemanticValidationFailed,
        detail: detail.into(),
    }
}

// ---------------------------------------------------------------------------
// schema3 报告 typed 结构（wire 格式与 diagnosis-report.schema.json 一致）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report3 {
    pub schema_version: String,
    pub report_id: String,
    pub generation_session_id: String,
    pub workspace_id: String,
    pub run_id: String,
    pub framework: FrameworkProjection,
    pub scope: ReportScope,
    pub period: ReportPeriod,
    pub revision_no: u32,
    pub summary: String,
    pub source_manifest: Vec<ReportSource>,
    pub findings: Vec<Finding3>,
    pub facts_and_hypotheses: Vec<FactItem3>,
    pub evidence_refs: Vec<EvidenceRef3>,
    pub clarification_questions: Vec<OpenQuestion3>,
    pub warnings: Vec<Warning3>,
    pub stop_reason: Option<String>,
    pub can_continue: bool,
    pub created_at: String,
    pub updated_at: String,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportScope {
    pub problem_statement: String,
    pub focus_metric_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportPeriod {
    pub from: Option<String>,
    pub to: Option<String>,
    pub period_policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportSource {
    pub source_id: String,
    pub file_label: String,
    pub sheet: Option<String>,
    pub selected_range: String,
    pub content_hash: String,
    pub selection_hash: String,
}

/// 报告 finding：三种框架形状（schema3 oneOf，各字段封闭）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Finding3 {
    W5h2(FindingW5h2),
    W5(FindingW5),
    Why5(FindingWhy5),
}

impl Finding3 {
    pub fn finding_id(&self) -> &str {
        match self {
            Finding3::W5h2(f) => &f.finding_id,
            Finding3::W5(f) => &f.finding_id,
            Finding3::Why5(f) => &f.finding_id,
        }
    }
}

/// 5WHY 因果链层（1–5）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingWhy5 {
    pub finding_id: String,
    pub level: u32,
    pub question: String,
    pub answer: String,
    pub fact_status: String,
    pub evidence_ids: Vec<String>,
    pub reasoning_summary: String,
    pub confidence: String,
    /// 必填可空：序列化为 null。
    pub validation_question: Option<String>,
    pub status: String,
}

/// 5W 问题界定维度。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingW5 {
    pub finding_id: String,
    pub dimension: String,
    pub question: String,
    pub answer: String,
    pub fact_status: String,
    pub evidence_ids: Vec<String>,
    pub confidence: String,
    pub missing_information: Vec<String>,
    pub clarification_questions: Vec<String>,
    pub status: String,
}

/// 5W2H 问题/行动要素（observed/proposal）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingW5h2 {
    pub finding_id: String,
    pub dimension: String,
    pub kind: String,
    pub question: String,
    pub answer: String,
    pub fact_status: String,
    pub evidence_ids: Vec<String>,
    pub confidence: String,
    pub missing_information: Vec<String>,
    pub clarification_questions: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactItem3 {
    pub item_id: String,
    pub statement: String,
    pub fact_status: String,
    pub evidence_ids: Vec<String>,
    pub user_quote: Option<String>,
    pub confidence: String,
    pub reasoning_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRef3 {
    pub evidence_id: String,
    pub source_id: String,
    pub source_label: String,
    pub locator: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenQuestion3 {
    pub question_id: String,
    pub question: String,
    pub status: String,
    pub answer_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Warning3 {
    pub warning_id: String,
    pub message: String,
    pub status: String,
    pub resolution_requirement: String,
    pub evidence_ids: Vec<String>,
    pub fact_ids: Vec<String>,
    pub resolution_note: Option<String>,
}

// ---------------------------------------------------------------------------
// turn 级 report patch（diagnosis-turn-*.schema.json $defs/reportPatch）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportPatch {
    pub summary_update: Option<String>,
    pub findings_upsert: Vec<Finding3>,
    pub facts_and_hypotheses_upsert: Vec<FactItem3>,
    pub evidence_refs_append: Vec<EvidenceRef3>,
    pub clarification_questions_upsert: Vec<OpenQuestion3>,
    pub warnings_upsert: Vec<Warning3>,
    pub stop_reason: Option<String>,
    pub can_continue: bool,
}

/// 建报告所需的会话/范围元数据（来自诊断输入投影，不来自模型）。
#[derive(Debug, Clone, PartialEq)]
pub struct ReportSeed {
    pub report_id: String,
    pub generation_session_id: String,
    pub workspace_id: String,
    pub run_id: String,
    pub framework: FrameworkProjection,
    pub scope: ReportScope,
    pub period: ReportPeriod,
    pub source_manifest: Vec<ReportSource>,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
    /// ISO-8601 时间戳。
    pub now: String,
    /// 本轮用户真实输入原文（pending.user_message 保存用）。
    pub current_user_message: Option<String>,
}

/// 应用 turn patch：prior=None 时创建 revision 1，否则在 prior 副本上合并并 +1。
pub fn apply_report_patch(
    prior: Option<&Report3>,
    patch: &ReportPatch,
    seed: &ReportSeed,
) -> Result<Report3, MergeError> {
    let mut report = match prior {
        Some(p) => p.clone(),
        None => Report3 {
            schema_version: "3".into(),
            report_id: seed.report_id.clone(),
            generation_session_id: seed.generation_session_id.clone(),
            workspace_id: seed.workspace_id.clone(),
            run_id: seed.run_id.clone(),
            framework: seed.framework,
            scope: seed.scope.clone(),
            period: seed.period.clone(),
            revision_no: 0,
            summary: String::new(),
            source_manifest: seed.source_manifest.clone(),
            findings: vec![],
            facts_and_hypotheses: vec![],
            evidence_refs: vec![],
            clarification_questions: vec![],
            warnings: vec![],
            stop_reason: None,
            can_continue: true,
            created_at: seed.now.clone(),
            updated_at: seed.now.clone(),
            analysis_revision_no: seed.analysis_revision_no,
            analysis_input_fingerprint: seed.analysis_input_fingerprint.clone(),
        },
    };

    if let Some(summary) = patch.summary_update.as_deref() {
        if !summary.is_empty() {
            report.summary = summary.to_string();
        }
    }
    upsert_by_id(&mut report.findings, patch.findings_upsert.clone(), |f| {
        f.finding_id().to_string()
    });
    // 同一 item 的确定字段冲突拒绝（§4 更正与矛盾：不覆盖旧事实、
    // 假设不因重复出现升级、user_report 不升格；更正须另建报告）
    for item in &patch.facts_and_hypotheses_upsert {
        if let Some(existing) = report
            .facts_and_hypotheses
            .iter()
            .find(|f| f.item_id == item.item_id)
        {
            if existing.fact_status != item.fact_status {
                return Err(MergeError {
                    code: CliErrorCode::SemanticValidationFailed,
                    detail: format!(
                        "item {} 的 fact_status {} 与已存在 {} 冲突；更正须另建报告",
                        item.item_id, item.fact_status, existing.fact_status
                    ),
                });
            }
        }
    }
    upsert_by_id(
        &mut report.facts_and_hypotheses,
        patch.facts_and_hypotheses_upsert.clone(),
        |f| f.item_id.clone(),
    );
    append_unique(
        &mut report.evidence_refs,
        &patch.evidence_refs_append,
        |e| e.evidence_id.clone(),
    );
    upsert_by_id(
        &mut report.clarification_questions,
        patch.clarification_questions_upsert.clone(),
        |q| q.question_id.clone(),
    );
    merge_warnings(&mut report.warnings, &patch.warnings_upsert)?;
    report.stop_reason = patch.stop_reason.clone();
    report.can_continue = patch.can_continue;
    report.revision_no += 1; // 有效 patch：首个 → 1，之后 +1
    report.updated_at = seed.now.clone();
    Ok(report)
}

fn upsert_by_id<T, F: Fn(&T) -> String>(target: &mut Vec<T>, incoming: Vec<T>, id: F) {
    for item in incoming {
        let item_id = id(&item);
        if let Some(slot) = target.iter_mut().find(|t| id(t) == item_id) {
            *slot = item;
        } else {
            target.push(item);
        }
    }
}

fn append_unique<T: Clone, F: Fn(&T) -> String>(target: &mut Vec<T>, incoming: &[T], id: F) {
    for item in incoming {
        let item_id = id(item);
        if !target.iter().any(|t| id(t) == item_id) {
            target.push((*item).clone());
        }
    }
}

/// warning 证据合同（§5）：requirement/message 不可变、resolved 只读、
/// active→resolved 证据标准、复发新 ID。
fn merge_warnings(target: &mut Vec<Warning3>, incoming: &[Warning3]) -> Result<(), MergeError> {
    for warning in incoming {
        match target.iter().find(|w| w.warning_id == warning.warning_id) {
            Some(existing) => {
                if existing.message != warning.message {
                    return Err(err(format!(
                        "warning {} 风险主题（message）不可改；复发必须新 ID",
                        warning.warning_id
                    )));
                }
                if existing.resolution_requirement != warning.resolution_requirement {
                    return Err(err(format!(
                        "warning {} 的 resolution_requirement 首建后不可改",
                        warning.warning_id
                    )));
                }
                if existing.status == "resolved" {
                    // resolved 只读：不允许任何字段变化的重放
                    if existing != warning {
                        return Err(err(format!(
                            "warning {} 已 resolved，只读不可重开；复发必须新 ID",
                            warning.warning_id
                        )));
                    }
                    continue;
                }
                // active → resolved：非空 note + 至少一个 evidence/fact 引用
                if warning.status == "resolved" {
                    let note_ok = warning
                        .resolution_note
                        .as_deref()
                        .is_some_and(|n| !n.trim().is_empty());
                    let refs_ok = !warning.evidence_ids.is_empty() || !warning.fact_ids.is_empty();
                    if !note_ok || !refs_ok {
                        return Err(err(format!(
                            "warning {} 解除需非空 resolution_note 且至少一个 evidence/fact 引用",
                            warning.warning_id
                        )));
                    }
                    // source_record 型只能由来源记录证据解除；本地不校验语义，
                    // 但引用必须非空（语义支持由提示词与内容验收负责）
                }
                if warning.status == "active" && warning.resolution_note.is_some() {
                    return Err(err(format!(
                        "warning {} 保持 active 时 resolution_note 必须为 null",
                        warning.warning_id
                    )));
                }
                let slot = target
                    .iter_mut()
                    .find(|w| w.warning_id == warning.warning_id)
                    .expect("已找到");
                *slot = warning.clone();
            }
            None => {
                if warning.status == "active" && warning.resolution_note.is_some() {
                    return Err(err(format!(
                        "新 warning {} active 状态不得携带 resolution_note",
                        warning.warning_id
                    )));
                }
                target.push(warning.clone());
            }
        }
    }
    Ok(())
}

impl Report3 {
    /// 当前 open 问题 ID 集合（scope.open_question_ids 一致性校验的源）。
    pub fn open_question_ids(&self) -> Vec<String> {
        self.clarification_questions
            .iter()
            .filter(|q| q.status == "open")
            .map(|q| q.question_id.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试（patch 合并语义，非 Provider 调用）。
    use super::*;

    fn seed() -> ReportSeed {
        ReportSeed {
            report_id: "rep-1".into(),
            generation_session_id: "sess-1".into(),
            workspace_id: "ws-1".into(),
            run_id: "run-1".into(),
            framework: FrameworkProjection::Why5,
            scope: ReportScope {
                problem_statement: "销售额下降".into(),
                focus_metric_ids: vec![],
            },
            period: ReportPeriod {
                from: Some("2026-10-01".into()),
                to: Some("2026-10-31".into()),
                period_policy_version: "pp-1".into(),
            },
            source_manifest: vec![ReportSource {
                source_id: "src-1".into(),
                file_label: "合成表".into(),
                sheet: Some("数据".into()),
                selected_range: "A1:B2".into(),
                content_hash: "a".repeat(64),
                selection_hash: "b".repeat(64),
            }],
            analysis_revision_no: 1,
            analysis_input_fingerprint: "c".repeat(64),
            now: "2026-11-02T00:00:00Z".into(),
            current_user_message: Some("9月销售额下降".into()),
        }
    }

    fn patch() -> ReportPatch {
        ReportPatch {
            summary_update: Some("首轮摘要".into()),
            findings_upsert: vec![Finding3::Why5(FindingWhy5 {
                finding_id: "f1".into(),
                level: 1,
                question: "为什么销售额下降？".into(),
                answer: "客流减少".into(),
                fact_status: "fact".into(),
                evidence_ids: vec!["ev-1".into()],
                reasoning_summary: "来源记录支持".into(),
                confidence: "high".into(),
                validation_question: None,
                status: "candidate".into(),
            })],
            facts_and_hypotheses_upsert: vec![],
            evidence_refs_append: vec![EvidenceRef3 {
                evidence_id: "ev-1".into(),
                source_id: "src-1".into(),
                source_label: "合成表".into(),
                locator: "数据!B2".into(),
            }],
            clarification_questions_upsert: vec![OpenQuestion3 {
                question_id: "q1".into(),
                question: "哪几天下滑最明显？".into(),
                status: "open".into(),
                answer_summary: None,
            }],
            warnings_upsert: vec![Warning3 {
                warning_id: "w1".into(),
                message: "记录不完整".into(),
                status: "active".into(),
                resolution_requirement: "source_record".into(),
                evidence_ids: vec![],
                fact_ids: vec![],
                resolution_note: None,
            }],
            stop_reason: None,
            can_continue: true,
        }
    }

    #[test]
    fn first_patch_creates_revision_1() {
        let report = apply_report_patch(None, &patch(), &seed()).unwrap();
        assert_eq!(report.revision_no, 1);
        assert_eq!(report.open_question_ids(), vec!["q1"]);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.summary, "首轮摘要");
    }

    #[test]
    fn subsequent_patch_increments_and_upserts() {
        let r1 = apply_report_patch(None, &patch(), &seed()).unwrap();
        let mut p2 = patch();
        p2.summary_update = Some("续问摘要".into());
        p2.findings_upsert = vec![Finding3::Why5(FindingWhy5 {
            finding_id: "f1".into(),
            level: 1,
            question: "为什么销售额下降？".into(),
            answer: "更新：周中客流减少".into(),
            fact_status: "fact".into(),
            evidence_ids: vec!["ev-1".into()],
            reasoning_summary: "更新".into(),
            confidence: "high".into(),
            validation_question: None,
            status: "resolved".into(),
        })];
        p2.findings_upsert.push(Finding3::Why5(FindingWhy5 {
            finding_id: "f2".into(),
            level: 2,
            question: "为什么客流减少？".into(),
            answer: "下雨".into(),
            fact_status: "hypothesis".into(),
            evidence_ids: vec![],
            reasoning_summary: "假设".into(),
            confidence: "low".into(),
            validation_question: None,
            status: "candidate".into(),
        }));
        // 同一 evidence 追加不重复
        let r2 = apply_report_patch(Some(&r1), &p2, &seed()).unwrap();
        assert_eq!(r2.revision_no, 2);
        assert_eq!(r2.findings.len(), 2);
        match &r2.findings[0] {
            Finding3::Why5(f) => assert_eq!(f.answer, "更新：周中客流减少"),
            other => panic!("框架形状应保持 5WHY: {other:?}"),
        }
        assert_eq!(r2.evidence_refs.len(), 1, "evidence 按 ID 去重追加");
        assert_eq!(r2.summary, "续问摘要");
    }

    #[test]
    fn warning_rules() {
        let r1 = apply_report_patch(None, &patch(), &seed()).unwrap();
        // 主题不可改
        let mut theme = patch();
        theme.warnings_upsert[0].message = "换了主题".into();
        let e = apply_report_patch(Some(&r1), &theme, &seed()).unwrap_err();
        assert_eq!(e.code, CliErrorCode::SemanticValidationFailed);
        assert!(e.detail.contains("主题"));
        // requirement 不可改
        let mut req = patch();
        req.warnings_upsert[0].resolution_requirement = "user_confirmation".into();
        assert!(apply_report_patch(Some(&r1), &req, &seed()).is_err());
        // active→resolved 需 note+引用
        let mut resolve = patch();
        resolve.warnings_upsert[0].status = "resolved".into();
        resolve.warnings_upsert[0].resolution_note = Some("已补充来源".into());
        let e2 = apply_report_patch(Some(&r1), &resolve, &seed()).unwrap_err();
        assert!(e2.detail.contains("resolution_note"));
        resolve.warnings_upsert[0].evidence_ids = vec!["ev-1".into()];
        let r2 = apply_report_patch(Some(&r1), &resolve, &seed()).unwrap();
        assert_eq!(r2.warnings[0].status, "resolved");
        // resolved 只读：重放同对象 OK（幂等），变更则拒绝；重开拒绝
        let r3 = apply_report_patch(Some(&r2), &resolve, &seed()).unwrap();
        assert_eq!(r3.warnings[0].status, "resolved");
        let mut reopen = resolve.clone();
        reopen.warnings_upsert[0].status = "active".into();
        reopen.warnings_upsert[0].resolution_note = None;
        assert!(apply_report_patch(Some(&r2), &reopen, &seed()).is_err());
        // 复发必须新 ID：同一主题新 warning_id 允许
        let mut recur = patch();
        recur.warnings_upsert[0].warning_id = "w2".into();
        recur.warnings_upsert[0].status = "active".into();
        recur.warnings_upsert[0].resolution_note = None;
        let r4 = apply_report_patch(Some(&r2), &recur, &seed()).unwrap();
        assert_eq!(r4.warnings.len(), 2);
        // active 携带 note 拒绝
        let mut note = patch();
        note.warnings_upsert[0].resolution_note = Some("口述已处理".into());
        assert!(apply_report_patch(Some(&r1), &note, &seed()).is_err());
    }

    #[test]
    fn fact_status_flip_rejected_correction_requires_new_report() {
        let r1 = apply_report_patch(None, &patch(), &seed()).unwrap();
        // 假设不因重复出现升级为事实
        let mut p = patch();
        p.findings_upsert = vec![];
        p.facts_and_hypotheses_upsert = vec![FactItem3 {
            item_id: "h1".into(),
            statement: "假设".into(),
            fact_status: "hypothesis".into(),
            evidence_ids: vec![],
            user_quote: None,
            confidence: "low".into(),
            reasoning_summary: "".into(),
        }];
        let with_hypo = apply_report_patch(Some(&r1), &p, &seed()).unwrap();
        let mut p2 = p.clone();
        p2.facts_and_hypotheses_upsert[0].fact_status = "fact".into();
        let e = apply_report_patch(Some(&with_hypo), &p2, &seed()).unwrap_err();
        assert!(e.detail.contains("冲突"), "实际: {}", e.detail);
        // 已核实事实也不得降级为 user_report
        let mut p3 = p.clone();
        p3.facts_and_hypotheses_upsert[0].fact_status = "fact".into();
        let with_fact = apply_report_patch(Some(&r1), &p3, &seed()).unwrap();
        let mut p4 = p.clone();
        p4.facts_and_hypotheses_upsert[0].fact_status = "user_report".into();
        assert!(apply_report_patch(Some(&with_fact), &p4, &seed()).is_err());
    }

    #[test]
    fn merged_report_validates_against_schema3() {
        let r1 = apply_report_patch(None, &patch(), &seed()).unwrap();
        let value = serde_json::to_value(&r1).unwrap();
        if let Err(errors) = crate::provider::schema::validate_diagnosis_report(&value) {
            panic!(
                "合并后的报告必须通过 schema3: {errors:?}\n{}",
                serde_json::to_string_pretty(&value).unwrap()
            );
        }
        let mut p2 = patch();
        p2.summary_update = Some("二轮".into());
        let r2 = apply_report_patch(Some(&r1), &p2, &seed()).unwrap();
        assert!(crate::provider::schema::validate_diagnosis_report(
            &serde_json::to_value(&r2).unwrap()
        )
        .is_ok());
    }
}
