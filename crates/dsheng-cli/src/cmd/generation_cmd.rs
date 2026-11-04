//! generation 命令组（cli-surface 23/24/30/31/32/33）——双门禁接线。
//!
//! 任何一条请求都必须：预览（--dry-run 或无 flag）→ 用户拿到
//! confirmation_id（TTL 600s）→ `--confirmation-id <id> --confirm-send` 双门禁
//! → 发送前逐项重验快照（provider revision/分析 revision/指纹/manifest/预算）
//! → 原子消费建 attempt → 注入的 dispatcher 派发一次。
//! 发送前的本地拒绝：缺 --problem（clap 必填）、focus metric 不属本 run、
//! 证据未关联、诊断引用 revision 不匹配（STALE_DIAGNOSIS_REFERENCE）。

use crate::cli;
use crate::cmd::{parse_gate, resolve_provider, Gate};
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::jsonio;
use crate::store::generation as gen;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::provider::{Framework, GenerationOperation, Workflow};
use serde_json::json;

const TASK_PROMPT: &str = dsheng_core::contracts::prompts::TASK_PROMPT_VERSION;
const TASK_SCHEMA: &str = dsheng_core::contracts::prompts::TASK_OUTPUT_SCHEMA_VERSION;
const DIAG_PROMPT: &str = dsheng_core::contracts::prompts::DIAGNOSIS_PROMPT_VERSION;
const DIAG_SCHEMA: &str = dsheng_core::contracts::prompts::DIAGNOSIS_SCHEMA_VERSION;

pub fn dispatch(
    action: cli::GenerationAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    match action {
        cli::GenerationAction::Retry {
            workspace,
            attempt,
            provider,
            dry_run,
            confirmation_id,
            confirm_send,
        } => retry(
            workspace,
            attempt,
            provider,
            dry_run,
            confirmation_id,
            confirm_send,
            db,
            None,
        ),
        cli::GenerationAction::Status { workspace, attempt } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let attempt_id = ctx::uuid_arg(&attempt, "--attempt")?;
            gen::attempt_status(&conn, ws.workspace_id, attempt_id)
        }
        cli::GenerationAction::Cancel { workspace, attempt } => {
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let attempt_id = ctx::uuid_arg(&attempt, "--attempt")?;
            gen::cancel_attempt(&mut conn, ws.workspace_id, attempt_id)
        }
    }
}

/// diagnosis generate/continue（经 diagnosis_cmd 转发；CLI 进度＝stderr 节流）。
pub fn diagnosis_generation(
    action: cli::DiagnosisAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    diagnosis_generation_with_progress(action, db, None)
}

