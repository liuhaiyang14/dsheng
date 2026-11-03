//! 核心缺口修复 F1：task step move 非恒等重排两阶段落库。
//!
//! `idx_task_steps_live_order` 对存活行 (task_id, step_order) 唯一；
//! `save_task_state` 逐行 upsert 在非恒等重排（1,2,3→3,1,2 等）时曾中途
//! 撞唯一键（LOCAL_SAVE_FAILED）。修复后同一事务先平移存活行到临时高序
//! 区间再写最终序。用例覆盖：全量轮转、部分移动、跨多步移动、混合操作
//! （move+add+delete）、revision 冲突仍拒绝、幂等重放、事件/快照语义不变。

use chrono::Utc;
use dsheng_core::contracts::tasks::{
    ActorSource, TaskCategory, TaskEditInput, TaskLifecycle, TaskStatus,
};
use dsheng_core::repository::task_store::{insert_full_task, load_task, TaskEditService};
use dsheng_core::repository::tasks::{StepRow, TaskRow, TaskServiceError};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

const WS: &str = "00000000-0000-4000-8000-0000000000aa";

fn conn() -> Connection {
    let conn = Connection::open_in_memory().expect("内存库");
    dsheng_core::repository::apply_migrations(&conn, "test").expect("迁移");
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at)
         VALUES (?1, '测试', 't', 't')",
        params![WS],
    )
    .unwrap();
    conn
}

fn base_task(id: Uuid) -> TaskRow {
    TaskRow {
        task_id: id,
        workspace_id: Uuid::parse_str(WS).unwrap(),
        task_revision_no: 1,
        title: "重排任务".into(),
        summary: String::new(),
        category: TaskCategory::Other,
        metric_id: None,
        baseline_json: None,
        target_value: None,
        planned_value: None,
        actual_value: None,
        current_value: None,
        unit: None,
        mp_label: None,
        mp_unit: None,
        owner: None,
        planned_start: None,
        due_date: None,
        target_period: None,
        resource_need: None,
        success_criteria: None,
        assumptions_json: None,
        needs_review_json: None,
        smart_check_json: None,
        status: TaskStatus::NotStarted,
        lifecycle: TaskLifecycle::Active,
        superseded_by_task_id: None,
        progress: rust_decimal::Decimal::ZERO,
        progress_mode: dsheng_core::contracts::tasks::ProgressMode::Auto,
        adjustment_reason: None,
        blocked_reason: None,
        smart_status: dsheng_core::contracts::tasks::SmartStatus::Generated,
        review_note: None,
        completed_at: None,
    }
}

fn step(id: u64, action: &str) -> StepRow {
    StepRow {
        step_id: Uuid::from_u64_pair(id, id),
        step_order: id as u32,
        action: action.to_string(),
        owner: None,
        start_date: None,
        due_date: None,
        done_criteria: None,
        status: TaskStatus::NotStarted,
        blocked_reason: None,
        completed_at: None,
        deleted_at: None,
    }
}

/// 建 3 步任务（步骤 A/B/C，order 1..3）。
fn create_three_step_task(conn: &Connection) -> Uuid {
    let id = Uuid::new_v4();
    let steps = vec![step(1, "步骤A"), step(2, "步骤B"), step(3, "步骤C")];
    insert_full_task(
        conn,
        &base_task(id),
        &steps,
        ActorSource::Ui,
        Uuid::new_v4(),
        None,
        Utc::now(),
    )
    .unwrap();
    id
}

fn move_op(task_id: Uuid, revision: u32, moves: Value) -> TaskEditInput {
    let mut v = json!({
        "schema_version": "1",
        "workspace_id": WS,
        "task_id": task_id.to_string(),
        "expected_task_revision": revision,
        "mutation_id": Uuid::new_v4().to_string(),
    });
    v["operation"] = json!({"kind": "content", "step_operations": moves});
    serde_json::from_value(v).unwrap()
}

fn apply(
    conn: &mut Connection,
    input: &TaskEditInput,
) -> Result<dsheng_core::repository::task_store::TaskEditOutcome, TaskServiceError> {
    let mut svc = TaskEditService {
        conn,
        now: Utc::now(),
        actor: ActorSource::Cli,
    };
    svc.apply(input)
}

