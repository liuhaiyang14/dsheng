//! 设置持久化服务（CLI 所有权）：workspace / period policy / provider 的
//! SQLite 读写与公开视图。dsheng-core 冻结了类型与校验
//! （`SettingsOperation`、`validate_base_url`、`validate_budget_timeouts`、
//! `PeriodPolicyInputDefaults`），本模块只做事务性持久化与视图投影；
//! API Key 仅落 providers.api_key（0001 冻结决策），任何视图不投影。

use crate::error::{CliError, CliResult};
use chrono::Utc;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::domain::Cadence;
use dsheng_core::contracts::settings::{
    validate_base_url, validate_budget_timeouts, KeyAction, PeriodPolicy,
    PeriodPolicyInputDefaults, ProviderPublic, SettingsOperation,
};
use rusqlite::{params, Connection};
use serde_json::json;
use uuid::Uuid;

fn now_iso() -> String {
    dsheng_core::repository::utc_iso(Utc::now())
}

// ---- workspace 视图与操作 ----

pub fn workspace_view(conn: &Connection, id: Uuid) -> CliResult<serde_json::Value> {
    let row = conn
        .query_row(
            "SELECT workspace_id, name, config_revision, active_provider_id,
                    current_period_policy_id, created_at, updated_at
             FROM workspaces WHERE workspace_id = ?1",
            params![id.to_string()],
            map_workspace_row,
        )
        .map_err(|_| {
            CliError::new(CliErrorCode::WorkspaceMismatch, "workspace 不存在")
                .with_field("--workspace")
        })?;
    Ok(row)
}

fn map_workspace_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    let active: Option<String> = r.get(3)?;
    Ok(json!({
        "workspace_id": r.get::<_, String>(0)?,
        "name": r.get::<_, String>(1)?,
        "config_revision": r.get::<_, i64>(2)?,
        "active_provider_id": active,
        "current_period_policy_id": r.get::<_, String>(4)?,
        "created_at": r.get::<_, String>(5)?,
        "updated_at": r.get::<_, String>(6)?,
    }))
}

pub fn list_workspaces(conn: &Connection) -> CliResult<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT workspace_id, name, config_revision, active_provider_id,
                current_period_policy_id, created_at, updated_at
         FROM workspaces ORDER BY created_at",
    )?;
    let rows = stmt.query_map([], map_workspace_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn bump_workspace_revision(conn: &Connection, id: Uuid) -> CliResult<i64> {
    conn.execute(
        "UPDATE workspaces SET config_revision = config_revision + 1, updated_at = ?2
         WHERE workspace_id = ?1",
        params![id.to_string(), now_iso()],
    )?;
    let rev: i64 = conn.query_row(
        "SELECT config_revision FROM workspaces WHERE workspace_id = ?1",
        params![id.to_string()],
        |r| r.get(0),
    )?;
    Ok(rev)
}

fn check_revision(conn: &Connection, id: Uuid, expected: u32) -> CliResult<()> {
    let current: i64 = conn.query_row(
        "SELECT config_revision FROM workspaces WHERE workspace_id = ?1",
        params![id.to_string()],
        |r| r.get(0),
    )?;
    if current != expected as i64 {
        return Err(CliError::new(
            CliErrorCode::RevisionConflict,
            format!("expected workspace revision {expected}，当前 {current}"),
        ));
    }
    Ok(())
}

// ---- period policy ----

fn cadence_str(c: Cadence) -> String {
    serde_json::to_value(c)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}

fn parse_cadence(raw: &str) -> Cadence {
    serde_json::from_value(json!(raw)).unwrap()
}

pub fn policy_view(conn: &Connection, period_policy_id: &str) -> CliResult<serde_json::Value> {
    conn.query_row(
        "SELECT period_policy_id, workspace_id, version, cadence, effective_from, effective_to,
                week_start_day, fiscal_year_start_month, timezone_id, created_at
         FROM period_policies WHERE period_policy_id = ?1",
        params![period_policy_id],
        map_policy_row,
    )
    .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "period policy 不存在"))
}

