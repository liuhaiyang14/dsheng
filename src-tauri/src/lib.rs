//! dsheng 桌面壳（Tauri 2）。业务逻辑全部在 dsheng-core；
//! 本 crate 只做 IPC command 接线与窗口生命周期。

pub mod contracts;
pub mod export;

/// contracts：UI/CLI 共用命令 DTO（主 Agent 所有权；映射 dsheng-core 服务）。
/// export：导出实现（CLI/导出工作包所有权）。

pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            contracts::ping,
        ])
        .run(tauri::generate_context!())
        .expect("dsheng 桌面窗口启动失败");
}
