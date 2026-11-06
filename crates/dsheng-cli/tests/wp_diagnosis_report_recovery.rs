//! Local report read/save regressions. Synthetic SQLite + pure turn data; no Provider dispatch.
mod fixture;

use dsheng_cli::store::diagnosis;
use dsheng_core::provider::schema::validate_diagnosis_report;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn patch() -> Value {
    json!({
        "summary_update": "合成报告摘要",
        "findings_upsert": [{"finding_id":"why-1","level":1,"question":"为什么下降？",
            "answer":"客流减少","fact_status":"fact","evidence_ids":["ev-1"],
            "reasoning_summary":"来源支持","confidence":"high","validation_question":null,"status":"candidate"}],
        "facts_and_hypotheses_upsert": [{"item_id":"fact-1","statement":"客流下降",
            "fact_status":"fact","evidence_ids":["ev-1"],"user_quote":null,"confidence":"high","reasoning_summary":"来源支持"}],
        "evidence_refs_append": [{"evidence_id":"ev-1","source_id":"src-1","source_label":"合成表","locator":"数据!B2"}],
        "clarification_questions_upsert": [{"question_id":"q-1","question":"哪些日期？","status":"open","answer_summary":null}],
        "warnings_upsert": [{"warning_id":"w-1","message":"需补充数据","status":"active","resolution_requirement":"source_record","evidence_ids":[],"fact_ids":[],"resolution_note":null},
            {"warning_id":"w-2","message":"已核对","status":"resolved","resolution_requirement":"source_record","evidence_ids":["ev-1"],"fact_ids":[],"resolution_note":"已核对"}],
        "stop_reason":"等待补充","can_continue":false
    })
}

fn insert_legacy(f: &Fixture, conn: &Connection) -> (String, String, String) {
    let session = Uuid::new_v4().to_string();
    let report = Uuid::new_v4().to_string();
    let raw = json!({"schema_version":"3","framework":"5why",
        "scope":{"problem_statement":"销售下降"},"summary":"合成报告摘要",
        "turn":{"report_patch":patch()},"turn_disposition":"on_topic","stop_reason":null})
    .to_string();
    conn.execute("INSERT INTO generation_sessions (session_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,generation_input_fingerprint,workflow,framework,state,prompt_version,schema_version,input_schema_version,source_manifest_hash,created_at,updated_at) VALUES (?1,?2,?3,1,'fp','gfp','diagnosis','5why','active','2.0','3','2','h','2026-11-05T00:00:00Z','2026-11-05T00:00:00Z')",params![session,f.workspace.to_string(),f.run_id]).unwrap();
    conn.execute("INSERT INTO diagnosis_reports (report_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,report_revision_no,session_id,framework,problem_statement,report_json,state,created_at,updated_at) VALUES (?1,?2,?3,1,'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,?4,'5why','销售下降',?5,'accepted','2026-11-05T00:00:00Z','2026-11-05T00:00:00Z')",params![report,f.workspace.to_string(),f.run_id,session,raw]).unwrap();
    (report, session, raw)
}

