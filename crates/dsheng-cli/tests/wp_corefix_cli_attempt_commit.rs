//! 核心缺口修复 F4：attempt 终态落库接线（TS-22 / 用例 60；复审 P2-4）。
//!
//! 发送路径已覆盖：派发错误→timed_out/interrupted/failed、finish_reason 非
//! stop→failed、提取失败→failed、取消→cancelled。缺口为「迟到」：响应返回
//! 后、提交前不核对当前分析 revision——commit_gate_before_save 补齐：
//! 迟到 → attempt 置 failed(error_code=INPUT_REVISION_CHANGED)、不提交输出、
//! 不覆盖上一有效结果；终态不可回退。

mod fixture;

use chrono::{NaiveDate, Utc};
use dsheng_cli::store::generation::{
    build_task_payload, cancel_attempt, commit_gate_before_save, create_preview,
    finish_attempt_input_revision_changed, provider_snapshot, SqliteGenerationStore,
};
use dsheng_core::analysis::fingerprint::{AnalysisRevisionInput, SourceSelection};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::domain::DateRange;
use dsheng_core::contracts::provider::GenerationOperation;
use dsheng_core::provider::confirm::ConfirmationGate;
use dsheng_core::repository::analysis_runs::{
    current_revision, upsert_analysis_revision, RevisionPayload,
};
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::json;

fn range(from: &str, to: &str) -> DateRange {
    DateRange::new(
        NaiveDate::parse_from_str(from, "%Y-%m-%d").unwrap(),
        NaiveDate::parse_from_str(to, "%Y-%m-%d").unwrap(),
    )
    .unwrap()
}

/// 重做输入（与首版不同 → 新指纹）。
fn redo_input(metrics_value: &str) -> AnalysisRevisionInput {
    AnalysisRevisionInput {
        sources: vec![SourceSelection {
            source_id: "s1".into(),
            sheet: Some("核算表".into()),
            range: "A1:E3".into(),
            content_hash: "redo".into(),
            source_profile_id: "generic".into(),
            source_profile_version: 1,
            source_profile_hash: "generic".into(),
        }],
        mapping_version: "auto-v1".into(),
        mappings: json!({}),
        metric_definitions: vec![],
        manual_inputs: vec![],
        conflict_resolutions: vec![],
        period_policy: json!({"timezone": "Asia/Shanghai"}),
        scope: "全公司".into(),
        current_period: range("2026-10-01", "2026-10-31"),
        target_period: None,
        normalized_metrics: json!({"sales_net": metrics_value}),
        comparisons: json!({}),
        comparison_base_refs: vec![],
    }
}

