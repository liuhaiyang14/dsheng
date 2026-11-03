//! workspace / period / provider command 实现（对应 UI DshengClient
//! workspace*/period*/provider* 方法；CLI 同名命令 workspace/provider 组）。

use super::{
    dispatch, envelope, json_input_file, opt_str, provider_cmd, req_str, req_u32, workspace_cmd,
};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use dsheng_core::contracts::settings::SettingsOperation;
use serde_json::Value;

/// dsheng workspace list → data `{workspaces:[...]}`。
pub fn workspace_list(env: &DesktopEnv) -> Value {
    envelope(dispatch(env, workspace_cmd(args::WorkspaceAction::List)))
}

/// dsheng workspace create（input = settings.schema workspace_create 输入）。
pub fn workspace_create(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let file = json_input_file(&input)?;
        dispatch(
            env,
            workspace_cmd(args::WorkspaceAction::Create {
                input_json: file.path().to_string_lossy().into_owned(),
            }),
        )
    })())
}

/// dsheng workspace update（--workspace 取输入内 workspace_id；命令层交叉校验一致）。
pub fn workspace_update(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = req_str(&input, "workspace_id")?;
        let file = json_input_file(&input)?;
        dispatch(
            env,
            workspace_cmd(args::WorkspaceAction::Update {
                workspace,
                input_json: file.path().to_string_lossy().into_owned(),
            }),
        )
    })())
}

/// dsheng workspace use（写入 app_preferences.active_workspace_id）。
pub fn workspace_use(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = req_str(&input, "workspace_id")?;
        dispatch(env, workspace_cmd(args::WorkspaceAction::Use { workspace }))
    })())
}

/// dsheng workspace period list。
pub fn period_list(env: &DesktopEnv, workspace_id: String) -> Value {
    envelope(dispatch(
        env,
        workspace_cmd(args::WorkspaceAction::Period {
            action: args::PeriodAction::List {
                workspace: Some(workspace_id),
            },
        }),
    ))
}

/// dsheng workspace period set（input = period_policy_set 输入）。
pub fn period_set(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = opt_str(&input, "workspace_id");
        let file = json_input_file(&input)?;
        dispatch(
            env,
            workspace_cmd(args::WorkspaceAction::Period {
                action: args::PeriodAction::Set {
                    workspace,
                    input_json: file.path().to_string_lossy().into_owned(),
                },
            }),
        )
    })())
}

/// dsheng provider list → data `{providers:[...]}`。
pub fn provider_list(env: &DesktopEnv, workspace_id: String) -> Value {
    envelope(dispatch(
        env,
        provider_cmd(args::ProviderAction::List {
            workspace: Some(workspace_id),
        }),
    ))
}

/// dsheng provider set。
///
/// Key 走单独通道（对应 CLI `--api-key-stdin`；绝不进 input JSON）：
/// CLI 传输层的 stdin 读取不适用于进程内调用，故直达同一服务函数
/// `store::settings::apply_settings_operation`（op 解析/校验/事务/Key
/// 非空校验全部复用；key_action=replace 且 apiKey 缺失由服务层拒绝）。
pub fn provider_set(env: &DesktopEnv, input: Value, api_key: Option<String>) -> Value {
    envelope((|| {
        let op = dsheng_cli::jsonio::parse_settings_operation(input)?;
        if !matches!(op, SettingsOperation::ProviderUpsert { .. }) {
            return Err(dsheng_cli::error::CliError::invalid(
                "provider set 的 operation 必须是 provider_upsert",
            ));
        }
        let db_path = env.db_arg()?;
        let mut conn = dsheng_cli::ctx::open_db_at(std::path::Path::new(&db_path))?;
        dsheng_cli::store::settings::apply_settings_operation(&mut conn, &op, api_key.as_deref())
    })())
}

/// dsheng provider use（provider_id=null → "none" 清除默认）。
pub fn provider_use(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let provider = opt_str(&input, "provider_id").unwrap_or_else(|| "none".into());
        let expected_revision = req_u32(&input, "expected_revision")?;
        dispatch(
            env,
            provider_cmd(args::ProviderAction::Use {
                workspace,
                provider,
                expected_revision,
            }),
        )
    })())
}

/// dsheng provider clear-key（confirm_local=true 为 UI 合同字面值）。
pub fn provider_clear_key(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let provider = req_str(&input, "provider_id")?;
        let expected_revision = req_u32(&input, "expected_revision")?;
        dispatch(
            env,
            provider_cmd(args::ProviderAction::ClearKey {
                workspace,
                provider,
                expected_revision,
                confirm_local: true,
            }),
        )
    })())
}

/// dsheng provider delete（依赖预览计数由命令层附带返回）。
pub fn provider_delete(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let provider = req_str(&input, "provider_id")?;
        let expected_revision = req_u32(&input, "expected_revision")?;
        dispatch(
            env,
            provider_cmd(args::ProviderAction::Delete {
                workspace,
                provider,
                expected_revision,
                confirm_local: true,
            }),
        )
    })())
}
