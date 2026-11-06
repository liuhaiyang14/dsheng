//! S4/S6 流式派发接线测试（测试层次：依赖注入 dispatcher＋临时 SQLite；
//! 无网络、无 Mock Provider/假 endpoint——记录器只拒绝派发并回放进度，
//! 不伪造模型响应）。
//!
//! 断言：
//! - provider 快照 stream 标志从 DB（0003 列）正确投影：0→false / 1→true；
//! - `dispatch_path` 纯函数映射（HttpDispatch 分支依据：true→Streaming）；
//! - `send_confirmed` 把原始 chunk 三元组包装为
//!   `(attempt_id 关联, stage=receiving)` 事件交给消费端（DI 断言，不发网络）；
//! - ProviderRuntime.stream 随快照传递（Recording 捕获）；
//! - ProgressSink::Stderr 节流判定（纯函数：首条立即、30s 内不重复）；
//! - provider set 落库 stream=true（S1 放宽后的真实写路径）。

mod fixture;

use fixture::Fixture;
use std::cell::RefCell;

use dsheng_cli::store::generation::{
    self as gen, DispatchPath, GenerationMaterial, ProgressSink, ProviderRuntime, RequestDispatch,
    StreamProgressEvent,
};

fn material_of_fixture(f: &Fixture) -> GenerationMaterial {
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    gen::build_task_payload(&conn, f.workspace, &f.run_id, "（流式接线测试）", None)
        .expect("任务输入组装")
}

/// 记录 dispatcher：捕获 runtime.stream 与回放的进度三元组；派发恒拒绝
/// （依赖注入层，不触网、不伪造成功响应）。
struct RecordingDispatch {
    runtime_stream: RefCell<Option<bool>>,
}

impl RequestDispatch for RecordingDispatch {
    fn dispatch(
        &mut self,
        runtime: &ProviderRuntime,
        _request: &dsheng_core::contracts::provider::ChatCompletionsRequest,
        on_chunk: &mut dyn FnMut(dsheng_core::provider::http::ModelProgress),
    ) -> Result<
        dsheng_core::contracts::provider::ChatCompletionsResponse,
        (dsheng_core::contracts::cli::CliErrorCode, String),
    > {
        *self.runtime_stream.borrow_mut() = Some(runtime.stream);
        // 回放三块进度（模拟流式 chunk 到达；仅进度通道，无响应数据）
        for (chunk_seq, bytes, elapsed_ms, content_chars, reasoning_delta) in [
            (1, 128, 40, 0, "合成本地思考片段"),
            (2, 4_096, 30_050, 12, ""),
            (3, 9_216, 60_900, 12, ""),
        ] {
            on_chunk(dsheng_core::provider::http::ModelProgress {
                chunk_seq,
                bytes,
                elapsed_ms,
                content_chars,
                reasoning_delta: reasoning_delta.to_string(),
            });
        }
        Err((
            dsheng_core::contracts::cli::CliErrorCode::NetworkError,
            "测试注入：拒绝派发（依赖注入层）".into(),
        ))
    }
}

#[test]
fn provider_snapshot_projects_stream_flag_from_db() {
    let f = Fixture::new("stream-flag");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    // fixture 写入 stream=0
    let snapshot = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    assert!(!snapshot.stream, "stream=0 → 非流式");
    // DB 置 1（0003 放宽后合法）→ 快照 true → HttpDispatch 走流式分支
    conn.execute(
        "UPDATE providers SET stream = 1 WHERE provider_id = ?1",
        rusqlite::params![f.provider.to_string()],
    )
    .unwrap();
    let snapshot = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    assert!(snapshot.stream, "stream=1 → 流式");
}

#[test]
fn dispatch_path_selects_streaming_only_when_flag_true() {
    assert_eq!(gen::dispatch_path(true), DispatchPath::Streaming);
    assert_eq!(gen::dispatch_path(false), DispatchPath::NonStreaming);
}

