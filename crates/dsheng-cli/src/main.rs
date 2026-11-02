//! dsheng CLI 入口（Phase 0 占位骨架）。
//!
//! 完整命令面（53 条命令、确认门禁、JSON envelope）由 CLI 与导出工作包按
//! contracts/v1/cli-surface.json 实现；本骨架只保证 `--version` / `--help`
//! 契约先行可用且不误报能力。stdout 只输出单个 JSON 对象（--json 时）。

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            let payload = serde_json::json!({
                "schema_version": "1",
                "ok": true,
                "data": {
                    "app_version": dsheng_core::APP_VERSION,
                    "identifier": dsheng_core::APP_IDENTIFIER,
                    "db_schema_version": dsheng_core::repository::CURRENT_SCHEMA_VERSION,
                    "task_prompt_version": dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
                    "diagnosis_prompt_version": dsheng_core::contracts::prompts::DIAGNOSIS_PROMPT_VERSION,
                    "task_output_schema_version": dsheng_core::contracts::prompts::TASK_OUTPUT_SCHEMA_VERSION,
                    "diagnosis_schema_version": dsheng_core::contracts::prompts::DIAGNOSIS_SCHEMA_VERSION,
                    "input_contract_version": dsheng_core::contracts::prompts::INPUT_CONTRACT_VERSION,
                    "platform": std::env::consts::OS,
                    "architecture": std::env::consts::ARCH,
                }
            });
            println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            ExitCode::SUCCESS
        }
        Some("--help") | None => {
            eprintln!("dsheng 两表一单本地经营分析工具 CLI");
            eprintln!();
            eprintln!("用法: dsheng <command> [flags] --json");
            eprintln!();
            eprintln!("完整命令面见 contracts/v1/cli-surface.json（53 条命令）。");
            eprintln!("当前构建为工程骨架：仅 --version / --help 可用；");
            eprintln!("其余命令由 CLI 工作包按冻结合同实现。");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!(
                "{{\"schema_version\":\"1\",\"ok\":false,\"error\":{{\"code\":\"INVALID_ARGUMENT\",\"message\":\"未知或不完整的命令: {other}\",\"field\":null,\"details\":null}}}}"
            );
            ExitCode::from(2)
        }
    }
}
