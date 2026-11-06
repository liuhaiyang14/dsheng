//! 人工任务 CLI 全链路：临时合成数据库，期间/幂等/校验失败原子性，无模型请求。
mod fixture;

use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn run(args: &[&str]) -> Value {
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let (mut stdout, mut stderr) = (vec![], vec![]);
    dsheng_cli::run_with_output(&args, &mut stdout, &mut stderr);
    serde_json::from_slice(&stdout).expect("CLI JSON envelope")
}

fn ok(result: Value) -> Value {
    assert_eq!(result["ok"], true, "{result}");
    result["data"].clone()
}

fn input(f: &Fixture) -> Value {
    let conn = Connection::open(&f.db).unwrap();
    let fingerprint: String = conn
        .query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",
            [&f.run_id],
            |row| row.get(0),
        )
        .unwrap();
    json!({"schema_version":"1","workspace_id":f.workspace,"run_id":f.run_id,
        "analysis_revision_no":1,"input_fingerprint":fingerprint,"mutation_id":Uuid::new_v4(),
        "cycle_role":"plan","source_note":"人工确认目标依据",
        "content":{"title":"人工核对安排","summary":"依据现场情况制定","category":"other","metric_id":null,
            "target_value":"12345.125","unit":"元","note":"人工创建备注","review_note":"复盘记录"},
        "steps":[{"action":"核对原始记录","owner":null,"start_date":null,"due_date":null,"done_criteria":"有核对记录"}]
    })
}

fn create(f: &Fixture, input: &Value) -> Value {
    let path = f.dir.join("manual-task-create.json");
    std::fs::write(&path, input.to_string()).unwrap();
    run(&[
        "task",
        "create",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--input-json",
        &path.to_string_lossy(),
        "--json",
    ])
}

fn edit(f: &Fixture, id: &str, revision: u32, patch: Value, source_note: Option<&str>) -> Value {
    let path = f.dir.join("manual-task-edit.json");
    std::fs::write(
        &path,
        json!({"schema_version":"1","workspace_id":f.workspace,"task_id":id,
        "expected_task_revision":revision,"mutation_id":Uuid::new_v4(),
        "operation":{"kind":"content","patch":patch,"source_note":source_note}})
        .to_string(),
    )
    .unwrap();
    run(&[
        "task",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--task",
        id,
        "--patch-json",
        &path.to_string_lossy(),
        "--json",
    ])
}

fn counts(conn: &Connection) -> Vec<i64> {
    [
        "tasks",
        "task_steps",
        "task_events",
        "task_revisions",
        "task_period_links",
        "local_mutations",
        "run_task_snapshots",
    ]
    .into_iter()
    .map(|table| {
        conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    })
    .collect()
}