#[test]
fn send_confirmed_forwards_chunk_progress_with_attempt_binding() {
    let f = Fixture::new("stream-progress");
    let mut conn = rusqlite::Connection::open(&f.db).unwrap();
    // 流式 Provider：快照 stream=true → runtime.stream=true（DI 捕获断言）
    conn.execute(
        "UPDATE providers SET stream = 1 WHERE provider_id = ?1",
        rusqlite::params![f.provider.to_string()],
    )
    .unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    assert!(provider.stream);
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
    let mut recorder = RecordingDispatch {
        runtime_stream: RefCell::new(None),
    };
    let mut events: Vec<StreamProgressEvent> = Vec::new();
    let err = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        confirmation_id,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut recorder,
        &mut |event| events.push(event),
    )
    .unwrap_err();
    assert_eq!(
        err.code,
        dsheng_core::contracts::cli::CliErrorCode::NetworkError
    );
    // runtime.stream 随快照传入派发层（HttpDispatch 分支依据）
    assert_eq!(*recorder.runtime_stream.borrow(), Some(true));
    // 原始三元组 → 消费端事件（attempt 关联＋stage）
    let attempt_id: String = conn
        .query_row(
            "SELECT attempt_id FROM generation_attempts LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let attempt = uuid::Uuid::parse_str(&attempt_id).unwrap();
    assert_eq!(
        events,
        vec![
            StreamProgressEvent {
                attempt_id: attempt,
                confirmation_id,
                workspace_id: f.workspace,
                chunk_seq: 0,
                bytes: 0,
                elapsed_ms: 0,
                content_chars: 0,
                reasoning_delta: String::new(),
                stage: "waiting",
            },
            StreamProgressEvent {
                attempt_id: attempt,
                confirmation_id,
                workspace_id: f.workspace,
                chunk_seq: 1,
                bytes: 128,
                elapsed_ms: 40,
                content_chars: 0,
                reasoning_delta: "合成本地思考片段".into(),
                stage: "reasoning",
            },
            StreamProgressEvent {
                attempt_id: attempt,
                confirmation_id,
                workspace_id: f.workspace,
                chunk_seq: 2,
                bytes: 4_096,
                elapsed_ms: 30_050,
                content_chars: 12,
                reasoning_delta: String::new(),
                stage: "content",
            },
            StreamProgressEvent {
                attempt_id: attempt,
                confirmation_id,
                workspace_id: f.workspace,
                chunk_seq: 3,
                bytes: 9_216,
                elapsed_ms: 60_900,
                content_chars: 12,
                reasoning_delta: String::new(),
                stage: "receiving",
            },
        ]
    );
}

#[test]
fn stderr_progress_throttle_first_immediate_then_every_30s() {
    // 首条立即（last=i64::MIN）
    assert!(ProgressSink::stderr_emit_due(i64::MIN, 0));
    // 30s 内不重复
    assert!(!ProgressSink::stderr_emit_due(0, 29_999));
    assert!(!ProgressSink::stderr_emit_due(40, 30_039));
    // 满 30s 再出一条
    assert!(ProgressSink::stderr_emit_due(0, 30_000));
    assert!(ProgressSink::stderr_emit_due(30_050, 60_900));
}

#[test]
fn provider_set_persists_stream_true_end_to_end() {
    let f = Fixture::new("stream-set");
    let input = format!(
        r#"{{"schema_version":"1","operation":"provider_upsert","workspace_id":"{}","provider_id":null,"expected_revision":null,"config":{{"name":"流式Provider","protocol":"chat_completions","base_url":"https://api.example.com/v1","model":"m","stream":true,"timeouts":{{"connect_seconds":15,"response_seconds":180,"total_seconds":1800}},"budget":{{"context_window_tokens":32768,"output_reserve_tokens":8192}}}},"key_action":"keep"}}"#,
        f.workspace
    );
    let input_path = std::path::Path::new(&f.dir).join("stream-provider.json");
    std::fs::write(&input_path, input).unwrap();
    let owned: Vec<String> = [
        "provider".to_string(),
        "set".to_string(),
        "--db".to_string(),
        f.db.clone(),
        "--workspace".to_string(),
        f.workspace.to_string(),
        "--input-json".to_string(),
        input_path.to_string_lossy().into_owned(),
        "--json".to_string(),
    ]
    .to_vec();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    let text = String::from_utf8(out).unwrap();
    assert_eq!(code, 0, "{text}");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    // 视图回显 stream=true（settings.schema 字段集）；total=1800 增量上限通过
    assert_eq!(value["data"]["provider"]["config"]["stream"], true);
    assert_eq!(
        value["data"]["provider"]["config"]["timeouts"]["total_seconds"],
        1800
    );
    // 落库为 1（快照/派发路径据此选流式）
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let stream: i64 = conn
        .query_row(
            "SELECT stream FROM providers WHERE name = '流式Provider'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stream, 1);
}
