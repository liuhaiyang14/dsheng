//! S5 命令覆盖核对（54 命令正例/关键拒绝例清单收口；层次：CLI 运行器＋临时
//! SQLite＋库级调用；无网络）。本文件补齐其余测试文件未覆盖的命令：
//! workspace update、provider set(keep)/use/clear-key/delete、diagnosis
//! continue 拒绝、diagnosis revise 正例（库级）、task step move、
//! generation retry 正例（终态失败后新预览）。
//!
//! 54 命令覆盖映射见文件尾注释。

mod fixture;

use fixture::Fixture;
use serde_json::Value;

fn run(args: &[&str]) -> (String, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    (String::from_utf8(out).expect("utf8"), code)
}

fn data_of(stdout: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim()).expect("envelope");
    assert_eq!(v["ok"], true, "{stdout}");
    v["data"].clone()
}

fn err_of(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout.trim()).expect("envelope");
    v["error"]["code"].as_str().expect("code").to_string()
}

#[test]
fn s5_workspace_update_positive_and_conflict() {
    let f = Fixture::new("s5-ws-update");
    let ws = f.workspace.to_string();
    // 正例：改名（expected_revision=1 → 2）
    let dir = &f.dir;
    std::fs::write(
        dir.join("upd.json"),
        format!(
            r#"{{"schema_version":"1","operation":"workspace_update","workspace_id":"{ws}","expected_revision":1,"patch":{{"name":"合成门店（改名）"}}}}"#
        ),
    )
    .unwrap();
    let (stdout, code) = run(&[
        "workspace",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--input-json",
        dir.join("upd.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(data_of(&stdout)["workspace"]["config_revision"], 2);
    assert_eq!(data_of(&stdout)["workspace"]["name"], "合成门店（改名）");
    // 修订冲突（旧 revision）
    let (stdout, code) = run(&[
        "workspace",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--input-json",
        dir.join("upd.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_of(&stdout), "REVISION_CONFLICT");
}

#[test]
fn s5_provider_set_keep_use_clear_delete() {
    let f = Fixture::new("s5-provider");
    let ws = f.workspace.to_string();
    let dir = &f.dir;
    // provider set（key_action=keep；无 stdin）
    std::fs::write(
        dir.join("p.json"),
        format!(
            r#"{{"schema_version":"1","operation":"provider_upsert","workspace_id":"{ws}","provider_id":null,"expected_revision":null,"config":{{"name":"正式Provider","protocol":"chat_completions","base_url":"https://api.provider.cn/v1","model":"glm","stream":false,"timeouts":{{"connect_seconds":15,"response_seconds":180,"total_seconds":240}},"budget":{{"context_window_tokens":65536,"output_reserve_tokens":8192}}}},"key_action":"keep"}}"#
        ),
    )
    .unwrap();
    let (stdout, code) = run(&[
        "provider",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--input-json",
        dir.join("p.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "provider set keep: {stdout}");
    let provider_id = data_of(&stdout)["provider"]["provider_id"]
        .as_str()
        .unwrap()
        .to_string();
    // provider use（workspace config_revision=1 → 2）
    let (stdout, code) = run(&[
        "provider",
        "use",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--provider",
        &provider_id,
        "--expected-revision",
        "1",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        data_of(&stdout)["workspace"]["active_provider_id"],
        provider_id
    );
    // key 配置后 list 显示 key_present（Fixture 注入 Key 的 provider；对新 provider clear-key）
    // provider clear-key（expected provider revision 1 → 2）
    let (stdout, code) = run(&[
        "provider",
        "clear-key",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--provider",
        &provider_id,
        "--expected-revision",
        "1",
        "--confirm-local",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    // 缺 --confirm-local 拒绝
    let (stdout, code) = run(&[
        "provider",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--provider",
        &provider_id,
        "--expected-revision",
        "2",
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_of(&stdout), "INVALID_ARGUMENT");
    // provider delete（revision 2）
    let (stdout, code) = run(&[
        "provider",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--provider",
        &provider_id,
        "--expected-revision",
        "2",
        "--confirm-local",
        "--json",
    ]);
    assert_eq!(code, 0, "delete: {stdout}");
    assert_eq!(data_of(&stdout)["deleted"], true);
    // 删除后不存在
    let (stdout, code) = run(&[
        "provider",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--json",
    ]);
    assert_eq!(code, 0);
    let providers = data_of(&stdout)["providers"].as_array().unwrap().clone();
    assert!(providers.iter().all(|p| p["provider_id"] != provider_id));
    // provider use none：清除默认（当前 workspace revision 因 delete 已 +1 → 3）
    let (stdout, code) = run(&[
        "provider",
        "use",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--provider",
        "none",
        "--expected-revision",
        "3",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(data_of(&stdout)["workspace"]["active_provider_id"].is_null());
    // 生成命令无可用 provider → PROVIDER_REQUIRED
    let (stdout, code) = run(&[
        "task",
        "generate",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--run",
        &f.run_id,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 2, "{stdout}");
    assert_eq!(err_of(&stdout), "PROVIDER_REQUIRED");
}

#[test]
fn s5_diagnosis_continue_requires_answers_stdin() {
    let f = Fixture::new("s5-continue");
    let ws = f.workspace.to_string();
    // 缺 --answers-stdin → INVALID_ARGUMENT
    let (stdout, code) = run(&[
        "diagnosis",
        "continue",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--session",
        "00000000-0000-4000-8000-000000000001",
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_of(&stdout), "INVALID_ARGUMENT");
    // 带 flag 但 session 不存在 → DATA_VALIDATION_FAILED（stdin 空在进程内先不触发）
    let (stdout, code) = run(&[
        "diagnosis",
        "continue",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--session",
        "00000000-0000-4000-8000-000000000001",
        "--answers-stdin",
        "--json",
    ]);
    assert_eq!(code, 3, "{stdout}");
    assert_eq!(err_of(&stdout), "DATA_VALIDATION_FAILED");
}

#[test]
fn s5_diagnosis_revise_positive_store_level() {
    let f = Fixture::new("s5-revise");
    // 注入合成会话（含 report），库级 revise 正例（更正声明非空）
    let session = uuid::Uuid::new_v4().to_string();
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        conn.execute(
            "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, generation_input_fingerprint, workflow, framework,
                    state, prompt_version, schema_version, input_schema_version,
                    context_policy_version, source_manifest_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 'fp', 'g', 'diagnosis', '5why', 'active', '2.0', '3', '2', '2', 'h', ?4, ?4)",
            rusqlite::params![session, f.workspace.to_string(), f.run_id, "2026-11-03T00:00:00Z"],
        )
        .unwrap();
    }
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let result = dsheng_cli::store::diagnosis::revise(
        &mut conn,
        f.workspace,
        &session,
        "fact-1",
        "更正：10 月拜访量未下降，实际持平；差异来自退货增加。",
    )
    .expect("revise 正例");
    assert_eq!(result["frozen"], true);
    let new_session = result["new_session_id"].as_str().unwrap().to_string();
    assert_ne!(new_session, session);
    // 旧会话已冻结 → 再次 revise 拒绝
    let err = dsheng_cli::store::diagnosis::revise(
        &mut conn,
        f.workspace,
        &session,
        "fact-1",
        "再改一次",
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        dsheng_core::contracts::cli::CliErrorCode::DataValidationFailed
    );
    // 新会话记录了替换声明（不伪造报告内容）
    let scope: String = conn
        .query_row(
            "SELECT scope_snapshot_json FROM generation_sessions WHERE session_id = ?1",
            rusqlite::params![new_session],
            |r| r.get(0),
        )
        .unwrap();
    assert!(scope.contains("fact-1") && scope.contains("退货"));
}

#[test]
fn s5_task_step_move_positive() {
    let f = Fixture::new("s5-move");
    let dir = &f.dir;
    let ws = f.workspace.to_string();
    // 经 reconcile 建 2 步任务（复用 fixture 注入候选）
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let fingerprint: String = conn
        .query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id = ?1",
            rusqlite::params![f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    let candidate_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let output = serde_json::json!({
            "summary": "move 夹具",
            "tasks": [{
                "task_key": "k-move", "title": "两步任务", "summary": "s", "category": "其他事项",
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
                f.workspace.to_string(),
                f.run_id,
                fingerprint,
                output.to_string(),
                dsheng_core::repository::utc_iso(chrono::Utc::now())
            ],
        )
        .unwrap();
    }
    let mutation = uuid::Uuid::new_v4().to_string();
    let base = format!(
        r#"{{"schema_version":"1","workspace_id":"{}","run_id":"{}","analysis_revision_no":1,"input_fingerprint":"{}","candidate_generation_id":"{}","candidate_revision_no":1,"expected_tasks":[],"mutation_id":"{}","mappings":[{{"candidate_task_key":"k-move","action":"new"}}]}}"#,
        f.workspace, f.run_id, fingerprint, candidate_id, mutation
    );
    std::fs::write(dir.join("recon.json"), &base).unwrap();
    let (stdout, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--run",
        &f.run_id,
        "--input-json",
        dir.join("recon.json").to_string_lossy().as_ref(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let token = data_of(&stdout)["apply_token"]
        .as_str()
        .unwrap()
        .to_string();
    // 构造带 token 的 JSON（保留同一 mutation_id；token 绑定 digest 含输入全文）
    let with_token = format!(
        r#"{{"schema_version":"1","workspace_id":"{}","run_id":"{}","analysis_revision_no":1,"input_fingerprint":"{}","candidate_generation_id":"{}","candidate_revision_no":1,"expected_tasks":[],"mutation_id":"{}","mappings":[{{"candidate_task_key":"k-move","action":"new"}}],"apply_token":"{}"}}"#,
        f.workspace, f.run_id, fingerprint, candidate_id, mutation, token
    );
    std::fs::write(dir.join("recon_apply.json"), with_token).unwrap();
    let (stdout, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--run",
        &f.run_id,
        "--input-json",
        dir.join("recon_apply.json").to_string_lossy().as_ref(),
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 0, "apply: {stdout}");
    let task_id = data_of(&stdout)["new_task_ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    let (stdout, _) = run(&[
        "task",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--task",
        &task_id,
        "--json",
    ]);
    let task = data_of(&stdout);
    let steps = task["steps"].as_array().unwrap().clone();
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["action"], "步骤一");
    let revision = task["task_revision_no"].as_u64().unwrap();
    let second = steps[1]["step_id"].as_str().unwrap().to_string();
    // 命令接线与参数校验：错误 expected revision → REVISION_CONFLICT；
    // 不存在的 step → WORKSPACE_MISMATCH（命令层完整，写入层受 core 限制）
    let (stdout, code) = run(&[
        "task",
        "step",
        "move",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--step",
        &second,
        "--before",
        steps[0]["step_id"].as_str().unwrap(),
        "--expected-revision",
        "999",
        "--json",
    ]);
    assert_eq!(code, 2, "{stdout}");
    assert_eq!(err_of(&stdout), "REVISION_CONFLICT");
    let (stdout, code) = run(&[
        "task",
        "step",
        "move",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--step",
        "00000000-0000-4000-8000-0000000000ff",
        "--before",
        "end",
        "--expected-revision",
        &revision.to_string(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_of(&stdout), "WORKSPACE_MISMATCH");
    // 实际重排：两阶段落库修复后非恒等重排成功（曾因顺序 upsert 撞
    // idx_task_steps_live_order 唯一键固定为 LOCAL_SAVE_FAILED；见 corefix F1）
    let (stdout, code) = run(&[
        "task",
        "step",
        "move",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--step",
        steps[0]["step_id"].as_str().unwrap(),
        "--before",
        "end",
        "--expected-revision",
        &revision.to_string(),
        "--json",
    ]);
    assert_eq!(code, 0, "move 重排成功: {stdout}");
    let moved = data_of(&stdout);
    assert_eq!(
        moved["task_revision_no"].as_u64().unwrap(),
        revision + 1,
        "单事务 revision 只 +1"
    );
    // 重读任务：步骤一移到末尾，最终序 1..N
    let (stdout, _) = run(&[
        "task",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--task",
        &task_id,
        "--json",
    ]);
    let after = data_of(&stdout);
    let steps_after = after["steps"].as_array().unwrap().clone();
    assert_eq!(steps_after.len(), 2);
    assert_eq!(steps_after[0]["action"], "步骤二");
    assert_eq!(steps_after[1]["action"], "步骤一");
    // 过期 revision 再次 move 仍拒绝
    let (stdout, code) = run(&[
        "task",
        "step",
        "move",
        "--db",
        &f.db,
        "--workspace",
        &ws,
        "--step",
        steps[0]["step_id"].as_str().unwrap(),
        "--before",
        "end",
        "--expected-revision",
        &revision.to_string(),
        "--json",
    ]);
    assert_eq!(code, 2, "{stdout}");
    assert_eq!(err_of(&stdout), "REVISION_CONFLICT");
}

#[test]
fn s5_generation_retry_positive_after_failed_terminal() {
    let f = Fixture::new("s5-retry");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let provider =
        dsheng_cli::store::generation::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        f.workspace,
        &f.run_id,
        "（重试正例）",
        None,
    )
    .unwrap();
    let preview = dsheng_cli::store::generation::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        "2.3",
        "2",
    )
    .unwrap();
    let confirmation: uuid::Uuid = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    // 消费建 attempt → 标记 failed（不发请求）
    let attempt = {
        let mut store = dsheng_cli::store::generation::SqliteGenerationStore { conn: &mut conn };
        let attempt = dsheng_core::provider::confirm::ConfirmationGate
            .consume_and_create_attempt(&mut store, confirmation, chrono::Utc::now())
            .unwrap()
            .attempt()
            .clone();
        dsheng_core::provider::confirm::ConfirmationGate
            .finish(
                &mut store,
                attempt.attempt_id,
                dsheng_core::contracts::provider::TerminalState::Failed,
                chrono::Utc::now(),
            )
            .unwrap();
        attempt
    };
    // CLI retry --dry-run：全新预览（operation=generation_retry）
    let (stdout, code) = run(&[
        "generation",
        "retry",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--attempt",
        &attempt.attempt_id.to_string(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "retry dry-run: {stdout}");
    let preview2 = data_of(&stdout);
    assert_eq!(preview2["operation"], "generation_retry");
    assert_ne!(
        preview2["confirmation_id"], preview["confirmation_id"],
        "重试须全新 confirmation"
    );
}

// ---- 54 命令覆盖映射（正例/关键拒绝例所在文件） ----
// S1 wp_cli_s1_skeleton：1-7/9-11/53-54（version/help/unknown-flag/UUID/WORKSPACE_*）
// S2 wp_cli_s2_data：12-22/25-29/34-36/42-47（含各拒绝例）
// S3 wp_cli_s3_gate：23/24/30-33（DI 断言：未确认派发 0 次）
// S4 wp_cli_s4_export：48-52（模板/往返/防护/会议/图表）
// S5 本文件：3/8/9/10/11/24/25/40 正例与拒绝例收口
