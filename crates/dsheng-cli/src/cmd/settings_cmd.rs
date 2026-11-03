//! workspace / period / provider 命令（cli-surface 1–11）。
//!
//! Key 仅经 `--api-key-stdin`（与 `--input-json -` 互斥）；`key_action=replace`
//! 须非空 stdin；无模型连接测试。输出按 settings.schema.json 视图字段。

use crate::cli::{ProviderAction, WorkspaceAction};
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::jsonio;
use crate::store::settings as store;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::settings::KeyAction;

pub fn workspace(action: WorkspaceAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        WorkspaceAction::List => {
            let conn = ctx::open_db(db)?;
            Ok(serde_json::json!({ "workspaces": store::list_workspaces(&conn)? }))
        }
        WorkspaceAction::Create { input_json } => {
            // --input-json - 与 --api-key-stdin 互斥在 provider set 层校验；
            // workspace create 不接受 stdin Key。
            let value = jsonio::read_json(&input_json)?;
            let op = jsonio::parse_settings_operation(value)?;
            if !matches!(
                &op,
                dsheng_core::contracts::settings::SettingsOperation::WorkspaceCreate { .. }
            ) {
                return Err(CliError::invalid(
                    "workspace create 的 operation 必须是 workspace_create",
                ));
            }
            let mut conn = ctx::open_db(db)?;
            let data = store::apply_settings_operation(&mut conn, &op, None)?;
            Ok(serde_json::json!({
                "created": true,
                "workspace": data.get("workspace").cloned().unwrap_or(serde_json::Value::Null),
                "initial_period_policy": data.get("policy").cloned().unwrap_or(serde_json::Value::Null),
            }))
        }
        WorkspaceAction::Update {
            workspace,
            input_json,
        } => {
            let value = jsonio::read_json(&input_json)?;
            let op = jsonio::parse_settings_operation(value)?;
            let input_ws = match &op {
                dsheng_core::contracts::settings::SettingsOperation::WorkspaceUpdate {
                    workspace_id,
                    ..
                } => *workspace_id,
                _ => {
                    return Err(CliError::invalid(
                        "workspace update 的 operation 必须是 workspace_update",
                    ))
                }
            };
            let flag_id = ctx::uuid_arg(&workspace, "--workspace")?;
            if input_ws != flag_id {
                return Err(CliError::new(
                    CliErrorCode::WorkspaceMismatch,
                    "--workspace 与输入中的 workspace_id 不一致",
                ));
            }
            let mut conn = ctx::open_db(db)?;
            store::apply_settings_operation(&mut conn, &op, None)
        }
        WorkspaceAction::Use { workspace } => {
            let id = ctx::uuid_arg(&workspace, "--workspace")?;
            let mut conn = ctx::open_db(db)?;
            let op = dsheng_core::contracts::settings::SettingsOperation::WorkspaceUse {
                workspace_id: id,
            };
            store::apply_settings_operation(&mut conn, &op, None)
        }
        WorkspaceAction::Period { action } => period(action, db),
    }
}

fn period(action: crate::cli::PeriodAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        crate::cli::PeriodAction::List { workspace } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            Ok(serde_json::json!({ "policies": store::list_policies(&conn, ws.workspace_id)? }))
        }
        crate::cli::PeriodAction::Set {
            workspace,
            input_json,
        } => {
            let raw = jsonio::read_json(&input_json)?;
            // 输入形如 {schema_version, operation, workspace_id, expected_revision, policy{...}}
            // policy 字段缺省由 CLI 层补文档化默认值（cadence/effective_from 必填）。
            let workspace_id_flag = raw
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| CliError::invalid("缺少 workspace_id"))?;
            let input_id = ctx::uuid_arg(workspace_id_flag, "workspace_id")?;
            if let Some(flag) = workspace.as_deref() {
                let flag_id = ctx::uuid_arg(flag, "--workspace")?;
                if flag_id != input_id {
                    return Err(CliError::new(
                        CliErrorCode::WorkspaceMismatch,
                        "--workspace 与输入中的 workspace_id 不一致",
                    ));
                }
            }
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, Some(&input_id.to_string()))?;
            let expected_revision: u32 = raw
                .get("expected_revision")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| CliError::invalid("expected_revision 必须为正整数"))?;
            let policy_raw = raw
                .get("policy")
                .cloned()
                .ok_or_else(|| CliError::invalid("缺少 policy"))?;
            let policy = store::materialize_policy_defaults(policy_raw)?;
            let op = dsheng_core::contracts::settings::SettingsOperation::PeriodPolicySet {
                workspace_id: ws.workspace_id,
                expected_revision,
                policy,
            };
            let mut conn = ctx::open_db(db)?;
            store::apply_settings_operation(&mut conn, &op, None)
        }
    }
}