fn redo_analysis(conn: &mut Connection, workspace: &str, run_id: &str) {
    // 复用 fixture run 的 period_key（同 key upsert → 同 run revision 推进）
    let period_key: String = conn
        .query_row(
            "SELECT period_key FROM analysis_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    upsert_analysis_revision(
        conn,
        workspace,
        &period_key,
        &RevisionPayload {
            input: redo_input("99999"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
}

/// 建在途 attempt（预览确认→消费→已派发，无网络）。
fn inflight_attempt(conn: &mut Connection, f: &Fixture) -> (uuid::Uuid, uuid::Uuid, u32, String) {
    let provider = provider_snapshot(conn, f.workspace, f.provider).unwrap();
    let material =
        build_task_payload(conn, f.workspace, &f.run_id, "（提交门禁核对）", None).unwrap();
    let (revision, fingerprint) = (
        material.analysis_revision_no,
        material.analysis_input_fingerprint.clone(),
    );
    let preview = create_preview(
        conn,
        f.workspace,
        &provider,
        &material,
        GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let confirmation: uuid::Uuid = preview["confirmation_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    let attempt = {
        let mut store = SqliteGenerationStore { conn };
        let outcome = ConfirmationGate
            .consume_and_create_attempt(&mut store, confirmation, Utc::now())
            .unwrap()
            .attempt()
            .clone();
        ConfirmationGate
            .mark_dispatched(&mut store, outcome.attempt_id, Utc::now())
            .unwrap();
        outcome
    };
    (attempt.attempt_id, confirmation, revision, fingerprint)
}

fn attempt_terminal(conn: &Connection, attempt_id: uuid::Uuid) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT terminal_state, error_code FROM generation_attempts WHERE attempt_id = ?1",
        params![attempt_id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

#[test]
fn late_arrival_marks_failed_and_keeps_previous_valid_result() {
    let f = Fixture::new("f4-late");
    let mut conn = Connection::open(&f.db).unwrap();
    let (attempt_id, _confirmation, revision, fingerprint) = inflight_attempt(&mut conn, &f);
    // 上一有效结果：已应用候选
    conn.execute(
        "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id,
                analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                candidate_revision_no, attempt_id, output_json, state, prompt_version,
                schema_version, created_at)
         VALUES ('prev-valid', ?1, ?2, 1, ?3, 'g', 1, 'prev-attempt', '{}', 'applied',
                 'p', 's', 't')",
        params![f.workspace.to_string(), f.run_id, fingerprint],
    )
    .unwrap();
    // 发送期间分析被重做（指纹变化 → revision 2）
    redo_analysis(&mut conn, &f.workspace.to_string(), &f.run_id);
    let (cur_rev, _) = current_revision(&conn, &f.workspace.to_string(), &f.run_id).unwrap();
    assert_ne!(cur_rev, revision, "前提：分析已推进");
    // 提交门禁：迟到 → Err(INPUT_REVISION_CHANGED)
    let err = commit_gate_before_save(
        &mut conn,
        f.workspace,
        uuid::Uuid::parse_str(&f.run_id).unwrap(),
        attempt_id,
        revision,
        &fingerprint,
    )
    .unwrap_err();
    assert_eq!(err.code, CliErrorCode::InputRevisionChanged);
    // attempt 终态落库：failed + error_code
    let (terminal, error_code) = attempt_terminal(&conn, attempt_id);
    assert_eq!(terminal.as_deref(), Some("failed"));
    assert_eq!(error_code.as_deref(), Some("INPUT_REVISION_CHANGED"));
    let finished_at: Option<String> = conn
        .query_row(
            "SELECT finished_at FROM generation_attempts WHERE attempt_id = ?1",
            params![attempt_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(finished_at.is_some(), "finished_at 已落库");
    // 上一有效结果保留（未被失败覆盖）
    let prev_state: String = conn
        .query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id = 'prev-valid'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(prev_state, "applied", "previous_valid_result_retained");
    // 终态不可回退：取消被拒（已是终态）
    let err = cancel_attempt(&mut conn, f.workspace, attempt_id).unwrap_err();
    assert_eq!(err.code, CliErrorCode::IdempotencyConflict);
    // 再次 finish（服务函数）同样拒绝
    let mut store = SqliteGenerationStore { conn: &mut conn };
    let err = finish_attempt_input_revision_changed(&mut store, attempt_id).unwrap_err();
    assert_eq!(err.code, CliErrorCode::IdempotencyConflict, "终态不可回退");
}

#[test]
fn commit_gate_passes_when_revision_unchanged() {
    let f = Fixture::new("f4-ontime");
    let mut conn = Connection::open(&f.db).unwrap();
    let (attempt_id, _confirmation, revision, fingerprint) = inflight_attempt(&mut conn, &f);
    // 未重做分析 → 门禁通过，attempt 仍非终态（在途）
    commit_gate_before_save(
        &mut conn,
        f.workspace,
        uuid::Uuid::parse_str(&f.run_id).unwrap(),
        attempt_id,
        revision,
        &fingerprint,
    )
    .unwrap();
    let (terminal, error_code) = attempt_terminal(&conn, attempt_id);
    assert!(terminal.is_none(), "门禁通过不改变在途状态");
    assert!(error_code.is_none());
    // 正常提交路径：置 success
    let mut store = SqliteGenerationStore { conn: &mut conn };
    ConfirmationGate
        .finish(
            &mut store,
            attempt_id,
            dsheng_core::contracts::provider::TerminalState::Success,
            Utc::now(),
        )
        .unwrap();
    let (terminal, _) = attempt_terminal(store.conn, attempt_id);
    assert_eq!(terminal.as_deref(), Some("success"));
}

#[test]
fn cancel_marks_cancelled_then_terminal_locked() {
    let f = Fixture::new("f4-cancel");
    let mut conn = Connection::open(&f.db).unwrap();
    let (attempt_id, _confirmation, _revision, _fingerprint) = inflight_attempt(&mut conn, &f);
    // 在途取消 → cancelled（仅本地取消语义）
    let out = cancel_attempt(&mut conn, f.workspace, attempt_id).unwrap();
    assert_eq!(out["cancelled"], json!(true));
    let (terminal, _) = attempt_terminal(&conn, attempt_id);
    assert_eq!(terminal.as_deref(), Some("cancelled"));
    // 已终态再取消 → IDEMPOTENCY_CONFLICT
    let err = cancel_attempt(&mut conn, f.workspace, attempt_id).unwrap_err();
    assert_eq!(err.code, CliErrorCode::IdempotencyConflict);
}
