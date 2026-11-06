//! 任务生成显式引用的诊断报告：只读绑定校验和输入投影，不调用模型。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::generation::input::{
    FactStatusProjection, OptionalDiagnosisReference, ReportFactProjection,
};
use dsheng_core::generation::report::Report3;
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

fn stale(message: impl Into<String>) -> CliError {
    CliError::new(CliErrorCode::StaleDiagnosisReference, message)
}

/// 按显式选择的报告 ID 读取当前保存版本；不能通过传入伪造 JSON 绕过绑定。
pub fn load_task_report(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    analysis_revision: u32,
    fingerprint: &str,
    report_id: &str,
) -> CliResult<Value> {
    let binding = conn
        .query_row(
            "SELECT r.workspace_id,r.run_id,r.analysis_revision_no,
                    r.analysis_input_fingerprint,r.report_revision_no,r.framework,r.state,
                    s.session_id,s.workspace_id,s.run_id,s.frozen_at
             FROM diagnosis_reports r LEFT JOIN generation_sessions s ON s.session_id=r.session_id
             WHERE r.report_id=?1 ORDER BY r.report_revision_no DESC LIMIT 1",
            [report_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, u32>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            CliError::new(
                CliErrorCode::DataValidationFailed,
                "所选诊断报告不存在，请重新选择或取消引用",
            )
        })?;
    let workspace = workspace_id.to_string();
    if binding.0 != workspace || binding.8.as_deref() != Some(workspace.as_str()) {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "所选诊断报告不属于当前企业",
        ));
    }
    if binding.1 != run_id
        || binding.2 != analysis_revision
        || binding.3 != fingerprint
        || binding.6 != "accepted"
        || binding.9.as_deref() != Some(run_id)
        || binding.10.is_some()
    {
        return Err(stale(
            "所选诊断报告已失效、已冻结或与当前分析不一致，请重新选择或取消引用",
        ));
    }
    let report = super::diagnosis::report_show(conn, workspace_id, report_id, Some(binding.4))?;
    // JSON 中的身份也须与持久化元数据一致；旧 turn 由 report_show 只读恢复。
    if report["report_id"] != report_id
        || report["workspace_id"] != workspace
        || report["run_id"] != run_id
        || report["analysis_revision_no"] != analysis_revision
        || report["analysis_input_fingerprint"] != fingerprint
        || report["revision_no"] != binding.4
        || report["framework"] != binding.5
        || report["generation_session_id"] != json!(binding.7)
    {
        return Err(stale(
            "所选诊断报告内容与保存版本绑定不一致，请重新选择或取消引用",
        ));
    }
    Ok(report)
}

