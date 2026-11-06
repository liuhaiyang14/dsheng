//! S3 门禁测试（测试层次：库级依赖注入＋内存记录 dispatcher；无网络、无 Mock
//! Provider/假 endpoint——记录器只拒绝派发或返回错误，不伪造模型响应）。
//!
//! 断言：
//! - 任何未确认路径（预览/无 flag/错误 confirmation-id/快照变化）派发次数为 0；
//! - 双门禁齐备且快照逐项一致时，恰好派发 1 次（provider::http 仅在生产
//!   HttpDispatch 内调用；本文件永不触网）；
//! - 派发失败 → attempt 终态 failed；重试需全新确认。

mod fixture;

use fixture::Fixture;
use serde_json::json;
use std::cell::Cell;

use dsheng_cli::store::generation::{
    self as gen, GenerationMaterial, ProviderRuntime, RequestDispatch,
};

/// 记录 dispatcher：计数派发；按构造参数决定返回错误（永不伪造成功响应）。
struct Recorder {
    calls: Cell<usize>,
    fail_with: Option<dsheng_core::contracts::cli::CliErrorCode>,
}

impl RequestDispatch for Recorder {
    fn dispatch(
        &mut self,
        _runtime: &ProviderRuntime,
        _request: &dsheng_core::contracts::provider::ChatCompletionsRequest,
        _on_chunk: &mut dyn FnMut(dsheng_core::provider::http::ModelProgress),
    ) -> Result<
        dsheng_core::contracts::provider::ChatCompletionsResponse,
        (dsheng_core::contracts::cli::CliErrorCode, String),
    > {
        self.calls.set(self.calls.get() + 1);
        Err((
            self.fail_with
                .unwrap_or(dsheng_core::contracts::cli::CliErrorCode::NetworkError),
            "测试注入：拒绝派发（依赖注入层）".into(),
        ))
    }
}

fn material_of_fixture(f: &Fixture) -> GenerationMaterial {
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    gen::build_task_payload(&conn, f.workspace, &f.run_id, "（S3 门禁测试）", None)
        .expect("任务输入组装")
}

#[test]
fn s3_preview_creates_confirmation_without_dispatch() {
    let f = Fixture::new("gate-preview");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let material = material_of_fixture(&f);
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .expect("预览成功");
    assert!(preview["confirmation_id"]
        .as_str()
        .is_some_and(|s| s.len() == 36));
    assert_eq!(preview["request_count_maximum"], 1);
    assert_eq!(preview["key_present"], true);
    assert!(preview["request_bytes"].as_u64().unwrap() > 0);
    // 未确认派发计数为 0（本测试根本没有 dispatcher——预览路径不可达网络）
    let attempts: i64 = conn
        .query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(attempts, 0, "预览不得创建 attempt");
}

#[test]
fn s3_unconfirmed_send_paths_never_dispatch() {
    let f = Fixture::new("gate-nodispatch");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let material = material_of_fixture(&f);
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let confirmation_id: uuid::Uuid = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut recorder = Recorder {
        calls: Cell::new(0),
        fail_with: Some(dsheng_core::contracts::cli::CliErrorCode::NetworkError),
    };
    // 1) 不存在的 confirmation → CONFIRMATION_REQUIRED，派发 0 次
    let err = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        uuid::Uuid::new_v4(),
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut recorder,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        dsheng_core::contracts::cli::CliErrorCode::ConfirmationRequired
    );
    assert_eq!(recorder.calls.get(), 0, "确认不存在时绝不派发");
    // 2) 快照变化（篡改 material 输入哈希）→ CONFIRMATION_STALE，派发 0 次
    let mut drifted = material_of_fixture(&f);
    drifted.input_hash = format!("{}x", drifted.input_hash);
    let err = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &drifted,
        confirmation_id,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut recorder,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        dsheng_core::contracts::cli::CliErrorCode::ConfirmationStale,
        "发送前逐项重验必须拦住漂移: {err}"
    );
    assert_eq!(recorder.calls.get(), 0, "快照漂移时绝不派发");
}

