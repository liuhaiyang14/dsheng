//! 生成 attempt 取消（UI 面板取消按钮；与 CLI `generation cancel` 同一实现）。
//!
//! 语义不变：仅本地取消（attempt 置 cancelled 终态）；不保证远端停止计费；
//! 取消后迟到响应不提交（服务层终态门禁，见 store::generation）。

use super::{dispatch, envelope};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use serde_json::Value;

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
