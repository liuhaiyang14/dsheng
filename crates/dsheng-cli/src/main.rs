//! CLI 入口（薄壳；全部逻辑在库 crate，集成测试直接注入输出流运行）。

use dsheng_cli::run;

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::ExitCode::from(run(&args) as u8)
}
