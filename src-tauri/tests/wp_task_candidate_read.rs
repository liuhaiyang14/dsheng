//! 任务生成收据→读视图与当前候选恢复：临时 SQLite 合成记录，无 Provider。
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;

use dsheng_app::contracts::{core::task, env::DesktopEnv};
use dsheng_cli::store::views;
use dsheng_core::contracts::cli::CliErrorCode;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn fingerprint(conn: &Connection, f: &Fixture) -> String {
    conn.query_row(
        "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",
        [&f.run_id],
        |row| row.get(0),
    )
    .unwrap()
}

fn output(f: &Fixture) -> Value {
    let criteria = json!({
        "S":{"status":"met","rationale":"核对对象明确","evidence_ids":[]},
        "M":{"status":"needs_confirmation","rationale":"金额目标待确认","evidence_ids":[]},
        "A":{"status":"needs_confirmation","rationale":"资源待确认","evidence_ids":[]},
        "R":{"status":"met","rationale":"关联经营差异","evidence_ids":[]},
        "T":{"status":"not_applicable","rationale":null,"evidence_ids":[]}
    });
    // 旧候选的中文类别也按与 reconcile 相同的 exact 映射读取。
    json!({"schema_version":"2", "summary":"本地合成记录", "tasks":[{
        "task_key":"local-task", "title":"本地候选", "summary":"核对经营数据",
        "category":"销售最大化", "metric_id":f.metric_id,
        "source_period":{"from":"2026-10-01","to":"2026-10-31"},
        "target_period":{"from":"2026-11-01","to":"2026-11-30"},
        "target_value":serde_json::from_str::<Value>("10000000000000000.125").unwrap(),
        "owner":null,
        "smart_check":{
            "overall_goal":{"goal_text":"核对经营数据","criteria":criteria},
            "subgoals":[{"subgoal_id":"local-step","goal_text":"核对原始凭据","criteria":criteria}]
        },
        "steps":[{"step_id":"local-step", "order":1, "action":"核对原始凭据"}]
    }]})
}

fn seed(
    conn: &Connection,
    f: &Fixture,
    revision: u32,
    fingerprint: &str,
    state: &str,
    created: &str,
    raw: &str,
) -> String {
    let id = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO task_candidates (candidate_generation_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,generation_input_fingerprint,candidate_revision_no,attempt_id,output_json,envelope_locally_reconstructed,state,prompt_version,schema_version,created_at) VALUES(?1,?2,?3,?4,?5,'local-fixture',1,'local-fixture',?6,1,?7,'2.3','2',?8)",params![id,f.workspace.to_string(),f.run_id,revision,fingerprint,raw,state,created]).unwrap();
    id
}

fn ok(value: Value) -> Value {
    assert_eq!(value["ok"], true, "{value}");
    value["data"].clone()
}

fn assert_task_read_contract(view: &Value) {
    let mut schema: Value =
        serde_json::from_str(include_str!("../../contracts/v1/read-views.schema.json")).unwrap();
    schema["$ref"] = json!("#/$defs/taskView");
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let errors: Vec<String> = validator.iter_errors(view).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "任务详情未遵循正式读合同：{errors:?}");
}