pub fn provider(action: ProviderAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        ProviderAction::List { workspace } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            Ok(serde_json::json!({ "providers": store::list_providers(&conn, ws.workspace_id)? }))
        }
        ProviderAction::Set {
            workspace,
            input_json,
            api_key_stdin,
        } => {
            if api_key_stdin && input_json == "-" {
                return Err(CliError::invalid(
                    "--api-key-stdin 与 --input-json - 不能同时占用 stdin",
                )
                .with_field("--api-key-stdin"));
            }
            let value = jsonio::read_json(&input_json)?;
            let op = jsonio::parse_settings_operation(value)?;
            let (input_ws, needs_key) = match &op {
                dsheng_core::contracts::settings::SettingsOperation::ProviderUpsert {
                    workspace_id,
                    key_action,
                    ..
                } => (*workspace_id, matches!(key_action, KeyAction::Replace)),
                _ => {
                    return Err(CliError::invalid(
                        "provider set 的 operation 必须是 provider_upsert",
                    ))
                }
            };
            if let Some(flag) = workspace.as_deref() {
                let flag_id = ctx::uuid_arg(flag, "--workspace")?;
                if input_ws != flag_id {
                    return Err(CliError::new(
                        CliErrorCode::WorkspaceMismatch,
                        "--workspace 与输入中的 workspace_id 不一致",
                    ));
                }
            }
            if needs_key && !api_key_stdin {
                return Err(CliError::new(
                    CliErrorCode::DataValidationFailed,
                    "key_action=replace 必须配合 --api-key-stdin（Key 不得经 JSON/argv）",
                )
                .with_field("--api-key-stdin"));
            }
            let stdin_key = if api_key_stdin {
                let text = jsonio::read_stdin_text()?;
                if text.trim().is_empty() {
                    return Err(CliError::new(
                        CliErrorCode::DataValidationFailed,
                        "--api-key-stdin 收到空 Key",
                    )
                    .with_field("--api-key-stdin"));
                }
                Some(text)
            } else {
                None
            };
            let mut conn = ctx::open_db(db)?;
            store::apply_settings_operation(&mut conn, &op, stdin_key.as_deref())
        }
        ProviderAction::Use {
            workspace,
            provider,
            expected_revision,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let provider_id = if provider == "none" {
                None
            } else {
                Some(ctx::uuid_arg(&provider, "--provider")?)
            };
            let op = dsheng_core::contracts::settings::SettingsOperation::ProviderUse {
                workspace_id: ws.workspace_id,
                provider_id,
                expected_revision,
            };
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            store::apply_settings_operation(&mut conn, &op, None)
        }
        ProviderAction::ClearKey {
            workspace,
            provider,
            expected_revision,
            confirm_local,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let provider_id = ctx::uuid_arg(&provider, "--provider")?;
            let op = dsheng_core::contracts::settings::SettingsOperation::ProviderClearKey {
                workspace_id: ws.workspace_id,
                provider_id,
                expected_revision,
                confirm_local,
            };
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            store::apply_settings_operation(&mut conn, &op, None)
        }
        ProviderAction::Delete {
            workspace,
            provider,
            expected_revision,
            confirm_local,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let provider_id = ctx::uuid_arg(&provider, "--provider")?;
            // 依赖预览先行（notes: preview dependency count first）
            let (attempts, pending) = store::provider_dependency_counts(&conn, provider_id)?;
            if attempts > 0 {
                return Err(CliError::new(
                    CliErrorCode::ProviderInUse,
                    "存在未终态生成请求，禁止删除",
                )
                .with_details(serde_json::json!({"active_attempts": attempts})));
            }
            let op = dsheng_core::contracts::settings::SettingsOperation::ProviderDelete {
                workspace_id: ws.workspace_id,
                provider_id,
                expected_revision,
                confirm_local,
            };
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            let mut data = store::apply_settings_operation(&mut conn, &op, None)?;
            if let Some(obj) = data.as_object_mut() {
                obj.insert(
                    "preview".into(),
                    serde_json::json!({"active_attempts": attempts, "pending_confirmations": pending}),
                );
            }
            Ok(data)
        }
    }
}
