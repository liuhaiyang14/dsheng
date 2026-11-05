//! Tauri command 声明（薄封装：State 取环境 → core::* 实现）。
//!
//! 命名/参数与 src-ui/src/api/tauriClient.ts 逐一对齐（见 registry::COMMANDS；
//! Rust snake_case 参数 ↔ UI camelCase 键）。全部 `#[tauri::command(async)]`：
//! 阻塞的 SQLite/生成发送在 async runtime 线程执行，不冻结主线程/WebView。
//! 返回值为 CliEnvelope JSON（成功/失败均以 envelope 表达，绝不 reject
//! invoke promise——UI unwrapEnvelope 按 ok 字段解包或抛 DshengCliError）。

use crate::contracts::core;
use crate::contracts::env::DesktopEnv;
use serde_json::Value;

/// 生成服务拥有同步 SQLite 连接及独立网络 runtime，须离开 Tauri async
/// executor 执行；停止命令仍可并行使用另一个短连接更新同一 attempt。
async fn run_generation_blocking(work: impl FnOnce() -> Value + Send + 'static) -> Value {
    match tauri::async_runtime::spawn_blocking(work).await {
        Ok(value) => value,
        Err(_) => core::envelope(Err(dsheng_cli::error::CliError::new(
            dsheng_core::contracts::cli::CliErrorCode::LocalSaveFailed,
            "生成工作线程意外中断，请查看本地错误日志",
        ))),
    }
}

// ---- workspace / period ----

#[tauri::command(async)]
pub fn workspace_list(state: tauri::State<'_, DesktopEnv>) -> Value {
    core::settings::workspace_list(&state)
}

#[tauri::command(async)]
pub fn workspace_create(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::workspace_create(&state, input)
}

#[tauri::command(async)]
pub fn workspace_update(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::workspace_update(&state, input)
}

#[tauri::command(async)]
pub fn workspace_use(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::workspace_use(&state, input)
}

#[tauri::command(async)]
pub fn period_list(state: tauri::State<'_, DesktopEnv>, workspace_id: String) -> Value {
    core::settings::period_list(&state, workspace_id)
}

#[tauri::command(async)]
pub fn period_set(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::period_set(&state, input)
}

// ---- provider ----

#[tauri::command(async)]
pub fn provider_list(state: tauri::State<'_, DesktopEnv>, workspace_id: String) -> Value {
    core::settings::provider_list(&state, workspace_id)
}

/// API Key 单独通道（UI 不经 input JSON 传 Key）。
#[tauri::command(async)]
pub fn provider_set(
    state: tauri::State<'_, DesktopEnv>,
    input: Value,
    api_key: Option<String>,
) -> Value {
    core::settings::provider_set(&state, input, api_key)
}

#[tauri::command(async)]
pub fn provider_use(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::provider_use(&state, input)
}

#[tauri::command(async)]
pub fn provider_clear_key(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::provider_clear_key(&state, input)
}

#[tauri::command(async)]
pub fn provider_delete(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::settings::provider_delete(&state, input)
}

// ---- 数据源 / 经营分析 ----

#[tauri::command(async)]
pub fn scan(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    directory_label: String,
) -> Value {
    core::data::scan(&state, workspace_id, directory_label)
}

#[tauri::command(async)]
pub fn mapping_show(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    run_id: String,
) -> Value {
    core::data::mapping_show(&state, workspace_id, run_id)
}

#[tauri::command(async)]
pub fn analyze(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::data::analyze(&state, input)
}

// ---- 诊断 ----

#[tauri::command(async)]
pub fn diagnosis_evidence(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    run_id: String,
    revision_no: u32,
    metric_id: Option<String>,
) -> Value {
    core::diagnosis::diagnosis_evidence(&state, workspace_id, run_id, revision_no, metric_id)
}

#[tauri::command(async)]
pub fn diagnosis_generate_preview(state: tauri::State<'_, DesktopEnv>, request: Value) -> Value {
    core::diagnosis::diagnosis_generate_preview(&state, request)
}

#[tauri::command]
pub async fn diagnosis_generate_confirm(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopEnv>,
    confirmation_id: String,
) -> Result<Value, ()> {
    let env = state.inner().clone();
    Ok(run_generation_blocking(move || {
        core::diagnosis::diagnosis_generate_confirm(
            &env,
            confirmation_id,
            Some(crate::contracts::progress::emitting_sink(&app)),
        )
    })
    .await)
}

#[tauri::command(async)]
pub fn diagnosis_continue_preview(state: tauri::State<'_, DesktopEnv>, request: Value) -> Value {
    core::diagnosis::diagnosis_continue_preview(&state, request)
}