fn map_policy_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    let cadence: String = r.get(3)?;
    Ok(json!({
        "period_policy_id": r.get::<_, String>(0)?,
        "workspace_id": r.get::<_, String>(1)?,
        "version": r.get::<_, i64>(2)?,
        "policy": {
            "cadence": cadence,
            "week_start_day": r.get::<_, i64>(6)?,
            "fiscal_year_start_month": r.get::<_, i64>(7)?,
            "timezone_id": r.get::<_, String>(8)?,
            "effective_from": r.get::<_, String>(4)?,
        },
        "effective_to": r.get::<_, Option<String>>(5)?,
        "created_at": r.get::<_, String>(9)?,
    }))
}

pub fn list_policies(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<serde_json::Value>> {
    let mut stmt = conn.prepare(
        "SELECT period_policy_id, workspace_id, version, cadence, effective_from, effective_to,
                week_start_day, fiscal_year_start_month, timezone_id, created_at
         FROM period_policies WHERE workspace_id = ?1 ORDER BY effective_from",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], map_policy_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// 注册新策略版本（次版本号）并前移 workspace.current_period_policy_id。
fn insert_policy(
    conn: &Connection,
    workspace_id: Uuid,
    policy: &PeriodPolicy,
) -> CliResult<serde_json::Value> {
    let next_version: i64 = conn.query_row(
        "SELECT COALESCE(MAX(version), 0) + 1 FROM period_policies WHERE workspace_id = ?1",
        params![workspace_id.to_string()],
        |r| r.get(0),
    )?;
    let id = Uuid::new_v4().to_string();
    let effective_from = policy.effective_from.format("%Y-%m-%d").to_string();
    // 冻结表只有 [from,to) 版本行；CLI set 的策略开放至下一版本起点或 null。
    let effective_to: Option<String> = conn
        .query_row(
            "SELECT effective_from FROM period_policies WHERE workspace_id = ?1
             AND effective_from > ?2 ORDER BY effective_from LIMIT 1",
            params![workspace_id.to_string(), effective_from],
            |r| r.get(0),
        )
        .ok();
    // §3.1 [from,to)：新版本起点关闭其前的开放版本（to = 新版本 from），避免自重叠。
    conn.execute(
        "UPDATE period_policies SET effective_to = ?2
         WHERE workspace_id = ?1 AND effective_to IS NULL AND effective_from < ?3",
        params![workspace_id.to_string(), effective_from, effective_from],
    )?;
    // 校验重叠（复用 core periods 校验，经 repository::analysis_runs 同一规则）
    dsheng_core::repository::analysis_runs::register_period_policy(
        conn,
        &workspace_id.to_string(),
        &dsheng_core::repository::analysis_runs::PeriodPolicyRecord {
            period_policy_id: id.clone(),
            version: next_version as u32,
            cadence: policy.cadence,
            effective_from: policy.effective_from,
            effective_to: chrono::NaiveDate::parse_from_str(
                effective_to.as_deref().unwrap_or("9999-12-31"),
                "%Y-%m-%d",
            )
            .ok(),
            week_start_day: policy.week_start_day as u32,
            fiscal_year_start_month: policy.fiscal_year_start_month as u32,
            timezone_id: policy.timezone_id.clone(),
        },
    )
    .map_err(CliError::from)?;
    conn.execute(
        "UPDATE workspaces SET current_period_policy_id = ?2, updated_at = ?3 WHERE workspace_id = ?1",
        params![workspace_id.to_string(), id, now_iso()],
    )?;
    policy_view(conn, &id)
}

// ---- provider 视图与操作 ----

fn map_provider_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<serde_json::Value> {
    Ok(json!({
        "provider_id": r.get::<_, String>(0)?,
        "workspace_id": r.get::<_, String>(1)?,
        "revision": r.get::<_, i64>(2)?,
        "config": {
            "name": r.get::<_, String>(3)?,
            "protocol": "chat_completions",
            "base_url": r.get::<_, String>(4)?,
            "model": r.get::<_, String>(5)?,
            "stream": false,
            "timeouts": {
                "connect_seconds": r.get::<_, i64>(7)?,
                "response_seconds": r.get::<_, i64>(8)?,
                "total_seconds": r.get::<_, i64>(9)?,
            },
            "budget": {
                "context_window_tokens": r.get::<_, i64>(10)?,
                "output_reserve_tokens": r.get::<_, i64>(11)?,
            },
        },
        "key_present": r.get::<_, Option<String>>(6)?.is_some(),
        "created_at": r.get::<_, String>(12)?,
        "updated_at": r.get::<_, String>(13)?,
    }))
}

const PROVIDER_COLS: &str = "provider_id, workspace_id, revision, name, base_url, model, api_key,
       connect_seconds, response_seconds, total_seconds, context_window_tokens,
       output_reserve_tokens, created_at, updated_at";

pub fn list_providers(conn: &Connection, workspace_id: Uuid) -> CliResult<Vec<serde_json::Value>> {
    let sql = format!(
        "SELECT {PROVIDER_COLS} FROM providers WHERE workspace_id = ?1 ORDER BY created_at"
    );
    let mut stmt = conn.prepare(sql.as_str())?;
    let rows = stmt.query_map(params![workspace_id.to_string()], map_provider_row)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn provider_view(conn: &Connection, provider_id: Uuid) -> CliResult<serde_json::Value> {
    let sql = format!("SELECT {PROVIDER_COLS} FROM providers WHERE provider_id = ?1");
    conn.query_row(
        sql.as_str(),
        params![provider_id.to_string()],
        map_provider_row,
    )
    .map_err(|_| CliError::new(CliErrorCode::WorkspaceMismatch, "provider 不存在"))
}

/// 仅服务层内存使用（发送时取 Key）；任何视图/日志/导出不得调用后输出。
pub fn provider_api_key(conn: &Connection, provider_id: Uuid) -> CliResult<Option<String>> {
    let key: Option<String> = conn
        .query_row(
            "SELECT api_key FROM providers WHERE provider_id = ?1",
            params![provider_id.to_string()],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    Ok(key)
}

/// 依赖计数（delete 预览）：活动 attempt 引用 + 是否活跃默认。
pub fn provider_dependency_counts(conn: &Connection, provider_id: Uuid) -> CliResult<(i64, i64)> {
    let attempts: i64 = conn.query_row(
        "SELECT COUNT(*) FROM generation_attempts WHERE provider_id = ?1
         AND terminal_state IS NULL",
        params![provider_id.to_string()],
        |r| r.get(0),
    )?;
    let confirmations: i64 = conn.query_row(
        "SELECT COUNT(*) FROM confirmations WHERE provider_id = ?1 AND consumed_at IS NULL",
        params![provider_id.to_string()],
        |r| r.get(0),
    )?;
    Ok((attempts, confirmations))
}

// ---- SettingsOperation 应用（单事务） ----

/// 应用一条设置操作；返回 (data, applied_defaults)。
pub fn apply_settings_operation(
    conn: &mut Connection,
    op: &SettingsOperation,
    api_key_stdin: Option<&str>,
) -> CliResult<serde_json::Value> {
    let tx = conn
        .transaction()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let data = apply_in_tx(&tx, op, api_key_stdin)?;
    tx.commit()
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    Ok(data)
}

fn apply_in_tx(
    tx: &rusqlite::Transaction<'_>,
    op: &SettingsOperation,
    api_key_stdin: Option<&str>,
) -> CliResult<serde_json::Value> {
    use SettingsOperation::*;
    match op {
        WorkspaceCreate {
            name,
            initial_period_policy,
        } => {
            let workspace_id = Uuid::new_v4();
            let policy_id = Uuid::new_v4().to_string();
            let now = now_iso();
            tx.execute(
                "INSERT INTO workspaces (workspace_id, name, config_revision,
                        active_provider_id, current_period_policy_id, created_at, updated_at)
                 VALUES (?1, ?2, 1, NULL, ?3, ?4, ?4)",
                params![workspace_id.to_string(), name, policy_id, now],
            )?;
            tx.execute(
                "INSERT INTO period_policies (period_policy_id, workspace_id, version, cadence,
                        effective_from, effective_to, week_start_day, fiscal_year_start_month,
                        timezone_id, created_at)
                 VALUES (?1, ?2, 1, ?3, ?4, NULL, ?5, ?6, ?7, ?8)",
                params![
                    policy_id,
                    workspace_id.to_string(),
                    cadence_str(initial_period_policy.cadence),
                    initial_period_policy
                        .effective_from
                        .format("%Y-%m-%d")
                        .to_string(),
                    initial_period_policy.week_start_day as i64,
                    initial_period_policy.fiscal_year_start_month as i64,
                    initial_period_policy.timezone_id,
                    now
                ],
            )?;
            Ok(json!({
                "workspace": workspace_view(tx, workspace_id)?,
                "policy": policy_view(tx, &policy_id)?,
            }))
        }
        WorkspaceUpdate {
            workspace_id,
            expected_revision,
            patch,
        } => {
            check_revision(tx, *workspace_id, *expected_revision)?;
            let trimmed = patch.name.trim();
            if trimmed.is_empty() {
                return Err(CliError::new(
                    CliErrorCode::DataValidationFailed,
                    "workspace 名称不能为空",
                ));
            }
            tx.execute(
                "UPDATE workspaces SET name = ?2 WHERE workspace_id = ?1",
                params![workspace_id.to_string(), trimmed],
            )?;
            bump_workspace_revision(tx, *workspace_id)?;
            Ok(json!({"workspace": workspace_view(tx, *workspace_id)?}))
        }
        WorkspaceUse { workspace_id } => {
            crate::ctx::set_active_workspace(tx, *workspace_id)?;
            Ok(json!({"workspace": workspace_view(tx, *workspace_id)?, "active": true}))
        }
        PeriodPolicySet {
            workspace_id,
            expected_revision,
            policy,
        } => {
            check_revision(tx, *workspace_id, *expected_revision)?;
            let view = insert_policy(tx, *workspace_id, policy)?;
            bump_workspace_revision(tx, *workspace_id)?;
            Ok(json!({
                "policy": view,
                "applied_defaults": {
                    "week_start_day": policy.week_start_day,
                    "fiscal_year_start_month": policy.fiscal_year_start_month,
                    "timezone_id": policy.timezone_id,
                },
            }))
        }
        ProviderUpsert {
            workspace_id,
            provider_id,
            expected_revision,
            config,
            key_action,
        } => {
            validate_provider_config(config)?;
            match (provider_id, expected_revision) {
                (None, None) => {
                    let id = Uuid::new_v4();
                    let (key_present, key_value) = match key_action {
                        KeyAction::Replace => {
                            let key = api_key_stdin.unwrap_or("");
                            if key.trim().is_empty() {
                                return Err(CliError::new(
                                    CliErrorCode::DataValidationFailed,
                                    "key_action=replace 需要 --api-key-stdin 提供非空 Key",
                                )
                                .with_field("--api-key-stdin"));
                            }
                            (true, Some(key.trim().to_string()))
                        }
                        KeyAction::Keep => (false, None),
                    };
                    let name = config.name.trim();
                    ensure_provider_name_free(tx, *workspace_id, name)?;
                    let now = now_iso();
                    tx.execute(
                        "INSERT INTO providers (provider_id, workspace_id, revision, name, protocol,
                                base_url, model, stream, api_key, connect_seconds, response_seconds,
                                total_seconds, context_window_tokens, output_reserve_tokens,
                                created_at, updated_at)
                         VALUES (?1, ?2, 1, ?3, 'chat_completions', ?4, ?5, 0, ?6, ?7, ?8, ?9,
                                 ?10, ?11, ?12, ?12)",
                        params![
                            id.to_string(),
                            workspace_id.to_string(),
                            name,
                            config.base_url,
                            config.model,
                            key_value,
                            config.timeouts.connect_seconds as i64,
                            config.timeouts.response_seconds as i64,
                            config.timeouts.total_seconds as i64,
                            config.budget.context_window_tokens as i64,
                            config.budget.output_reserve_tokens as i64,
                            now
                        ],
                    )?;
                    Ok(json!({"provider": provider_view(tx, id)?, "key_present": key_present}))
                }
                (Some(id), Some(expected)) => {
                    let owner = crate::ctx::object_workspace(
                        tx,
                        "providers",
                        "provider_id",
                        &id.to_string(),
                    )?
                    .ok_or_else(|| {
                        CliError::new(CliErrorCode::WorkspaceMismatch, "provider 不存在")
                    })?;
                    if owner != *workspace_id {
                        return Err(CliError::new(
                            CliErrorCode::WorkspaceMismatch,
                            "provider 不属于所选 workspace",
                        ));
                    }
                    let current: i64 = tx.query_row(
                        "SELECT revision FROM providers WHERE provider_id = ?1",
                        params![id.to_string()],
                        |r| r.get(0),
                    )?;
                    if current != *expected as i64 {
                        return Err(CliError::new(
                            CliErrorCode::RevisionConflict,
                            format!("expected provider revision {expected}，当前 {current}"),
                        ));
                    }
                    let name = config.name.trim();
                    ensure_provider_name_free_for(tx, *workspace_id, name, *id)?;
                    let key_value: Option<String> = match key_action {
                        KeyAction::Replace => {
                            let key = api_key_stdin.unwrap_or("");
                            if key.trim().is_empty() {
                                return Err(CliError::new(
                                    CliErrorCode::DataValidationFailed,
                                    "key_action=replace 需要 --api-key-stdin 提供非空 Key",
                                )
                                .with_field("--api-key-stdin"));
                            }
                            Some(key.trim().to_string())
                        }
                        KeyAction::Keep => None,
                    };
                    tx.execute(
                        "UPDATE providers SET revision = revision + 1, name = ?2, base_url = ?3,
                                model = ?4, connect_seconds = ?5, response_seconds = ?6,
                                total_seconds = ?7, context_window_tokens = ?8,
                                output_reserve_tokens = ?9, updated_at = ?10,
                                api_key = COALESCE(?11, api_key)
                         WHERE provider_id = ?1",
                        params![
                            id.to_string(),
                            name,
                            config.base_url,
                            config.model,
                            config.timeouts.connect_seconds as i64,
                            config.timeouts.response_seconds as i64,
                            config.timeouts.total_seconds as i64,
                            config.budget.context_window_tokens as i64,
                            config.budget.output_reserve_tokens as i64,
                            now_iso(),
                            key_value
                        ],
                    )?;
                    Ok(json!({"provider": provider_view(tx, *id)?}))
                }
                _ => Err(CliError::new(
                    CliErrorCode::DataValidationFailed,
                    "provider_id 与 expected_revision 必须同现同缺",
                )),
            }
        }
        ProviderUse {
            workspace_id,
            provider_id,
            expected_revision,
        } => {
            check_revision(tx, *workspace_id, *expected_revision)?;
            if let Some(id) = provider_id {
                crate::ctx::ensure_object_in_workspace(
                    tx,
                    "providers",
                    "provider_id",
                    &id.to_string(),
                    *workspace_id,
                    "provider",
                )?;
            }
            tx.execute(
                "UPDATE workspaces SET active_provider_id = ?2 WHERE workspace_id = ?1",
                params![workspace_id.to_string(), provider_id.map(|i| i.to_string())],
            )?;
            bump_workspace_revision(tx, *workspace_id)?;
            Ok(json!({"workspace": workspace_view(tx, *workspace_id)?}))
        }
        ProviderClearKey {
            workspace_id,
            provider_id,
            expected_revision,
            confirm_local,
        } => {
            if !confirm_local {
                return Err(CliError::new(
                    CliErrorCode::InvalidArgument,
                    "provider clear-key 需要 --confirm-local",
                )
                .with_field("--confirm-local"));
            }
            crate::ctx::ensure_object_in_workspace(
                tx,
                "providers",
                "provider_id",
                &provider_id.to_string(),
                *workspace_id,
                "provider",
            )?;
            let current: i64 = tx.query_row(
                "SELECT revision FROM providers WHERE provider_id = ?1",
                params![provider_id.to_string()],
                |r| r.get(0),
            )?;
            if current != *expected_revision as i64 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    format!("expected provider revision {expected_revision}，当前 {current}"),
                ));
            }
            tx.execute(
                "UPDATE providers SET api_key = NULL, revision = revision + 1, updated_at = ?2
                 WHERE provider_id = ?1",
                params![provider_id.to_string(), now_iso()],
            )?;
            Ok(json!({"provider": provider_view(tx, *provider_id)?}))
        }
        ProviderDelete {
            workspace_id,
            provider_id,
            expected_revision,
            confirm_local,
        } => {
            if !confirm_local {
                return Err(CliError::new(
                    CliErrorCode::InvalidArgument,
                    "provider delete 需要 --confirm-local",
                )
                .with_field("--confirm-local"));
            }
            crate::ctx::ensure_object_in_workspace(
                tx,
                "providers",
                "provider_id",
                &provider_id.to_string(),
                *workspace_id,
                "provider",
            )?;
            let current: i64 = tx.query_row(
                "SELECT revision FROM providers WHERE provider_id = ?1",
                params![provider_id.to_string()],
                |r| r.get(0),
            )?;
            if current != *expected_revision as i64 {
                return Err(CliError::new(
                    CliErrorCode::RevisionConflict,
                    format!("expected provider revision {expected_revision}，当前 {current}"),
                ));
            }
            let (attempts, confirmations) = provider_dependency_counts(tx, *provider_id)?;
            if attempts > 0 {
                return Err(CliError::new(
                    CliErrorCode::ProviderInUse,
                    "存在未终态生成请求，禁止删除",
                ));
            }
            let active: Option<String> = tx.query_row(
                "SELECT active_provider_id FROM workspaces WHERE workspace_id = ?1",
                params![workspace_id.to_string()],
                |r| r.get(0),
            )?;
            tx.execute(
                "DELETE FROM confirmations WHERE provider_id = ?1 AND consumed_at IS NULL",
                params![provider_id.to_string()],
            )?;
            tx.execute(
                "DELETE FROM providers WHERE provider_id = ?1",
                params![provider_id.to_string()],
            )?;
            if active.as_deref() == Some(provider_id.to_string().as_str()) {
                tx.execute(
                    "UPDATE workspaces SET active_provider_id = NULL WHERE workspace_id = ?1",
                    params![workspace_id.to_string()],
                )?;
                bump_workspace_revision(tx, *workspace_id)?;
            }
            Ok(json!({
                "deleted": true,
                "cleared_pending_confirmations": confirmations,
            }))
        }
    }
}

