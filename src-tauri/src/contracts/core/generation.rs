//! 生成 attempt 取消（UI 面板取消按钮；与 CLI `generation cancel` 同一实现）。
//!
//! 语义不变：仅本地取消（attempt 置 cancelled 终态）；不保证远端停止计费；
//! 取消后迟到响应不提交（服务层终态门禁，见 store::generation）。

use super::{dispatch, envelope};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use serde_json::Value;

/// 在首次进度到达前也能停止；与确认消费以 SQLite 事务协调。
pub fn generation_cancel_by_confirmation(
    env: &DesktopEnv,
    workspace_id: String,
    confirmation_id: String,
) -> Value {
    envelope((|| {
        let db = env.db_arg()?;
        let mut conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let confirmation = uuid::Uuid::parse_str(&confirmation_id)
            .map_err(|_| dsheng_cli::error::CliError::invalid("确认编号必须是 UUID"))?;
        dsheng_cli::store::generation::cancel_by_confirmation(
            &mut conn,
            workspace.workspace_id,
            confirmation,
        )
    })())
}

/// dsheng generation cancel（attempt 须非终态）。
pub fn generation_cancel(env: &DesktopEnv, workspace_id: String, attempt_id: String) -> Value {
    envelope(dispatch(
        env,
        args::Command::Generation {
            action: args::GenerationAction::Cancel {
                workspace: Some(workspace_id),
                attempt: attempt_id,
            },
        },
    ))
}
