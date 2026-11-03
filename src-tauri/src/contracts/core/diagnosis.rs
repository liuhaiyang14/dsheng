//! 诊断 command 实现（UI diagnosis* 方法；CLI diagnosis 组）。
//!
//! generate/continue 走服务层双门禁：preview*=无 flag 预览（本地，不派发），
//! confirm*=以 preview 记录的原始请求重建 + `--confirmation-id/--confirm-send`
//! → 服务层逐项重验快照后原子消费并派发（不新增任何 Provider 直连路径；
//! HttpDispatch 与 CLI 同一实现）。UI 请求缺 evidence 字段——服务层按合同
//! 拒绝空证据（INPUT_CONTRACT_INVALID；合同缺口见工作包报告）。

use super::{
    diagnosis_cmd, dispatch, envelope, export_doc_format_of, framework_of, opt_str, req_str,
    str_array,
};
use crate::contracts::env::{ConfirmationKind, DesktopEnv};
use dsheng_cli::cli as args;
use dsheng_cli::cmd::generation_cmd::{self, AnswerSource};
use dsheng_cli::error::CliResult;
use serde_json::Value;

/// diagnosis generate（preview=Gate::Preview / confirm=Gate::Send 重建）。
fn diagnosis_generate_call(
    env: &DesktopEnv,
    request: &Value,
    confirmation_id: Option<String>,
    confirm_send: bool,
) -> CliResult<Value> {
    let workspace = opt_str(request, "workspace_id");
    let run = req_str(request, "run_id")?;
    let framework = framework_of(&req_str(request, "framework")?)?;
    let problem = req_str(request, "problem")?;
    let focus_metric = opt_str(request, "focus_metric_id");
    let evidence = str_array(request, "evidence")?;
    let provider = opt_str(request, "provider_id");
    dispatch(
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
    )
}

/// 预览（无 flag）：本地组装 + confirmation 记录（TTL 600s）。
pub fn diagnosis_generate_preview(env: &DesktopEnv, request: Value) -> Value {
    let result = diagnosis_generate_call(env, &request, None, false);
    if let Ok(data) = &result {
        env.remember_confirmation(ConfirmationKind::DiagnosisGenerate, &request, data);
    }
    envelope(result)
}

/// 确认发送（双门禁）：原始请求取自本会话 preview 备忘。
pub fn diagnosis_generate_confirm(env: &DesktopEnv, confirmation_id: String) -> Value {
    envelope((|| {
        let request =
            env.lookup_confirmation(ConfirmationKind::DiagnosisGenerate, &confirmation_id)?;
        diagnosis_generate_call(env, &request, Some(confirmation_id), true)
    })())
}

/// diagnosis continue（答复=supplement_fact 内联；与 CLI 同一服务实现）。
fn diagnosis_continue_call(
    env: &DesktopEnv,
    request: &Value,
    confirmation_id: Option<String>,
    confirm_send: bool,
) -> CliResult<Value> {
    let workspace = opt_str(request, "workspace_id");
    let session = req_str(request, "session_id")?;
    let answer = req_str(request, "supplement_fact")?;
    let provider = opt_str(request, "provider_id");
    let db = env.db_arg()?;
    generation_cmd::diagnosis_continue_value(
        workspace,
        session,
        AnswerSource::Inline(answer),
        provider,
        false,
        confirmation_id,
        confirm_send,
        Some(&db),
    )
}

/// 继续诊断预览。
pub fn diagnosis_continue_preview(env: &DesktopEnv, request: Value) -> Value {
    let result = diagnosis_continue_call(env, &request, None, false);
    if let Ok(data) = &result {
        env.remember_confirmation(ConfirmationKind::DiagnosisContinue, &request, data);
    }
    envelope(result)
}

/// 继续诊断确认发送。
pub fn diagnosis_continue_confirm(env: &DesktopEnv, confirmation_id: String) -> Value {
    envelope((|| {
        let request =
            env.lookup_confirmation(ConfirmationKind::DiagnosisContinue, &confirmation_id)?;
        diagnosis_continue_call(env, &request, Some(confirmation_id), true)
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