/// diagnosis generate/continue（可选注入进度 sink；桌面壳经
/// cmd::dispatch_with_progress 传入事件桥）。
pub fn diagnosis_generation_with_progress(
    action: cli::DiagnosisAction,
    db: Option<&String>,
    progress: Option<gen::ProgressSink>,
) -> CliResult<serde_json::Value> {
    match action {
        cli::DiagnosisAction::Generate {
            workspace,
            run,
            framework,
            problem,
            focus_metric,
            evidence,
            provider,
            dry_run,
            confirmation_id,
            confirm_send,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            ctx::ensure_object_in_workspace(
                &conn,
                "analysis_runs",
                "run_id",
                &run,
                ws.workspace_id,
                "run",
            )?;
            // 本地发送前校验：focus metric 属于本 run 的指标集合
            let snapshot_metrics = run_metric_ids(&conn, &run)?;
            if let Some(focus) = focus_metric.as_deref() {
                if !snapshot_metrics.iter().any(|m| m == focus) {
                    return Err(CliError::new(
                        CliErrorCode::InputContractInvalid,
                        "focus metric 不属于该 run 的指标集合",
                    )
                    .with_field("--focus-metric"));
                }
            }
            if evidence.is_empty() {
                return Err(CliError::new(
                    CliErrorCode::InputContractInvalid,
                    "至少提供一个证据 ID（--evidence）",
                )
                .with_field("--evidence"));
            }
            // 证据以 run 扫描来源内容哈希/手工输入命名空间校验（未关联 → 拒绝发送）
            let all_sources: Vec<String> = {
                let mut stmt =
                    conn.prepare("SELECT content_hash FROM source_files WHERE workspace_id = ?1")?;
                let hashes: Vec<String> = stmt
                    .query_map(rusqlite::params![ws.workspace_id.to_string()], |r| r.get(0))?
                    .collect::<Result<_, _>>()
                    .map_err(CliError::from)?;
                hashes
            };
            for e in evidence.iter() {
                if !(all_sources.iter().any(|s| s == e)
                    || e.starts_with("manual:")
                    || e.starts_with("cell:"))
                {
                    return Err(CliError::new(
                        CliErrorCode::InputContractInvalid,
                        format!("证据 {e} 未与本 run 关联（扫描来源/手工输入之外的证据不得发送）"),
                    )
                    .with_field("--evidence"));
                }
            }
            drop(conn);
            let framework = match framework {
                cli::FrameworkArg::Why5 => Framework::Why5,
                cli::FrameworkArg::W5 => Framework::W5,
                cli::FrameworkArg::W5h2 => Framework::W5h2,
            };
            let mut conn = ctx::open_db(db)?;
            let material = gen::build_diagnosis_payload(
                &conn,
                ws.workspace_id,
                &run,
                framework,
                &problem,
                &focus_metric.clone().map(|f| vec![f]).unwrap_or_default(),
                &evidence,
                dsheng_core::generation::input::SessionStateProjection {
                    accepted_facts: vec![],
                    prior_report_summary: None,
                    unresolved_questions: vec![],
                    pending_clarification: None,
                },
                None,
                &problem,
            )?;
            gate_and_run(
                &mut conn,
                ws.workspace_id,
                provider,
                dry_run,
                confirmation_id,
                confirm_send,
                material,
                GenerationOperation::DiagnosisGenerate,
                DIAG_PROMPT,
                DIAG_SCHEMA,
                Box::new(gen::HttpDispatch),
                OutputKind::Diagnosis { framework },
                progress,
            )
        }
        cli::DiagnosisAction::Continue {
            workspace,
            session,
            answers_stdin,
            provider,
            dry_run,
            confirmation_id,
            confirm_send,
        } => diagnosis_continue_value_with_progress(
            workspace,
            session,
            AnswerSource::Stdin {
                require_flag: answers_stdin,
            },
            provider,
            dry_run,
            confirmation_id,
            confirm_send,
            db,
            progress,
        ),
        other => {
            let _ = other;
            Err(CliError::invalid("该 diagnosis 子命令不经生成门禁层"))
        }
    }
}

/// diagnosis continue 答复来源：CLI=stdin（`--answers-stdin` flag 校验）；
/// 桌面壳=进程内联文本（supplement_fact）。
pub enum AnswerSource {
    Stdin { require_flag: bool },
    Inline(String),
}

/// diagnosis continue（进程内入口；CLI 与桌面壳共用同一实现）。
/// answer 读取位置/校验顺序与原 CLI 路径一致（仅来源不同）。
#[allow(clippy::too_many_arguments)]
pub fn diagnosis_continue_value(
    workspace: Option<String>,
    session: String,
    answer: AnswerSource,
    provider: Option<String>,
    dry_run: bool,
    confirmation_id: Option<String>,
    confirm_send: bool,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    diagnosis_continue_value_with_progress(
        workspace,
        session,
        answer,
        provider,
        dry_run,
        confirmation_id,
        confirm_send,
        db,
        None,
    )
}

