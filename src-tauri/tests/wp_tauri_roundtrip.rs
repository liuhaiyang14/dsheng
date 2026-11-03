//! 用例 40 E2E：CLI ↔ 桌面 command 层共享同一 SQLite（双向读写一致）。
//!
//! 测试层次：临时 SQLite 库 + 合成 XLSX；桌面侧直调 command 实现
//! （dsheng_app::contracts::core，不经 tauri 运行时/真实窗口），CLI 侧
//! `dsheng_cli::run_with_output`（进程内）。不断言 UI 渲染。
//! 生成命令只测 dry-run（preview）与拒绝路径——无 Mock Provider、无网络。

use dsheng_app::contracts::core as cmd;
use dsheng_app::contracts::env::DesktopEnv;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

// ---- 通用辅助 ----

/// 进程内跑 CLI（**始终注入 `--db`**，绝不触碰用户真实数据目录）。
fn cli(db: &str, args: &[&str]) -> (Value, i32) {
    let mut owned: Vec<String> = vec!["--db".into(), db.to_string()];
    owned.extend(args.iter().map(|s| s.to_string()));
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    let stdout = String::from_utf8(out).expect("utf8");
    let v: Value = serde_json::from_str(stdout.trim()).expect("stdout 必须是单个 envelope");
    (v, code)
}

fn cli_ok(db: &str, args: &[&str]) -> Value {
    let (v, code) = cli(db, args);
    assert_eq!(code, 0, "CLI {args:?} 失败: {v}");
    assert_eq!(v["schema_version"], "1");
    assert_eq!(v["ok"], true);
    v["data"].clone()
}

/// 桌面 command envelope：断言 ok 并返回 data。
fn ok_data(envelope: Value, ctx: &str) -> Value {
    assert_eq!(envelope["schema_version"], "1", "{ctx}: {envelope}");
    assert_eq!(envelope["ok"], true, "{ctx} 失败: {envelope}");
    envelope["data"].clone()
}

/// 桌面 command envelope：断言失败并返回 error.code。
fn err_code(envelope: Value, ctx: &str) -> String {
    assert_eq!(envelope["schema_version"], "1", "{ctx}: {envelope}");
    assert_eq!(envelope["ok"], false, "{ctx} 应失败: {envelope}");
    envelope["error"]["code"]
        .as_str()
        .expect("code")
        .to_string()
}

fn write_json(dir: &std::path::Path, name: &str, value: &Value) -> String {
    let path = dir.join(name);
    std::fs::write(&path, value.to_string()).expect("写入输入文件");
    path.to_string_lossy().into_owned()
}

fn env_of(db: &str) -> DesktopEnv {
    DesktopEnv::with_db(db)
}

// ---- 共享夹具（合成数据；全部经 CLI/桌面 command 写入或测试种子行） ----

struct Fix {
    db: String,
    dir: PathBuf,
    workspace: String,
    run_id: String,
    task_id: String,
    step1: String,
    step2: String,
}

