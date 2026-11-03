//! dsheng CLI 库入口：命令分发、envelope 输出、退出码。
//!
//! stdout：`--json` 单个最终 JSON envelope（成功/失败各一形）；进度/诊断仅 stderr。
//! 退出码按 contracts/v1/cli-surface.json errors 目录（成功 0；2/3/4/5 分类）。
//! 测试经 [`run_with_output`] 注入输出流（无网络；Provider 发送路径见 cmd/generation）。

pub mod cli;
pub mod cmd;
pub mod ctx;
pub mod error;
pub mod jsonio;
pub mod out;
pub mod store;

/// 共享导出库：与桌面壳挂载**同一源文件**（src-tauri/src/export/**），
/// 桌面与 CLI 不分别实现导出逻辑。
#[path = "../../../src-tauri/src/export/mod.rs"]
pub mod export;

use clap::Parser;
use dsheng_core::contracts::cli::{OkEnvelope, RESPONSE_SCHEMA_VERSION};
use error::{CliError, CliResult};

/// 以进程 stdout/stderr 运行（main 入口）。
pub fn run(args: &[String]) -> i32 {
    let mut out = std::io::stdout();
    let mut err_out = std::io::stderr();
    run_with_output(args, &mut out, &mut err_out)
}

/// 以注入输出流运行（集成测试层：进程内、无网络）。
pub fn run_with_output<W: std::io::Write, E: std::io::Write>(
    args: &[String],
    out: &mut W,
    err_out: &mut E,
) -> i32 {
    match dispatch(args) {
        Ok((value, stderr_note)) => {
            if let Some(note) = stderr_note {
                let _ = writeln!(err_out, "{note}");
            }
            let envelope = OkEnvelope::new(value);
            let _ = writeln!(out, "{}", serde_json::to_string(&envelope).expect("json"));
            0
        }
        Err(e) => {
            let envelope = e.envelope();
            let _ = writeln!(out, "{}", serde_json::to_string(&envelope).expect("json"));
            let _ = writeln!(err_out, "dsheng: {e}");
            e.exit_code()
        }
    }
}

/// 版本契约载荷（无用户数据）。
pub fn version_payload() -> serde_json::Value {
    serde_json::json!({
        "app_version": dsheng_core::APP_VERSION,
        "identifier": dsheng_core::APP_IDENTIFIER,
        "db_schema_version": dsheng_core::repository::CURRENT_SCHEMA_VERSION,
        "task_prompt_version": dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "diagnosis_prompt_version": dsheng_core::contracts::prompts::DIAGNOSIS_PROMPT_VERSION,
        "task_output_schema_version": dsheng_core::contracts::prompts::TASK_OUTPUT_SCHEMA_VERSION,
        "diagnosis_schema_version": dsheng_core::contracts::prompts::DIAGNOSIS_SCHEMA_VERSION,
        "input_contract_version": dsheng_core::contracts::prompts::INPUT_CONTRACT_VERSION,
        "context_policy_version": dsheng_core::contracts::prompts::CONTEXT_POLICY_VERSION,
        "platform": std::env::consts::OS,
        "architecture": std::env::consts::ARCH,
    })
}

fn dispatch(args: &[String]) -> CliResult<(serde_json::Value, Option<String>)> {
    // `dsheng --version`（cli-surface：version 命令）优先于子命令解析。
    if args.iter().any(|a| a == "--version") {
        return Ok((version_payload(), None));
    }
    let parsed =
        cli::Cli::try_parse_from(std::iter::once("dsheng").chain(args.iter().map(|s| s.as_str())));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(e) if e.use_stderr() => Err(map_clap_error(e))?,
        Err(_) => {
            // --help：帮助文本走 stderr；stdout 仍是成功 envelope。
            return Ok((serde_json::json!({ "help": true }), Some(help_text())));
        }
    };
    let Some(command) = cli.command else {
        // `dsheng --help` / 无参数：帮助文本走 stderr，stdout 仍是成功 envelope。
        return Ok((serde_json::json!({"help": true}), Some(help_text())));
    };
    let data = cmd::dispatch(command, cli.db.as_ref())?;
    Ok((data, None))
}

fn map_clap_error(e: clap::Error) -> CliError {
    use clap::error::ErrorKind::*;
    match e.kind() {
        DisplayHelp | DisplayHelpOnMissingArgumentOrSubcommand => {
            // 帮助不算错误：直接打印到 stderr，退出 0（由 dispatch 前置处理，
            // 此分支实际不可达，保守返回成功形错误）。
            CliError::invalid("请求帮助")
        }
        UnknownArgument => CliError::invalid("未知参数（CLI 只接受合同列出的 flag）").with_details(
            serde_json::json!({"kind": "unknown_flag", "argument": first_flag_of(&e)}),
        ),
        InvalidValue | InvalidSubcommand => {
            CliError::invalid("未知命令或取值（见 contracts/v1/cli-surface.json）")
        }
        MissingRequiredArgument | MissingSubcommand => CliError::invalid("缺少必需参数或子命令"),
        _ => CliError::invalid("参数解析失败"),
    }
}

pub(crate) fn help_text() -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "dsheng 两表一单本地经营分析工具 CLI");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "用法: dsheng <command> [flags] --json   （合同版本 {RESPONSE_SCHEMA_VERSION}）"
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "命令组: workspace provider scan mapping manual-input metric"
    );
    let _ = writeln!(
        s,
        "        source-conflict analyze analysis diagnosis generation task"
    );
    let _ = writeln!(s, "        history export template version");
    let _ = writeln!(s, "完整语法见 contracts/v1/cli-surface.json（54 条命令）。");
    s
}

/// 从 clap 错误 context 中提取非法参数名（envelope.error.field 用）。
fn first_flag_of(e: &clap::Error) -> Option<String> {
    for (kind, value) in e.context() {
        if matches!(kind, clap::error::ContextKind::InvalidArg) {
            if let clap::error::ContextValue::String(s) = value {
                return Some(s.clone());
            }
        }
    }
    None
}