#[test]
fn generated_receipt_loads_complete_normalized_candidate_without_changing_saved_output() {
    let f = Fixture::new("desktop-task-receipt");
    let conn = Connection::open(&f.db).unwrap();
    let raw = output(&f).to_string();
    let id = seed(
        &conn,
        &f,
        1,
        &fingerprint(&conn, &f),
        "candidate",
        "2026-11-06T00:00:00Z",
        &raw,
    );
    let receipt = json!({"attempt_id":"local-fixture", "candidate_generation_id":id,"state":"candidate","next":"task reconcile --dry-run 应用候选"});
    assert!(receipt.get("tasks").is_none(), "复现真实 CLI 收据形状");
    let view = task::task_candidate_from_receipt(
        &DesktopEnv::with_db(&f.db),
        &json!({"workspace_id":f.workspace,"run_id":f.run_id}),
        &receipt,
    )
    .unwrap();
    assert_eq!(view["candidate_generation_id"], id);
    assert_eq!(view["candidate_revision_no"], 1);
    assert_eq!(view["analysis_revision_no"], 1);
    assert_eq!(view["envelope_locally_reconstructed"], true);
    assert_eq!(view["tasks"][0]["category"], "revenue");
    assert_eq!(view["tasks"][0]["target_value"], "10000000000000000.125");
    assert_eq!(view["tasks"][0]["owner"], Value::Null);
    assert_eq!(
        view["tasks"][0]["steps"],
        json!([{"step_id":"local-step","action":"核对原始凭据"}])
    );
    assert!(view.get("attempt_id").is_none());
    let after: String = conn
        .query_row(
            "SELECT output_json FROM task_candidates WHERE candidate_generation_id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(after, raw);
    let attempts: i64 = conn
        .query_row("SELECT count(*) FROM generation_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(attempts, 0, "只读恢复不发请求、不创建 attempt");
}

#[test]
fn recovery_selects_latest_matching_revision_fingerprint_and_candidate_state() {
    let f = Fixture::new("desktop-task-recover");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let raw = output(&f).to_string();
    seed(&conn, &f, 1, &fp, "candidate", "2026-11-05T00:00:00Z", &raw);
    let expected = seed(&conn, &f, 1, &fp, "candidate", "2026-11-06T00:00:00Z", &raw);
    for (revision, fingerprint, state) in [
        (2, fp.as_str(), "candidate"),
        (1, "stale-fingerprint", "candidate"),
        (1, fp.as_str(), "applied"),
        (1, fp.as_str(), "discarded"),
    ] {
        seed(
            &conn,
            &f,
            revision,
            fingerprint,
            state,
            "2026-12-01T00:00:00Z",
            &raw,
        );
    }
    // 同 created_at 时也必须稳定选择最后保存的一份，不能靠 UUID 排序。
    let latest = seed(&conn, &f, 1, &fp, "candidate", "2026-11-06T00:00:00Z", &raw);
    assert_ne!(latest, expected);
    let env = DesktopEnv::with_db(&f.db);
    let recovered = ok(task::task_candidate_latest(
        &env,
        f.workspace.to_string(),
        f.run_id.clone(),
        1,
    ));
    assert_eq!(recovered["candidate"]["candidate_generation_id"], latest);
    assert_eq!(recovered["analysis_input_fingerprint"], fp);
    assert_eq!(recovered["candidate"]["tasks"].as_array().unwrap().len(), 1);
    let state: String = conn
        .query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id=?1",
            [&latest],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "candidate");
    assert!(
        recovered["candidate"]
            .get("analysis_input_fingerprint")
            .is_none(),
        "冻结读视图不加字段"
    );
    let stale = task::task_candidate_latest(&env, f.workspace.to_string(), f.run_id.clone(), 2);
    assert_eq!(stale["error"]["code"], "INPUT_REVISION_CHANGED");
}

#[test]
fn empty_or_finished_candidates_do_not_invent_suggestions_and_foreign_run_is_rejected() {
    let f = Fixture::new("desktop-task-empty");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let env = DesktopEnv::with_db(&f.db);
    assert_eq!(
        ok(task::task_candidate_latest(
            &env,
            f.workspace.to_string(),
            f.run_id.clone(),
            1
        )),
        Value::Null
    );
    let raw = json!({"schema_version":"2","summary":"无建议","tasks":[]}).to_string();
    let id = seed(&conn, &f, 1, &fp, "candidate", "2026-11-06T00:00:00Z", &raw);
    assert!(views::task_candidate_view(&conn, f.workspace, &id)
        .unwrap()
        .tasks
        .is_empty());
    assert_eq!(
        views::task_candidate_view(&conn, Uuid::new_v4(), &id)
            .unwrap_err()
            .code,
        CliErrorCode::WorkspaceMismatch
    );
    let other = Uuid::new_v4().to_string();
    let err = task::task_candidate_latest(&env, f.workspace.to_string(), other.clone(), 1);
    assert_eq!(err["error"]["code"], "WORKSPACE_MISMATCH");
    let err = task::task_candidate_from_receipt(
        &env,
        &json!({"workspace_id":f.workspace,"run_id":other}),
        &json!({"candidate_generation_id":id}),
    )
    .unwrap_err();
    assert_eq!(err.code, CliErrorCode::InputRevisionChanged);
    conn.execute(
        "UPDATE task_candidates SET state='applied' WHERE candidate_generation_id=?1",
        [&id],
    )
    .unwrap();
    assert_eq!(
        ok(task::task_candidate_latest(
            &env,
            f.workspace.to_string(),
            f.run_id.clone(),
            1
        )),
        Value::Null
    );
}

#[test]
fn malformed_saved_candidate_returns_read_error_instead_of_a_success_receipt() {
    let f = Fixture::new("desktop-task-invalid");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let mut missing_steps = output(&f);
    missing_steps["tasks"][0]
        .as_object_mut()
        .unwrap()
        .remove("steps");
    for raw in [
        "not-json".to_string(),
        json!({"summary":"missing tasks"}).to_string(),
        missing_steps.to_string(),
    ] {
        let id = seed(&conn, &f, 1, &fp, "candidate", "2026-11-06T00:00:00Z", &raw);
        let err = task::task_candidate_from_receipt(
            &DesktopEnv::with_db(&f.db),
            &json!({"workspace_id":f.workspace,"run_id":f.run_id}),
            &json!({"candidate_generation_id":id}),
        )
        .unwrap_err();
        assert_eq!(err.code, CliErrorCode::DataValidationFailed);
        assert!(err.message.contains("请勿重复生成"));
    }
}

#[test]
fn recovered_candidate_can_be_applied_without_another_generation_request() {
    let f = Fixture::new("desktop-task-recover-apply");
    let conn = Connection::open(&f.db).unwrap();
    let fp = fingerprint(&conn, &f);
    let id = seed(
        &conn,
        &f,
        1,
        &fp,
        "candidate",
        "2026-11-06T00:00:00Z",
        &output(&f).to_string(),
    );
    let env = DesktopEnv::with_db(&f.db);
    let recovered = ok(task::task_candidate_latest(
        &env,
        f.workspace.to_string(),
        f.run_id.clone(),
        1,
    ));
    let input = json!({
        "schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
        "analysis_revision_no":1,
        "input_fingerprint":recovered["analysis_input_fingerprint"],
        "candidate_generation_id":recovered["candidate"]["candidate_generation_id"],
        "candidate_revision_no":recovered["candidate"]["candidate_revision_no"],
        "expected_tasks":[], "mutation_id":Uuid::new_v4().to_string(),
        "mappings":[{"action":"new","candidate_task_key":"local-task"}]
    });
    let dry = ok(task::task_reconcile_dry_run(&env, input.clone()));
    ok(task::task_reconcile_apply(
        &env,
        input,
        dry["apply_token"].as_str().unwrap().to_string(),
    ));
    let state: String = conn
        .query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id=?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "applied");
    assert_eq!(
        ok(task::task_candidate_latest(
            &env,
            f.workspace.to_string(),
            f.run_id.clone(),
            1
        )),
        Value::Null
    );
    let tasks = ok(task::task_list(
        &env,
        f.workspace.to_string(),
        Some(f.run_id.clone()),
    ));
    assert!(tasks.to_string().contains("本地候选"));
    let list_view = &tasks["tasks"][0];
    assert_task_read_contract(list_view);
    let task_id = list_view["task_id"].as_str().unwrap().to_string();
    let shown = ok(task::task_show(
        &env,
        f.workspace.to_string(),
        task_id.clone(),
    ));
    assert_task_read_contract(&shown);
    assert_eq!(shown, *list_view);
    let checks = shown["smart_check"]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 10, "总目标和子目标各五项评价完整保留");
    for group in checks.chunks(5) {
        assert_eq!(group[0]["dimension"], "S");
        assert_eq!(group[0]["status"], "met");
        assert_eq!(group[1]["status"], "needs_confirmation");
        assert_eq!(group[4]["status"], "not_applicable");
    }
    assert_eq!(shown["smart_check"]["smart_status"], "generated");
    assert_eq!(shown["smart_check"]["assessed_task_revision"], 1);
    let saved_smart = || -> String {
        conn.query_row(
            "SELECT smart_check_json FROM tasks WHERE task_id=?1",
            [&task_id],
            |row| row.get(0),
        )
        .unwrap()
    };
    let original_smart = saved_smart();
    assert_eq!(
        serde_json::from_str::<Value>(&original_smart).unwrap(),
        output(&f)["tasks"][0]["smart_check"]
    );
    // 编辑只标记旧评价待复核；重读不能把旧评价冒充当前版本重新评估。
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1", "workspace_id":f.workspace, "task_id":task_id,
            "expected_task_revision":1, "mutation_id":Uuid::new_v4(),
            "operation":{"kind":"content", "patch":{"title":"本地修改后的任务"}, "step_operations":[], "source_note":"合成回归"}
        }),
    ));
    let edited = ok(task::task_show(
        &env,
        f.workspace.to_string(),
        task_id.clone(),
    ));
    assert_task_read_contract(&edited);
    assert_eq!(edited["task_revision_no"], 2);
    assert_eq!(edited["smart_check"]["smart_status"], "stale");
    assert_eq!(edited["smart_check"]["assessed_task_revision"], 1);
    assert_eq!(
        edited["smart_check"]["checks"],
        shown["smart_check"]["checks"]
    );
    assert_eq!(saved_smart(), original_smart, "读视图转换不改写已有评价");
    let attempts: i64 = conn
        .query_row("SELECT count(*) FROM generation_attempts", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(attempts, 0, "恢复和应用均不生成或重复收费调用");
}

