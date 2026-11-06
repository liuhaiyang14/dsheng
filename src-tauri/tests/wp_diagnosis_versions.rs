//! 桌面报告版本闭环：真实命令/保存/读合同，合成数据，无 Provider 派发。
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;

use dsheng_app::contracts::{core as cmd, env::DesktopEnv};
use dsheng_cli::store::generation as gen;
use dsheng_core::contracts::provider::Framework;
use dsheng_core::generation::input::SessionStateProjection;
use dsheng_core::provider::confirm::ConfirmationGate;
use fixture::Fixture;
use rusqlite::Connection;
use serde_json::{json, Value};

fn ok(envelope: Value) -> Value {
    assert_eq!(envelope["ok"], true, "{envelope}");
    envelope["data"].clone()
}

fn validate_view(value: &Value, definition: &str, settings: bool) {
    let mut schema: Value = serde_json::from_str(if settings {
        include_str!("../../contracts/v1/settings.schema.json")
    } else {
        include_str!("../../contracts/v1/read-views.schema.json")
    })
    .unwrap();
    schema.as_object_mut().unwrap().remove("oneOf");
    schema["$ref"] = json!(format!("#/$defs/{definition}"));
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{definition}: {errors:?}");
}

fn commit_synthetic_turn(
    f: &Fixture,
    preview: &Value,
    evidence: &[String],
    summary: &str,
) -> String {
    let mut conn = Connection::open(&f.db).unwrap();
    let material = gen::build_diagnosis_payload(
        &conn,
        f.workspace,
        &f.run_id,
        Framework::W5,
        "合成销售差异",
        &[],
        evidence,
        SessionStateProjection {
            accepted_facts: vec![],
            prior_report_summary: None,
            unresolved_questions: vec![],
            pending_clarification: None,
        },
        None,
        "合成销售差异",
    )
    .unwrap();
    let payload: Value = serde_json::from_str(&material.payload_json).unwrap();
    let source = &payload["source_records"][0];
    let evidence_id = source["evidence_id"].as_str().unwrap();
    let findings: Vec<_> = ["What", "Why", "Where", "When", "Who"]
        .into_iter()
        .map(|dimension| {
            json!({
                "finding_id":format!("finding-{dimension}"),"dimension":dimension,
                "question":"合成核对问题", "answer":summary, "fact_status":"fact",
                "evidence_ids":[evidence_id],"confidence":"high","missing_information":[],
                "clarification_questions":[],"status":"resolved"
            })
        })
        .collect();
    let output = json!({
        "schema_version":"3","framework":"5w","turn_disposition":"on_topic",
        "accepted_facts":[],"redirect_message":null,"clarification_questions":[],"stop_reason":null,
        "report_patch":{
            "summary_update":summary,"findings_upsert":findings,"facts_and_hypotheses_upsert":[],
            "evidence_refs_append":[{"evidence_id":evidence_id,"source_id":source["source_id"],
                "source_label":source["file_label"],"locator":source["cell_or_row"]}],
            "clarification_questions_upsert":[],"warnings_upsert":[],"stop_reason":null,"can_continue":false
        }
    });
    let attempt = {
        let mut store = gen::SqliteGenerationStore { conn: &mut conn };
        ConfirmationGate
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
            .attempt_id
    };
    gen::save_diagnosis_output(
        &mut conn,
        f.workspace,
        &material,
        Framework::W5,
        attempt,
        &output,
    )
    .unwrap()["report_id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn desktop_versions_read_old_body_and_only_latest_can_be_referenced() {
    let f = Fixture::new("desktop-report-versions");
    let env = DesktopEnv::with_db(&f.db);
    let conn = Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER forbid_test_dispatch BEFORE UPDATE OF request_count ON generation_attempts WHEN NEW.request_count>0 BEGIN SELECT RAISE(ABORT,'test forbids network dispatch'); END;").unwrap();
    let evidence =
        dsheng_cli::store::analysis::diagnosis_evidence_ids(&conn, f.workspace, &f.run_id, 1, None)
            .unwrap();
    let request = json!({"workspace_id":f.workspace,"run_id":f.run_id,"framework":"5w","problem":"合成销售差异","evidence_ids":evidence});
    let first_preview = ok(cmd::diagnosis::diagnosis_generate_preview(
        &env,
        request.clone(),
    ));
    assert!(first_preview.get("diagnosis_replacement").is_none());
    let first = commit_synthetic_turn(&f, &first_preview, &evidence, "第一版合成正文");
    let old_task_preview = ok(cmd::task::task_generate_preview(
        &env,
        json!({"workspace_id":f.workspace,"run_id":f.run_id,"diagnosis_report_id":first}),
    ));
    let second_preview = ok(cmd::diagnosis::diagnosis_generate_preview(&env, request));
    validate_view(&second_preview, "confirmationPreview", true);
    assert_eq!(
        second_preview["diagnosis_replacement"],
        json!({"report_id":first,"version_no":1,"next_version_no":2})
    );
    let second = commit_synthetic_turn(&f, &second_preview, &evidence, "第二版合成正文");
    assert_ne!(first, second);

    let list = ok(cmd::diagnosis::diagnosis_list(
        &env,
        f.workspace.to_string(),
        f.run_id.clone(),
    ));
    let reports = list["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    let latest = &reports[0];
    validate_view(latest, "diagnosisReportSummaryView", false);
    assert_eq!(latest["report_id"], second);
    assert_eq!(latest["version_no"], 2);
    assert_eq!(latest["referenceable"], true);
    assert_eq!(latest["versions"].as_array().unwrap().len(), 2);
    for (id, expected) in [(&first, "第一版合成正文"), (&second, "第二版合成正文")] {
        let report = ok(cmd::diagnosis::diagnosis_show(
            &env,
            f.workspace.to_string(),
            id.clone(),
            Some(1),
        ));
        assert_eq!(report["summary"], expected);
        assert_eq!(report["revision_no"], 1, "系列编号不改写旧报告快照版本");
    }
    let stale = cmd::task::task_generate_preview(
        &env,
        json!({"workspace_id":f.workspace,"run_id":f.run_id,"diagnosis_report_id":first}),
    );
    assert_eq!(stale["error"]["code"], "STALE_DIAGNOSIS_REFERENCE");
    let stale_confirm = cmd::task::task_generate_confirm(
        &env,
        old_task_preview["confirmation_id"]
            .as_str()
            .unwrap()
            .to_string(),
        None,
    );
    assert_eq!(stale_confirm["ok"], false);
    assert_eq!(stale_confirm["error"]["code"], "STALE_DIAGNOSIS_REFERENCE");
    ok(cmd::task::task_generate_preview(
        &env,
        json!({"workspace_id":f.workspace,"run_id":f.run_id,"diagnosis_report_id":second}),
    ));
    let count: i64 = conn
        .query_row(
            "SELECT coalesce(sum(request_count),0) FROM generation_attempts",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
}