#[test]
fn manual_plan_is_visible_in_current_run_and_retry_after_edit_returns_same_current_task() {
    let f = Fixture::new("cli-manual-task-flow");
    let conn = Connection::open(&f.db).unwrap();
    conn.execute("UPDATE workspaces SET active_provider_id=NULL", [])
        .unwrap();
    conn.execute("DELETE FROM providers", []).unwrap();
    let raw = input(&f);
    let first = ok(create(&f, &raw));
    let id = first["task_id"].as_str().unwrap();
    assert_eq!(
        first["target_period"],
        json!({"from":"2026-11-01","to":"2026-11-30"})
    );
    assert_eq!(first["task_revision_no"], 1);
    assert_eq!(first["target_value"], "12345.125");
    assert_eq!(first["note"], "人工创建备注");
    assert_eq!(first["review_note"], "复盘记录");
    let listed = ok(run(&[
        "task",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--json",
    ]));
    assert!(listed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["task_id"] == id));
    let detail = ok(run(&[
        "history",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--json",
    ]));
    assert_eq!(
        detail["analysis_input_fingerprint"],
        raw["input_fingerprint"]
    );
    assert!(detail["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|task| task["task_id"] == id));
    let reason: String = conn
        .query_row(
            "SELECT reason FROM task_events WHERE task_id=?1 AND event_type='task_created'",
            [id],
            |row| row.get(0),
        )
        .unwrap();
    assert!(reason.contains("人工确认目标依据"));
    ok(edit(
        &f,
        id,
        1,
        json!({"title":"人工修订后的任务","actual_value":"678.125","note":"新的备注","review_note":"新的复盘"}),
        Some("依据现场核对结果"),
    ));
    let replay = ok(create(&f, &raw));
    assert_eq!(replay["task_id"], id);
    assert_eq!(replay["task_revision_no"], 2);
    assert_eq!(replay["title"], "人工修订后的任务");
    assert_eq!(replay["actual_value"], "678.125");
    assert_eq!(replay["note"], "新的备注");
    assert_eq!(replay["review_note"], "新的复盘");
    let reason: String = conn.query_row("SELECT reason FROM task_events WHERE task_id=?1 AND event_type='content_patched' ORDER BY event_seq DESC LIMIT 1",[id],|row|row.get(0)).unwrap();
    assert!(reason.contains("依据现场核对结果"));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("SELECT count(*) FROM generation_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let mut different = raw;
    different["content"]["title"] = json!("相同编号的不同意图");
    assert_eq!(
        create(&f, &different)["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );
}

#[test]
fn invalid_manual_creations_leave_no_partial_tasks_events_links_or_snapshots() {
    let f = Fixture::new("cli-manual-task-reject");
    let conn = Connection::open(&f.db).unwrap();
    let original = input(&f);
    // 在同一合成库改变已存在对象的归属，验证真实的企业边界。
    let foreign_ws = Uuid::new_v4().to_string();
    dsheng_core::repository::ensure_workspace(&conn, &foreign_ws, "另一个合成企业").unwrap();
    let before = counts(&conn);
    let cases = [
        (
            "/input_fingerprint",
            json!("0".repeat(64)),
            "INPUT_REVISION_CHANGED",
        ),
        ("/analysis_revision_no", json!(2), "INPUT_REVISION_CHANGED"),
        ("/workspace_id", json!(foreign_ws), "WORKSPACE_MISMATCH"),
        (
            "/content/metric_id",
            json!(Uuid::new_v4()),
            "TASK_CATEGORY_METRIC_MISMATCH",
        ),
        ("/content/title", json!("   "), "DATA_VALIDATION_FAILED"),
        ("/steps/0/action", json!(""), "DATA_VALIDATION_FAILED"),
        ("/steps", json!([]), "AT_LEAST_ONE_STEP"),
        ("/source_note", Value::Null, "SOURCE_NOTE_REQUIRED"),
        (
            "/steps/0/start_date",
            json!("2026-10-30"),
            "STEP_OUTSIDE_TASK_PERIOD",
        ),
    ];
    for (pointer, value, code) in cases {
        let mut invalid = original.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        let result = create(&f, &invalid);
        assert_eq!(result["ok"], false, "{pointer}: {result}");
        assert_eq!(result["error"]["code"], code, "{pointer}: {result}");
        assert_eq!(counts(&conn), before, "{pointer} 不得部分写入");
    }
    let mut invalid = original.clone();
    invalid["content"]["target_period"] = json!({"from":"2026-11-01","to":"2027-01-01"});
    assert_eq!(create(&f, &invalid)["ok"], false);
    invalid = original.clone();
    invalid["steps"][0]["start_date"] = json!("2026-11-20");
    invalid["steps"][0]["due_date"] = json!("2026-11-10");
    assert_eq!(create(&f, &invalid)["error"]["code"], "DATE_ORDER_INVALID");
    conn.execute(
        "UPDATE analysis_runs SET workspace_id=?1 WHERE run_id=?2",
        params![foreign_ws, f.run_id],
    )
    .unwrap();
    assert_eq!(create(&f, &original)["error"]["code"], "WORKSPACE_MISMATCH");
    conn.execute(
        "UPDATE analysis_runs SET workspace_id=?1 WHERE run_id=?2",
        params![f.workspace.to_string(), f.run_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE metric_definitions SET workspace_id=?1 WHERE workspace_id=?2",
        params![foreign_ws, f.workspace.to_string()],
    )
    .unwrap();
    invalid = original;
    invalid["content"]["metric_id"] = json!(f.metric_id);
    assert_eq!(
        create(&f, &invalid)["error"]["code"],
        "TASK_CATEGORY_METRIC_MISMATCH"
    );
    assert_eq!(counts(&conn), before);
}

#[test]
fn saved_month_anchor_31_uses_short_month_last_day_and_review_uses_source_period() {
    for (tag, source_to, plan_from) in [
        ("normal", "2027-02-27", "2027-02-28"),
        ("leap", "2028-02-28", "2028-02-29"),
    ] {
        let f = Fixture::new(&format!("cli-manual-anchor-{tag}"));
        let conn = Connection::open(&f.db).unwrap();
        let (normalized,policy):(String,String) = conn.query_row("SELECT normalized_metrics_json,period_policy_snapshot_json FROM run_revisions WHERE run_id=?1 AND revision_no=1",[&f.run_id],|row| Ok((row.get(0)?,row.get(1)?))).unwrap();
        let mut normalized: Value = serde_json::from_str(&normalized).unwrap();
        let mut policy: Value = serde_json::from_str(&policy).unwrap();
        let year = &source_to[..4];
        let source = json!({"from":format!("{year}-01-31"),"to":source_to});
        normalized["period"] = source.clone();
        policy["month_start_day"] = json!(31);
        conn.execute("UPDATE run_revisions SET normalized_metrics_json=?1,period_policy_snapshot_json=?2,action_period_snapshot_json=NULL WHERE run_id=?3 AND revision_no=1",params![normalized.to_string(),policy.to_string(),f.run_id]).unwrap();
        let raw = input(&f);
        let planned = ok(create(&f, &raw));
        assert_eq!(
            planned["target_period"],
            json!({"from":plan_from,"to":format!("{year}-03-30")})
        );
        let mut review = raw;
        review["cycle_role"] = json!("review");
        review["mutation_id"] = json!(Uuid::new_v4());
        assert_eq!(ok(create(&f, &review))["target_period"], source);
        let links: Vec<String> = conn
            .prepare("SELECT cycle_role FROM task_period_links ORDER BY cycle_role")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(links, vec!["plan", "review"]);
    }
}

#[test]
fn nullable_manual_edits_distinguish_omitted_fields_from_explicit_clears() {
    let f = Fixture::new("cli-manual-task-nullable");
    let conn = Connection::open(&f.db).unwrap();
    let mut raw = input(&f);
    raw["content"]["owner"] = json!("任务负责人");
    raw["content"]["planned_start"] = json!("2026-11-01");
    raw["content"]["due_date"] = json!("2026-11-30");
    raw["steps"][0]["owner"] = json!("行动负责人");
    raw["steps"][0]["start_date"] = json!("2026-11-02");
    raw["steps"][0]["due_date"] = json!("2026-11-25");
    let created = ok(create(&f, &raw));
    let id = created["task_id"].as_str().unwrap();
    let step_id = created["steps"][0]["step_id"].as_str().unwrap();
    ok(edit(&f, id, 1, json!({"summary":"仅补充任务说明"}), None));
    let show = || {
        ok(run(&[
            "task",
            "show",
            "--db",
            &f.db,
            "--workspace",
            &f.workspace.to_string(),
            "--task",
            id,
            "--json",
        ]))
    };
    let unchanged = show();
    for field in [
        "owner",
        "planned_start",
        "due_date",
        "target_value",
        "note",
        "review_note",
    ] {
        assert_eq!(unchanged[field], created[field], "省略 {field} 应保留");
    }
    for field in ["owner", "start_date", "due_date", "done_criteria"] {
        assert_eq!(
            unchanged["steps"][0][field], created["steps"][0][field],
            "省略行动项 {field} 应保留"
        );
    }
    assert_eq!(
        edit(&f, id, 2, json!({"target_value":null}), None)["error"]["code"],
        "SOURCE_NOTE_REQUIRED"
    );
    assert_eq!(show()["task_revision_no"], 2, "无依据的数值清空不得写入");
    let patch = json!({"schema_version":"1","workspace_id":f.workspace,"task_id":id,
    "expected_task_revision":2,"mutation_id":Uuid::new_v4(),"operation":{
        "kind":"content","source_note":"人工确认清空目标",
        "patch":{"owner":null,"planned_start":null,"due_date":null,"target_value":null,"note":null,"review_note":""},
        "step_operations":[{"op":"update","step_id":step_id,"patch":{
            "owner":null,"start_date":null,"due_date":null,"done_criteria":null
        }}]
    }});
    let path = f.dir.join("manual-task-clear.json");
    std::fs::write(&path, patch.to_string()).unwrap();
    ok(run(&[
        "task",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--task",
        id,
        "--patch-json",
        &path.to_string_lossy(),
        "--json",
    ]));
    let cleared = show();
    assert_eq!(cleared["task_revision_no"], 3);
    for field in ["owner", "planned_start", "due_date", "target_value", "note"] {
        assert_eq!(cleared[field], Value::Null, "显式清空 {field}");
    }
    assert_eq!(cleared["review_note"], "");
    for field in ["owner", "start_date", "due_date", "done_criteria"] {
        assert_eq!(
            cleared["steps"][0][field],
            Value::Null,
            "显式清空行动项 {field}"
        );
    }
    let reason:String=conn.query_row("SELECT reason FROM task_events WHERE task_id=?1 AND event_type='content_patched' ORDER BY event_seq DESC LIMIT 1",[id],|row|row.get(0)).unwrap();
    assert!(reason.contains("人工确认清空目标"));
    assert_eq!(
        conn.query_row("SELECT count(*) FROM generation_attempts", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