/// 转为冻结任务输入合同。调用方先补齐报告所需来源并规范化证据别名。
/// 任何不可提供的引用都明确报错，不静默剥除事实的证据或改写事实状态。
pub fn project_task_report(
    report: &Value,
    allowed_evidence: &BTreeSet<String>,
) -> CliResult<OptionalDiagnosisReference> {
    let typed: Report3 = serde_json::from_value(report.clone()).map_err(|e| {
        CliError::new(
            CliErrorCode::InputContractInvalid,
            format!("所选诊断报告不能投影为任务背景：{e}"),
        )
    })?;
    let mut evidence_ids = BTreeSet::new();
    collect_evidence(report, &mut evidence_ids);
    let missing: Vec<_> = evidence_ids.difference(allowed_evidence).cloned().collect();
    if !missing.is_empty() {
        return Err(CliError::new(
            CliErrorCode::InputContractInvalid,
            "所选诊断报告引用的证据不在本次可发送范围，请核对来源或取消报告引用",
        )
        .with_details(json!({"unavailable_evidence_ids":missing})));
    }
    let mut facts = Vec::new();
    let mut hypotheses = Vec::new();
    let mut used_ids = BTreeSet::new();
    for item in typed.facts_and_hypotheses {
        used_ids.insert(item.item_id.clone());
        push_fact(
            ReportFactProjection {
                item_id: item.item_id,
                statement: item.statement,
                fact_status: parse_status(&item.fact_status)?,
                evidence_ids: item.evidence_ids,
                user_quote: item.user_quote,
                confidence: serde_json::from_value(json!(item.confidence))
                    .map_err(|_| stale("诊断事实的置信度无效"))?,
                reasoning_summary: item.reasoning_summary,
            },
            &mut facts,
            &mut hypotheses,
        );
    }
    // 输入合同无 findings 字段；逐条保留发现内容及原状态，不把候选发现升为已核实。
    for finding in report["findings"].as_array().into_iter().flatten() {
        let mut item_id = format!(
            "finding:{}",
            finding["finding_id"].as_str().unwrap_or_default()
        );
        while !used_ids.insert(item_id.clone()) {
            item_id.insert_str(0, "finding:");
        }
        let mut text = vec![format!(
            "诊断发现（状态：{}；类型：{}）：{} {}",
            finding["status"].as_str().unwrap_or_default(),
            finding["kind"].as_str().unwrap_or("observed"),
            finding["question"].as_str().unwrap_or_default(),
            finding["answer"].as_str().unwrap_or_default(),
        )];
        for (field, label) in [
            ("reasoning_summary", "依据说明"),
            ("validation_question", "待核实"),
            ("missing_information", "缺失信息"),
            ("clarification_questions", "待澄清问题"),
        ] {
            match &finding[field] {
                Value::String(value) if !value.is_empty() => text.push(format!("{label}：{value}")),
                Value::Array(values) if !values.is_empty() => text.push(format!(
                    "{label}：{}",
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("；")
                )),
                _ => {}
            }
        }
        push_fact(
            ReportFactProjection {
                item_id,
                statement: text.join("\n"),
                fact_status: parse_status(finding["fact_status"].as_str().unwrap_or_default())?,
                evidence_ids: serde_json::from_value(finding["evidence_ids"].clone())
                    .map_err(|_| stale("诊断发现的证据列表无效"))?,
                user_quote: None,
                confidence: serde_json::from_value(finding["confidence"].clone())
                    .map_err(|_| stale("诊断发现的置信度无效"))?,
                reasoning_summary: finding["reasoning_summary"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            },
            &mut facts,
            &mut hypotheses,
        );
    }
    let mut summary = format!(
        "问题范围：{}\n{}",
        typed.scope.problem_statement, typed.summary
    );
    for warning in typed.warnings {
        summary.push_str(&format!(
            "\n报告提示（{}）：{}；解除条件：{}；处理记录：{}",
            if warning.status == "resolved" {
                "已解除"
            } else {
                "未解除"
            },
            warning.message,
            warning.resolution_requirement,
            warning.resolution_note.as_deref().unwrap_or("无")
        ));
    }
    for question in typed.clarification_questions {
        summary.push_str(&format!(
            "\n澄清问题（{}）：{}；回答记录：{}",
            question.status,
            question.question,
            question.answer_summary.as_deref().unwrap_or("未回答")
        ));
    }
    if let Some(reason) = typed.stop_reason {
        summary.push_str(&format!("\n报告停止原因：{reason}"));
    }
    Ok(OptionalDiagnosisReference {
        report_id: typed.report_id,
        report_revision_no: typed.revision_no,
        analysis_revision_no: typed.analysis_revision_no,
        analysis_input_fingerprint: typed.analysis_input_fingerprint,
        framework: typed.framework,
        summary,
        facts,
        hypotheses,
        evidence_ids: evidence_ids.into_iter().collect(),
    })
}

fn parse_status(status: &str) -> CliResult<FactStatusProjection> {
    serde_json::from_value(json!(status)).map_err(|_| stale("所选报告含未知事实状态"))
}

fn push_fact(
    item: ReportFactProjection,
    facts: &mut Vec<ReportFactProjection>,
    hypotheses: &mut Vec<ReportFactProjection>,
) {
    match item.fact_status {
        FactStatusProjection::Fact | FactStatusProjection::UserReport => facts.push(item),
        FactStatusProjection::Hypothesis | FactStatusProjection::Unknown => hypotheses.push(item),
    }
}

fn collect_evidence(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key == "evidence_id" {
                    if let Some(id) = value.as_str() {
                        ids.insert(id.to_string());
                    }
                } else if key == "evidence_ids" {
                    if let Some(values) = value.as_array() {
                        ids.extend(values.iter().filter_map(Value::as_str).map(str::to_string));
                    }
                } else {
                    collect_evidence(value, ids);
                }
            }
        }
        Value::Array(values) => values.iter().for_each(|value| collect_evidence(value, ids)),
        _ => {}
    }
}
