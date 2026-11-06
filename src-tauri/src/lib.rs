//! dsheng 桌面壳（Tauri 2）。业务逻辑全部在 dsheng-core/dsheng-cli 服务层；
//! 本 crate 只做 IPC command 接线与窗口生命周期（用例 40：桌面与 CLI
//! 共享同一 SQLite——库路径见 contracts::paths）。

pub mod contracts;
pub mod export;

use contracts::env::DesktopEnv;

/// 桌面启动：注册 UI DshengClient 全部 command（名/参数对齐
/// src-ui/src/api/tauriClient.ts；清单见 contracts::COMMANDS）。
/// 飞行记录仪：GUI 侧命令异常/panic 落 /tmp/dsheng-cmd.log（排障用，无敏感数据）。
fn install_flight_recorder() {
    use std::io::Write as _;
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/dsheng-cmd.log")
            .ok();
        if let Some(f) = f.as_mut() {
            let _ = writeln!(f, "[panic] {info}\n[backtrace]\n{}", std::backtrace::Backtrace::force_capture());
        }
        default(info);
    }));
}

pub fn run() {
    install_flight_recorder();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DesktopEnv::resolve())
        .invoke_handler(tauri::generate_handler![
            contracts::ping,
            contracts::js_log,
            contracts::commands::template_download,
            contracts::commands::metric_register_defaults,
            contracts::commands::workspace_list,
            contracts::commands::workspace_create,
            contracts::commands::workspace_update,
            contracts::commands::workspace_use,
            contracts::commands::period_list,
            contracts::commands::period_set,
            contracts::commands::provider_list,
            contracts::commands::provider_set,
            contracts::commands::provider_use,
            contracts::commands::provider_clear_key,
            contracts::commands::provider_delete,
            contracts::commands::scan,
            contracts::commands::mapping_show,
            contracts::commands::analyze,
            contracts::commands::diagnosis_evidence,
            contracts::commands::diagnosis_generate_preview,
            contracts::commands::diagnosis_generate_confirm,
            contracts::commands::generation_cancel,
            contracts::commands::generation_cancel_by_confirmation,
            contracts::commands::diagnosis_continue_preview,
            contracts::commands::diagnosis_continue_confirm,
            contracts::commands::diagnosis_list,
            contracts::commands::diagnosis_show,
            contracts::commands::diagnosis_export,
            contracts::commands::task_generate_preview,
            contracts::commands::task_generate_confirm,
            contracts::commands::task_candidate_latest,
            contracts::commands::task_reconcile_dry_run,
            contracts::commands::task_reconcile_apply,
            contracts::commands::task_create,
            contracts::commands::task_list,
            contracts::commands::task_show,
            contracts::commands::task_update,
            contracts::commands::task_timeline,
            contracts::commands::history_list,
            contracts::commands::history_show,
        ])
        .run(tauri::generate_context!())
        .expect("dsheng 桌面窗口启动失败");
}
