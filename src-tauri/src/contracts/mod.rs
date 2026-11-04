//! UI/CLI 共用命令接线（集成轮 2：用例 40——桌面 UI 与 CLI 共享同一 SQLite）。
//!
//! 分层：
//! - [`paths`]：S1 桌面数据目录解析（`com.dsheng.desktop`；系统 API，不依赖 cwd）。
//! - [`env`]：执行环境（库路径解析结果 + 生成确认备忘；tauri State 管理）。
//! - [`core`]：32 条 command 的进程内实现（直调 dsheng-cli 服务层，测试可
//!   不经 tauri 运行时调用）。
//! - [`commands`]：#[tauri::command] 薄封装（参数名 ↔ UI camelCase 键）。
//! - [`registry`]：S5 联调冒烟基准（COMMANDS 清单与 tauriClient.ts 比对）。
//!
//! 服务层=dsheng-cli lib（cmd::dispatch / store::** / ctx::open_db_at）：
//! 桌面与 CLI 复用同一实现，本壳不写业务逻辑；返回 CliEnvelope JSON 原样。

pub mod commands;
pub mod core;
pub mod env;
pub mod paths;
pub mod progress;
pub mod registry;

pub use registry::COMMANDS;

/// 连通性探测（无业务语义）。
#[tauri::command]
pub fn ping() -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1",
        "ok": true,
        "data": { "app": "dsheng", "core_version": dsheng_core::APP_VERSION }
    })
}

/// JS 飞行记录仪：webview 的 error/unhandledrejection 落入同一日志（排障，无敏感数据）。
#[tauri::command(async)]
pub fn js_log(message: String) {
    use std::io::Write as _;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/dsheng-cmd.log") {
        let _ = writeln!(f, "[js] {message}");
    }
}
