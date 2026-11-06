//! 人工任务在桌面桥接中的创建、读回和编辑；只用合成 SQLite，不配置 Provider。
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;

use dsheng_app::contracts::{core::task, env::DesktopEnv};
use fixture::Fixture;
use rusqlite::Connection;
use serde_json::{json, Value};
use uuid::Uuid;

fn ok(value: Value) -> Value {
    assert_eq!(value["ok"], true, "{value}");
    value["data"].clone()
}

fn assert_task_view(view: &Value) {
    let mut schema: Value =
        serde_json::from_str(include_str!("../../contracts/v1/read-views.schema.json")).unwrap();
    schema["$ref"] = json!("#/$defs/taskView");
    let validator = jsonschema::options()
        .should_validate_formats(true)
        .build(&schema)
        .unwrap();
    let errors: Vec<_> = validator.iter_errors(view).map(|e| e.to_string()).collect();
    assert!(errors.is_empty(), "任务读视图不符合合同：{errors:?}");
}

#[test]
fn manual_create_returns_editable_task_without_provider_or_generation() {
    let f = Fixture::new("desktop-manual-task");
    let conn = Connection::open(&f.db).unwrap();
    conn.execute("UPDATE workspaces SET active_provider_id=NULL", [])
        .unwrap();
    conn.execute("DELETE FROM providers", []).unwrap();
    let fingerprint: String = conn
        .query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",
            [&f.run_id],
            |row| row.get(0),
        )
        .unwrap();
    let task_count: i64 = conn
        .query_row("SELECT count(*) FROM tasks", [], |row| row.get(0))
        .unwrap();
    let env = DesktopEnv::with_db(&f.db);
    let input = json!({
        "schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
        "analysis_revision_no":1, "input_fingerprint":fingerprint,
        "mutation_id":Uuid::new_v4(), "cycle_role":"review",
        "source_note":"人工确认的合成目标", "content":{
            "title":"人工核对交付安排", "summary":"核对后记录实际安排", "category":"other",
            "metric_id":null, "target_value":"12.5", "unit":"项", "owner":"运营负责人"
        },
        "steps":[{"action":"核对原始记录", "owner":"经办人", "start_date":"2026-10-01",
            "due_date":"2026-10-10", "done_criteria":"留下核对记录"}]
    });
    let created = ok(task::task_create(&env, input.clone()));
    assert_task_view(&created);
    let id = created["task_id"].as_str().unwrap().to_string();
    let first_step = created["steps"][0]["step_id"].as_str().unwrap().to_string();
    assert_eq!(created["title"], "人工核对交付安排");
    assert_eq!(created["task_revision_no"], 1);
    assert_eq!(created["status"], "not_started");
    assert_eq!(created["progress"], "0");
    assert_eq!(created["target_value"], "12.5");
    assert_eq!(
        created["target_period"],
        json!({"from":"2026-10-01","to":"2026-10-31"})
    );
    assert_eq!(created["smart_check"]["smart_status"], "stale");
    assert_eq!(created["smart_check"]["checks"], json!([]));
    let replay = ok(task::task_create(&env, input));
    assert_eq!(replay["task_id"], id);
    assert_eq!(
        conn.query_row("SELECT count(*) FROM tasks", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        task_count + 1,
        "创建重试不能重复生成任务"
    );
    let listed = ok(task::task_list(
        &env,
        f.workspace.to_string(),
        Some(f.run_id.clone()),
    ));
    assert!(listed["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["task_id"] == id));
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1","workspace_id":f.workspace,"task_id":id,"expected_task_revision":1,
            "mutation_id":Uuid::new_v4(),"operation":{"kind":"content","patch":{"title":"人工修订交付安排"},
                "step_operations":[
                    {"op":"update","step_id":first_step,"patch":{"action":"逐项核对原始记录","owner":"复核人","done_criteria":"完成复核签字"}},
                    {"op":"add","client_step_key":"followup","before_step_id":null,
                        "content":{"action":"跟进差异","owner":null,"start_date":null,"due_date":null,"done_criteria":"记录处理结果"}}
                ]}
        }),
    ));
    let edited = ok(task::task_show(&env, f.workspace.to_string(), id.clone()));
    assert_task_view(&edited);
    assert_eq!(edited["title"], "人工修订交付安排");
    assert_eq!(edited["steps"].as_array().unwrap().len(), 2);
    assert_eq!(edited["steps"][0]["action"], "逐项核对原始记录");
    assert_eq!(edited["steps"][0]["owner"], "复核人");
    assert_eq!(edited["steps"][0]["done_criteria"], "完成复核签字");
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1","workspace_id":f.workspace,"task_id":id,"expected_task_revision":2,
            "mutation_id":Uuid::new_v4(),"operation":{"kind":"progress","progress_mode":"manual","progress":"37.5",
                "status":"in_progress","adjustment_reason":"按现场执行情况记录","blocked_reason":null}
        }),
    ));
    ok(task::task_update(
        &env,
        json!({
            "schema_version":"1","workspace_id":f.workspace,"task_id":id,"expected_task_revision":3,
            "mutation_id":Uuid::new_v4(),"operation":{"kind":"content","patch":{"summary":"人工补充现场说明"}}
        }),
    ));
    let final_view = ok(task::task_show(&env, f.workspace.to_string(), id));
    assert_task_view(&final_view);
    assert_eq!(final_view["summary"], "人工补充现场说明");
    assert_eq!(final_view["progress"], "37.5");
    assert_eq!(final_view["status"], "in_progress");
    assert_eq!(final_view["progress_mode"], "manual");
    for table in ["providers", "task_candidates", "generation_attempts"] {
        let count: i64 = conn
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "人工编辑不能依赖或创建 {table}");
    }
}
