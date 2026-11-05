//! Synthetic local lifecycle checks: no HTTP, fake Provider, endpoint, or business DB.
mod fixture;

use chrono::Utc;
use dsheng_cli::store::generation::{
    self as gen, GenerationMaterial, ProviderConfigSnapshot, ProviderRuntime, RequestDispatch,
    SqliteGenerationStore,
};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::provider::{
    ChatCompletionsRequest, ChatCompletionsResponse, GenerationOperation, RequestStage,
    TerminalState,
};
use dsheng_core::provider::confirm::{ConfirmationGate, GenerationStore};
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn preview(
    conn: &mut Connection,
    f: &Fixture,
) -> (ProviderConfigSnapshot, GenerationMaterial, Uuid) {
    let provider = gen::provider_snapshot(conn, f.workspace, f.provider).unwrap();
    let material =
        gen::build_task_payload(conn, f.workspace, &f.run_id, "local lifecycle", None).unwrap();
    let result = gen::create_preview(
        conn,
        f.workspace,
        &provider,
        &material,
        GenerationOperation::TaskGenerate,
        "2.3",
        "2",
    )
    .unwrap();
    (
        provider,
        material,
        result["confirmation_id"].as_str().unwrap().parse().unwrap(),
    )
}

fn consume(conn: &mut Connection, confirmation: Uuid) -> Uuid {
    ConfirmationGate
        .consume_and_create_attempt(
            &mut SqliteGenerationStore { conn },
            confirmation,
            Utc::now(),
        )
        .unwrap()
        .attempt()
        .attempt_id
}

fn status(conn: &Connection, f: &Fixture, attempt: Uuid) -> Value {
    gen::attempt_status(conn, f.workspace, attempt).unwrap()
}

fn candidate_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM task_candidates", [], |r| r.get(0))
        .unwrap()
}

fn local_output() -> Value {
    json!({"schema_version":"2", "summary":"locally authored synthetic output", "tasks":[]})
}

#[derive(Default)]
struct RefuseDispatch(usize);
impl RequestDispatch for RefuseDispatch {
    fn dispatch(
        &mut self,
        _: &ProviderRuntime,
        _: &ChatCompletionsRequest,
        _: &mut dyn FnMut(u64, u64, i64),
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        self.0 += 1;
        Err((
            CliErrorCode::NetworkError,
            "local dispatch recorder refuses network".into(),
        ))
    }
}

#[test]
fn stop_before_attempt_consumption_is_atomic_and_cannot_send() {
    let f = Fixture::new("cancel-before");
    let mut conn = Connection::open(&f.db).unwrap();
    let (provider, material, confirmation) = preview(&mut conn, &f);
    let result = gen::cancel_by_confirmation(&mut conn, f.workspace, confirmation).unwrap();
    let attempt = result["attempt_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(result["cancelled"], true);
    assert_eq!(status(&conn, &f, attempt)["request_count"], 0);
    assert_eq!(
        gen::cancel_by_confirmation(&mut conn, f.workspace, confirmation).unwrap()["attempt_id"],
        result["attempt_id"]
    );
    let mut dispatch = RefuseDispatch::default();
    let mut events = Vec::new();
    let error = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        confirmation,
        "2.3",
        "2",
        &mut dispatch,
        &mut |event| events.push(event),
    )
    .unwrap_err();
    assert_eq!(error.code, CliErrorCode::Cancelled);
    assert_eq!(dispatch.0, 0);
    assert!(events.is_empty());
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "cancelled");
}

#[test]
fn waiting_event_allows_cross_connection_stop_before_network() {
    let f = Fixture::new("cancel-waiting");
    let mut conn = Connection::open(&f.db).unwrap();
    let (provider, material, confirmation) = preview(&mut conn, &f);
    let mut dispatch = RefuseDispatch::default();
    let mut events = Vec::new();
    let error = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        confirmation,
        "2.3",
        "2",
        &mut dispatch,
        &mut |event| {
            assert_eq!(event.stage, "waiting");
            assert_eq!(event.confirmation_id, confirmation);
            assert_eq!(event.workspace_id, f.workspace);
            let mut control = Connection::open(&f.db).unwrap();
            assert_eq!(
                gen::cancel_by_confirmation(&mut control, f.workspace, confirmation).unwrap()
                    ["attempt_id"],
                event.attempt_id.to_string()
            );
            events.push(event);
        },
    )
    .unwrap_err();
    assert_eq!(error.code, CliErrorCode::Cancelled);
    assert_eq!(dispatch.0, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(status(&conn, &f, events[0].attempt_id)["request_count"], 0);
}

