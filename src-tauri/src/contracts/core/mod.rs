//! 桌面 command 核心实现（进程内可直调；测试层不经 tauri 运行时）。
//!
//! 每个 command：解析 `DSHENG_DB`/平台目录定位库 → 以与 CLI 完全相同的
//! 入口调用 dsheng-cli 服务层（[`dsheng_cli::cmd::dispatch`]，构造同一
//! clap 命令枚举；`--input-json` 类输入经临时文件走同一读取/校验路径；
//! stdin 专通道（Provider Key、continue 答复）以内联文本直达服务函数）
//! → 原样返回 CliEnvelope JSON（成功/失败均不抛出，envelope 即结果）。
//!
//! 子模块：settings / data / diagnosis / task / history。

pub mod data;
pub mod metric_defaults;
pub mod diagnosis;
pub mod generation;
pub mod history;
pub mod settings;
pub mod task;

use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use dsheng_cli::error::{CliError, CliResult};
use dsheng_core::contracts::cli::{CliErrorCode, OkEnvelope, RESPONSE_SCHEMA_VERSION};
use serde_json::Value;
use std::io::Write as _;

/// 把服务层结果包成 CliEnvelope JSON（原样透传 data/error；不抛出）。
pub fn envelope(result: CliResult<Value>) -> Value {
    match result {
        Ok(data) => serde_json::to_value(OkEnvelope::new(data)).unwrap_or_else(|_| {
            serde_json::json!({
                "schema_version": RESPONSE_SCHEMA_VERSION,
                "ok": true,
                "data": Value::Null,
            })
        }),
        Err(e) => serde_json::to_value(e.envelope()).unwrap_or_else(|_| {
            serde_json::json!({
                "schema_version": RESPONSE_SCHEMA_VERSION,
                "ok": false,
                "error": {
                    "code": "LOCAL_SAVE_FAILED",
                    "message": "envelope 序列化失败",
                    "field": Value::Null,
                    "details": Value::Null,
                },
            })
        }),
    }
}

/// 以桌面库路径 dispatch CLI 命令（与 CLI 进程同一实现）。
pub fn dispatch(env: &DesktopEnv, command: args::Command) -> CliResult<Value> {
    let db = env.db_arg()?;
    dsheng_cli::cmd::dispatch(command, Some(&db))
}

/// 带生成进度 sink 的 dispatch（confirm* 流式进度经
/// [`crate::contracts::progress`] 事件桥转发给 UI；其余命令忽略）。
pub fn dispatch_with_progress(
    env: &DesktopEnv,
    command: args::Command,
    progress: Option<dsheng_cli::store::generation::ProgressSink>,
) -> CliResult<Value> {
    let db = env.db_arg()?;
    dsheng_cli::cmd::dispatch_with_progress(command, Some(&db), progress)
}

// clap 命令组构造（子命令枚举均为 struct variant；集中便于阅读）。
pub fn workspace_cmd(action: args::WorkspaceAction) -> args::Command {
    args::Command::Workspace { action }
}
pub fn provider_cmd(action: args::ProviderAction) -> args::Command {
    args::Command::Provider { action }
}
pub fn mapping_cmd(action: args::MappingAction) -> args::Command {
    args::Command::Mapping { action }
}
pub fn diagnosis_cmd(action: args::DiagnosisAction) -> args::Command {
    args::Command::Diagnosis { action }
}
pub fn task_cmd(action: args::TaskAction) -> args::Command {
    args::Command::Task { action }
}
pub fn history_cmd(action: args::HistoryAction) -> args::Command {
    args::Command::History { action }
}

/// 把 UI 传入的 JSON 对象落临时文件，走 CLI `--input-json` 同一读取与
/// serde 校验路径（deny_unknown_fields 等）。调用方须持有返回值至 dispatch
/// 结束（drop 即删）。
pub fn json_input_file(value: &Value) -> CliResult<tempfile::NamedTempFile> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| {
        CliError::new(
            CliErrorCode::LocalSaveFailed,
            format!("临时输入文件创建失败: {e}"),
        )
    })?;
    file.write_all(value.to_string().as_bytes()).map_err(|e| {
        CliError::new(
            CliErrorCode::LocalSaveFailed,
            format!("临时输入文件写入失败: {e}"),
        )
    })?;
    Ok(file)
}

