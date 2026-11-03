//! 历史 command 实现（UI historyList/historyShow；CLI history list/show）。

use super::{dispatch, envelope, history_cmd};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use serde_json::Value;

/// dsheng history list → data `{runs:[...]}`。
pub fn history_list(env: &DesktopEnv, workspace_id: String) -> Value {
    envelope(dispatch(
        env,
        history_cmd(args::HistoryAction::List {
            workspace: Some(workspace_id),
        }),
    ))
}

/// dsheng history show（revision 缺省=当前）。
pub fn history_show(
    env: &DesktopEnv,
    workspace_id: String,
    run_id: String,
    revision: Option<u32>,
) -> Value {
    envelope(dispatch(
        env,
        history_cmd(args::HistoryAction::Show {
            workspace: Some(workspace_id),
            run: run_id,
            revision,
        }),
    ))
}
