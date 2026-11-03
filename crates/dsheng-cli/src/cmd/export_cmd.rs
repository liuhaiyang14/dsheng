//! export / template 命令（cli-surface 48–52）。S4 接线。

use crate::cli;
use crate::error::{CliError, CliResult};

pub fn run(args: cli::ExportArgs, _db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = args;
    Err(CliError::invalid(
        "命令 export 已解析但未接线（本阶段交付参数解析）",
    ))
}

pub fn template(action: cli::TemplateAction, _db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    Err(CliError::invalid(
        "命令 template download 已解析但未接线（本阶段交付参数解析）",
    ))
}
