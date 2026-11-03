//! CLI 上下文：数据库定位/打开（版本协商+迁移）、workspace 解析、
//! Provider 解析（cli-surface command_defaults）。
//!
//! - DB 路径：`--db` > `DSHENG_DB` > 平台数据目录（不写入导出/日志）。
//! - workspace：显式 `--workspace` > 已确认 `app_preferences.active_workspace_id`；
//!   两者皆无 → `WORKSPACE_REQUIRED`；绝不从对象 ID 推断。
//! - Provider：显式 `--provider` > workspace `active_provider_id`（生成命令用）。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::repository::migration::{open_with_negotiation, DatabasePaths};
use rusqlite::Connection;
use std::path::PathBuf;
use uuid::Uuid;

/// 平台默认数据目录（不包含本机绝对路径的输出物，仅本地定位数据库）。
pub fn default_db_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("DSHENG_DB") {
        return PathBuf::from(explicit);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/dsheng")
    } else if cfg!(target_os = "windows") {
        PathBuf::from(home).join("AppData/Local/dsheng")
    } else {
        PathBuf::from(home).join(".local/share/dsheng")
    };
    dir.join("dsheng.sqlite3")
}

/// 打开数据库：`--db` 显式 > `DSHENG_DB` > 平台数据目录。
pub fn open_db(explicit: Option<&String>) -> CliResult<Connection> {
    let path = explicit.map(PathBuf::from).unwrap_or_else(default_db_path);
    open_db_at(&path)
}

/// 在显式路径打开数据库（CLI `--db` 与桌面壳共用；目录创建 → 版本协商 →
/// 首启迁移（空库）/已建库校验）。桌面壳据此与 CLI 共享同一实现与错误映射。
pub fn open_db_at(path: &std::path::Path) -> CliResult<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CliError::new(
                CliErrorCode::DataDirectoryUnavailable,
                format!("无法创建数据目录: {e}"),
            )
        })?;
    }
    let paths = DatabasePaths::new(path);
    let conn = open_with_negotiation(
        &paths,
        dsheng_core::repository::migration::current_support(),
    )
    .map_err(CliError::from)?;
    let version =
        dsheng_core::repository::migration::read_schema_version(&conn).map_err(CliError::from)?;
    if version.is_none() {
        // 首启建库：最小应用器按序执行冻结 migration（完整协议在桌面端首启）。
        dsheng_core::repository::apply_migrations(&conn, dsheng_core::APP_VERSION)
            .map_err(|e| CliError::new(CliErrorCode::DbMigrationFailed, e.to_string()))?;
    }
    Ok(conn)
}

/// workspace 解析结果。
pub struct WorkspaceCtx {
    pub workspace_id: Uuid,
}

/// 读 app_preferences.active_workspace_id（单行表）。
pub fn active_workspace(conn: &Connection) -> CliResult<Option<Uuid>> {
    let found: Option<String> = conn
        .query_row(
            "SELECT active_workspace_id FROM app_preferences WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    match found {
        None => Ok(None),
        Some(raw) => Uuid::parse_str(&raw)
            .map(Some)
            .map_err(|_| CliError::new(CliErrorCode::ContextInvalid, "active_workspace_id 非法")),
    }
}

/// 解析 workspace：显式 flag 必须存在；否则用已确认偏好；否则 WORKSPACE_REQUIRED。
pub fn resolve_workspace(conn: &Connection, explicit: Option<&String>) -> CliResult<WorkspaceCtx> {
    let id = if let Some(raw) = explicit {
        let parsed = Uuid::parse_str(raw)
            .map_err(|_| CliError::invalid("workspace ID 必须是 UUID").with_field("--workspace"))?;
        ensure_workspace_exists(conn, parsed)?;
        parsed
    } else {
        let preferred = active_workspace(conn)?.ok_or_else(|| {
            CliError::new(
                CliErrorCode::WorkspaceRequired,
                "未指定 --workspace 且无已确认的活跃 workspace",
            )
            .with_field("--workspace")
        })?;
        ensure_workspace_exists(conn, preferred)?;
        preferred
    };
    Ok(WorkspaceCtx { workspace_id: id })
}

/// `workspace use` 等命令写入偏好。
pub fn set_active_workspace(conn: &Connection, workspace_id: Uuid) -> CliResult<()> {
    conn.execute(
        "INSERT INTO app_preferences (id, active_workspace_id) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET active_workspace_id = excluded.active_workspace_id",
        rusqlite::params![workspace_id.to_string()],
    )
    .map_err(CliError::from)?;
    Ok(())
}

fn ensure_workspace_exists(conn: &Connection, id: Uuid) -> CliResult<()> {
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM workspaces WHERE workspace_id = ?1",
            rusqlite::params![id.to_string()],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    if !exists {
        return Err(
            CliError::new(CliErrorCode::WorkspaceMismatch, "workspace 不存在")
                .with_field("--workspace"),
        );
    }
    Ok(())
}

/// 对象归属校验：表中对象的 workspace 必须与所选一致，否则 WORKSPACE_MISMATCH。
/// 表不存在该对象时返回 `None`（调用方按 not found 处理）。
pub fn object_workspace(
    conn: &Connection,
    table: &str,
    id_column: &str,
    object_id: &str,
) -> CliResult<Option<Uuid>> {
    let sql = format!("SELECT workspace_id FROM {table} WHERE {id_column} = ?1");
    let found: Option<String> = conn
        .query_row(sql.as_str(), rusqlite::params![object_id], |r| r.get(0))
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    match found {
        None => Ok(None),
        Some(raw) => Uuid::parse_str(&raw)
            .map(Some)
            .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "workspace_id 非法")),
    }
}

/// 校验对象属于 workspace；不存在/跨域分别报错。
pub fn ensure_object_in_workspace(
    conn: &Connection,
    table: &str,
    id_column: &str,
    object_id: &str,
    workspace_id: Uuid,
    object_label: &str,
) -> CliResult<()> {
    match object_workspace(conn, table, id_column, object_id)? {
        None => Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            format!("{object_label} 不存在"),
        )),
        Some(owner) if owner != workspace_id => Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            format!("{object_label} 不属于所选 workspace"),
        )),
        Some(_) => Ok(()),
    }
}

/// 解析 UUID flag。
pub(crate) fn uuid_arg(value: &str, flag: &str) -> CliResult<Uuid> {
    Uuid::parse_str(value)
        .map_err(|_| CliError::invalid(format!("{flag} 必须是 UUID")).with_field(flag))
}
