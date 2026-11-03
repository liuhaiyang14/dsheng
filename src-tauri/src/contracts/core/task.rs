//! 任务 command 实现（UI task* 方法；CLI task 组）。
//!
//! generate 走服务层双门禁（同诊断：preview 备忘 → confirm 重建 + 双 flag，
//! 服务层重验快照后派发）；reconcile 两段式（dry_run 出 apply_token →
//! apply 携带 token）；task update 传完整 task-edit 输入（`--patch-json`
//! 通道，含幂等 mutation_id 与乐观并发 expected_task_revision）。

use super::{dispatch, envelope, json_input_file, opt_str, req_str, task_cmd, timeline_grain_of};
use crate::contracts::env::{ConfirmationKind, DesktopEnv};
use dsheng_cli::cli as args;
use dsheng_cli::error::CliResult;
use serde_json::Value;

/// task generate（preview/confirm 共用；flags 与 CLI 同形）。
fn task_generate_call(
    env: &DesktopEnv,
    request: &Value,
    confirmation_id: Option<String>,
    confirm_send: bool,
) -> CliResult<Value> {
    let workspace = opt_str(request, "workspace_id");
    let run = req_str(request, "run_id")?;
    let diagnosis_report = opt_str(request, "diagnosis_report_id");
    let provider = opt_str(request, "provider_id");
    dispatch(
        env,
        task_cmd(args::TaskAction::Generate {
            workspace,
            run,
            diagnosis_report,
            provider,
            dry_run: false,
            confirmation_id,
            confirm_send,
        }),
    )
}

/// 任务生成预览（本地组装 + confirmation 记录）。
pub fn task_generate_preview(env: &DesktopEnv, request: Value) -> Value {
    let result = task_generate_call(env, &request, None, false);
    if let Ok(data) = &result {
        env.remember_confirmation(ConfirmationKind::TaskGenerate, &request, data);
    }
    envelope(result)
}

/// 任务生成确认发送（双门禁）。
pub fn task_generate_confirm(env: &DesktopEnv, confirmation_id: String) -> Value {
    envelope((|| {
        let request = env.lookup_confirmation(ConfirmationKind::TaskGenerate, &confirmation_id)?;
        task_generate_call(env, &request, Some(confirmation_id), true)
    })())
}

/// task reconcile --dry-run（input = task-reconcile.schema 输入）。
pub fn task_reconcile_dry_run(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let run = req_str(&input, "run_id")?;
        let file = json_input_file(&input)?;
        dispatch(
            env,
            task_cmd(args::TaskAction::Reconcile {
                workspace,
                run,
                input_json: file.path().to_string_lossy().into_owned(),
                dry_run: true,
                apply: false,
            }),
        )
    })())
}

/// task reconcile --apply（apply_token 注入输入顶层；CLI 层校验与消费）。
pub fn task_reconcile_apply(env: &DesktopEnv, input: Value, apply_token: String) -> Value {
    envelope((|| {
        let mut input = input;
        if let Some(obj) = input.as_object_mut() {
            obj.insert("apply_token".into(), Value::String(apply_token));
        }
        let workspace = Some(req_str(&input, "workspace_id")?);
        let run = req_str(&input, "run_id")?;
        let file = json_input_file(&input)?;
        dispatch(
            env,
            task_cmd(args::TaskAction::Reconcile {
                workspace,
                run,
                input_json: file.path().to_string_lossy().into_owned(),
                dry_run: false,
                apply: true,
            }),
        )
    })())
}

/// dsheng task list（run_id 缺省=全 workspace）。
pub fn task_list(env: &DesktopEnv, workspace_id: String, run_id: Option<String>) -> Value {
    envelope(dispatch(
        env,
        task_cmd(args::TaskAction::List {
            workspace: Some(workspace_id),
            run: run_id,
        }),
    ))
}

/// dsheng task show。
pub fn task_show(env: &DesktopEnv, workspace_id: String, task_id: String) -> Value {
    envelope(dispatch(
        env,
        task_cmd(args::TaskAction::Show {
            workspace: Some(workspace_id),
            task: task_id,
        }),
    ))
}

/// dsheng task update --patch-json（完整 task-edit 输入；CLI 层交叉校验
/// workspace/task 一致后走 core TaskEditService：幂等＋乐观并发＋单事务）。
pub fn task_update(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let task = req_str(&input, "task_id")?;
        let file = json_input_file(&input)?;
        dispatch(
            env,
            task_cmd(args::TaskAction::Update {
                workspace,
                task,
                patch_json: Some(file.path().to_string_lossy().into_owned()),
                status: None,
                progress: None,
                progress_mode: None,
                adjustment_reason: None,
                blocked_reason: None,
                note: None,
                expected_revision: None,
                mutation_id: None,
            }),
        )
    })())
}

/// dsheng task timeline（query: workspace_id/from/to/grain）。
pub fn task_timeline(env: &DesktopEnv, query: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&query, "workspace_id")?);
        let from = req_str(&query, "from")?;
        let to = req_str(&query, "to")?;
        let grain = timeline_grain_of(&req_str(&query, "grain")?)?;
        dispatch(
            env,
            task_cmd(args::TaskAction::Timeline {
                workspace,
                from,
                to,
                grain,
            }),
        )
    })())
}
