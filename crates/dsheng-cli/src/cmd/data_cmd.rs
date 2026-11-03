//! scan / mapping / manual-input / metric / source-conflict / analyze / analysis show
//! （cli-surface 12–22）。S2 接线。

use crate::cli;
use crate::error::{CliError, CliResult};

pub fn scan(
    workspace: Option<String>,
    dir: String,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    not_wired("scan", workspace, dir, db)
}

pub fn mapping(action: cli::MappingAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    not_wired("mapping", None, String::new(), db)
}

pub fn manual_input(
    action: cli::ManualInputAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let _ = action;
    not_wired("manual-input", None, String::new(), db)
}

pub fn metric(action: cli::MetricAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    not_wired("metric", None, String::new(), db)
}

pub fn source_conflict(
    action: cli::SourceConflictAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let _ = action;
    not_wired("source-conflict", None, String::new(), db)
}

#[allow(clippy::too_many_arguments)]
#[allow(unused_variables)]
pub fn analyze(
    workspace: Option<String>,
    dir: String,
    from: String,
    to: String,
    grain: cli::AnalyzeGrain,
    policy: Option<String>,
    recalculate: bool,
    input_json: Option<String>,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    not_wired("analyze", workspace, dir, db)
}

pub fn analysis(action: cli::AnalysisAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    not_wired("analysis", None, String::new(), db)
}

fn not_wired(
    name: &str,
    workspace: Option<String>,
    dir: String,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    // S1：先完成 workspace 解析规则（显式 --workspace 或偏好；否则 WORKSPACE_REQUIRED），
    // 便于阶段测试与后续接线。
    let conn = crate::ctx::open_db(db)?;
    crate::ctx::resolve_workspace(&conn, workspace.as_ref())?;
    let _ = dir;
    Err(CliError::invalid(format!(
        "命令 {name} 已解析但未接线（本阶段交付参数解析与 workspace 解析）"
    )))
}