fn ensure_provider_name_free(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: Uuid,
    name: &str,
) -> CliResult<()> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM providers WHERE workspace_id = ?1 AND name = ?2",
        params![workspace_id.to_string(), name],
        |r| r.get(0),
    )?;
    if count > 0 {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "provider 名称在同 workspace 内必须唯一",
        ));
    }
    Ok(())
}

fn ensure_provider_name_free_for(
    tx: &rusqlite::Transaction<'_>,
    workspace_id: Uuid,
    name: &str,
    except: Uuid,
) -> CliResult<()> {
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM providers WHERE workspace_id = ?1 AND name = ?2 AND provider_id != ?3",
        params![workspace_id.to_string(), name, except.to_string()],
        |r| r.get(0),
    )?;
    if count > 0 {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "provider 名称在同 workspace 内必须唯一",
        ));
    }
    Ok(())
}

/// 公共配置校验（复用 core；无连接测试）。
pub fn validate_provider_config(config: &ProviderPublic) -> CliResult<()> {
    validate_base_url(&config.base_url).map_err(|reason| {
        CliError::new(
            CliErrorCode::ProviderUrlInvalid,
            format!("base_url 非法: {reason}"),
        )
        .with_field("config.base_url")
    })?;
    validate_budget_timeouts(&config.budget, &config.timeouts).map_err(|reason| {
        CliError::new(
            CliErrorCode::InvalidTimeoutBudget,
            format!("超时/预算组合非法: {reason}"),
        )
    })?;
    Ok(())
}

