//! 诊断报告本地读写（cli-surface 27/28/29/25/26 的本地部分）。
//!
//! - list/show：diagnosis_reports 不可变 revision 快照 →
//!   read-views.diagnosisReportSummaryView / report_revision snapshot。
//! - revise：冻结旧会话＋准备新会话（无 Provider 请求；新报告须经正常预览
//!   与确认后生成——不伪造报告内容）。
//! - pending cancel：仅清除匹配 pending 澄清；保留轮次历史与报告。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::generation::report::{
    apply_report_patch, ReportPatch, ReportPeriod, ReportScope, ReportSeed,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn now_iso() -> String {
    dsheng_core::repository::utc_iso(chrono::Utc::now())
}

fn validation(msg: impl Into<String>) -> CliError {
    CliError::new(CliErrorCode::DataValidationFailed, msg.into())
}

/// diagnosisReportSummaryView（每 report 一行，当前 revision）。
pub fn report_list(conn: &Connection, workspace_id: Uuid, run_id: &str) -> CliResult<Value> {
    crate::ctx::ensure_object_in_workspace(
        conn,
        "analysis_runs",
        "run_id",
        run_id,
        workspace_id,
        "run",
    )?;
    let mut stmt = conn.prepare(
        "SELECT r.report_id, r.workspace_id, r.run_id, r.analysis_revision_no, r.framework,
                r.problem_statement, r.report_revision_no, r.state, r.report_json,
                r.session_id, r.created_at, r.updated_at,
                (SELECT s.frozen_at FROM generation_sessions s WHERE s.session_id = r.session_id),
                (SELECT COUNT(*) FROM diagnosis_turns t
                  WHERE t.session_id = r.session_id AND t.turn_state = 'pending')
         FROM diagnosis_reports r
         WHERE r.workspace_id = ?1 AND r.run_id = ?2
         ORDER BY r.created_at",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string(), run_id], |r| {
        let report_json: String = r.get(8)?;
        let report = serde_json::from_str::<Value>(&report_json).unwrap_or(Value::Null);
        let content = report.pointer("/turn/report_patch").unwrap_or(&report);
        let active_warnings = content
            .get("warnings")
            .or_else(|| content.get("warnings_upsert"))
            .and_then(Value::as_array)
            .map(|warnings| warnings.iter().filter(|w| w["status"] == "active").count())
            .unwrap_or(0);
        Ok(json!({
            "report_id": r.get::<_, String>(0)?,
            "workspace_id": r.get::<_, String>(1)?,
            "run_id": r.get::<_, String>(2)?,
            "analysis_revision_no": r.get::<_, i64>(3)?,
            "framework": r.get::<_, String>(4)?,
            "problem_statement": r.get::<_, String>(5)?,
            "report_revision_no": r.get::<_, i64>(6)?,
            "state": r.get::<_, String>(7)?,
            "stop_reason": content.get("stop_reason").cloned().unwrap_or(Value::Null),
            "can_continue": r.get::<_, Option<String>>(12)?.is_none()
                && content.get("can_continue").and_then(Value::as_bool).unwrap_or(false),
            "active_warning_count": active_warnings,
            "created_at": r.get::<_, String>(10)?,
            "updated_at": r.get::<_, String>(11)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(json!({ "run_id": run_id, "reports": out }))
}

/// show：report_revision 快照（revision 省略取最大）。
pub fn report_show(
    conn: &Connection,
    workspace_id: Uuid,
    report_id: &str,
    revision: Option<u32>,
) -> CliResult<Value> {
    let owner: Option<String> = conn
        .query_row(
            "SELECT workspace_id FROM diagnosis_reports WHERE report_id = ?1",
            params![report_id],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    match owner.as_deref() {
        None => return Err(validation("report 不存在")),
        Some(ws) if ws != workspace_id.to_string() => {
            return Err(CliError::new(
                CliErrorCode::WorkspaceMismatch,
                "report 不属于所选 workspace",
            ))
        }
        Some(_) => {}
    }
    let revision = match revision {
        Some(r) => r as i64,
        None => conn
            .query_row(
                "SELECT MAX(report_revision_no) FROM diagnosis_reports WHERE report_id = ?1",
                params![report_id],
                |r| r.get(0),
            )
            .map_err(CliError::from)?,
    };
    let (raw, run_id, analysis_revision, fingerprint, session_id, framework, problem, focus,
        created_at, updated_at): (String, String, u32, String, String, String, String, Option<String>, String, String) = conn
        .query_row(
            "SELECT report_json, run_id, analysis_revision_no, analysis_input_fingerprint,
                    session_id, framework, problem_statement, focus_metric_id, created_at, updated_at
             FROM diagnosis_reports WHERE report_id = ?1 AND report_revision_no = ?2",
            params![report_id, revision],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?)),
        )
        .map_err(|_| validation("report revision 不存在"))?;
    let mut report: Value = serde_json::from_str(&raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "report 快照 JSON 损坏"))?;
    // 0.1.1 曾把 turn 包装直接存为 report；仅从不可变的绑定 revision 恢复。
    // 不读取当前来源文件，不写回旧快照，也不猜测未保存的发送范围。
    if let Some(patch_value) = report.pointer("/turn/report_patch") {
        let patch: ReportPatch = serde_json::from_value(patch_value.clone())
            .map_err(|e| validation(format!("旧诊断报告 patch 无效：{e}")))?;
        let (normalized, policy): (String, String) = conn
            .query_row(
                "SELECT normalized_metrics_json, period_policy_snapshot_json FROM run_revisions
             WHERE run_id = ?1 AND revision_no = ?2",
                params![run_id, analysis_revision],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| validation("诊断报告绑定的分析 revision 不存在"))?;
        let normalized: Value =
            serde_json::from_str(&normalized).map_err(|_| validation("分析快照 JSON 损坏"))?;
        let policy: Value =
            serde_json::from_str(&policy).map_err(|_| validation("周期策略快照 JSON 损坏"))?;
        let seed = ReportSeed {
            report_id: report_id.to_string(),
            generation_session_id: session_id,
            workspace_id: workspace_id.to_string(),
            run_id,
            framework: serde_json::from_value(json!(framework))
                .map_err(|_| validation("诊断框架无效"))?,
            scope: ReportScope {
                problem_statement: problem,
                focus_metric_ids: focus.into_iter().collect(),
            },
            period: ReportPeriod {
                from: normalized
                    .pointer("/period/from")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                to: normalized
                    .pointer("/period/to")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                period_policy_version: policy
                    .get("version")
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default(),
            },
            source_manifest: vec![],
            analysis_revision_no: analysis_revision,
            analysis_input_fingerprint: fingerprint,
            now: created_at,
            current_user_message: None,
        };
        let mut recovered =
            apply_report_patch(None, &patch, &seed).map_err(|e| CliError::new(e.code, e.detail))?;
        recovered.revision_no = revision as u32;
        recovered.updated_at = updated_at;
        report = serde_json::to_value(recovered).map_err(|e| validation(e.to_string()))?;
    }
    dsheng_core::provider::schema::validate_diagnosis_report(&report)
        .map_err(|errors| validation(format!("诊断报告快照不完整：{}", errors.join("; "))))?;
    Ok(report)
}

/// report JSON → 会议/归档 Markdown（本地投影，不调模型）。
pub fn report_markdown(report: &Value) -> String {
    let mut out = String::new();
    let title = report
        .get("scope")
        .and_then(|s| s.get("problem_statement"))
        .and_then(|p| p.as_str())
        .unwrap_or("诊断报告");
    use std::fmt::Write as _;
    let _ = writeln!(out, "# {title}");
    let _ = writeln!(out);
    for (key, heading) in [
        ("findings", "发现"),
        ("facts_and_hypotheses", "事实与假设"),
        ("warnings", "警告"),
        ("open_questions", "待澄清问题"),
    ] {
        if let Some(items) = report.get(key).and_then(|v| v.as_array()) {
            if items.is_empty() {
                continue;
            }
            let _ = writeln!(out, "## {heading}");
            for item in items {
                let line = item
                    .get("summary")
                    .or_else(|| item.get("statement"))
                    .or_else(|| item.get("note"))
                    .or_else(|| item.get("question"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("（无摘要）");
                let _ = writeln!(out, "- {line}");
            }
            let _ = writeln!(out);
        }
    }
    out
}

/// session 归属校验。
fn session_of(
    conn: &Connection,
    workspace_id: Uuid,
    session_id: &str,
) -> CliResult<(String, Option<String>)> {
    let row: Option<(String, Option<String>)> = conn
        .query_row(
            "SELECT workspace_id, frozen_at FROM generation_sessions WHERE session_id = ?1",
            params![session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    match row {
        None => Err(validation("session 不存在")),
        Some((ws, frozen)) => {
            if ws != workspace_id.to_string() {
                return Err(CliError::new(
                    CliErrorCode::WorkspaceMismatch,
                    "session 不属于所选 workspace",
                ));
            }
            Ok((ws, frozen))
        }
    }
}

/// 会话行投影（revise 复制到新会话所需列）。
type SessionRow = (
    String,
    i64,
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
);

/// revise：冻结旧会话＋建新会话（记录替换声明；不生成报告、不发请求）。
pub fn revise(
    conn: &mut Connection,
    workspace_id: Uuid,
    session_id: &str,
    fact_id: &str,
    replacement: &str,
) -> CliResult<Value> {
    let replacement_trimmed = replacement.trim();
    if replacement_trimmed.is_empty() {
        return Err(validation("replacement-stdin 必须提供非空更正声明"));
    }
    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let (_, frozen) = session_of(&tx, workspace_id, session_id)?;
    if frozen.is_some() {
        return Err(validation("session 已冻结"));
    }
    let (
        run_id,
        analysis_revision_no,
        analysis_fingerprint,
        generation_fingerprint,
        workflow,
        framework,
        prompt_version,
        schema_version,
        input_schema_version,
        context_policy_version,
        source_manifest_hash,
        scope_snapshot,
    ): SessionRow = tx
        .query_row(
            "SELECT run_id, analysis_revision_no, analysis_input_fingerprint,
                    generation_input_fingerprint, workflow, framework, prompt_version,
                    schema_version, input_schema_version, context_policy_version,
                    source_manifest_hash, scope_snapshot_json
             FROM generation_sessions WHERE session_id = ?1",
            params![session_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .map_err(|_| validation("session 读取失败"))?;
    let now = now_iso();
    tx.execute(
        "UPDATE generation_sessions SET frozen_at = ?2,
                frozen_reason = 'diagnosis_revise:fact_replaced', updated_at = ?2
         WHERE session_id = ?1 AND frozen_at IS NULL",
        params![session_id, now],
    )
    .map_err(CliError::from)?;
    // 新会话：记录被替换 fact 与用户更正声明（scope_snapshot；无报告伪造）
    let new_session = Uuid::new_v4().to_string();
    let mut new_scope = serde_json::from_str::<Value>(scope_snapshot.as_deref().unwrap_or("{}"))
        .unwrap_or_else(|_| json!({}));
    if !new_scope.is_object() {
        new_scope = json!({});
    }
    if let Some(obj) = new_scope.as_object_mut() {
        obj.insert(
            "replacement_of".into(),
            json!({
                "old_session_id": session_id,
                "fact_id": fact_id,
                "replacement_statement": replacement_trimmed,
            }),
        );
    }
    tx.execute(
        "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
                analysis_input_fingerprint, generation_input_fingerprint, workflow, framework,
                state, prompt_version, schema_version, input_schema_version,
                context_policy_version, source_manifest_hash, scope_snapshot_json,
                created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'awaiting_generation', ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?15)",
        params![
            new_session,
            workspace_id.to_string(),
            run_id,
            analysis_revision_no,
            analysis_fingerprint,
            generation_fingerprint,
            workflow,
            framework,
            prompt_version,
            schema_version,
            input_schema_version,
            context_policy_version,
            source_manifest_hash,
            serde_json::to_string(&new_scope).unwrap(),
            now
        ],
    )
    .map_err(CliError::from)?;
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    Ok(json!({
        "old_session_id": session_id,
        "new_session_id": new_session,
        "frozen": true,
        "fact_id": fact_id,
        "replacement_recorded": true,
        "next": "新会话须经 diagnosis generate/continue 的预览与双门禁确认后才发请求",
    }))
}

/// pending cancel：仅清除匹配 pending 澄清（保留轮次历史；不改报告）。
pub fn pending_cancel(
    conn: &mut Connection,
    workspace_id: Uuid,
    session_id: &str,
    turn_id: &str,
    expected_revision: u32,
    confirm_local: bool,
) -> CliResult<Value> {
    if !confirm_local {
        return Err(
            CliError::invalid("pending cancel 需要 --confirm-local").with_field("--confirm-local")
        );
    }
    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    session_of(&tx, workspace_id, session_id)?;
    let turn: Option<(String, i64)> = tx
        .query_row(
            "SELECT turn_state, turn_no FROM diagnosis_turns WHERE turn_id = ?1 AND session_id = ?2",
            params![turn_id, session_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let Some((state, _turn_no)) = turn else {
        return Err(validation("turn 不存在或不属于该 session"));
    };
    if state != "pending" {
        return Err(validation("该轮次不是 pending 澄清（无可清除项）"));
    }
    // expected_revision＝该 session 当前报告 revision（无报告时为 0）
    let current_revision: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(report_revision_no), 0) FROM diagnosis_reports
             WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    if expected_revision != current_revision as u32 {
        return Err(CliError::new(
            CliErrorCode::RevisionConflict,
            format!(
                "expected_revision {}，当前报告 revision {current_revision}",
                expected_revision
            ),
        ));
    }
    tx.execute(
        "UPDATE diagnosis_turns SET turn_state = 'withdrawn' WHERE turn_id = ?1",
        params![turn_id],
    )
    .map_err(CliError::from)?;
    // 会话 pending 指针（scope_snapshot.pending）清除
    let scope: Option<String> = tx
        .query_row(
            "SELECT scope_snapshot_json FROM generation_sessions WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    if let Some(raw) = scope {
        if let Ok(mut v) = serde_json::from_str::<Value>(&raw) {
            if let Some(obj) = v.as_object_mut() {
                if obj.remove("pending").is_some() {
                    tx.execute(
                        "UPDATE generation_sessions SET scope_snapshot_json = ?2, updated_at = ?3
                         WHERE session_id = ?1",
                        params![session_id, serde_json::to_string(&v).unwrap(), now_iso()],
                    )
                    .map_err(CliError::from)?;
                }
            }
        }
    }
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    Ok(json!({
        "session_id": session_id,
        "turn_id": turn_id,
        "cleared": true,
        "report_untouched": true,
    }))
}
