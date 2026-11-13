//! 命令分发：把 clap 解析结果路由到各命令组模块。
//!
//! 每个处理器：打开数据库（--db/默认）→ workspace 解析（显式>偏好>WORKSPACE_REQUIRED）
//! → 调用 core 服务/CLI 持久化层 → 返回 envelope data。

pub mod data_cmd;
pub mod diagnosis_cmd;
pub mod export_cmd;
pub mod generation_cmd;
pub mod history_cmd;
pub mod settings_cmd;
pub mod task_cmd;

use crate::cli;
use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;

pub fn dispatch(command: cli::Command, db: Option<&String>) -> CliResult<serde_json::Value> {
    dispatch_with_progress(command, db, None)
}

/// 带进度 sink 的派发（生成类命令流式进度消费端注入；其余命令忽略）。
/// None＝生成发送路径默认 CLI stderr 节流 sink。
pub fn dispatch_with_progress(
    command: cli::Command,
    db: Option<&String>,
    progress: Option<crate::store::generation::ProgressSink>,
) -> CliResult<serde_json::Value> {
    match command {
        cli::Command::Workspace { action } => settings_cmd::workspace(action, db),
        cli::Command::Provider { action } => settings_cmd::provider(action, db),
        cli::Command::Scan {
            workspace,
            dir,
            source_profile,
            generic,
        } => data_cmd::scan(workspace, dir, source_profile, generic, db),
        cli::Command::Mapping { action } => data_cmd::mapping(action, db),
        cli::Command::ManualInput { action } => data_cmd::manual_input(action, db),
        cli::Command::Metric { action } => data_cmd::metric(action, db),
        cli::Command::SourceConflict { action } => data_cmd::source_conflict(action, db),
        cli::Command::Analyze {
            workspace,
            dir,
            from,
            to,
            grain,
            policy,
            recalculate,
            input_json,
            source_profile,
            generic,
        } => data_cmd::analyze(
            workspace,
            dir,
            from,
            to,
            grain,
            policy,
            recalculate,
            input_json,
            source_profile,
            generic,
            db,
        ),
        cli::Command::Analysis { action } => data_cmd::analysis(action, db),
        cli::Command::Diagnosis { action } => {
            diagnosis_cmd::dispatch_with_progress(action, db, progress)
        }
        cli::Command::Generation { action } => generation_cmd::dispatch(action, db),
        cli::Command::Task { action } => task_cmd::dispatch_with_progress(action, db, progress),
        cli::Command::History { action } => history_cmd::dispatch(action, db),
        cli::Command::Export(args) => export_cmd::run(args, db),
        cli::Command::Template { action } => export_cmd::template(action, db),
    }
}

/// 双门禁 flag 组合解析：无 flag / --dry-run / (--confirmation-id + --confirm-send)。
/// 其余组合一律 INVALID_ARGUMENT。（S3 生成命令接线使用。）
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Gate {
    /// 本地预览（不派发请求）。
    Preview,
    /// 已具备 confirmation_id + confirm_send。
    Send { confirmation_id: uuid::Uuid },
}

#[allow(dead_code)]
pub(crate) fn parse_gate(
    dry_run: bool,
    confirmation_id: &Option<String>,
    confirm_send: bool,
) -> CliResult<Gate> {
    let has_id = confirmation_id.is_some();
    match (dry_run, has_id, confirm_send) {
        (false, false, false) => Ok(Gate::Preview),
        (true, false, false) => Ok(Gate::Preview),
        (false, true, true) => {
            let raw = confirmation_id.as_deref().expect("checked");
            let id = uuid::Uuid::parse_str(raw).map_err(|_| {
                CliError::invalid("--confirmation-id 必须是 UUID").with_field("--confirmation-id")
            })?;
            Ok(Gate::Send {
                confirmation_id: id,
            })
        }
        (true, true, _) => Err(CliError::invalid(
            "--dry-run 与 --confirmation-id/--confirm-send 互斥",
        )),
        (_, false, true) => Err(CliError::invalid(
            "--confirm-send 必须与 --confirmation-id 同时提供（双门禁）",
        )
        .with_field("--confirm-send")),
        (false, true, false) => Err(CliError::invalid(
            "--confirmation-id 必须与 --confirm-send 同时提供（双门禁）",
        )
        .with_field("--confirm-send")),
    }
}

/// 生成命令解析（--provider 显式 > workspace active_provider_id）。
#[allow(dead_code)]
pub(crate) fn resolve_provider(
    conn: &rusqlite::Connection,
    workspace_id: uuid::Uuid,
    explicit: &Option<String>,
) -> CliResult<uuid::Uuid> {
    if let Some(raw) = explicit {
        let id = crate::ctx::uuid_arg(raw, "--provider")?;
        crate::ctx::ensure_object_in_workspace(
            conn,
            "providers",
            "provider_id",
            &id.to_string(),
            workspace_id,
            "provider",
        )?;
        return Ok(id);
    }
    let active: Option<String> = conn
        .query_row(
            "SELECT active_provider_id FROM workspaces WHERE workspace_id = ?1",
            rusqlite::params![workspace_id.to_string()],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    match active {
        Some(raw) => uuid::Uuid::parse_str(&raw)
            .map_err(|_| CliError::new(CliErrorCode::ContextInvalid, "active_provider_id 非法")),
        None => Err(CliError::new(
            CliErrorCode::ProviderRequired,
            "未指定 --provider 且 workspace 无默认 Provider",
        )
        .with_field("--provider")),
    }
}