struct CancelDuringDispatch<'a> {
    f: &'a Fixture,
    confirmation: Uuid,
}
impl RequestDispatch for CancelDuringDispatch<'_> {
    fn dispatch(
        &mut self,
        _: &ProviderRuntime,
        _: &ChatCompletionsRequest,
        _: &mut dyn FnMut(u64, u64, i64),
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        panic!("service must use cancellable dispatch")
    }
    fn dispatch_cancellable(
        &mut self,
        _: &ProviderRuntime,
        _: &ChatCompletionsRequest,
        _: &mut dyn FnMut(u64, u64, i64),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        assert!(!should_cancel());
        let mut control = Connection::open(&self.f.db).unwrap();
        gen::cancel_by_confirmation(&mut control, self.f.workspace, self.confirmation).unwrap();
        assert!(
            should_cancel(),
            "poll sees another connection's committed cancellation without a chunk"
        );
        Err((
            CliErrorCode::NetworkError,
            "late local transport error".into(),
        ))
    }
}

#[test]
fn cancellation_without_chunks_wins_over_late_transport_error_for_both_paths() {
    for stream in [false, true] {
        let f = Fixture::new(if stream {
            "cancel-stream"
        } else {
            "cancel-json"
        });
        let mut conn = Connection::open(&f.db).unwrap();
        conn.execute("UPDATE providers SET stream = ?1", [stream])
            .unwrap();
        let (provider, material, confirmation) = preview(&mut conn, &f);
        let mut dispatch = CancelDuringDispatch {
            f: &f,
            confirmation,
        };
        let error = gen::send_confirmed(
            &mut conn,
            f.workspace,
            &provider,
            &material,
            confirmation,
            "2.3",
            "2",
            &mut dispatch,
            &mut |_| {},
        )
        .unwrap_err();
        assert_eq!(error.code, CliErrorCode::Cancelled);
        let attempt = consume(&mut conn, confirmation);
        assert_eq!(status(&conn, &f, attempt)["terminal_state"], "cancelled");
        assert_eq!(status(&conn, &f, attempt)["error_code"], "CANCELLED");
        assert_eq!(candidate_count(&conn), 0);
    }
}

#[test]
fn stale_gate_updates_cannot_overwrite_cancellation_or_dispatch_twice() {
    let f = Fixture::new("cancel-cas");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, _, confirmation) = preview(&mut conn, &f);
    let attempt_id = consume(&mut conn, confirmation);
    let mut stale = SqliteGenerationStore { conn: &mut conn }
        .load_attempt(attempt_id)
        .unwrap();
    stale.request_count = 1;
    stale.stage = RequestStage::Dispatched;
    SqliteGenerationStore { conn: &mut conn }
        .update_attempt(&stale)
        .unwrap();
    assert!(SqliteGenerationStore { conn: &mut conn }
        .update_attempt(&stale)
        .is_err());
    gen::cancel_by_confirmation(&mut conn, f.workspace, confirmation).unwrap();
    stale.stage = RequestStage::Terminal;
    stale.terminal = Some(TerminalState::Success);
    assert!(SqliteGenerationStore { conn: &mut conn }
        .update_attempt(&stale)
        .is_err());
    assert_eq!(status(&conn, &f, attempt_id)["terminal_state"], "cancelled");
}