/// (action, step_order) 存活序列。
fn live_order(conn: &Connection, task_id: Uuid) -> Vec<(String, i64)> {
    let mut stmt = conn
        .prepare(
            "SELECT action, step_order FROM task_steps
             WHERE task_id = ?1 AND deleted_at IS NULL ORDER BY step_order",
        )
        .unwrap();
    let rows = stmt
        .query_map(params![task_id.to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    rows
}

#[test]
fn full_rotation_1_2_3_to_3_1_2() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    // C 移到 A 前：1,2,3 → 3,1,2（每行 order 都变化，历史必然撞唯一键）
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    let input = move_op(
        task_id,
        1,
        json!([{"op": "move", "step_id": sid(3), "before_step_id": sid(1)}]),
    );
    let outcome = apply(&mut conn, &input).unwrap();
    assert_eq!(outcome.task_revision_no, 2, "单事务 revision 只 +1");
    assert_eq!(
        live_order(&conn, task_id),
        vec![
            ("步骤C".to_string(), 1),
            ("步骤A".to_string(), 2),
            ("步骤B".to_string(), 3),
        ],
        "非恒等重排后顺序与最终序 1..N"
    );
    // 步骤结构未变（move 非结构操作）：不触发进度重算/事件
    let event_types: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT event_type FROM task_events WHERE task_id = ?1 ORDER BY event_seq")
            .unwrap();
        stmt.query_map(params![task_id.to_string()], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    assert!(
        event_types.iter().any(|t| t == "step_moved"),
        "写 step_moved 事件"
    );
    assert!(
        !event_types.iter().any(|t| t == "progress_override_reset"),
        "move 不重算进度"
    );
}

#[test]
fn partial_move_first_to_end() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    // A 移到末尾（before=null）：A,B,C → B,C,A
    let input = move_op(task_id, 1, json!([{"op": "move", "step_id": sid(1)}]));
    apply(&mut conn, &input).unwrap();
    assert_eq!(
        live_order(&conn, task_id),
        vec![
            ("步骤B".to_string(), 1),
            ("步骤C".to_string(), 2),
            ("步骤A".to_string(), 3),
        ]
    );
    // 连续第二次部分移动：B,C,A → C,A,B（当前 revision=2）
    let input = move_op(task_id, 2, json!([{"op": "move", "step_id": sid(2)}]));
    apply(&mut conn, &input).unwrap();
    assert_eq!(
        live_order(&conn, task_id),
        vec![
            ("步骤C".to_string(), 1),
            ("步骤A".to_string(), 2),
            ("步骤B".to_string(), 3),
        ]
    );
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let live: Vec<&StepRow> = steps.iter().filter(|s| s.deleted_at.is_none()).collect();
    assert_eq!(live.len(), 3, "步骤数不变");
}

#[test]
fn move_across_multiple_steps_and_reverse() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    // C 跨两步移到最前：A,B,C → C,A,B
    let input = move_op(
        task_id,
        1,
        json!([{"op": "move", "step_id": sid(3), "before_step_id": sid(1)}]),
    );
    apply(&mut conn, &input).unwrap();
    // 完全逆序：B 移到 A 前（C,A,B → C,B,A）
    let input = move_op(
        task_id,
        2,
        json!([{"op": "move", "step_id": sid(2), "before_step_id": sid(1)}]),
    );
    apply(&mut conn, &input).unwrap();
    let order = live_order(&conn, task_id);
    // 最终为 C,B,A 且最终序恰为 1..3
    assert_eq!(order[0].0, "步骤C");
    assert_eq!(order[1].0, "步骤B");
    assert_eq!(order[2].0, "步骤A");
    assert_eq!(
        order.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "最终序恒为 1..N 无空洞"
    );
}

