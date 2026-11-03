//! task 命令组（cli-surface 30/34–42）。S2/S3 接线。

use crate::cli;
use crate::error::{CliError, CliResult};

pub fn dispatch(action: cli::TaskAction, _db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    Err(CliError::invalid(
        "命令 task 已解析但未接线（本阶段交付参数解析与 workspace 解析）",
    ))
}