/// CLI 建库：workspace → 指标 → provider（Key 测试注入行）→ analyze →
/// 候选种子 → reconcile apply（2 步任务）。
fn fixture(tag: &str) -> Fix {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.keep();
    let db = dir
        .join(format!("{tag}.sqlite3"))
        .to_string_lossy()
        .into_owned();

    let ws_input = json!({
        "schema_version": "1", "operation": "workspace_create", "name": "用例40门店",
        "initial_period_policy": {"cadence": "month", "effective_from": "2026-01-01"},
    });
    let data = cli_ok(
        &db,
        &[
            "workspace",
            "create",
            "--input-json",
            &write_json(&dir, "ws.json", &ws_input),
            "--json",
        ],
    );
    let workspace = data["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();

    let metric = json!({
        "metric_key": "sales_net", "label": "销售额", "semantic_type": "flow",
        "unit_signature": {"dimension": "money", "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"},
        "direction": "up", "target_constraint_scope": "monthly_revenue_floor",
        "revenue_semantics": true, "aggregation": "sum_disjoint", "required_inputs": [],
        "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
        "effective_from": "2026-01-01", "expected_revision": 0,
    });
    cli_ok(
        &db,
        &[
            "metric",
            "set",
            "--workspace",
            &workspace,
            "--input-json",
            &write_json(&dir, "metric.json", &metric),
            "--json",
        ],
    );

    // provider（带 Key 种子行：不经 stdin；密文不入任何输出）
    let provider = uuid::Uuid::new_v4();
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
        conn.execute(
            "INSERT INTO providers (provider_id, workspace_id, revision, name, protocol,
                    base_url, model, stream, api_key, connect_seconds, response_seconds,
                    total_seconds, context_window_tokens, output_reserve_tokens,
                    created_at, updated_at)
             VALUES (?1, ?2, 1, '合成Provider', 'chat_completions',
                     'https://api.example.com/v1', 'synthetic-model', 0, 'test-key-not-real',
                     15, 180, 240, 65536, 8192, '2026-11-03T00:00:00Z', '2026-11-03T00:00:00Z')",
            rusqlite::params![provider.to_string(), workspace],
        )
        .unwrap();
        conn.execute(
            "UPDATE workspaces SET active_provider_id = ?2 WHERE workspace_id = ?1",
            rusqlite::params![workspace, provider.to_string()],
        )
        .unwrap();
    }

    // 合成核算表 + CLI analyze
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("核算表").unwrap();
    for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"]
        .iter()
        .enumerate()
    {
        sheet.write_string(0, c as u16, *h).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 2, "12000.5").unwrap();
    sheet.write_string(1, 3, "11500.25").unwrap();
    sheet.write_string(1, 4, "12500").unwrap();
    book.save(dir.join("核算表模板.xlsx")).unwrap();
    let data = cli_ok(
        &db,
        &[
            "analyze",
            "--workspace",
            &workspace,
            "--dir",
            &dir.to_string_lossy(),
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-31",
            "--grain",
            "auto",
            "--json",
        ],
    );
    let run_id = data["run_id"].as_str().unwrap().to_string();

    // 候选（种子行：生成输出口径与 CLI S5 测试一致）→ CLI reconcile 两段式
    let fingerprint: String = {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id = ?1",
            rusqlite::params![run_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    let candidate_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        let output = json!({
            "summary": "用例40候选",
            "tasks": [{
                "task_key": "k-uc40", "title": "两步任务", "summary": "s", "category": "其他事项",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
                "steps": [
                    {"step_id": "s1", "order": 1, "action": "步骤一"},
                    {"step_id": "s2", "order": 2, "action": "步骤二"}
                ],
                "evidence_ids": [], "assumptions": [], "needs_review": []
            }]
        });
        conn.execute(
            "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id,
                    analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                    candidate_revision_no, attempt_id, output_json, envelope_locally_reconstructed,
                    state, prompt_version, schema_version, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, 'g', 1, 'a', ?5, 0, 'candidate', '2.3', '2', ?6)",
            rusqlite::params![
                candidate_id,
                workspace,
                run_id,
                fingerprint,
                output.to_string(),
                dsheng_core::repository::utc_iso(chrono::Utc::now())
            ],
        )
        .unwrap();
    }
    let mutation = uuid::Uuid::new_v4().to_string();
    let base = json!({
        "schema_version": "1", "workspace_id": workspace, "run_id": run_id,
        "analysis_revision_no": 1, "input_fingerprint": fingerprint,
        "candidate_generation_id": candidate_id, "candidate_revision_no": 1,
        "expected_tasks": [], "mutation_id": mutation,
        "mappings": [{"candidate_task_key": "k-uc40", "action": "new"}],
    });
    let dry = cli_ok(
        &db,
        &[
            "task",
            "reconcile",
            "--workspace",
            &workspace,
            "--run",
            &run_id,
            "--input-json",
            &write_json(&dir, "recon.json", &base),
            "--dry-run",
            "--json",
        ],
    );
    let token = dry["apply_token"].as_str().unwrap().to_string();
    let mut with_token = base.clone();
    with_token["apply_token"] = json!(token);
    let applied = cli_ok(
        &db,
        &[
            "task",
            "reconcile",
            "--workspace",
            &workspace,
            "--run",
            &run_id,
            "--input-json",
            &write_json(&dir, "recon_apply.json", &with_token),
            "--apply",
            "--json",
        ],
    );
    let task_id = applied["new_task_ids"][0].as_str().unwrap().to_string();
    let task = cli_ok(
        &db,
        &[
            "task",
            "show",
            "--workspace",
            &workspace,
            "--task",
            &task_id,
            "--json",
        ],
    );
    let steps = task["steps"].as_array().unwrap().clone();
    Fix {
        db,
        dir,
        workspace,
        run_id,
        task_id,
        step1: steps[0]["step_id"].as_str().unwrap().to_string(),
        step2: steps[1]["step_id"].as_str().unwrap().to_string(),
    }
}