#[test]
fn s3_confirmed_gate_dispatches_exactly_once_and_marks_terminal() {
    let f = Fixture::new("gate-send");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let material = material_of_fixture(&f);
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let confirmation_id: uuid::Uuid = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut recorder = Recorder {
        calls: Cell::new(0),
        fail_with: Some(dsheng_core::contracts::cli::CliErrorCode::NetworkError),
    };
    // 双门禁齐备：恰好派发一次；网络错误 → attempt 终态 failed
    let err = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        confirmation_id,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut recorder,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        dsheng_core::contracts::cli::CliErrorCode::NetworkError
    );
    assert_eq!(recorder.calls.get(), 1, "双门禁后恰好一次派发（上限 1）");
    let terminal: Option<String> = conn
        .query_row(
            "SELECT terminal_state FROM generation_attempts LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(terminal.as_deref(), Some("failed"));
    let request_count: i64 = conn
        .query_row(
            "SELECT request_count FROM generation_attempts LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(request_count, 1, "每动作最多 1 条请求");
    // 重复消费同一确认：返回已有 attempt（不派发第二条）
    let mut second = Recorder {
        calls: Cell::new(0),
        fail_with: None,
    };
    let err2 = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        confirmation_id,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut second,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(
        err2.code,
        dsheng_core::contracts::cli::CliErrorCode::IdempotencyConflict,
        "已终态 attempt 不得再次派发: {err2}"
    );
    assert_eq!(second.calls.get(), 0, "重复消费不产生第二条请求");
}

#[test]
fn s3_cli_level_gate_rejections() {
    let f = Fixture::new("gate-cli");
    let owned = |args: &[&str]| -> Vec<String> { args.iter().map(|s| s.to_string()).collect() };
    let run_cli = |args: &[&str]| {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = dsheng_cli::run_with_output(&owned(args), &mut out, &mut err);
        (String::from_utf8(out).unwrap(), code)
    };
    let run = f.run_id.clone();
    let ws = f.workspace.to_string();
    let db = f.db.clone();
    // 无 Key 的 provider → PROVIDER_KEY_MISSING（发送前本地拒绝）
    let nokey_provider = {
        let conn = rusqlite::Connection::open(&db).unwrap();
        let id = uuid::Uuid::new_v4();
        conn.execute(
            "INSERT INTO providers (provider_id, workspace_id, revision, name, protocol, base_url,
                    model, stream, api_key, connect_seconds, response_seconds, total_seconds,
                    context_window_tokens, output_reserve_tokens, created_at, updated_at)
             VALUES (?1, ?2, 1, '无Key', 'chat_completions', 'https://api.example.com/v1', 'm', 0,
                     NULL, 15, 180, 240, 32768, 8192, ?3, ?3)",
            rusqlite::params![id.to_string(), ws, "2026-11-03T00:00:00Z"],
        )
        .unwrap();
        id.to_string()
    };
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "manual:test",
        "--provider",
        &nokey_provider,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "PROVIDER_KEY_MISSING");

    // focus metric 不属 run → INPUT_CONTRACT_INVALID
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "manual:test",
        "--focus-metric",
        "not_in_run",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "INPUT_CONTRACT_INVALID");

    // 证据未关联 → INPUT_CONTRACT_INVALID
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "random-unlinked-evidence",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "INPUT_CONTRACT_INVALID");

    // 双门禁缺一：只有 --confirm-send → INVALID_ARGUMENT；伪 confirmation-id → CONFIRMATION_REQUIRED（不触网）
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "manual:test",
        "--confirm-send",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "INVALID_ARGUMENT");

    let bogus = uuid::Uuid::new_v4().to_string();
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "manual:test",
        "--confirmation-id",
        &bogus,
        "--confirm-send",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "CONFIRMATION_REQUIRED");

    // 正常 dry-run 走 CLI：返回 confirmationPreview
    let provider_id = f.provider.to_string();
    let (out, code) = run_cli(&[
        "diagnosis",
        "generate",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--run",
        &run,
        "--framework",
        "5why",
        "--problem",
        "销售额下滑",
        "--evidence",
        "manual:test",
        "--provider",
        &provider_id,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "dry-run 预览: {out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"]["workflow"], "diagnosis");
    assert!(v["data"]["expires_at"].as_str().is_some());
}

