//! 首期候选→应用→次期任务输入；纯合成数据库，不调用模型。
mod fixture;

use chrono::Utc;
use dsheng_cli::store::generation::{self as gen, ProviderRuntime, RequestDispatch};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::prompts::TASK_PROMPT_VERSION;
use dsheng_core::contracts::provider::{
    ChatCompletionsRequest, ChatCompletionsResponse, GenerationOperation,
};
use dsheng_core::contracts::tasks::{ActorSource, TaskReconcileInput};
use dsheng_core::repository::{reconcile, task_store::TaskEditService};
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn payload(conn: &Connection, f: &Fixture, run: &str) -> gen::GenerationMaterial {
    gen::build_task_payload(conn, f.workspace, run, "制定下期任务", None).unwrap()
}

fn next_analysis(f: &Fixture) -> String {
    let args = [
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-11-01",
        "--to",
        "2026-11-30",
        "--grain",
        "auto",
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
    serde_json::from_slice::<Value>(&stdout).unwrap()["data"]["run_id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn pending_candidate(conn: &Connection, f: &Fixture) -> TaskReconcileInput {
    let id = Uuid::new_v4();
    let fingerprint: String = conn
        .query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",
            [&f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let tasks: Vec<Value> = ["核对交付", "回款跟进"].iter().enumerate().map(|(i,title)| json!({
        "task_key":format!("task-{i}"), "title":title, "summary":"合成任务", "category":"other", "metric_id":null,
        "source_period":{"from":"2026-10-01","to":"2026-10-31"},
        "target_period":{"from":"2026-11-01","to":"2026-11-30"},
        "steps":[{"step_id":"check", "order":1,"action":"核对原始记录"}, {"step_id":"follow", "order":2,"action":"记录后续安排"}]
    })).collect();
    conn.execute("INSERT INTO task_candidates (candidate_generation_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,generation_input_fingerprint,candidate_revision_no,attempt_id,output_json,envelope_locally_reconstructed,state,prompt_version,schema_version,created_at) VALUES (?1,?2,?3,1,?4,'local-fixture',1,'local-fixture',?5,0,'candidate',?6,'2','2026-11-01T00:00:00Z')",
        params![id.to_string(),f.workspace.to_string(),f.run_id,fingerprint,json!({"schema_version":"2","summary":"合成候选","tasks":tasks}).to_string(),TASK_PROMPT_VERSION]).unwrap();
    serde_json::from_value(json!({"schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
        "analysis_revision_no":1,"input_fingerprint":fingerprint,"candidate_generation_id":id,
        "candidate_revision_no":1,"expected_tasks":[],"mutation_id":Uuid::new_v4(),
        "mappings":[{"action":"new","candidate_task_key":"task-0"},{"action":"new","candidate_task_key":"task-1"}]})).unwrap()
}

#[derive(Default)]
struct NoNetwork(usize);
impl RequestDispatch for NoNetwork {
    fn dispatch(
        &mut self,
        _: &ProviderRuntime,
        _: &ChatCompletionsRequest,
        _: &mut dyn FnMut(dsheng_core::provider::http::ModelProgress),
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        self.0 += 1;
        panic!("测试不应进入网络派发");
    }
}

#[test]
fn first_period_applied_tasks_flow_into_second_period_and_changed_progress_invalidates_preview() {
    let f = Fixture::new("task-two-periods");
    let mut conn = Connection::open(&f.db).unwrap();
    let first: Value = serde_json::from_str(&payload(&conn, &f, &f.run_id).payload_json).unwrap();
    assert_eq!(first["business_review"]["prior_task_outcomes"], json!([]));
    let candidate = pending_candidate(&conn, &f);
    let next_run = next_analysis(&f);
    let before_apply: Value =
        serde_json::from_str(&payload(&conn, &f, &next_run).payload_json).unwrap();
    assert_eq!(
        before_apply["business_review"]["prior_task_outcomes"],
        json!([]),
        "未应用候选不冒充执行记录"
    );
    let prepared = reconcile::prepare_reconcile(&conn, &candidate, Utc::now()).unwrap();
    let applied = reconcile::apply_reconcile(
        &mut conn,
        &candidate,
        &prepared.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(applied.new_task_ids.len(), 2);
    let material = payload(&conn, &f, &next_run);
    let second: Value = serde_json::from_str(&material.payload_json).unwrap();
    let outcomes = second["business_review"]["prior_task_outcomes"]
        .as_array()
        .unwrap();
    assert_eq!(outcomes.len(), 2);
    assert!(outcomes
        .iter()
        .all(|o| o["status"] == "not_started" && o["progress"] == 0));
    assert!(outcomes[0]["outcome_note"]
        .as_str()
        .unwrap()
        .contains("非期末快照"));
    assert!(outcomes[0]["outcome_note"]
        .as_str()
        .unwrap()
        .contains("记录后续安排"));
    let provider = gen::provider_snapshot(&conn, f.workspace, f.provider).unwrap();
    let preview = gen::create_preview(
        &mut conn,
        f.workspace,
        &provider,
        &material,
        GenerationOperation::TaskGenerate,
        TASK_PROMPT_VERSION,
        "2",
    )
    .unwrap();
    assert_eq!(
        preview["task_context_summary"]["prior_task_titles"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        preview["task_context_summary"]["diagnosis_framework"],
        Value::Null
    );
    let raw = json!({"schema_version":"1","workspace_id":f.workspace,"task_id":outcomes[0]["task_id"],
        "expected_task_revision":1,"mutation_id":Uuid::new_v4(),
        "operation":{"kind":"progress","progress_mode":"manual","progress":"37.5","status":"in_progress",
            "adjustment_reason":"合成执行进展","blocked_reason":null}});
    TaskEditService {
        conn: &mut conn,
        now: Utc::now(),
        actor: ActorSource::Ui,
    }
    .apply_raw(&raw)
    .unwrap();
    let updated = payload(&conn, &f, &next_run);
    assert_ne!(
        material.input_hash, updated.input_hash,
        "执行记录变化必须重新预览"
    );
    let changed: Value = serde_json::from_str(&updated.payload_json).unwrap();
    assert!(changed["business_review"]["prior_task_outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["progress"] == json!(37.5) && t["status"] == "in_progress"));
    let content_edit = json!({"schema_version":"1","workspace_id":f.workspace,"task_id":outcomes[0]["task_id"],
        "expected_task_revision":2,"mutation_id":Uuid::new_v4(),
        "operation":{"kind":"content","patch":{"note":"合成备注：需要核对回款","target_value":"125","unit":"元"},
            "source_note":"合成目标依据"}});
    TaskEditService {
        conn: &mut conn,
        now: Utc::now(),
        actor: ActorSource::Ui,
    }
    .apply_raw(&content_edit)
    .unwrap();
    let updated = payload(&conn, &f, &next_run);
    let changed: Value = serde_json::from_str(&updated.payload_json).unwrap();
    let edited = changed["business_review"]["prior_task_outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["task_id"] == outcomes[0]["task_id"])
        .unwrap();
    assert_eq!(edited["progress"], json!(37.5), "修改内容保留执行进度");
    let note = edited["outcome_note"].as_str().unwrap();
    assert!(note.contains("合成备注：需要核对回款"));
    assert!(note.contains("合成目标依据"));
    let mut dispatch = NoNetwork::default();
    let err = gen::send_confirmed(
        &mut conn,
        f.workspace,
        &provider,
        &updated,
        preview["confirmation_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
        TASK_PROMPT_VERSION,
        "2",
        &mut dispatch,
        &mut |_| {},
    )
    .unwrap_err();
    assert_eq!(err.code, CliErrorCode::ConfirmationStale);
    assert_eq!(dispatch.0, 0);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM generation_attempts", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