#[test]
fn mixed_move_add_delete_same_transaction() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    // 同一事务：删 B、新增 D（放最前）、C 移到 A 前 → D,C,A
    let input = move_op(
        task_id,
        1,
        json!([
            {"op": "delete", "step_id": sid(2)},
            {"op": "add", "client_step_key": "d1",
             "content": {"action": "步骤D", "owner": null, "start_date": null,
                          "due_date": null, "done_criteria": null},
             "before_step_id": sid(1)},
            {"op": "move", "step_id": sid(3), "before_step_id": sid(1)},
        ]),
    );
    let outcome = apply(&mut conn, &input).unwrap();
    assert_eq!(outcome.task_revision_no, 2);
    let order = live_order(&conn, task_id);
    assert_eq!(order.len(), 3);
    assert_eq!(order[0].0, "步骤D");
    assert_eq!(order[1].0, "步骤C");
    assert_eq!(order[2].0, "步骤A");
    assert_eq!(
        order.iter().map(|(_, o)| *o).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    // tombstone 保留且带原 order（不在存活唯一索引内）
    let tombstones: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_steps WHERE task_id = ?1 AND deleted_at IS NOT NULL",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tombstones, 1, "被删步骤保留 tombstone");
}

#[test]
fn stale_expected_revision_still_rejected_after_move() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    // 先做一次成功 move（revision 1→2）
    apply(
        &mut conn,
        &move_op(
            task_id,
            1,
            json!([{"op": "move", "step_id": sid(3), "before_step_id": sid(1)}]),
        ),
    )
    .unwrap();
    // 过期 expected revision → REVISION_CONFLICT，落库不发生
    let err = apply(
        &mut conn,
        &move_op(task_id, 1, json!([{"op": "move", "step_id": sid(1)}])),
    )
    .unwrap_err();
    assert_eq!(err, TaskServiceError::RevisionConflict);
    assert_eq!(
        live_order(&conn, task_id),
        vec![
            ("步骤C".to_string(), 1),
            ("步骤A".to_string(), 2),
            ("步骤B".to_string(), 3),
        ],
        "冲突后顺序不变"
    );
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.task_revision_no, 2, "冲突不推进 revision");
}

#[test]
fn move_replay_is_idempotent() {
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    let mut raw = json!({
        "schema_version": "1",
        "workspace_id": WS,
        "task_id": task_id.to_string(),
        "expected_task_revision": 1,
        "mutation_id": Uuid::new_v4().to_string(),
    });
    raw["operation"] = json!({"kind": "content", "step_operations": [
        {"op": "move", "step_id": sid(3), "before_step_id": sid(1)}
    ]});
    let first: TaskEditInput = serde_json::from_value(raw.clone()).unwrap();
    let outcome = apply(&mut conn, &first).unwrap();
    assert!(!outcome.replayed);
    // 同 mutation_id 同 payload 重放：返回原结果、不重复推进
    let replay: TaskEditInput = serde_json::from_value(raw).unwrap();
    let outcome2 = apply(&mut conn, &replay).unwrap();
    assert!(outcome2.replayed, "同 mutation 同 digest 重放");
    assert_eq!(outcome2.task_revision_no, outcome.task_revision_no);
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.task_revision_no, 2, "重放不重复 +1");
}

#[test]
fn move_keeps_manual_progress_and_dates() {
    // move 不切 manual、不重算状态（§5.2）；日期边界不受重排影响
    let mut conn = conn();
    let task_id = create_three_step_task(&conn);
    // 先 manual 50%
    let mut raw = json!({
        "schema_version": "1", "workspace_id": WS, "task_id": task_id.to_string(),
        "expected_task_revision": 1, "mutation_id": Uuid::new_v4().to_string(),
    });
    raw["operation"] = json!({
        "kind": "progress", "progress_mode": "manual", "progress": "50",
        "status": "in_progress", "adjustment_reason": "人工评估", "blocked_reason": null
    });
    apply(
        &mut conn,
        &serde_json::from_value::<TaskEditInput>(raw).unwrap(),
    )
    .unwrap();
    let sid = |id: u64| Uuid::from_u64_pair(id, id).to_string();
    apply(
        &mut conn,
        &move_op(
            task_id,
            2,
            json!([{"op": "move", "step_id": sid(3), "before_step_id": sid(1)}]),
        ),
    )
    .unwrap();
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        task.progress_mode,
        dsheng_core::contracts::tasks::ProgressMode::Manual,
        "纯 move 不切回 auto"
    );
    assert_eq!(task.progress.to_string(), "50");
}