#[test]
fn new_and_ignored_candidates_preserve_existing_edited_task_and_progress() {
    let f = Fixture::new("desktop-task-apply-existing");
    let conn = Connection::open(&f.db).unwrap();
    conn.execute_batch("CREATE TRIGGER forbid_candidate_test_dispatch BEFORE INSERT ON generation_attempts BEGIN SELECT RAISE(ABORT,'candidate application must stay local'); END;").unwrap();
    let fp = fingerprint(&conn, &f);
    let env = DesktopEnv::with_db(&f.db);
    let created = ok(task::task_create(
        &env,
        json!({
            "schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
            "analysis_revision_no":1, "input_fingerprint":fp, "mutation_id":Uuid::new_v4(),
            "cycle_role":"review", "content":{
                "title":"已有现场核对任务", "summary":"人工维护的正式任务", "category":"other"
            }, "steps":[{"action":"核对现场执行记录"}]
        }),
    ));
    let existing_id = created["task_id"].as_str().unwrap().to_string();
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1", "workspace_id":f.workspace, "task_id":existing_id,
            "expected_task_revision":1, "mutation_id":Uuid::new_v4(),
            "operation":{"kind":"content", "patch":{"title":"人工修订的现场核对任务","summary":"保留人工补充说明"}}
        }),
    ));
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1", "workspace_id":f.workspace, "task_id":existing_id,
            "expected_task_revision":2, "mutation_id":Uuid::new_v4(),
            "operation":{"kind":"progress", "progress_mode":"manual", "progress":"37.5",
                "status":"in_progress", "adjustment_reason":"按现场执行情况记录", "blocked_reason":null}
        }),
    ));
    let existing_before = ok(task::task_show(
        &env,
        f.workspace.to_string(),
        existing_id.clone(),
    ));
    assert_eq!(existing_before["progress"], "37.5");
    let event_count = || {
        conn.query_row(
            "SELECT count(*) FROM task_events WHERE task_id=?1",
            [&existing_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap()
    };
    let original_event_count = event_count();

    let mut raw = output(&f);
    let template = raw["tasks"][0].clone();
    raw["tasks"] = json!(["selected-a", "ignored-b", "selected-c"].map(|key| {
        let mut candidate = template.clone();
        candidate["task_key"] = json!(key);
        candidate["title"] = json!(format!("新增合成任务 {key}"));
        candidate
    }));
    let candidate_id = seed(
        &conn,
        &f,
        1,
        &fp,
        "candidate",
        "2026-11-06T00:00:00Z",
        &raw.to_string(),
    );
    let recovered = ok(task::task_candidate_latest(
        &env,
        f.workspace.to_string(),
        f.run_id.clone(),
        1,
    ));
    assert_eq!(recovered["candidate"]["tasks"].as_array().unwrap().len(), 3);
    let mut input = json!({
        "schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
        "analysis_revision_no":1, "input_fingerprint":recovered["analysis_input_fingerprint"],
        "candidate_generation_id":candidate_id, "candidate_revision_no":1,
        "expected_tasks":[{"task_id":existing_id,"task_revision_no":existing_before["task_revision_no"]}],
        "mutation_id":Uuid::new_v4(), "mappings":[
            {"action":"new","candidate_task_key":"selected-a"},
            {"action":"ignore","candidate_task_key":"ignored-b"},
            {"action":"new","candidate_task_key":"selected-c"}
        ]
    });
    // Reproduce the old desktop payload: unrelated existing tasks are not update targets.
    let rejected = task::task_reconcile_dry_run(&env, input.clone());
    assert_eq!(rejected["ok"], false);
    assert!(rejected["error"]["message"]
        .as_str()
        .unwrap()
        .contains("expected_tasks_mismatch"));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id=?1",
            [&candidate_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "candidate"
    );
    assert_eq!(event_count(), original_event_count);

    // Only new/ignore mappings means no existing task revision belongs in the request.
    input["expected_tasks"] = json!([]);
    let dry = ok(task::task_reconcile_dry_run(&env, input.clone()));
    assert_eq!(dry["created_new"], json!(["selected-a", "selected-c"]));
    assert_eq!(dry["ignored"], json!(["ignored-b"]));
    ok(task::task_reconcile_apply(
        &env,
        input,
        dry["apply_token"].as_str().unwrap().to_string(),
    ));
    let listed = ok(task::task_list(
        &env,
        f.workspace.to_string(),
        Some(f.run_id.clone()),
    ));
    let tasks = listed["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 3);
    for key in ["selected-a", "selected-c"] {
        let view = tasks
            .iter()
            .find(|t| t["title"] == format!("新增合成任务 {key}"))
            .unwrap();
        assert_task_read_contract(view);
        assert_eq!(view["target_value"], "10000000000000000.125");
        assert_eq!(view["progress"], "0");
        assert_eq!(view["status"], "not_started");
    }
    assert!(!tasks.iter().any(|t| t["title"] == "新增合成任务 ignored-b"));
    assert_eq!(
        ok(task::task_show(
            &env,
            f.workspace.to_string(),
            existing_id.clone()
        )),
        existing_before
    );
    assert_eq!(
        event_count(),
        original_event_count,
        "new/ignore must not alter existing task history"
    );
    assert_eq!(
        conn.query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id=?1",
            [&candidate_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "applied"
    );
    assert_eq!(
        ok(task::task_candidate_latest(
            &env,
            f.workspace.to_string(),
            f.run_id.clone(),
            1
        )),
        Value::Null
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM generation_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