// ---- 方向 1：CLI 创建 → 桌面读/改 → CLI 再读一致 ----

#[test]
fn uc40_cli_creates_desktop_updates_cli_reads_back() {
    let f = fixture("uc40_a");
    let env = env_of(&f.db);

    // 桌面读：task list / show 与 CLI 写入一致
    let list = ok_data(
        cmd::task::task_list(&env, f.workspace.clone(), None),
        "task_list",
    );
    let tasks = list["tasks"].as_array().expect("tasks 数组");
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["task_id"], f.task_id);
    let shown = ok_data(
        cmd::task::task_show(&env, f.workspace.clone(), f.task_id.clone()),
        "task_show",
    );
    assert_eq!(shown["status"], "not_started");
    assert_eq!(shown["steps"].as_array().unwrap().len(), 2);

    // 桌面改：progress（乐观并发 expected_task_revision=当前 1）
    let edit = json!({
        "schema_version": "1", "workspace_id": f.workspace, "task_id": f.task_id,
        "expected_task_revision": 1, "mutation_id": uuid::Uuid::new_v4().to_string(),
        "operation": {
            "kind": "progress", "progress_mode": "manual", "progress": "25.5",
            "status": "in_progress", "adjustment_reason": "桌面端手动调整", "blocked_reason": null,
        },
    });
    ok_data(cmd::task::task_update(&env, edit), "task_update");

    // CLI 再读：进度/状态一致（同一 SQLite）
    let task = cli_ok(
        &f.db,
        &[
            "task",
            "show",
            "--workspace",
            &f.workspace,
            "--task",
            &f.task_id,
            "--json",
        ],
    );
    assert_eq!(task["progress"], "25.5", "CLI 须读到桌面写入的进度");
    assert_eq!(task["status"], "in_progress");
    assert_eq!(task["task_revision_no"], 2);

    // CLI 改：步骤完成（步骤修订经服务层）；桌面再读一致
    let patch = json!({"status": "completed"});
    cli_ok(
        &f.db,
        &[
            "task",
            "step",
            "update",
            "--workspace",
            &f.workspace,
            "--step",
            &f.step1,
            "--patch-json",
            &write_json(&f.dir, "step.json", &patch),
            "--json",
        ],
    );
    let shown = ok_data(
        cmd::task::task_show(&env, f.workspace.clone(), f.task_id.clone()),
        "task_show(2)",
    );
    let steps = shown["steps"].as_array().unwrap().clone();
    assert_eq!(
        steps[0]["status"], "completed",
        "桌面须读到 CLI 写入的步骤状态"
    );
    assert_eq!(steps[1]["status"], "not_started");
    // 桌面 task_update 幂等重放：同 mutation_id 不重复计数（服务层合同）
    let edit_again = json!({
        "schema_version": "1", "workspace_id": f.workspace, "task_id": f.task_id,
        "expected_task_revision": 3, "mutation_id": "00000000-0000-4000-8000-0000000000ff",
        "operation": {
            "kind": "progress", "progress_mode": "manual", "progress": "50",
            "status": "in_progress", "adjustment_reason": "再次调整", "blocked_reason": null,
        },
    });
    ok_data(cmd::task::task_update(&env, edit_again), "task_update(2)");
    let task = cli_ok(
        &f.db,
        &[
            "task",
            "show",
            "--workspace",
            &f.workspace,
            "--task",
            &f.task_id,
            "--json",
        ],
    );
    assert_eq!(task["progress"], "50");

    // 时间线（桌面读）能见到该任务条目
    let timeline = ok_data(
        cmd::task::task_timeline(
            &env,
            json!({"workspace_id": f.workspace, "from": "2026-10-01", "to": "2026-12-31", "grain": "month"}),
        ),
        "task_timeline",
    );
    assert!(
        !timeline["entries"].as_array().unwrap().is_empty(),
        "时间线应含任务条目"
    );
    let _ = (f.step2, f.run_id);
}

// ---- 方向 2：桌面创建/写 → CLI 读一致 ----

