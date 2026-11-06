//! 临时合成 SQLite 的报告引用闭环；不发送 Provider、不使用业务数据库。
mod fixture;

use dsheng_cli::store::task_diagnosis_context::{load_task_report, project_task_report};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::generation::input::FactStatusProjection;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use uuid::Uuid;

fn fingerprint(conn: &Connection, f: &Fixture) -> String {
    conn.query_row(
        "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",
        [&f.run_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn report(f: &Fixture, fp: &str, framework: &str) -> Value {
    let finding = match framework {
        "5why" => {
            json!({"finding_id":"finding-1","level":1,"question":"为何下降？","answer":"交付安排待核对","fact_status":"hypothesis","evidence_ids":["evidence-one"],"reasoning_summary":"仅是候选解释","confidence":"low","validation_question":"需要核对交付日吗？","status":"needs_confirmation"})
        }
        "5w" => {
            json!({"finding_id":"finding-1","dimension":"When","question":"何时发生？","answer":"本月","fact_status":"fact","evidence_ids":["evidence-one"],"confidence":"high","missing_information":[],"clarification_questions":[],"status":"resolved"})
        }
        "5w2h" => {
            json!({"finding_id":"finding-1","dimension":"How","kind":"proposal","question":"如何改善？","answer":"建议先核对排期","fact_status":"hypothesis","evidence_ids":["evidence-one"],"confidence":"low","missing_information":["排期表"],"clarification_questions":["排期是否确认？"],"status":"candidate"})
        }
        _ => unreachable!(),
    };
    let dimensions: &[&str] = match framework {
        "5w" => &["What", "Why", "Where", "When", "Who"],
        "5w2h" => &["What", "Why", "Where", "When", "Who", "How", "How much"],
        _ => &[],
    };
    let findings = if dimensions.is_empty() {
        vec![finding]
    } else {
        dimensions
            .iter()
            .map(|dimension| {
                let mut item = finding.clone();
                item["finding_id"] = json!(format!("finding-{dimension}"));
                item["dimension"] = json!(dimension);
                item
            })
            .collect()
    };
    json!({
        "schema_version":"3", "report_id":Uuid::new_v4(),"generation_session_id":Uuid::new_v4(),
        "workspace_id":f.workspace,"run_id":f.run_id,"framework":framework,
        "scope":{"problem_statement":"合成经营问题","focus_metric_ids":[f.metric_id]},
        "period":{"from":"2026-10-01","to":"2026-10-31","period_policy_version":"1"},
        "revision_no":1,"summary":"有依据的诊断摘要","source_manifest":[],"findings":findings,
        "facts_and_hypotheses":[
            {"item_id":"fact-1","statement":"实绩下降","fact_status":"fact","evidence_ids":["evidence-one"],"user_quote":null,"confidence":"high","reasoning_summary":"来源显示"},
            {"item_id":"user-1","statement":"用户称下周补货","fact_status":"user_report","evidence_ids":[],"user_quote":"下周补货","confidence":"unknown","reasoning_summary":"用户陈述"},
            {"item_id":"hypothesis-1","statement":"可能是排期原因","fact_status":"hypothesis","evidence_ids":["evidence-one"],"user_quote":null,"confidence":"low","reasoning_summary":"待核实"},
            {"item_id":"unknown-1","statement":"交付责任未知","fact_status":"unknown","evidence_ids":[],"user_quote":null,"confidence":"unknown","reasoning_summary":"暂无依据"}
        ],
        "evidence_refs":[{"evidence_id":"evidence-one","source_id":"synthetic-source","source_label":"合成表","locator":"数据!B2"}],
        "clarification_questions":[],"warnings":[],"stop_reason":null,"can_continue":true,
        "created_at":"2026-11-06T00:00:00Z","updated_at":"2026-11-06T00:00:00Z",
        "analysis_revision_no":1,"analysis_input_fingerprint":fp
    })
}

fn seed(conn: &Connection, f: &Fixture, report: &Value, raw: &Value) {
    dsheng_core::provider::schema::validate_diagnosis_report(report).unwrap();
    conn.execute("INSERT INTO generation_sessions (session_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,generation_input_fingerprint,workflow,framework,state,prompt_version,schema_version,input_schema_version,source_manifest_hash,created_at,updated_at) VALUES (?1,?2,?3,1,?4,'synthetic','diagnosis',?5,'active','2.1','3','2','synthetic','2026-11-06T00:00:00Z','2026-11-06T00:00:00Z')", params![report["generation_session_id"].as_str().unwrap(),f.workspace.to_string(),f.run_id,report["analysis_input_fingerprint"].as_str().unwrap(),report["framework"].as_str().unwrap()]).unwrap();
    conn.execute("INSERT INTO diagnosis_reports (report_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,report_revision_no,session_id,framework,problem_statement,report_json,state,created_at,updated_at) VALUES (?1,?2,?3,1,?4,1,?5,?6,'合成经营问题',?7,'accepted','2026-11-06T00:00:00Z','2026-11-06T00:00:00Z')",params![report["report_id"].as_str().unwrap(),f.workspace.to_string(),f.run_id,report["analysis_input_fingerprint"].as_str().unwrap(),report["generation_session_id"].as_str().unwrap(),report["framework"].as_str().unwrap(),raw.to_string()]).unwrap();
}

fn allowed() -> BTreeSet<String> {
    BTreeSet::from(["evidence-one".to_string()])
}

#[test]
fn selected_report_projects_actual_three_frameworks_without_elevating_user_or_hypothesis() {
    let f = Fixture::new("task-report-frameworks");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    for framework in ["5why", "5w", "5w2h"] {
        let saved = report(&f, &fp, framework);
        seed(&conn, &f, &saved, &saved);
        let id = saved["report_id"].as_str().unwrap();
        let read = load_task_report(&conn, f.workspace, &f.run_id, 1, &fp, id).unwrap();
        let projected = project_task_report(&read, &allowed()).unwrap();
        assert_eq!(
            serde_json::to_value(projected.framework).unwrap(),
            framework
        );
        assert_eq!(projected.report_id, id);
        assert!(projected.summary.contains("有依据的诊断摘要"));
        assert!(projected.summary.contains("合成经营问题"));
        let user = projected
            .facts
            .iter()
            .find(|item| item.item_id == "user-1")
            .unwrap();
        assert_eq!(user.fact_status, FactStatusProjection::UserReport);
        assert_eq!(user.user_quote.as_deref(), Some("下周补货"));
        assert!(projected
            .hypotheses
            .iter()
            .any(|item| item.fact_status == FactStatusProjection::Unknown));
        assert!(projected
            .hypotheses
            .iter()
            .any(|item| item.item_id == "hypothesis-1"
                && item.fact_status == FactStatusProjection::Hypothesis));
        let findings: Vec<_> = projected
            .facts
            .iter()
            .chain(&projected.hypotheses)
            .filter(|item| item.item_id.starts_with("finding:"))
            .collect();
        assert_eq!(
            findings.len(),
            match framework {
                "5w" => 5,
                "5w2h" => 7,
                _ => 1,
            }
        );
        if framework == "5w2h" {
            assert!(findings[0].statement.contains("proposal"));
            assert!(findings[0].statement.contains("candidate"));
            assert!(findings[0].statement.contains("排期表"));
            assert_eq!(findings[0].fact_status, FactStatusProjection::Hypothesis);
        }
        if framework == "5why" {
            assert!(findings[0].statement.contains("追问层级：第 1 层"));
        } else {
            assert!(findings
                .iter()
                .any(|finding| finding.statement.contains("分析维度：When")));
        }
        assert_eq!(projected.evidence_ids, vec!["evidence-one"]);
    }
}

#[test]
fn report_evidence_must_be_present_in_current_send_scope_not_silently_stripped() {
    let f = Fixture::new("task-report-evidence");
    let conn = Connection::open(&f.db).unwrap();
    let saved = report(&f, &fingerprint(&conn, &f), "5why");
    let err = project_task_report(&saved, &BTreeSet::new()).unwrap_err();
    assert_eq!(err.code, CliErrorCode::InputContractInvalid);
    assert_eq!(
        err.details["unavailable_evidence_ids"],
        json!(["evidence-one"])
    );
    assert!(err.message.contains("取消报告引用"));
}

#[test]
fn report_requires_same_workspace_run_revision_fingerprint_and_unfrozen_session() {
    let f = Fixture::new("task-report-bindings");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let saved = report(&f, &fp, "5why");
    seed(&conn, &f, &saved, &saved);
    let id = saved["report_id"].as_str().unwrap();
    assert_eq!(
        load_task_report(&conn, Uuid::new_v4(), &f.run_id, 1, &fp, id)
            .unwrap_err()
            .code,
        CliErrorCode::WorkspaceMismatch
    );
    for (run, revision, fingerprint) in [
        ("another-run", 1, fp.as_str()),
        (f.run_id.as_str(), 2, fp.as_str()),
        (f.run_id.as_str(), 1, "changed"),
    ] {
        assert_eq!(
            load_task_report(&conn, f.workspace, run, revision, fingerprint, id)
                .unwrap_err()
                .code,
            CliErrorCode::StaleDiagnosisReference
        );
    }
    conn.execute(
        "UPDATE generation_sessions SET frozen_at='2026-11-06T01:00:00Z' WHERE session_id=?1",
        [saved["generation_session_id"].as_str().unwrap()],
    )
    .unwrap();
    assert_eq!(
        load_task_report(&conn, f.workspace, &f.run_id, 1, &fp, id)
            .unwrap_err()
            .code,
        CliErrorCode::StaleDiagnosisReference
    );
    conn.execute("UPDATE generation_sessions SET frozen_at=NULL", [])
        .unwrap();
    conn.execute(
        "UPDATE diagnosis_reports SET state='archived' WHERE report_id=?1",
        [id],
    )
    .unwrap();
    assert_eq!(
        load_task_report(&conn, f.workspace, &f.run_id, 1, &fp, id)
            .unwrap_err()
            .code,
        CliErrorCode::StaleDiagnosisReference
    );
}

#[test]
fn report_snapshot_identity_must_match_database_binding() {
    let f = Fixture::new("task-report-json-binding");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let saved = report(&f, &fp, "5why");
    let mut mismatched = saved.clone();
    mismatched["revision_no"] = json!(2);
    seed(&conn, &f, &saved, &mismatched);
    assert_eq!(
        load_task_report(
            &conn,
            f.workspace,
            &f.run_id,
            1,
            &fp,
            saved["report_id"].as_str().unwrap()
        )
        .unwrap_err()
        .code,
        CliErrorCode::StaleDiagnosisReference
    );
}

#[test]
fn legacy_report_is_recovered_readonly_before_task_projection() {
    let f = Fixture::new("task-report-legacy");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let saved = report(&f, &fp, "5why");
    let raw = json!({"turn":{"report_patch":{
        "summary_update":saved["summary"],"findings_upsert":saved["findings"],
        "facts_and_hypotheses_upsert":saved["facts_and_hypotheses"],
        "evidence_refs_append":saved["evidence_refs"],"clarification_questions_upsert":[],"warnings_upsert":[],
        "stop_reason":null,"can_continue":true
    }}});
    seed(&conn, &f, &saved, &raw);
    let id = saved["report_id"].as_str().unwrap();
    let readonly =
        Connection::open_with_flags(&f.db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let read = load_task_report(&readonly, f.workspace, &f.run_id, 1, &fp, id).unwrap();
    let projected = project_task_report(&read, &allowed()).unwrap();
    assert_eq!(projected.facts.len(), 2);
    assert_eq!(projected.hypotheses.len(), 3);
    let after: String = conn
        .query_row(
            "SELECT report_json FROM diagnosis_reports WHERE report_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, raw.to_string());
    let attempts: i64 = conn
        .query_row("SELECT COUNT(*) FROM generation_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(attempts, 0);
}

#[test]
fn saved_report_reaches_real_task_payload_and_preview_while_caller_content_is_ignored() {
    use dsheng_cli::store::generation as gen;
    let f = Fixture::new("task-report-payload-preview");
    let mut conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let without =
        gen::build_task_payload(&conn, f.workspace, &f.run_id, "合成任务检视", None).unwrap();
    let without_json: Value = serde_json::from_str(&without.payload_json).unwrap();
    assert!(without_json["optional_diagnosis_reference"].is_null());
    let record = &without_json["source_records"][0];
    let evidence_id = record["evidence_id"].as_str().unwrap();
    let mut saved: Value = serde_json::from_str(
        &report(&f, &fp, "5w")
            .to_string()
            .replace("evidence-one", evidence_id),
    )
    .unwrap();
    saved["evidence_refs"][0]["source_id"] = record["source_id"].clone();
    saved["evidence_refs"][0]["source_label"] = record["file_label"].clone();
    saved["evidence_refs"][0]["locator"] = record["cell_or_row"].clone();
    seed(&conn, &f, &saved, &saved);
    let forged = json!({
        "report_id":saved["report_id"],"framework":"5why",
        "summary":"调用方伪造内容不应发送","facts":[{"statement":"伪造事实"}]
    });
    let selected =
        gen::build_task_payload(&conn, f.workspace, &f.run_id, "合成任务检视", Some(&forged))
            .unwrap();
    let selected_json: Value = serde_json::from_str(&selected.payload_json).unwrap();
    let reference = &selected_json["optional_diagnosis_reference"];
    assert_eq!(reference["framework"], "5w");
    assert_eq!(reference["report_id"], saved["report_id"]);
    assert!(reference["summary"]
        .as_str()
        .unwrap()
        .contains("有依据的诊断摘要"));
    assert!(reference["facts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["statement"] == "实绩下降"));
    assert!(reference["facts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["statement"]
            .as_str()
            .is_some_and(|text| text.contains("何时发生？"))));
    assert!(!selected.payload_json.contains("伪造"));
    assert_ne!(selected.input_hash, without.input_hash);
    assert_eq!(
        selected.report_id.map(|id| id.to_string()),
        saved["report_id"].as_str().map(str::to_string)
    );
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &selected,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    assert_eq!(preview["report_id"], saved["report_id"]);
    assert_eq!(preview["input_manifest_hash"], selected.input_hash);
    assert_eq!(preview["task_context_summary"]["diagnosis_framework"], "5w");
    assert_eq!(preview["task_context_summary"]["diagnosis_fact_count"], 7);
    assert_eq!(
        preview["task_context_summary"]["diagnosis_hypothesis_count"],
        2
    );
    let attempts: i64 = conn
        .query_row("SELECT COUNT(*) FROM generation_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(attempts, 0, "报告引用预览不派发模型请求");
    // 只消费本地确认后标为失败，不派发；手动重试仍须携带原选报告。
    use dsheng_core::provider::confirm::ConfirmationGate;
    let attempt = {
        let mut store = gen::SqliteGenerationStore { conn: &mut conn };
        let attempt = ConfirmationGate
            .consume_and_create_attempt(
                &mut store,
                preview["confirmation_id"]
                    .as_str()
                    .unwrap()
                    .parse()
                    .unwrap(),
                chrono::Utc::now(),
            )
            .unwrap()
            .attempt()
            .clone();
        ConfirmationGate
            .finish(
                &mut store,
                attempt.attempt_id,
                dsheng_core::contracts::provider::TerminalState::Failed,
                chrono::Utc::now(),
            )
            .unwrap();
        attempt
    };
    let args = [
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
    ]
    .map(str::to_string);
    let (mut stdout, mut stderr) = (vec![], vec![]);
    assert_eq!(
        dsheng_cli::run_with_output(&args, &mut stdout, &mut stderr),
        0,
        "{}",
        String::from_utf8_lossy(&stdout)
    );
    let retry: Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(retry["data"]["report_id"], saved["report_id"]);
    assert_eq!(
        retry["data"]["task_context_summary"]["diagnosis_framework"],
        "5w"
    );
    assert_ne!(retry["data"]["confirmation_id"], preview["confirmation_id"]);
    assert_eq!(
        conn.query_row(
            "SELECT SUM(request_count) FROM generation_attempts",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn report_aliases_keep_real_evidence_and_qualifications_without_importing_unavailable_sources() {
    use dsheng_cli::store::generation as gen;
    let f = Fixture::new("task-report-evidence-closure");
    let conn = Connection::open(&f.db).unwrap();
    let base = gen::build_task_payload(&conn, f.workspace, &f.run_id, "检视", None).unwrap();
    let base_json: Value = serde_json::from_str(&base.payload_json).unwrap();
    let record = &base_json["source_records"][0];
    let canonical = record["evidence_id"].as_str().unwrap();
    let alias = format!(
        "cell:{}:{}:{}",
        record["file_label"].as_str().unwrap(),
        record["sheet"].as_str().unwrap_or("csv"),
        record["cell_or_row"].as_str().unwrap()
    );
    let mut saved: Value = serde_json::from_str(
        &report(&f, &fingerprint(&conn, &f), "5why")
            .to_string()
            .replace("evidence-one", &alias),
    )
    .unwrap();
    saved["evidence_refs"][0]["source_id"] = record["source_id"].clone();
    saved["warnings"] = json!([{"warning_id":"warning-1","message":"经营原因尚未核实","status":"active",
        "resolution_requirement":"source_record","evidence_ids":[alias],"fact_ids":["hypothesis-1"],"resolution_note":null}]);
    saved["clarification_questions"] = json!([{"question_id":"question-1","question":"交付时间是否确认？","status":"open","answer_summary":null}]);
    seed(&conn, &f, &saved, &saved);
    let selected =
        gen::build_task_payload(&conn, f.workspace, &f.run_id, "检视", Some(&saved)).unwrap();
    let payload: Value = serde_json::from_str(&selected.payload_json).unwrap();
    let reference = &payload["optional_diagnosis_reference"];
    assert_eq!(reference["evidence_ids"], json!([canonical]));
    assert_eq!(reference["facts"][0]["confidence"], "high");
    assert_eq!(reference["facts"][0]["reasoning_summary"], "来源显示");
    assert!(reference["summary"]
        .as_str()
        .unwrap()
        .contains("经营原因尚未核实"));
    assert!(reference["summary"]
        .as_str()
        .unwrap()
        .contains("交付时间是否确认？"));
    assert_eq!(
        payload["source_manifest"], base_json["source_manifest"],
        "别名解析不扩大来源范围"
    );
    let stored: String = conn
        .query_row(
            "SELECT report_json FROM diagnosis_reports WHERE report_id=?1",
            [saved["report_id"].as_str().unwrap()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stored).unwrap(),
        saved,
        "只读投影不改写历史报告"
    );
    let foreign_id = format!("cell:{}:核算表:A1", Uuid::new_v4());
    let invalid = saved.to_string().replace(&alias, &foreign_id);
    conn.execute(
        "UPDATE diagnosis_reports SET report_json=?1 WHERE report_id=?2",
        params![invalid, saved["report_id"].as_str().unwrap()],
    )
    .unwrap();
    let err = gen::build_task_payload(&conn, f.workspace, &f.run_id, "检视", Some(&saved))
        .err()
        .expect("不可用证据必须阻止发送");
    assert_eq!(err.code, CliErrorCode::InputContractInvalid);
    assert_eq!(err.details["unavailable_evidence_ids"], json!([foreign_id]));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM generation_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn task_reference_rejects_old_series_version_while_history_stays_readable() {
    use dsheng_cli::store::{diagnosis, generation as gen};
    use dsheng_core::provider::confirm::ConfirmationGate;
    let f = Fixture::new("task-latest-report-only");
    let mut conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let base = gen::build_task_payload(&conn, f.workspace, &f.run_id, "检视", None).unwrap();
    let payload: Value = serde_json::from_str(&base.payload_json).unwrap();
    let evidence = payload["source_records"][0]["evidence_id"]
        .as_str()
        .unwrap();
    let old: Value = serde_json::from_str(
        &report(&f, &fp, "5w")
            .to_string()
            .replace("evidence-one", evidence),
    )
    .unwrap();
    seed(&conn, &f, &old, &old);
    let pending =
        gen::build_task_payload(&conn, f.workspace, &f.run_id, "检视", Some(&old)).unwrap();
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &pending,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let attempt = ConfirmationGate
        .consume_and_create_attempt(
            &mut gen::SqliteGenerationStore { conn: &mut conn },
            preview["confirmation_id"]
                .as_str()
                .unwrap()
                .parse()
                .unwrap(),
            chrono::Utc::now(),
        )
        .unwrap()
        .attempt()
        .attempt_id;
    let unsent_preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &pending,
        dsheng_core::contracts::provider::GenerationOperation::TaskGenerate,
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    let latest: Value = serde_json::from_str(
        &report(&f, &fp, "5w")
            .to_string()
            .replace("evidence-one", evidence),
    )
    .unwrap();
    seed(&conn, &f, &latest, &latest);
    #[derive(Default)]
    struct NoNetwork(usize);
    impl gen::RequestDispatch for NoNetwork {
        fn dispatch(
            &mut self,
            _: &gen::ProviderRuntime,
            _: &dsheng_core::contracts::provider::ChatCompletionsRequest,
            _: &mut dyn FnMut(dsheng_core::provider::http::ModelProgress),
        ) -> Result<dsheng_core::contracts::provider::ChatCompletionsResponse, (CliErrorCode, String)>
        {
            self.0 += 1;
            Err((
                CliErrorCode::NetworkError,
                "synthetic network tripwire".into(),
            ))
        }
    }
    let mut network = NoNetwork::default();
    let stale_send = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &pending,
        unsent_preview["confirmation_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        dsheng_core::contracts::prompts::TASK_PROMPT_VERSION,
        "2",
        &mut network,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(stale_send.code, CliErrorCode::StaleDiagnosisReference);
    assert_eq!(
        network.0, 0,
        "cached material must be rejected before dispatch"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM generation_attempts WHERE workflow='task'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    let late = gen::save_task_candidate(
        &mut conn,
        f.workspace,
        &pending,
        attempt,
        &json!({"schema_version":"2","summary":"合成迟到结果","tasks":[]}),
        false,
    )
    .unwrap_err();
    assert_eq!(late.code, CliErrorCode::StaleDiagnosisReference);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM task_candidates", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let old_id = old["report_id"].as_str().unwrap();
    let new_id = latest["report_id"].as_str().unwrap();
    let error = load_task_report(&conn, f.workspace, &f.run_id, 1, &fp, old_id).unwrap_err();
    assert_eq!(error.code, CliErrorCode::StaleDiagnosisReference);
    assert!(error.message.contains("最新版本"));
    assert!(load_task_report(&conn, f.workspace, &f.run_id, 1, &fp, new_id).is_ok());
    assert_eq!(
        diagnosis::report_show(&conn, f.workspace, old_id, None).unwrap(),
        old
    );
    conn.execute(
        "UPDATE diagnosis_reports SET state='archived' WHERE report_id=?1",
        [old_id],
    )
    .unwrap();
    let list = diagnosis::report_list(&conn, f.workspace, &f.run_id).unwrap();
    assert_eq!(list["reports"].as_array().unwrap().len(), 1);
    let head = &list["reports"][0];
    assert_eq!(head["report_id"], new_id);
    assert_eq!(head["version_no"], 2);
    assert_eq!(head["is_latest"], true);
    assert_eq!(head["referenceable"], true);
    assert_eq!(head["versions"][0]["report_id"], old_id);
    conn.execute(
        "UPDATE analysis_runs SET current_revision_no=2 WHERE run_id=?1",
        [&f.run_id],
    )
    .unwrap();
    let changed = diagnosis::report_list(&conn, f.workspace, &f.run_id).unwrap();
    assert_eq!(changed["reports"][0]["referenceable"], false);
    assert_eq!(
        diagnosis::report_show(&conn, f.workspace, old_id, None).unwrap(),
        old
    );
}
