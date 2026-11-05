//! Desktop command boundary checks, using only temporary synthetic data.
//! Confirmation tests install a local dispatch tripwire before calling the real
//! command: even a cancellation regression cannot reach the HTTP transport.

#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;

use dsheng_app::contracts::{core as cmd, env::DesktopEnv, progress::progress_payload};
use dsheng_cli::store::generation::StreamProgressEvent;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn ok(envelope: Value) -> Value {
    assert_eq!(envelope["schema_version"], "1", "{envelope}");
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["data"].clone()
}

fn error(envelope: Value, code: &str) {
    assert_eq!(envelope["schema_version"], "1", "{envelope}");
    assert_eq!(envelope["ok"], false, "{envelope}");
    assert_eq!(envelope["error"]["code"], code, "{envelope}");
}

fn task_preview(env: &DesktopEnv, f: &Fixture) -> String {
    ok(cmd::task::task_generate_preview(
        env,
        json!({"workspace_id":f.workspace, "run_id":f.run_id}),
    ))["confirmation_id"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn progress_payload_contains_only_correlation_and_progress_metadata() {
    let attempt = Uuid::new_v4();
    let confirmation = Uuid::new_v4();
    let workspace = Uuid::new_v4();
    for stage in ["waiting", "receiving", "validating", "saving"] {
        let payload = progress_payload(&StreamProgressEvent {
            attempt_id: attempt,
            confirmation_id: confirmation,
            workspace_id: workspace,
            chunk_seq: 2,
            bytes: 8192,
            elapsed_ms: 250,
            stage,
        });
        // Exact shape excludes prompts, raw output, credentials, and Provider
        // connection settings from this broadcast-to-all-windows event.
        assert_eq!(
            payload,
            json!({
                "attempt_id":attempt.to_string(),
                "confirmation_id":confirmation.to_string(),
                "workspace_id":workspace.to_string(),
                "chunk_seq":2, "bytes":8192, "elapsed_ms":250, "stage":stage,
            })
        );
    }
}

#[test]
fn desktop_cancel_before_confirmation_prevents_task_and_diagnosis_dispatch() {
    for diagnosis in [false, true] {
        let f = Fixture::new(if diagnosis {
            "desktop-stop-5why"
        } else {
            "desktop-stop-task"
        });
        let env = DesktopEnv::with_db(&f.db);
        let conn = Connection::open(&f.db).unwrap();
        let confirmation = if diagnosis {
            let evidence = dsheng_cli::store::analysis::diagnosis_evidence_ids(
                &conn,
                f.workspace,
                &f.run_id,
                1,
                None,
            )
            .unwrap();
            assert!(!evidence.is_empty());
            ok(cmd::diagnosis::diagnosis_generate_preview(
                &env,
                json!({
                    "workspace_id":f.workspace, "run_id":f.run_id, "framework":"5why",
                    "problem":"合成本地问题", "evidence_ids":evidence,
                }),
            ))["confirmation_id"]
                .as_str()
                .unwrap()
                .to_owned()
        } else {
            task_preview(&env, &f)
        };
        let count: i64 = conn
            .query_row("SELECT count(*) FROM generation_attempts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "preview must not dispatch or create an attempt");
        conn.execute_batch(
            "CREATE TRIGGER forbid_test_dispatch BEFORE UPDATE OF request_count ON generation_attempts
             WHEN NEW.request_count > 0 BEGIN SELECT RAISE(ABORT, 'test forbids network dispatch'); END;",
        ).unwrap();
        let stopped = ok(cmd::generation::generation_cancel_by_confirmation(
            &env,
            f.workspace.to_string(),
            confirmation.clone(),
        ));
        assert_eq!(stopped["cancelled"], true);
        assert!(stopped["remote_billing_note"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        let attempt = stopped["attempt_id"].as_str().unwrap();
        let before: (String, i64) = conn.query_row(
            "SELECT terminal_state, request_count FROM generation_attempts WHERE attempt_id = ?1",
            [attempt], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(before, ("cancelled".into(), 0));
        let confirmed = if diagnosis {
            cmd::diagnosis::diagnosis_generate_confirm(&env, confirmation.clone(), None)
        } else {
            cmd::task::task_generate_confirm(&env, confirmation.clone(), None)
        };
        error(confirmed, "CANCELLED");
        let repeated = ok(cmd::generation::generation_cancel_by_confirmation(
            &env,
            f.workspace.to_string(),
            confirmation,
        ));
        assert_eq!(repeated["attempt_id"], stopped["attempt_id"]);
        let after: (String, i64) = conn.query_row(
            "SELECT terminal_state, request_count FROM generation_attempts WHERE attempt_id = ?1",
            [attempt], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(after, before);
        let outputs: i64 = conn.query_row(
            "SELECT (SELECT count(*) FROM task_candidates) + (SELECT count(*) FROM diagnosis_reports)",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(outputs, 0);
    }
}

#[test]
fn desktop_cancel_rejects_another_workspace_without_consuming_confirmation() {
    let f = Fixture::new("desktop-stop-scope");
    let env = DesktopEnv::with_db(&f.db);
    let confirmation = task_preview(&env, &f);
    let other = ok(cmd::settings::workspace_create(
        &env,
        json!({
            "schema_version":"1", "operation":"workspace_create", "name":"另一合成工作区",
            "initial_period_policy":{"cadence":"month", "effective_from":"2026-01-01"},
        }),
    ))["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_owned();
    error(
        cmd::generation::generation_cancel_by_confirmation(&env, other, confirmation.clone()),
        "WORKSPACE_MISMATCH",
    );
    let conn = Connection::open(&f.db).unwrap();
    let consumed: Option<String> = conn
        .query_row(
            "SELECT consumed_at FROM confirmations WHERE confirmation_id = ?1",
            params![confirmation],
            |r| r.get(0),
        )
        .unwrap();
    assert!(consumed.is_none());
    let count: i64 = conn
        .query_row("SELECT count(*) FROM generation_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
