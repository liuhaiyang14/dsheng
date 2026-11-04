//! 复现用户 GUI 场景：真实库（schema 2）＋ 真实 ds 目录，直调 shell 层 scan。
//! 用法：cargo run --manifest-path src-tauri/Cargo.toml --example repro_scan -- <db> <workspace_id> <dir>
use dsheng_app::contracts::{core, env::DesktopEnv};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (db, ws, dir) = (
        args.get(1).expect("db path").clone(),
        args.get(2).expect("workspace id").clone(),
        args.get(3).expect("dir").clone(),
    );
    let env = DesktopEnv::with_db(&db);
    println!("[repro] db={db} ws={ws} dir={dir}");
    let t = Instant::now();
    // 与 GUI 完全同一函数
    let result = core::data::scan(&env, ws, dir);
    println!("[repro] scan returned in {:?}: {}", t.elapsed(), serde_json::to_string(&result).unwrap_or_default().chars().take(400).collect::<String>());
}