/// diagnosis continue（可选注入进度 sink；桌面壳续问确认经此发事件）。
#[allow(clippy::too_many_arguments)]
pub fn diagnosis_continue_value_with_progress(
    workspace: Option<String>,
    session: String,
    answer: AnswerSource,
    provider: Option<String>,
    dry_run: bool,
    confirmation_id: Option<String>,
    confirm_send: bool,
    db: Option<&String>,
    progress: Option<gen::ProgressSink>,
) -> CliResult<serde_json::Value> {
    if let AnswerSource::Stdin {
        require_flag: false,
    } = answer
    {
        return Err(
            CliError::invalid("diagnosis continue 需要 --answers-stdin 提供本轮完整答复")
                .with_field("--answers-stdin"),
        );
    }
    let conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    let (run, framework_raw, frozen): (String, String, Option<String>) = conn
        .query_row(
            "SELECT run_id, COALESCE(framework, '5why'), frozen_at
             FROM generation_sessions WHERE session_id = ?1",
            rusqlite::params![session],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "session 不存在"))?;
    if frozen.is_some() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "session 已冻结（revise 后的新会话继续）",
        ));
    }
    let framework: Framework = match framework_raw.as_str() {
        "5why" => Framework::Why5,
        "5w" => Framework::W5,
        "5w2h" => Framework::W5h2,
        _ => return Err(CliError::invalid("framework 取值非法")),
    };
    let answer = match answer {
        AnswerSource::Stdin { .. } => jsonio::read_stdin_text()?,
        AnswerSource::Inline(text) => text,
    };
    if answer.trim().is_empty() {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "answers-stdin 为空",
        ));
    }
    let problem: String = conn
        .query_row(
            "SELECT problem_statement FROM diagnosis_reports WHERE session_id = ?1
             ORDER BY report_revision_no DESC LIMIT 1",
            rusqlite::params![session],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "继续诊断".into());
    drop(conn);
    let mut conn = ctx::open_db(db)?;
    let material = gen::build_diagnosis_payload(
        &conn,
        ws.workspace_id,
        &run,
        framework,
        &problem,
        &[],
        &[],
        dsheng_core::generation::input::SessionStateProjection {
            accepted_facts: vec![],
            prior_report_summary: None,
            unresolved_questions: vec![],
            pending_clarification: None,
        },
        Some(
            dsheng_core::generation::input::ContinuationInputProjection {
                mode: dsheng_core::generation::input::ContinuationMode::Append,
                answer_to_turn_id: None,
                selected_open_question_ids: vec![],
                user_message: answer.trim().to_string(),
            },
        ),
        answer.trim(),
    )?;
    gate_and_run(
        &mut conn,
        ws.workspace_id,
        provider,
        dry_run,
        confirmation_id,
        confirm_send,
        material,
        GenerationOperation::DiagnosisContinue,
        DIAG_PROMPT,
        DIAG_SCHEMA,
        Box::new(gen::HttpDispatch),
        OutputKind::Diagnosis { framework },
        progress,
    )
}

/// task generate（经 task_cmd 转发；CLI 进度＝stderr 节流）。
pub fn task_generate(action: cli::TaskAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    task_generate_with_progress(action, db, None)
}

