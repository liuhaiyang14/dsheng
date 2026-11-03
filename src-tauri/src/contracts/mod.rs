//! UI/CLI 共用命令 DTO 与 Tauri command 接线（主 Agent 所有权）。
//!
//! 命令名/参数/返回与 contracts/v1/cli-surface.json 及 read-views.schema.json
//! 逐一对齐；实现调 dsheng-core 服务层。Phase：壳骨架（仅 ping），
//! 数据/Provider 服务接线随集成轮 2 完成（用例 40）。

/// 连通性探测（无业务语义）。
#[tauri::command]
pub fn ping() -> serde_json::Value {
    serde_json::json!({
        "schema_version": "1",
        "ok": true,
        "data": { "app": "dsheng", "core_version": dsheng_core::APP_VERSION }
    })
}