#[test]
fn s3_generation_status_cancel_and_retry_guards() {
    let f = Fixture::new("gate-status");
    // 经库级 DI 路径建一个未终态 attempt（consume 即建 attempt、不派发）
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let material = material_of_fixture(&f);
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let confirmation_id: uuid::Uuid = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let mut store = gen::SqliteGenerationStore { conn: &mut conn };
    let outcome = dsheng_core::provider::confirm::ConfirmationGate
        .consume_and_create_attempt(&mut store, confirmation_id, chrono::Utc::now())
        .expect("消费确认建 attempt");
    let attempt_id = outcome.attempt().attempt_id;
    drop(conn);

    let owned = |args: &[&str]| -> Vec<String> { args.iter().map(|s| s.to_string()).collect() };
    let run_cli = |args: &[&str]| {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = dsheng_cli::run_with_output(&owned(args), &mut out, &mut err);
        (String::from_utf8(out).unwrap(), code)
    };
    let ws = f.workspace.to_string();
    let db = f.db.clone();
    let aid = attempt_id.to_string();

    // status：非终态
    let (out, code) = run_cli(&[
        "generation",
        "status",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--attempt",
        &aid,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"]["request_count"], 0);
    assert!(v["data"]["terminal_state"].is_null());

    // 未终态不可重试
    let (out, code) = run_cli(&[
        "generation",
        "retry",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--attempt",
        &aid,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 2, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "IDEMPOTENCY_CONFLICT");

    // cancel（本地）→ 终态 cancelled
    let (out, code) = run_cli(&[
        "generation",
        "cancel",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--attempt",
        &aid,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["data"]["cancelled"], true);
    // 再次取消 → 拒绝
    let (out, code) = run_cli(&[
        "generation",
        "cancel",
        "--db",
        &db,
        "--workspace",
        &ws,
        "--attempt",
        &aid,
        "--json",
    ]);
    assert_eq!(code, 2);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["error"]["code"], "IDEMPOTENCY_CONFLICT");
}

#[test]
fn s3_task_generate_stale_diagnosis_reference_rejected() {
    let f = Fixture::new("gate-stale");
    let owned = |args: &[&str]| -> Vec<String> { args.iter().map(|s| s.to_string()).collect() };
    let mut out = Vec::new();
    let mut err = Vec::new();
    let args = owned(&[
        "task",
        "generate",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--diagnosis-report",
        "00000000-0000-4000-8000-0000000000aa",
        "--dry-run",
        "--json",
    ]);
    let code = dsheng_cli::run_with_output(&args, &mut out, &mut err);
    let stdout = String::from_utf8(out).unwrap();
    assert_eq!(code, 3, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        v["error"]["code"], "DATA_VALIDATION_FAILED",
        "报告不存在先于门禁"
    );
    // 存在但指纹不匹配（注入合成报告，模拟旧 revision）→ STALE_DIAGNOSIS_REFERENCE
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let session = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, generation_input_fingerprint, workflow, state,
                    prompt_version, schema_version, input_schema_version, source_manifest_hash,
                    created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 'stale-fp', 'gfp', 'task', 'active', '2.3', '2', '2', 'h', ?4, ?4)",
            rusqlite::params![session, f.workspace.to_string(), f.run_id, "2026-11-03T00:00:00Z"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diagnosis_reports (report_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, report_revision_no, session_id, framework,
                    problem_statement, report_json, state, created_at, updated_at)
             VALUES ('00000000-0000-4000-8000-0000000000aa', ?1, ?2, 1, 'stale-fp', 1, ?3, '5why',
                     'p', '{}', 'accepted', ?4, ?4)",
            rusqlite::params![
                f.workspace.to_string(),
                f.run_id,
                session,
                "2026-11-03T00:00:00Z"
            ],
        )
        .unwrap();
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    let args = owned(&[
        "task",
        "generate",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--diagnosis-report",
        "00000000-0000-4000-8000-0000000000aa",
        "--dry-run",
        "--json",
    ]);
    let code = dsheng_cli::run_with_output(&args, &mut out, &mut err);
    let stdout = String::from_utf8(out).unwrap();
    assert_eq!(code, 2, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["error"]["code"], "STALE_DIAGNOSIS_REFERENCE");
    // 不带报告的 dry-run 预览成功（confirmationPreview）
    let mut out = Vec::new();
    let mut err = Vec::new();
    let args = owned(&[
        "task",
        "generate",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--dry-run",
        "--json",
    ]);
    let code = dsheng_cli::run_with_output(&args, &mut out, &mut err);
    let stdout = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{stdout}");
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["data"]["workflow"], "task");
    assert_eq!(v["data"]["operation"], "task_generate");
}

#[test]
fn s3_candidate_saving_is_pure_local() {
    let f = Fixture::new("gate-save");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    let material = material_of_fixture(&f);
    // save_task_candidate：落库候选（输出原样；不经 Provider）
    let output =
        json!({"schema_version":"2", "summary":"合成候选（本地落库函数测试）", "tasks":[]});
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let confirmation_id = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let attempt_id = dsheng_core::provider::confirm::ConfirmationGate
        .consume_and_create_attempt(
            &mut gen::SqliteGenerationStore { conn: &mut conn },
            confirmation_id,
            chrono::Utc::now(),
        )
        .unwrap()
        .attempt()
        .attempt_id;
    let candidate_id = gen::save_task_candidate(
        &mut conn,
        f.workspace,
        &material,
        attempt_id,
        &output,
        false,
    )
    .unwrap();
    let state: String = conn
        .query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id = ?1",
            rusqlite::params![candidate_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "candidate");
    // 诊断轮次/完整报告落库覆盖见 wp_diagnosis_report_recovery.rs。
}