// ---- 输入字段提取（UI 传 camelCase JSON 对象；缺失/类型错误 → INVALID_ARGUMENT） ----

pub fn req_str(obj: &Value, key: &str) -> CliResult<String> {
    opt_str(obj, key)
        .ok_or_else(|| CliError::invalid(format!("缺少字符串参数 {key}")).with_field(key))
}

pub fn opt_str(obj: &Value, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

pub fn req_u32(obj: &Value, key: &str) -> CliResult<u32> {
    let raw = obj
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| CliError::invalid(format!("缺少非负整数参数 {key}")).with_field(key))?;
    u32::try_from(raw)
        .map_err(|_| CliError::invalid(format!("{key} 超出 u32 范围")).with_field(key))
}

pub fn opt_u32(obj: &Value, key: &str) -> CliResult<Option<u32>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => {
            let raw = v.as_u64().ok_or_else(|| {
                CliError::invalid(format!("{key} 必须是非负整数")).with_field(key)
            })?;
            u32::try_from(raw)
                .map(Some)
                .map_err(|_| CliError::invalid(format!("{key} 超出 u32 范围")).with_field(key))
        }
    }
}

/// 字符串数组参数（缺省为空数组；生成证据等）。
pub fn str_array(obj: &Value, key: &str) -> CliResult<Vec<String>> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| {
                v.as_str().map(str::to_string).ok_or_else(|| {
                    CliError::invalid(format!("{key} 必须是字符串数组")).with_field(key)
                })
            })
            .collect(),
        Some(_) => Err(CliError::invalid(format!("{key} 必须是字符串数组")).with_field(key)),
    }
}

// ---- 取值映射（UI 字符串 → clap 枚举；非法值 INVALID_ARGUMENT） ----

pub fn analyze_grain_of(raw: &str) -> CliResult<args::AnalyzeGrain> {
    match raw {
        "auto" => Ok(args::AnalyzeGrain::Auto),
        "day" => Ok(args::AnalyzeGrain::Day),
        "week" => Ok(args::AnalyzeGrain::Week),
        "month" => Ok(args::AnalyzeGrain::Month),
        "quarter" => Ok(args::AnalyzeGrain::Quarter),
        other => Err(CliError::invalid(format!("grain 取值非法: {other}")).with_field("grain")),
    }
}

pub fn timeline_grain_of(raw: &str) -> CliResult<args::TimelineGrain> {
    match raw {
        "day" => Ok(args::TimelineGrain::Day),
        "week" => Ok(args::TimelineGrain::Week),
        "month" => Ok(args::TimelineGrain::Month),
        "quarter" => Ok(args::TimelineGrain::Quarter),
        other => Err(CliError::invalid(format!("grain 取值非法: {other}")).with_field("grain")),
    }
}

pub fn framework_of(raw: &str) -> CliResult<args::FrameworkArg> {
    match raw {
        "5why" => Ok(args::FrameworkArg::Why5),
        "5w" => Ok(args::FrameworkArg::W5),
        "5w2h" => Ok(args::FrameworkArg::W5h2),
        other => {
            Err(CliError::invalid(format!("framework 取值非法: {other}")).with_field("framework"))
        }
    }
}

pub fn export_doc_format_of(raw: &str) -> CliResult<args::ExportDocFormat> {
    match raw {
        "md" => Ok(args::ExportDocFormat::Md),
        "json" => Ok(args::ExportDocFormat::Json),
        "pdf" => Ok(args::ExportDocFormat::Pdf),
        other => Err(CliError::invalid(format!("format 取值非法: {other}")).with_field("format")),
    }
}