/// 读取 workspace 当前策略（analyze/导出用）。
pub fn current_policy(
    conn: &Connection,
    workspace_id: Uuid,
) -> CliResult<(String, serde_json::Value)> {
    let policy_id: String = conn.query_row(
        "SELECT current_period_policy_id FROM workspaces WHERE workspace_id = ?1",
        params![workspace_id.to_string()],
        |r| r.get(0),
    )?;
    let view = policy_view(conn, &policy_id)?;
    Ok((policy_id, view))
}

/// period set 输入缺省补齐（cli-surface note：CLI 层应用文档化默认值并回显）。
pub fn materialize_policy_defaults(raw: serde_json::Value) -> CliResult<PeriodPolicy> {
    let input: PeriodPolicyInputDefaults = serde_json::from_value(raw)
        .map_err(|e| CliError::invalid(format!("policy 输入非法: {e}")))?;
    input
        .materialize()
        .map_err(|reason| CliError::new(CliErrorCode::DataValidationFailed, reason))
}

/// 解析 DB 中的 cadence 字符串。
pub fn cadence_of(view: &serde_json::Value) -> Option<Cadence> {
    view.get("policy")
        .and_then(|p| p.get("cadence"))
        .and_then(|c| c.as_str())
        .map(parse_cadence)
}