/// task generate（可选注入进度 sink）。
pub fn task_generate_with_progress(
    action: cli::TaskAction,
    db: Option<&String>,
    progress: Option<gen::ProgressSink>,
) -> CliResult<serde_json::Value> {
    let cli::TaskAction::Generate {
        workspace,
        run,
        diagnosis_report,
        provider,
        dry_run,
        confirmation_id,
        confirm_send,
    } = action
    else {
        return Err(CliError::invalid("task generate 参数不完整"));
    };
    let conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    ctx::ensure_object_in_workspace(
        &conn,
        "analysis_runs",
        "run_id",
        &run,
        ws.workspace_id,
        "run",
    )?;
    let (revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        &conn,
        &ws.workspace_id.to_string(),
        &run,
    )
    .map_err(CliError::from)?;
    let report_value = match diagnosis_report.as_deref() {
        Some(report_id) => {
            let (owner, report_revision, report_analysis_rev, report_fp): (
                String,
                i64,
                i64,
                String,
            ) = conn
                .query_row(
                    "SELECT workspace_id, report_revision_no, analysis_revision_no,
                            analysis_input_fingerprint
                     FROM diagnosis_reports WHERE report_id = ?1
                     ORDER BY report_revision_no DESC LIMIT 1",
                    rusqlite::params![report_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .map_err(|_| {
                    CliError::new(
                        CliErrorCode::DataValidationFailed,
                        "diagnosis report 不存在",
                    )
                })?;
            if owner != ws.workspace_id.to_string() {
                return Err(CliError::new(
                    CliErrorCode::WorkspaceMismatch,
                    "diagnosis report 不属于所选 workspace",
                ));
            }
            if report_analysis_rev != revision as i64 || report_fp != fingerprint {
                return Err(CliError::new(
                    CliErrorCode::StaleDiagnosisReference,
                    "诊断报告与当前分析 revision/指纹不匹配（重做分析后需重新生成报告）",
                ));
            }
            Some(json!({
                "report_id": report_id,
                "report_revision_no": report_revision,
                "framework": "5why",
                "scope": {"problem_statement": "（见报告）"},
            }))
        }
        None => None,
    };
    drop(conn);
    let mut conn = ctx::open_db(db)?;
    let material = gen::build_task_payload(
        &conn,
        ws.workspace_id,
        &run,
        "（CLI 任务生成；无额外用户上下文）",
        report_value.as_ref(),
    )?;
    gate_and_run(
        &mut conn,
        ws.workspace_id,
        provider,
        dry_run,
        confirmation_id,
        confirm_send,
        material,
        GenerationOperation::TaskGenerate,
        TASK_PROMPT,
        TASK_SCHEMA,
        Box::new(gen::HttpDispatch),
        OutputKind::Task,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
fn retry(
    workspace: Option<String>,
    attempt: String,
    provider: Option<String>,
    dry_run: bool,
    confirmation_id: Option<String>,
    confirm_send: bool,
    db: Option<&String>,
    progress: Option<gen::ProgressSink>,
) -> CliResult<serde_json::Value> {
    let conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    let old_attempt_id = ctx::uuid_arg(&attempt, "--attempt")?;
    let status = gen::attempt_status(&conn, ws.workspace_id, old_attempt_id)?;
    match status["terminal_state"].as_str() {
        Some("failed") | Some("cancelled") | Some("interrupted") | Some("timed_out") => {}
        None => {
            return Err(CliError::new(
                CliErrorCode::IdempotencyConflict,
                "attempt 未终态，不得重试",
            ))
        }
        Some("success") => {
            return Err(CliError::new(
                CliErrorCode::IdempotencyConflict,
                "成功 attempt 不可重试",
            ))
        }
        Some(other) => {
            return Err(CliError::invalid(format!("未知终态 {other}")));
        }
    }
    let (run, workflow_raw): (String, String) = conn
        .query_row(
            "SELECT s.run_id, s.workflow FROM generation_attempts a
             JOIN generation_sessions s ON s.session_id = a.session_id
             WHERE a.attempt_id = ?1",
            rusqlite::params![old_attempt_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "会话读取失败"))?;
    let workflow: Workflow = serde_json::from_value(json!(workflow_raw))
        .map_err(|_| CliError::invalid("workflow 非法"))?;
    drop(conn);
    let mut conn = ctx::open_db(db)?;
    let material = match workflow {
        Workflow::Task => {
            gen::build_task_payload(&conn, ws.workspace_id, &run, "（重试任务生成）", None)?
        }
        Workflow::Diagnosis => {
            return Err(CliError::new(
                CliErrorCode::DataValidationFailed,
                "诊断重试经 diagnosis generate/continue 全新会话执行",
            ))
        }
    };
    // 重试必须全新 confirmation（gate 层校验 id 不同）
    gate_and_run(
        &mut conn,
        ws.workspace_id,
        provider,
        dry_run,
        confirmation_id,
        confirm_send,
        material,
        GenerationOperation::GenerationRetry,
        TASK_PROMPT,
        TASK_SCHEMA,
        Box::new(gen::HttpDispatch),
        OutputKind::Task,
        progress,
    )
}

enum OutputKind {
    Task,
    Diagnosis { framework: Framework },
}

#[allow(clippy::too_many_arguments)]
fn gate_and_run(
    conn: &mut rusqlite::Connection,
    workspace_id: uuid::Uuid,
    provider_flag: Option<String>,
    dry_run: bool,
    confirmation_id: Option<String>,
    confirm_send: bool,
    material: gen::GenerationMaterial,
    operation: GenerationOperation,
    prompt_version: &str,
    schema_version: &str,
    dispatcher: Box<dyn gen::RequestDispatch>,
    output_kind: OutputKind,
    progress: Option<gen::ProgressSink>,
) -> CliResult<serde_json::Value> {
    let gate = parse_gate(dry_run, &confirmation_id, confirm_send)?;
    let provider_id = resolve_provider(conn, workspace_id, &provider_flag)?;
    let provider = gen::provider_snapshot(conn, workspace_id, provider_id)?;
    match gate {
        Gate::Preview => gen::create_preview(
            conn,
            workspace_id,
            &provider,
            &material,
            operation,
            prompt_version,
            schema_version,
        ),
        Gate::Send { confirmation_id } => {
            let mut dispatcher = dispatcher;
            let mut sink = progress.unwrap_or_else(gen::ProgressSink::stderr);
            let mut on_progress = |event: gen::StreamProgressEvent| sink.on_event(event);
            let (attempt_id, output) = gen::send_confirmed(
                conn,
                workspace_id,
                &provider,
                &material,
                confirmation_id,
                prompt_version,
                schema_version,
                dispatcher.as_mut(),
                &mut on_progress,
            )?;
            match output_kind {
                OutputKind::Task => {
                    let candidate = crate::store::generation::save_task_candidate(
                        conn,
                        workspace_id,
                        &material,
                        attempt_id,
                        &output,
                        false,
                    )?;
                    Ok(json!({
                        "attempt_id": attempt_id.to_string(),
                        "candidate_generation_id": candidate,
                        "state": "candidate",
                        "next": "task reconcile --dry-run 应用候选",
                    }))
                }
                OutputKind::Diagnosis { framework } => {
                    // turn schema 校验（框架对应）→ 状态机 → 报告/轮次落库
                    dsheng_core::provider::schema::validate_diagnosis_turn(framework, &output)
                        .map_err(|errors| {
                            CliError::new(CliErrorCode::SchemaValidationFailed, errors.join("; "))
                        })?;
                    let turn: TurnJson = serde_json::from_value(output.clone()).map_err(|e| {
                        CliError::new(CliErrorCode::SemanticValidationFailed, e.to_string())
                    })?;
                    let session_id: String = conn
                        .query_row(
                            "SELECT session_id FROM generation_sessions
                             WHERE workspace_id = ?1 AND run_id = ?2
                             ORDER BY created_at DESC LIMIT 1",
                            rusqlite::params![
                                workspace_id.to_string(),
                                material.run_id.to_string()
                            ],
                            |r| r.get(0),
                        )
                        .map_err(CliError::from)?;
                    let turn_no: i64 = conn
                        .query_row(
                            "SELECT COALESCE(MAX(turn_no), 0) + 1 FROM diagnosis_turns WHERE session_id = ?1",
                            rusqlite::params![session_id],
                            |r| r.get(0),
                        )
                        .unwrap_or(1);
                    let disposition = turn.turn_disposition.clone();
                    let report_json = json!({
                        "schema_version": "3",
                        "framework": gen::framework_db_str(framework),
                        "scope": {"problem_statement": material.user_supplied_text},
                        "summary": turn.report_patch.as_ref().and_then(|p| p.summary_update.clone()),
                        "turn_disposition": disposition,
                        "accepted_facts": turn.accepted_facts,
                        "clarification_questions": turn.clarification_questions,
                        "redirect_message": turn.redirect_message,
                        "stop_reason": turn.stop_reason,
                    });
                    let state = match disposition.as_str() {
                        "needs_clarification" => "clarification",
                        "off_topic_redirect" => "redirect",
                        _ => "patched",
                    };
                    let turn_id = crate::store::generation::save_diagnosis_turn(
                        conn,
                        workspace_id,
                        &material,
                        framework,
                        attempt_id,
                        &session_id,
                        turn_no,
                        &material.user_supplied_text,
                        &report_json,
                        if state == "patched" { 1 } else { 0 },
                        state,
                        turn.stop_reason.as_deref(),
                    )?;
                    Ok(json!({
                        "attempt_id": attempt_id.to_string(),
                        "session_id": session_id,
                        "turn_id": turn_id,
                        "turn_state": state,
                        "report_id": report_json.get("report_id"),
                    }))
                }
            }
        }
    }
}

fn run_metric_ids(conn: &rusqlite::Connection, run_id: &str) -> CliResult<Vec<String>> {
    let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &crate::store::generation::workspace_of_run(conn, run_id)?,
        run_id,
    )
    .map_err(CliError::from)?;
    let raw: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            rusqlite::params![run_id, current as i64],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    let v: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    Ok(v["metrics"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m["metric_key"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// 诊断 turn JSON（schema3 已校验后的本地投影；顶层字段与冻结 turn schema
/// 完全一致——2026-11-04 真实联调教训：投影窄于合同会误拒合法输出）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnJson {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    framework: Option<String>,
    turn_disposition: String,
    #[serde(default)]
    accepted_facts: Vec<serde_json::Value>,
    #[serde(default)]
    report_patch: Option<TurnPatchJson>,
    #[serde(default)]
    redirect_message: Option<String>,
    #[serde(default)]
    clarification_questions: Vec<serde_json::Value>,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnPatchJson {
    #[serde(default)]
    summary_update: Option<String>,
}
