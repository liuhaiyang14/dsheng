//! 诊断 command 实现（UI diagnosis* 方法；CLI diagnosis 组）。
//!
//! generate/continue 走服务层双门禁：preview*=无 flag 预览（本地，不派发），
//! confirm*=以 preview 记录的原始请求重建 + `--confirmation-id/--confirm-send`
//! → 服务层逐项重验快照后原子消费并派发（不新增任何 Provider 直连路径；
//! HttpDispatch 与 CLI 同一实现）。UI 请求证据键为 `evidence_ids`
//! （DiagnosisGenerateRequest，与 diagnosis-input 合同一致）；空证据按合同
//! 拒绝（INPUT_CONTRACT_INVALID）。

use super::{
    diagnosis_cmd, dispatch, dispatch_with_progress, envelope, export_doc_format_of, framework_of,
    opt_str, req_str, str_array,
};
use crate::contracts::env::{ConfirmationKind, DesktopEnv};
use dsheng_cli::cli as args;
use dsheng_cli::cmd::generation_cmd::{self, AnswerSource};
use dsheng_cli::error::CliResult;
use dsheng_cli::store::generation::ProgressSink;
use serde_json::Value;

/// 本地只读辅助：从指定分析快照取真实来源，不扩展冻结的指标读视图。
pub fn diagnosis_evidence(
    env: &DesktopEnv,
    workspace_id: String,
    run_id: String,
    revision_no: u32,
    metric_id: Option<String>,
) -> Value {
    envelope((|| {
        let db = env.db_arg()?;
        let conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let ids = dsheng_cli::store::analysis::diagnosis_evidence_ids(
            &conn,
            workspace.workspace_id,
            &run_id,
            revision_no,
            metric_id.as_deref(),
        )?;
        Ok(serde_json::json!(ids))
    })())
}

/// diagnosis generate（preview=Gate::Preview / confirm=Gate::Send 重建）。
/// `progress`：流式进度 sink（confirm 注入事件桥；preview 传 None）。
fn diagnosis_generate_call(
    env: &DesktopEnv,
    request: &Value,
    confirmation_id: Option<String>,
    confirm_send: bool,
    progress: Option<ProgressSink>,
) -> CliResult<Value> {
    let workspace = opt_str(request, "workspace_id");
    let run = req_str(request, "run_id")?;
    let framework = framework_of(&req_str(request, "framework")?)?;
    let problem = req_str(request, "problem")?;
    let focus_metric = opt_str(request, "focus_metric_id");
    // UI/合同键名 evidence_ids（复审轮 2 P1-1 修复；仅 UI 请求路径使用本函数）
    let evidence = str_array(request, "evidence_ids")?;
    let provider = opt_str(request, "provider_id");
    dispatch_with_progress(
        env,
        diagnosis_cmd(args::DiagnosisAction::Generate {
            workspace,
            run,
            framework,
            problem,
            focus_metric,
            evidence,
            provider,
            dry_run: false,
            confirmation_id,
            confirm_send,
        }),
        progress,
    )
}

/// 预览（无 flag）：本地组装 + confirmation 记录（TTL 600s）。
pub fn diagnosis_generate_preview(env: &DesktopEnv, request: Value) -> Value {
    let result = diagnosis_generate_call(env, &request, None, false, None);
    if let Ok(data) = &result {
        env.remember_confirmation(ConfirmationKind::DiagnosisGenerate, &request, data);
    }
    envelope(result)
}

/// 确认发送（双门禁）：原始请求取自本会话 preview 备忘。
pub fn diagnosis_generate_confirm(
    env: &DesktopEnv,
    confirmation_id: String,
    progress: Option<ProgressSink>,
) -> Value {
    envelope((|| {
        let request =
            env.lookup_confirmation(ConfirmationKind::DiagnosisGenerate, &confirmation_id)?;
        diagnosis_generate_call(env, &request, Some(confirmation_id), true, progress)
    })())
}

/// diagnosis continue（答复=supplement_fact 内联；与 CLI 同一服务实现）。
fn diagnosis_continue_call(
    env: &DesktopEnv,
    request: &Value,
    confirmation_id: Option<String>,
    confirm_send: bool,
    progress: Option<ProgressSink>,
) -> CliResult<Value> {
    let workspace = opt_str(request, "workspace_id");
    let session = req_str(request, "session_id")?;
    let answer = req_str(request, "supplement_fact")?;
    let provider = opt_str(request, "provider_id");
    let db = env.db_arg()?;
    generation_cmd::diagnosis_continue_value_with_progress(
        workspace,
        session,
        AnswerSource::Inline(answer),
        provider,
        false,
        confirmation_id,
        confirm_send,
        Some(&db),
        progress,
    )
}

/// 继续诊断预览。
pub fn diagnosis_continue_preview(env: &DesktopEnv, request: Value) -> Value {
    let result = diagnosis_continue_call(env, &request, None, false, None);
    if let Ok(data) = &result {
        env.remember_confirmation(ConfirmationKind::DiagnosisContinue, &request, data);
    }
    envelope(result)
}

/// 继续诊断确认发送。
pub fn diagnosis_continue_confirm(
    env: &DesktopEnv,
    confirmation_id: String,
    progress: Option<ProgressSink>,
) -> Value {
    envelope((|| {
        let request =
            env.lookup_confirmation(ConfirmationKind::DiagnosisContinue, &confirmation_id)?;
        diagnosis_continue_call(env, &request, Some(confirmation_id), true, progress)
    })())
}

/// dsheng diagnosis list。
pub fn diagnosis_list(env: &DesktopEnv, workspace_id: String, run_id: String) -> Value {
    envelope(dispatch(
        env,
        diagnosis_cmd(args::DiagnosisAction::List {
            workspace: Some(workspace_id),
            run: run_id,
        }),
    ))
}

/// dsheng diagnosis show（revision 缺省=当前）。
pub fn diagnosis_show(
    env: &DesktopEnv,
    workspace_id: String,
    report_id: String,
    revision: Option<u32>,
) -> Value {
    envelope(dispatch(
        env,
        diagnosis_cmd(args::DiagnosisAction::Show {
            workspace: Some(workspace_id),
            report: report_id,
            revision,
        }),
    ))
}

/// dsheng diagnosis export（md/json 组装走共享导出库，落盘到用户选择路径）。
pub fn diagnosis_export(
    env: &DesktopEnv,
    workspace_id: String,
    report_id: String,
    format: String,
    out_path: String,
) -> Value {
    envelope((|| {
        let format = export_doc_format_of(&format)?;
        dispatch(
            env,
            diagnosis_cmd(args::DiagnosisAction::Export {
                workspace: Some(workspace_id),
                report: report_id,
                revision: None,
                format,
                out: out_path,
            }),
        )
    })())
}