#[tauri::command]
pub async fn diagnosis_continue_confirm(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopEnv>,
    confirmation_id: String,
) -> Result<Value, ()> {
    let env = state.inner().clone();
    Ok(run_generation_blocking(move || {
        core::diagnosis::diagnosis_continue_confirm(
            &env,
            confirmation_id,
            Some(crate::contracts::progress::emitting_sink(&app)),
        )
    })
    .await)
}

#[tauri::command(async)]
pub fn diagnosis_list(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    run_id: String,
) -> Value {
    core::diagnosis::diagnosis_list(&state, workspace_id, run_id)
}

#[tauri::command(async)]
pub fn diagnosis_show(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    report_id: String,
    revision: Option<u32>,
) -> Value {
    core::diagnosis::diagnosis_show(&state, workspace_id, report_id, revision)
}

#[tauri::command(async)]
pub fn diagnosis_export(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    report_id: String,
    format: String,
    out_path: String,
) -> Value {
    core::diagnosis::diagnosis_export(&state, workspace_id, report_id, format, out_path)
}

// ---- 生成（进度事件经 generation://progress；取消走既有 generation cancel） ----

#[tauri::command(async)]
pub fn generation_cancel_by_confirmation(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    confirmation_id: String,
) -> Value {
    core::generation::generation_cancel_by_confirmation(&state, workspace_id, confirmation_id)
}

#[tauri::command(async)]
pub fn generation_cancel(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    attempt_id: String,
) -> Value {
    core::generation::generation_cancel(&state, workspace_id, attempt_id)
}

// ---- 任务生成 / 合并 ----

#[tauri::command(async)]
pub fn task_generate_preview(state: tauri::State<'_, DesktopEnv>, request: Value) -> Value {
    core::task::task_generate_preview(&state, request)
}

#[tauri::command]
pub async fn task_generate_confirm(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopEnv>,
    confirmation_id: String,
) -> Result<Value, ()> {
    let env = state.inner().clone();
    Ok(run_generation_blocking(move || {
        core::task::task_generate_confirm(
            &env,
            confirmation_id,
            Some(crate::contracts::progress::emitting_sink(&app)),
        )
    })
    .await)
}

#[tauri::command(async)]
pub fn task_reconcile_dry_run(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::task::task_reconcile_dry_run(&state, input)
}

#[tauri::command(async)]
pub fn task_reconcile_apply(
    state: tauri::State<'_, DesktopEnv>,
    input: Value,
    apply_token: String,
) -> Value {
    core::task::task_reconcile_apply(&state, input, apply_token)
}

// ---- 任务读写 / 时间线 ----

#[tauri::command(async)]
pub fn task_list(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    run_id: Option<String>,
) -> Value {
    core::task::task_list(&state, workspace_id, run_id)
}

#[tauri::command(async)]
pub fn task_show(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    task_id: String,
) -> Value {
    core::task::task_show(&state, workspace_id, task_id)
}

#[tauri::command(async)]
pub fn task_update(state: tauri::State<'_, DesktopEnv>, input: Value) -> Value {
    core::task::task_update(&state, input)
}

#[tauri::command(async)]
pub fn task_timeline(state: tauri::State<'_, DesktopEnv>, query: Value) -> Value {
    core::task::task_timeline(&state, query)
}

// ---- 历史 ----

#[tauri::command(async)]
pub fn history_list(state: tauri::State<'_, DesktopEnv>, workspace_id: String) -> Value {
    core::history::history_list(&state, workspace_id)
}

#[tauri::command(async)]
pub fn history_show(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
    run_id: String,
    revision: Option<u32>,
) -> Value {
    core::history::history_show(&state, workspace_id, run_id, revision)
}

#[tauri::command(async)]
pub fn template_download(state: tauri::State<'_, DesktopEnv>, out_path: String) -> Value {
    core::data::template_download(&state, out_path)
}

#[tauri::command(async)]
pub fn metric_register_defaults(
    state: tauri::State<'_, DesktopEnv>,
    workspace_id: String,
) -> Value {
    core::metric_defaults::register_defaults(&state, workspace_id)
}

#[cfg(test)]
mod generation_execution_tests {
    #[test]
    fn synchronous_generation_can_enter_runtime_off_the_async_executor() {
        let value = tauri::async_runtime::block_on(async {
            super::run_generation_blocking(|| {
                tauri::async_runtime::block_on(async { serde_json::json!({"ok": true}) })
            })
            .await
        });
        assert_eq!(value, serde_json::json!({"ok": true}));
    }
}