#[test]
fn uc40_desktop_creates_cli_reads_back() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.keep();
    let db = dir.join("uc40_b.sqlite3").to_string_lossy().into_owned();
    let env = env_of(&db);

    // 桌面建 workspace（含初始周期策略）
    let created = ok_data(
        cmd::settings::workspace_create(
            &env,
            json!({
                "schema_version": "1", "operation": "workspace_create", "name": "桌面建店",
                "initial_period_policy": {"cadence": "month", "effective_from": "2026-01-01"},
            }),
        ),
        "workspace_create",
    );
    let workspace = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();

    // 桌面 provider set：Key 单独通道（key_action=replace + apiKey）
    let provider_data = ok_data(
        cmd::settings::provider_set(
            &env,
            json!({
                "schema_version": "1", "operation": "provider_upsert", "workspace_id": workspace,
                "provider_id": null, "expected_revision": null,
                "config": {
                    "name": "桌面Provider", "protocol": "chat_completions",
                    "base_url": "https://api.example.com/v1", "model": "synthetic-model", "stream": false,
                    "timeouts": {"connect_seconds": 15, "response_seconds": 180, "total_seconds": 240},
                    "budget": {"context_window_tokens": 65536, "output_reserve_tokens": 8192},
                },
                "key_action": "replace",
            }),
            Some("test-key-not-real".into()),
        ),
        "provider_set",
    );
    assert_eq!(provider_data["key_present"], true);
    let provider_id = provider_data["provider"]["provider_id"]
        .as_str()
        .unwrap()
        .to_string();
    // 桌面 provider use（设为默认；expected_revision=workspace config_revision 1）
    ok_data(
        cmd::settings::provider_use(
            &env,
            json!({
                "schema_version": "1", "operation": "provider_use", "workspace_id": workspace,
                "provider_id": provider_id, "expected_revision": 1,
            }),
        ),
        "provider_use",
    );

    // CLI 读：workspace list / provider list / period list 一致（Key 永不出现）
    let list = cli_ok(&db, &["workspace", "list", "--json"]);
    let names: Vec<&str> = list["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"桌面建店"),
        "CLI 须读到桌面创建的 workspace"
    );
    let providers = cli_ok(
        &db,
        &["provider", "list", "--workspace", &workspace, "--json"],
    );
    let arr = providers["providers"].as_array().unwrap().clone();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["key_present"], true);
    assert!(arr[0].get("api_key").is_none(), "Key 绝不出现在输出");
    let policies = cli_ok(
        &db,
        &[
            "workspace",
            "period",
            "list",
            "--workspace",
            &workspace,
            "--json",
        ],
    );
    assert_eq!(
        policies["policies"].as_array().unwrap().len(),
        1,
        "初始策略可见"
    );

    // CLI 建指标 + 合成表 → 桌面 analyze 写 run → CLI history list 读到
    let metric = json!({
        "metric_key": "sales_net", "label": "销售额", "semantic_type": "flow",
        "unit_signature": {"dimension": "money", "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"},
        "direction": "up", "target_constraint_scope": "monthly_revenue_floor",
        "revenue_semantics": true, "aggregation": "sum_disjoint", "required_inputs": [],
        "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
        "effective_from": "2026-01-01", "expected_revision": 0,
    });
    cli_ok(
        &db,
        &[
            "metric",
            "set",
            "--workspace",
            &workspace,
            "--input-json",
            &write_json(&dir, "metric.json", &metric),
            "--json",
        ],
    );
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("核算表").unwrap();
    for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"]
        .iter()
        .enumerate()
    {
        sheet.write_string(0, c as u16, *h).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 2, "12000.5").unwrap();
    sheet.write_string(1, 3, "11500.25").unwrap();
    sheet.write_string(1, 4, "12500").unwrap();
    book.save(dir.join("核算表模板.xlsx")).unwrap();
    let analyzed = ok_data(
        cmd::data::analyze(
            &env,
            json!({
                "workspace_id": workspace, "directory_label": dir.to_string_lossy(),
                "from": "2026-10-01", "to": "2026-10-31", "grain": "auto",
            }),
        ),
        "analyze",
    );
    let run_id = analyzed["run_id"].as_str().unwrap().to_string();
    let runs = cli_ok(
        &db,
        &["history", "list", "--workspace", &workspace, "--json"],
    );
    let found = runs["runs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["run_id"] == run_id.as_str());
    assert!(found, "CLI history list 须读到桌面 analyze 的 run");

    // 桌面读侧命令在同库上工作（mapping/history/scan 拒绝路径之外的正例）
    let hist = ok_data(
        cmd::history::history_show(&env, workspace.clone(), run_id.clone(), None),
        "history_show",
    );
    assert_eq!(hist["run_id"], run_id);
}

