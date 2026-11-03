//! 输入 JSON 读取：`--input-json <file|->` / `--patch-json <file|->` /
//! stdin 专用 flag（`--api-key-stdin`、`--answers-stdin`、`--replacement-stdin`）。
//!
//! 未知输入 JSON 字段由各 serde 类型 `deny_unknown_fields` 拒绝
//! （cli-surface compatibility：reject; never silently ignore）。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use std::io::Read;

/// 读取 file 或 "-"（stdin）为文本。
pub fn read_source(file_or_dash: &str) -> CliResult<String> {
    if file_or_dash == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf).map_err(|e| {
            CliError::new(
                CliErrorCode::DataValidationFailed,
                format!("stdin 读取失败: {e}"),
            )
        })?;
        Ok(buf)
    } else {
        std::fs::read_to_string(file_or_dash).map_err(|e| {
            CliError::new(
                CliErrorCode::InvalidArgument,
                format!("输入文件不可读: {e}"),
            )
            .with_field("--input-json")
        })
    }
}

/// 解析 JSON（拒绝重复键等结构错误由 serde 承担）。
pub fn parse_json(text: &str) -> CliResult<serde_json::Value> {
    serde_json::from_str(text).map_err(|e| {
        CliError::new(
            CliErrorCode::InvalidArgument,
            format!("输入不是合法 JSON: {e}"),
        )
    })
}

/// 读入并解析 file|stdin。
pub fn read_json(file_or_dash: &str) -> CliResult<serde_json::Value> {
    parse_json(&read_source(file_or_dash)?)
}

/// 读 stdin 原文（不解析；Key/答案/更正声明）。
pub fn read_stdin_text() -> CliResult<String> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(|e| {
        CliError::new(
            CliErrorCode::DataValidationFailed,
            format!("stdin 读取失败: {e}"),
        )
    })?;
    Ok(buf)
}

/// settings 操作输入：剥离 schema_version（校验为 "1"）后交给 core 枚举解析。
pub fn parse_settings_operation(
    value: serde_json::Value,
) -> CliResult<dsheng_core::contracts::settings::SettingsOperation> {
    let mut obj = match value {
        serde_json::Value::Object(map) => map,
        _ => return Err(CliError::invalid("设置输入必须是 JSON 对象")),
    };
    match obj.get("schema_version") {
        Some(serde_json::Value::String(s)) if s == "1" => {
            obj.remove("schema_version");
        }
        _ => {
            return Err(CliError::invalid(
                "schema_version 必须为 \"1\"（settings.schema.json）",
            ))
        }
    }
    // 文档化默认值在解析前应用（cli-surface defaults；cadence/effective_from 必填）。
    let operation = obj
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match operation.as_str() {
        "workspace_create" => {
            let policy = obj
                .get("initial_period_policy")
                .cloned()
                .ok_or_else(|| CliError::invalid("缺少 initial_period_policy"))?;
            obj.insert(
                "initial_period_policy".into(),
                materialize_policy_json(policy)?,
            );
        }
        "period_policy_set" => {
            let policy = obj
                .get("policy")
                .cloned()
                .ok_or_else(|| CliError::invalid("缺少 policy"))?;
            obj.insert("policy".into(), materialize_policy_json(policy)?);
        }
        "provider_upsert" => {
            // core ChatCompletionsMarker 为 unit 类型（合同 const "chat_completions"
            // 的字符串形式在其 Deserialize 之外——core 类型缺口，报告主 Agent）。
            // CLI 在解析前校验 const 并转 null 供 unit 反序列化。
            let config = obj
                .get_mut("config")
                .ok_or_else(|| CliError::invalid("缺少 config"))?;
            if let Some(config) = config.as_object_mut() {
                match config.get("protocol") {
                    Some(serde_json::Value::String(s)) if s == "chat_completions" => {
                        config.insert("protocol".into(), serde_json::Value::Null);
                    }
                    _ => {
                        return Err(CliError::invalid(
                            "protocol 必须为 \"chat_completions\"（settings.schema const）",
                        ))
                    }
                }
            }
        }
        _ => {}
    }
    serde_json::from_value(serde_json::Value::Object(obj))
        .map_err(|e| CliError::new(CliErrorCode::InvalidArgument, format!("设置输入非法: {e}")))
}

/// 把可缺省的 policy 输入补成完整 PeriodPolicy JSON（回显实际采用值）。
fn materialize_policy_json(value: serde_json::Value) -> CliResult<serde_json::Value> {
    let input: dsheng_core::contracts::settings::PeriodPolicyInputDefaults =
        serde_json::from_value(value)
            .map_err(|e| CliError::invalid(format!("policy 输入非法: {e}")))?;
    let policy = input
        .materialize()
        .map_err(|reason| CliError::new(CliErrorCode::DataValidationFailed, reason))?;
    serde_json::to_value(policy)
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))
}
