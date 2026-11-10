//! 首次创建企业档案后的桌面刷新回归：创建返回的 workspace 视图可用于
//! 后续 history_list，且空历史按约定返回空数组。

use dsheng_app::contracts::core as cmd;
use dsheng_app::contracts::env::DesktopEnv;
use serde_json::json;

#[test]
fn workspace_create_then_history_list_returns_empty_runs() {
    let dir = tempfile::tempdir().expect("临时目录");
    let db = dir.path().join("first-workspace.sqlite3");
    let env = DesktopEnv::with_db(&db);

    let created = cmd::settings::workspace_create(
        &env,
        json!({
            "schema_version": "1",
            "operation": "workspace_create",
            "name": "首次创建企业",
            "initial_period_policy": {
                "cadence": "month",
                "week_start_day": 1,
                "fiscal_year_start_month": 1,
                "timezone_id": "Asia/Shanghai",
                "effective_from": "2026-11-01"
            }
        }),
    );
    assert_eq!(created["ok"], true, "创建失败: {created}");
    let workspace_id = created["data"]["workspace"]["workspace_id"]
        .as_str()
        .expect("workspace_create 必须返回 workspace.workspace_id")
        .to_string();

    let history = cmd::history::history_list(&env, workspace_id);
    assert_eq!(history["ok"], true, "读取空历史失败: {history}");
    assert_eq!(history["data"]["runs"], json!([]));
}