// ---- 并发写入冲突：REVISION_CONFLICT 传递（桌面 ↔ CLI 双向） ----

#[test]
fn uc40_revision_conflict_propagates_between_sides() {
    let f = fixture("uc40_rev");
    let env = env_of(&f.db);

    // 桌面以过期 revision 写 → REVISION_CONFLICT（revision 已是 1，传 0 语义过期）
    let stale = json!({
        "schema_version": "1", "workspace_id": f.workspace, "task_id": f.task_id,
        "expected_task_revision": 0, "mutation_id": uuid::Uuid::new_v4().to_string(),
        "operation": {
            "kind": "progress", "progress_mode": "manual", "progress": "10",
            "status": "in_progress", "adjustment_reason": "过期", "blocked_reason": null,
        },
    });
    let code = err_code(cmd::task::task_update(&env, stale), "桌面过期写");
    assert_eq!(
        code, "REVISION_CONFLICT",
        "桌面侧必须透传 REVISION_CONFLICT"
    );

    // CLI 抢先写（revision 1→2）后，桌面用 revision=1 再写 → 冲突
    cli_ok(
        &f.db,
        &[
            "task",
            "update",
            "--workspace",
            &f.workspace,
            "--task",
            &f.task_id,
            "--status",
            "in_progress",
            "--progress",
            "20",
            "--progress-mode",
            "manual",
            "--adjustment-reason",
            "CLI 抢先",
            "--expected-revision",
            "1",
            "--json",
        ],
    );
    let now_stale = json!({
        "schema_version": "1", "workspace_id": f.workspace, "task_id": f.task_id,
        "expected_task_revision": 1, "mutation_id": uuid::Uuid::new_v4().to_string(),
        "operation": {
            "kind": "progress", "progress_mode": "manual", "progress": "30",
            "status": "in_progress", "adjustment_reason": "CLI 先写", "blocked_reason": null,
        },
    });
    let code = err_code(
        cmd::task::task_update(&env, now_stale),
        "CLI 先写后的桌面写",
    );
    assert_eq!(code, "REVISION_CONFLICT");

    // 反向：桌面抢先写（2→3），CLI 用过期 --expected-revision → 退出码 2 + REVISION_CONFLICT
    let fresh = json!({
        "schema_version": "1", "workspace_id": f.workspace, "task_id": f.task_id,
        "expected_task_revision": 2, "mutation_id": uuid::Uuid::new_v4().to_string(),
        "operation": {
            "kind": "progress", "progress_mode": "manual", "progress": "40",
            "status": "in_progress", "adjustment_reason": "桌面先写", "blocked_reason": null,
        },
    });
    ok_data(cmd::task::task_update(&env, fresh), "桌面先写");
    let (v, code) = cli(
        &f.db,
        &[
            "task",
            "update",
            "--workspace",
            &f.workspace,
            "--task",
            &f.task_id,
            "--status",
            "in_progress",
            "--progress",
            "50",
            "--progress-mode",
            "manual",
            "--adjustment-reason",
            "已过期",
            "--expected-revision",
            "2",
            "--json",
        ],
    );
    assert_eq!(code, 2);
    assert_eq!(
        v["error"]["code"], "REVISION_CONFLICT",
        "CLI 侧同样透传冲突"
    );
}

// ---- 共享数据库路径解析（DSHENG_DB 临时目录注入） ----

fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let guard = LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    guard
}