#[test]
fn legacy_turn_is_read_as_complete_report_without_rewriting_snapshot() {
    let f = Fixture::new("diagnosis-legacy");
    let conn = Connection::open(&f.db).unwrap();
    let (id, session, raw) = insert_legacy(&f, &conn);
    let readonly =
        Connection::open_with_flags(&f.db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let report = diagnosis::report_show(&readonly, f.workspace, &id, None).unwrap();
    validate_diagnosis_report(&report).unwrap();
    assert_eq!(report["report_id"], id);
    assert_eq!(report["generation_session_id"], session);
    assert_eq!(report["period"]["from"], "2026-10-01");
    assert_eq!(report["findings"], patch()["findings_upsert"]);
    assert_eq!(
        report["facts_and_hypotheses"],
        patch()["facts_and_hypotheses_upsert"]
    );
    assert_eq!(report["evidence_refs"], patch()["evidence_refs_append"]);
    assert_eq!(
        report["clarification_questions"],
        patch()["clarification_questions_upsert"]
    );
    let after: String = conn
        .query_row(
            "SELECT report_json FROM diagnosis_reports WHERE report_id=?1",
            [&id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(raw, after);
    assert!(diagnosis::report_show(&conn, Uuid::new_v4(), &id, None).is_err());
    assert!(diagnosis::report_show(&conn, f.workspace, &id, Some(2)).is_err());
    let list = diagnosis::report_list(&conn, f.workspace, &f.run_id).unwrap();
    assert_eq!(list["reports"][0]["can_continue"], false);
    assert_eq!(list["reports"][0]["stop_reason"], "等待补充");
    assert_eq!(list["reports"][0]["active_warning_count"], 1);
}

fn material(f: &Fixture) -> dsheng_cli::store::generation::GenerationMaterial {
    use dsheng_core::generation::input::SessionStateProjection;
    dsheng_cli::store::generation::build_diagnosis_payload(
        &Connection::open(&f.db).unwrap(),
        f.workspace,
        &f.run_id,
        dsheng_core::contracts::provider::Framework::Why5,
        "销售下降",
        &[],
        &[],
        SessionStateProjection {
            accepted_facts: vec![],
            prior_report_summary: None,
            unresolved_questions: vec![],
            pending_clarification: None,
        },
        None,
        "请分析",
    )
    .unwrap()
}

fn attempt(conn: &Connection, f: &Fixture, session: &str) -> Uuid {
    let id = Uuid::new_v4();
    conn.execute("INSERT INTO generation_attempts (attempt_id,workspace_id,session_id,workflow,operation,confirmation_id,provider_id,provider_revision,provider_snapshot_json,prompt_version,schema_version,input_hash,attempt_seq,created_at) VALUES (?1,?2,?3,'diagnosis','diagnosis_generate','local-confirmation',?4,1,'{}','2.0','3','hash',(SELECT COALESCE(MAX(attempt_seq),0)+1 FROM generation_attempts WHERE session_id=?3),'2026-11-05T00:00:00Z')", params![id.to_string(),f.workspace.to_string(),session,f.provider.to_string()]).unwrap();
    id
}

#[test]
fn new_output_persists_full_report_and_returns_its_identity() {
    let f = Fixture::new("diagnosis-save");
    let mut conn = Connection::open(&f.db).unwrap();
    let (old_id, session, _) = insert_legacy(&f, &conn);
    conn.execute("DELETE FROM diagnosis_reports WHERE report_id=?1", [old_id])
        .unwrap();
    let attempt_id = attempt(&conn, &f, &session);
    // A different session exists for the same run; persistence must use the attempt's session.
    insert_legacy(&f, &conn);
    let material = material(&f);
    let output = json!({"schema_version":"3","framework":"5why","turn_disposition":"on_topic",
        "accepted_facts":[],"report_patch":patch(),"redirect_message":null,
        "clarification_questions":[],"stop_reason":null});
    let receipt = dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        attempt_id,
        &output,
    )
    .unwrap();
    assert_eq!(receipt["session_id"], session);
    assert_eq!(terminal(&conn, attempt_id), (Some("success".into()), None));
    let id = receipt["report_id"].as_str().unwrap();
    let raw: String = conn
        .query_row(
            "SELECT report_json FROM diagnosis_reports WHERE report_id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap();
    let saved: Value = serde_json::from_str(&raw).unwrap();
    validate_diagnosis_report(&saved).unwrap();
    assert_eq!(saved["scope"]["problem_statement"], "销售下降");
    assert_eq!(saved["period"]["from"], "2026-10-01");
    assert_eq!(
        saved["source_manifest"],
        serde_json::from_str::<Value>(&material.payload_json).unwrap()["source_manifest"]
    );
    assert_eq!(saved["findings"], patch()["findings_upsert"]);
    assert_eq!(
        diagnosis::report_show(&conn, f.workspace, id, None).unwrap(),
        saved
    );
    let (provider, patch_raw, revision): (String,String,i64) = conn.query_row("SELECT provider_id,report_patch_json,report_revision_after FROM diagnosis_turns WHERE turn_id=?1", [receipt["turn_id"].as_str().unwrap()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(provider, f.provider.to_string());
    assert_eq!(serde_json::from_str::<Value>(&patch_raw).unwrap(), patch());
    assert_eq!(revision, 1);
    let before: i64 = conn
        .query_row("SELECT COUNT(*) FROM diagnosis_turns", [], |r| r.get(0))
        .unwrap();
    assert!(dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        attempt_id,
        &output
    )
    .is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM diagnosis_turns", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
}

#[test]
fn clarification_turn_does_not_create_a_report() {
    let f = Fixture::new("diagnosis-clarification");
    let material = material(&f);
    let mut conn = Connection::open(&f.db).unwrap();
    let (old_id, session, _) = insert_legacy(&f, &conn);
    conn.execute("DELETE FROM diagnosis_reports WHERE report_id=?1", [old_id])
        .unwrap();
    let attempt_id = attempt(&conn, &f, &session);
    let output = json!({"schema_version":"3","framework":"5why","turn_disposition":"needs_clarification",
        "accepted_facts":[],"report_patch":null,"redirect_message":null,
        "clarification_questions":[],"stop_reason":"请补充范围"});
    let receipt = dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        attempt_id,
        &output,
    )
    .unwrap();
    assert_eq!(receipt["report_id"], Value::Null);
    assert_eq!(receipt["turn_state"], "clarification");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM diagnosis_reports", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn existing_report_continuation_is_rejected_before_confirmation_or_dispatch() {
    let f = Fixture::new("diagnosis-continuation");
    let conn = Connection::open(&f.db).unwrap();
    let (_, session, _) = insert_legacy(&f, &conn);
    let error = dsheng_cli::cmd::generation_cmd::diagnosis_continue_value(
        Some(f.workspace.to_string()),
        session,
        dsheng_cli::cmd::generation_cmd::AnswerSource::Inline("补充事实".into()),
        None,
        true,
        None,
        false,
        Some(&f.db),
    )
    .unwrap_err();
    assert!(error.to_string().contains("此次未发送模型请求"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM generation_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn stale_generation_head_is_rejected_before_preview() {
    use dsheng_cli::store::generation as gen;
    let f = Fixture::new("diagnosis-duplicate");
    let material = material(&f);
    let mut conn = Connection::open(&f.db).unwrap();
    let (_, session, _) = insert_legacy(&f, &conn);
    conn.execute(
        "UPDATE generation_sessions SET generation_input_fingerprint=?2 WHERE session_id=?1",
        params![session, material.input_hash],
    )
    .unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let error = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        dsheng_core::contracts::provider::GenerationOperation::DiagnosisGenerate,
        dsheng_core::contracts::prompts::DIAGNOSIS_PROMPT_VERSION,
        "3",
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        dsheng_core::contracts::cli::CliErrorCode::ConfirmationStale
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM confirmations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

fn output() -> Value {
    json!({"schema_version":"3","framework":"5why","turn_disposition":"on_topic",
        "accepted_facts":[],"report_patch":patch(),"redirect_message":null,
        "clarification_questions":[],"stop_reason":null})
}

fn unsaved_attempt(f: &Fixture, conn: &Connection) -> Uuid {
    let (old_id, session, _) = insert_legacy(f, conn);
    conn.execute("DELETE FROM diagnosis_reports WHERE report_id=?1", [old_id])
        .unwrap();
    attempt(conn, f, &session)
}

fn terminal(conn: &Connection, id: Uuid) -> (Option<String>, Option<String>) {
    conn.query_row(
        "SELECT terminal_state,error_code FROM generation_attempts WHERE attempt_id=?1",
        [id.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn assert_no_saved_output(conn: &Connection) {
    for table in ["diagnosis_reports", "diagnosis_turns"] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

#[test]
fn validation_failure_marks_attempt_failed_without_saved_output() {
    let f = Fixture::new("diagnosis-invalid-output");
    let material = material(&f);
    let mut conn = Connection::open(&f.db).unwrap();
    let id = unsaved_attempt(&f, &conn);
    let error = dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        id,
        &json!({}),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        dsheng_core::contracts::cli::CliErrorCode::SchemaValidationFailed
    );
    assert_eq!(
        terminal(&conn, id),
        (
            Some("failed".into()),
            Some("SCHEMA_VALIDATION_FAILED".into())
        )
    );
    assert_no_saved_output(&conn);
}

#[test]
fn save_failure_rolls_back_report_and_marks_attempt_failed() {
    let f = Fixture::new("diagnosis-save-failure");
    let material = material(&f);
    let mut conn = Connection::open(&f.db).unwrap();
    let id = unsaved_attempt(&f, &conn);
    conn.execute_batch("CREATE TRIGGER fail_diagnosis_turn BEFORE INSERT ON diagnosis_turns BEGIN SELECT RAISE(ABORT, 'synthetic save failure'); END;").unwrap();
    let error = dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        id,
        &output(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        dsheng_core::contracts::cli::CliErrorCode::LocalSaveFailed
    );
    assert_eq!(
        terminal(&conn, id),
        (Some("failed".into()), Some("LOCAL_SAVE_FAILED".into()))
    );
    assert_no_saved_output(&conn);
}

#[test]
fn cancelled_or_timed_out_diagnosis_never_commits_late_output() {
    for state in ["cancelled", "timed_out"] {
        let f = Fixture::new("diagnosis-late-terminal");
        let material = material(&f);
        let mut conn = Connection::open(&f.db).unwrap();
        let id = unsaved_attempt(&f, &conn);
        conn.execute(
            "UPDATE generation_attempts SET terminal_state=?2 WHERE attempt_id=?1",
            params![id.to_string(), state],
        )
        .unwrap();
        assert!(dsheng_cli::store::generation::save_diagnosis_output(
            &mut conn,
            f.workspace,
            &material,
            dsheng_core::contracts::provider::Framework::Why5,
            id,
            &output()
        )
        .is_err());
        assert_eq!(terminal(&conn, id), (Some(state.into()), None));
        assert_no_saved_output(&conn);
    }
}

#[test]
fn diagnosis_rechecks_revision_at_save_and_rejects_late_output() {
    let f = Fixture::new("diagnosis-late-revision");
    let material = material(&f);
    let mut conn = Connection::open(&f.db).unwrap();
    let id = unsaved_attempt(&f, &conn);
    conn.execute(
        "UPDATE analysis_runs SET current_revision_no=2 WHERE run_id=?1",
        [&f.run_id],
    )
    .unwrap();
    let error = dsheng_cli::store::generation::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        dsheng_core::contracts::provider::Framework::Why5,
        id,
        &output(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        dsheng_core::contracts::cli::CliErrorCode::InputRevisionChanged
    );
    assert_eq!(
        terminal(&conn, id),
        (Some("failed".into()), Some("INPUT_REVISION_CHANGED".into()))
    );
    assert_no_saved_output(&conn);
}

#[test]
fn recovered_report_markdown_includes_summary_answers_warnings_and_questions() {
    let f = Fixture::new("diagnosis-markdown");
    let conn = Connection::open(&f.db).unwrap();
    let (id, _, _) = insert_legacy(&f, &conn);
    let report = diagnosis::report_show(&conn, f.workspace, &id, None).unwrap();
    let md = diagnosis::report_markdown(&report);
    for text in [
        "合成报告摘要",
        "为什么下降？",
        "答复：客流减少",
        "需补充数据",
        "待澄清问题",
        "哪些日期？",
    ] {
        assert!(md.contains(text), "missing {text}: {md}");
    }
}

#[test]
fn identical_generation_can_replace_current_report_and_late_competitor_cannot_overwrite_it() {
    use dsheng_cli::store::generation as gen;
    use dsheng_core::contracts::provider::{Framework, GenerationOperation};
    let f = Fixture::new("diagnosis-regenerate-series");
    let mut conn = Connection::open(&f.db).unwrap();
    let (original, _, raw) = insert_legacy(&f, &conn);
    conn.execute(
        "UPDATE diagnosis_reports SET problem_statement=?2 WHERE report_id=?1",
        params![original, "\t\n\u{3000}销售下降\r\n"],
    )
    .unwrap();
    let head = diagnosis::report_head(&conn, f.workspace, &f.run_id, "5why", "\u{a0}销售下降\t")
        .unwrap()
        .unwrap();
    assert_eq!(head.report_id, original);
    let next = material(&f);
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &next,
        GenerationOperation::DiagnosisGenerate,
        dsheng_core::contracts::prompts::DIAGNOSIS_PROMPT_VERSION,
        "3",
    )
    .unwrap();
    assert_eq!(
        preview["diagnosis_replacement"],
        json!({"report_id":original,"version_no":1,"next_version_no":2})
    );
    let sid: String = conn
        .query_row(
            "SELECT session_id FROM generation_sessions WHERE generation_input_fingerprint=?1",
            [&next.input_hash],
            |r| r.get(0),
        )
        .unwrap();
    let id = attempt(&conn, &f, &sid);
    let late = attempt(&conn, &f, &sid);
    let receipt = gen::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &next,
        Framework::Why5,
        id,
        &output(),
    )
    .unwrap();
    let newer = receipt["report_id"].as_str().unwrap();
    let list = diagnosis::report_list(&conn, f.workspace, &f.run_id).unwrap();
    assert_eq!(list["reports"].as_array().unwrap().len(), 1);
    assert_eq!(list["reports"][0]["report_id"], newer);
    assert_eq!(list["reports"][0]["version_no"], 2);
    assert_eq!(list["reports"][0]["versions"].as_array().unwrap().len(), 2);
    let error = gen::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &next,
        Framework::Why5,
        late,
        &output(),
    )
    .unwrap_err();
    assert_eq!(
        error.code,
        dsheng_core::contracts::cli::CliErrorCode::ConfirmationStale
    );
    assert_eq!(
        conn.query_row(
            "SELECT report_json FROM diagnosis_reports WHERE report_id=?1",
            [&original],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        raw
    );
    let third = material(&f);
    assert_ne!(third.input_hash, next.input_hash);
    assert_eq!(third.report_id.unwrap().to_string(), newer);
    assert_eq!(
        conn.query_row(
            "SELECT SUM(request_count) FROM generation_attempts",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