#[test]
fn simultaneous_consume_and_cancel_share_exactly_one_attempt() {
    let f = Fixture::new("cancel-consume-race");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, _, confirmation) = preview(&mut conn, &f);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let db = f.db.clone();
    let workspace = f.workspace;
    let first = barrier.clone();
    let worker = std::thread::spawn(move || {
        let mut conn = Connection::open(db).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        first.wait();
        gen::cancel_by_confirmation(&mut conn, workspace, confirmation).unwrap()
    });
    conn.busy_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    barrier.wait();
    let attempt = consume(&mut conn, confirmation);
    let result = worker.join().unwrap();
    assert_eq!(result["attempt_id"], attempt.to_string());
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "cancelled");
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM generation_attempts WHERE confirmation_id = ?1",
            [confirmation.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn cancellation_rejects_other_workspace_without_consuming() {
    let f = Fixture::new("cancel-workspace");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, _, confirmation) = preview(&mut conn, &f);
    assert_eq!(
        gen::cancel_by_confirmation(&mut conn, Uuid::new_v4(), confirmation)
            .unwrap_err()
            .code,
        CliErrorCode::WorkspaceMismatch
    );
    let count: i64 = conn
        .query_row("SELECT count(*) FROM generation_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn task_candidate_and_success_commit_together_and_finished_cannot_be_cancelled() {
    let f = Fixture::new("task-save-success");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, material, confirmation) = preview(&mut conn, &f);
    let attempt = consume(&mut conn, confirmation);
    let candidate = gen::save_task_candidate(
        &mut conn,
        f.workspace,
        &material,
        attempt,
        &local_output(),
        false,
    )
    .unwrap();
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "success");
    let saved: String = conn
        .query_row(
            "SELECT output_json FROM task_candidates WHERE candidate_generation_id = ?1",
            [candidate],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap(),
        local_output()
    );
    assert_eq!(
        gen::cancel_by_confirmation(&mut conn, f.workspace, confirmation).unwrap()["cancelled"],
        false
    );
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "success");
}

#[test]
fn task_save_failure_rolls_back_candidate_and_marks_failed() {
    let f = Fixture::new("task-save-failure");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, material, confirmation) = preview(&mut conn, &f);
    let attempt = consume(&mut conn, confirmation);
    // Fail the last transaction step, after INSERT, proving both changes roll back.
    conn.execute_batch("CREATE TRIGGER fail_success BEFORE UPDATE OF terminal_state ON generation_attempts WHEN NEW.terminal_state = 'success' BEGIN SELECT RAISE(ABORT, 'synthetic local disk failure'); END;").unwrap();
    let error = gen::save_task_candidate(
        &mut conn,
        f.workspace,
        &material,
        attempt,
        &local_output(),
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, CliErrorCode::LocalSaveFailed);
    assert_eq!(candidate_count(&conn), 0);
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "failed");
    assert_eq!(
        status(&conn, &f, attempt)["error_code"],
        "LOCAL_SAVE_FAILED"
    );
}

#[test]
fn cancellation_before_task_commit_discards_output_and_preserves_terminal() {
    let f = Fixture::new("task-save-cancelled");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, material, confirmation) = preview(&mut conn, &f);
    let attempt = consume(&mut conn, confirmation);
    gen::cancel_by_confirmation(&mut conn, f.workspace, confirmation).unwrap();
    assert_eq!(
        gen::save_task_candidate(
            &mut conn,
            f.workspace,
            &material,
            attempt,
            &local_output(),
            false
        )
        .unwrap_err()
        .code,
        CliErrorCode::Cancelled
    );
    assert_eq!(candidate_count(&conn), 0);
    assert_eq!(status(&conn, &f, attempt)["terminal_state"], "cancelled");
}

#[test]
fn task_save_rechecks_current_revision_inside_commit_transaction() {
    let f = Fixture::new("task-save-revision");
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, material, confirmation) = preview(&mut conn, &f);
    let attempt = consume(&mut conn, confirmation);
    conn.execute("UPDATE analysis_runs SET current_revision_no = current_revision_no + 1, analysis_input_fingerprint = 'changed-local-input' WHERE run_id = ?1", params![f.run_id]).unwrap();
    assert_eq!(
        gen::save_task_candidate(
            &mut conn,
            f.workspace,
            &material,
            attempt,
            &local_output(),
            false
        )
        .unwrap_err()
        .code,
        CliErrorCode::InputRevisionChanged
    );
    assert_eq!(candidate_count(&conn), 0);
    assert_eq!(
        status(&conn, &f, attempt)["error_code"],
        "INPUT_REVISION_CHANGED"
    );
}