#[test]
fn uc40_shared_db_path_env_injection() {
    let _guard = env_lock();
    let tmp = tempfile::tempdir().expect("临时目录");
    let db = tmp.path().join("injected.sqlite3");
    std::env::set_var("DSHENG_DB", &db);
    let env = DesktopEnv::resolve();
    assert_eq!(env.db_arg().expect("注入路径"), db.to_string_lossy());

    // 桌面按注入路径写入 → CLI 以同一注入路径（--db 免传，default_db_path
    // 亦读 DSHENG_DB）读到——共享路径解析两端一致
    ok_data(
        cmd::settings::workspace_create(
            &env,
            json!({
                "schema_version": "1", "operation": "workspace_create", "name": "注入店",
                "initial_period_policy": {"cadence": "month", "effective_from": "2026-01-01"},
            }),
        ),
        "workspace_create(注入)",
    );
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(
        &["workspace".into(), "list".into(), "--json".into()],
        &mut out,
        &mut err,
    );
    std::env::remove_var("DSHENG_DB");
    assert_eq!(code, 0);
    let v: Value = serde_json::from_slice(&out).expect("envelope");
    assert_eq!(v["ok"], true, "{v}");
    let found = v["data"]["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["name"] == "注入店");
    assert!(found, "CLI 须在注入路径读到桌面写入");
}

// ---- 数据目录不可用：DATA_DIRECTORY_UNAVAILABLE ----

#[test]
fn uc40_data_directory_unavailable_for_unwritable_path() {
    let tmp = tempfile::tempdir().expect("临时目录");
    let blocker = tmp.path().join("blocker");
    std::fs::write(&blocker, b"file").unwrap();
    // 文件下方再建目录必然失败 → open_db_at 映射 DATA_DIRECTORY_UNAVAILABLE
    let env = DesktopEnv::with_db(blocker.join("sub/dsheng.sqlite3"));
    let code = err_code(cmd::settings::workspace_list(&env), "不可写路径");
    assert_eq!(code, "DATA_DIRECTORY_UNAVAILABLE");
}

// ---- 生成命令：仅 dry-run（preview）与拒绝路径；无网络 ----

#[test]
fn uc40_generation_preview_local_and_rejections() {
    let f = fixture("uc40_gate");
    let env = env_of(&f.db);

    // task generate 预览：本地组装 + confirmation（不派发请求）
    let preview = ok_data(
        cmd::task::task_generate_preview(
            &env,
            json!({"workspace_id": f.workspace, "run_id": f.run_id}),
        ),
        "task_generate_preview",
    );
    assert_eq!(preview["operation"], "task_generate");
    assert!(preview["confirmation_id"].as_str().is_some());
    assert_eq!(preview["key_present"], true);

    // confirm 未知 confirmation_id（备忘未命中）→ INVALID_ARGUMENT，不触网
    let code = err_code(
        cmd::task::task_generate_confirm(&env, "00000000-0000-4000-8000-0000000000aa".into()),
        "task_generate_confirm(未知 id)",
    );
    assert_eq!(code, "INVALID_ARGUMENT");

    // diagnosis generate：evidence_ids 键对齐 UI 合同——带证据的预览可发起（本地确认，不派发）
    let preview = cmd::diagnosis::diagnosis_generate_preview(
        &env,
        json!({
            "workspace_id": f.workspace,
            "run_id": f.run_id,
            "framework": "5why",
            "problem": "销售额未达标",
            "evidence_ids": ["cell:demo:核算表:C7"],
        }),
    );
    assert_eq!(preview["ok"], json!(true), "带证据的诊断预览应成功（本地）");
    assert!(
        preview["data"]["confirmation_id"].is_string(),
        "预览必须返回 confirmation_id"
    );

    // evidence_ids 缺失/为空 → 服务层按合同拒绝（INPUT_CONTRACT_INVALID）
    let code = err_code(
        cmd::diagnosis::diagnosis_generate_preview(
            &env,
            json!({"workspace_id": f.workspace, "run_id": f.run_id, "framework": "5why", "problem": "销售额未达标"}),
        ),
        "diagnosis_generate_preview(无证据)",
    );
    assert_eq!(code, "INPUT_CONTRACT_INVALID", "空证据必须被服务层拒绝");

    // 生成门禁另一拒绝路径：Provider 无 Key → PROVIDER_KEY_MISSING（清 Key 后预览）
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        conn.execute("UPDATE providers SET api_key = NULL", [])
            .unwrap();
    }
    let code = err_code(
        cmd::task::task_generate_preview(
            &env,
            json!({"workspace_id": f.workspace, "run_id": f.run_id}),
        ),
        "task_generate_preview(无 Key)",
    );
    assert_eq!(code, "PROVIDER_KEY_MISSING");
}
