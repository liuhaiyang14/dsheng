//! generation 命令组（cli-surface 23/24/30/31/32/33）——S3 门禁接线。
//!
//! S2 阶段：完成参数/workspace/Provider 解析与门禁 flag 组合校验后返回
//! 未接线错误（不触碰 Provider client）。

use crate::cli;
use crate::error::{CliError, CliResult};

pub fn dispatch(
    action: cli::GenerationAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let _ = action;
    let _ = db;
    Err(not_wired_yet("generation"))
}

pub fn task_generate(action: cli::TaskAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    let _ = action;
    let _ = db;
    Err(not_wired_yet("task generate"))
}

pub fn diagnosis_generation(
    action: cli::DiagnosisAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let _ = action;
    let _ = db;
    Err(not_wired_yet("diagnosis generate/continue"))
}

fn not_wired_yet(name: &str) -> CliError {
    CliError::invalid(format!(
        "命令 {name} 已解析但未接线（S3：确认门禁双门禁接线）"
    ))
}
