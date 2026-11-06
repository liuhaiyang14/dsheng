//! 纯本地合成数据库：任务编辑保留省略字段和显式 null 的区别。
mod fixture;

use dsheng_core::contracts::tasks::{ActorSource, TaskEditInput};
use dsheng_core::repository::task_store::TaskEditService;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn seed(f: &Fixture) -> String {
    let id = Uuid::new_v4().to_string();
    let conn = Connection::open(&f.db).unwrap();
    conn.execute(
        "INSERT INTO tasks (task_id,workspace_id,title,category,metric_id,created_at,updated_at)
         VALUES (?1,?2,'合成任务','revenue',?3,'2026-11-06T00:00:00Z','2026-11-06T00:00:00Z')",
        params![id, f.workspace.to_string(), f.metric_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO task_steps (step_id,task_id,step_order,action,created_at,updated_at)
         VALUES (?1,?2,1,'合成步骤','2026-11-06T00:00:00Z','2026-11-06T00:00:00Z')",
        params![Uuid::new_v4().to_string(), id],
    )
    .unwrap();
    id
}

fn input(f: &Fixture, id: &str, patch: Value) -> Value {
    json!({"schema_version":"1", "workspace_id":f.workspace, "task_id":id,
        "expected_task_revision":1,"mutation_id":Uuid::new_v4(),
        "operation":{"kind":"content","patch":patch}})
}

fn edit(f: &Fixture, raw: &Value) -> Value {
    let path = f.dir.join("task-edit-presence.json");
    std::fs::write(&path, raw.to_string()).unwrap();
    let args = [
        "task",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--task",
        raw["task_id"].as_str().unwrap(),
        "--patch-json",
        &path.to_string_lossy(),
        "--json",
    ]
    .map(str::to_string);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    dsheng_cli::run_with_output(&args, &mut stdout, &mut stderr);
    serde_json::from_slice(&stdout).unwrap()
}

fn metric(f: &Fixture, id: &str) -> Option<String> {
    Connection::open(&f.db)
        .unwrap()
        .query_row(
            "SELECT metric_id FROM tasks WHERE task_id=?1",
            [id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn omitted_metric_survives_title_edit_and_same_request_replays() {
    let f = Fixture::new("task-omitted-metric");
    let id = seed(&f);
    let raw = input(&f, &id, json!({"title":"更新标题"}));
    let first = edit(&f, &raw);
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["data"]["task_revision_no"], 2);
    assert_eq!(metric(&f, &id).as_deref(), Some(f.metric_id.as_str()));
    let replay = edit(&f, &raw);
    assert_eq!(replay["ok"], true, "{replay}");
    assert_eq!(replay["data"]["replayed"], true);
    let mut changed_intent = raw;
    changed_intent["operation"]["patch"]["metric_id"] = Value::Null;
    assert_eq!(
        edit(&f, &changed_intent)["error"]["code"],
        "IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(metric(&f, &id).as_deref(), Some(f.metric_id.as_str()));
}

#[test]
fn explicit_null_still_obeys_category_constraint_and_can_clear_other_task() {
    let f = Fixture::new("task-explicit-null");
    let id = seed(&f);
    let invalid = input(&f, &id, json!({"metric_id":null}));
    assert_eq!(
        edit(&f, &invalid)["error"]["code"],
        "TASK_CATEGORY_METRIC_MISMATCH"
    );
    assert_eq!(metric(&f, &id).as_deref(), Some(f.metric_id.as_str()));
    let valid = input(&f, &id, json!({"category":"other","metric_id":null}));
    let result = edit(&f, &valid);
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(metric(&f, &id), None);
}

#[test]
fn typed_edit_does_not_treat_default_none_as_explicit_metric_clear() {
    let f = Fixture::new("task-typed-metric");
    let id = seed(&f);
    let typed: TaskEditInput =
        serde_json::from_value(input(&f, &id, json!({"title":"类型调用标题"}))).unwrap();
    let mut conn = Connection::open(&f.db).unwrap();
    let result = TaskEditService {
        conn: &mut conn,
        now: chrono::Utc::now(),
        actor: ActorSource::Cli,
    }
    .apply(&typed)
    .unwrap();
    assert_eq!(result.task_revision_no, 2);
    assert_eq!(metric(&f, &id).as_deref(), Some(f.metric_id.as_str()));
}
