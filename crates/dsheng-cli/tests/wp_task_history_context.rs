//! 内存 SQLite 合成记录验证任务检视输入；不访问业务库、不调用 Provider。
use dsheng_cli::store::task_history::prior_task_outcomes;
use dsheng_core::contracts::domain::DateRange;
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

fn period(from: &str, to: &str) -> DateRange {
    DateRange {
        from: from.parse().unwrap(),
        to: to.parse().unwrap(),
    }
}

fn setup() -> (Connection, Uuid) {
    let conn = Connection::open_in_memory().unwrap();
    dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
    let workspace = add_workspace(&conn);
    (conn, workspace)
}

fn add_workspace(conn: &Connection) -> Uuid {
    let id = Uuid::new_v4();
    conn.execute("INSERT INTO workspaces (workspace_id,name,created_at,updated_at) VALUES(?1,'合成企业','2026-11-01T00:00:00Z','2026-11-01T00:00:00Z')",[id.to_string()]).unwrap();
    id
}

fn add_task(
    conn: &Connection,
    workspace: Uuid,
    title: &str,
    range: Option<DateRange>,
    status: &str,
) -> String {
    let id = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO tasks (task_id,workspace_id,title,category,status,target_period_json,created_at,updated_at) VALUES(?1,?2,?3,'other',?4,?5,'2026-10-31T00:00:00Z','2026-10-31T00:00:00Z')",params![id,workspace.to_string(),title,status,range.map(|r|serde_json::to_string(&r).unwrap())]).unwrap();
    id
}

fn link(conn: &Connection, workspace: Uuid, task: &str, range: DateRange, role: &str) {
    conn.execute("INSERT INTO task_period_links (id,workspace_id,task_id,run_id,analysis_revision_no,cycle_role,source_period_from,source_period_to,target_period_from,target_period_to,created_at) VALUES(?1,?2,?3,?4,1,?5,'2026-10-01','2026-10-31',?6,?7,'2026-11-01T00:00:00Z')",params![Uuid::new_v4().to_string(),workspace.to_string(),task,Uuid::new_v4().to_string(),role,range.from.to_string(),range.to.to_string()]).unwrap();
}

#[test]
fn first_period_has_no_history_and_unapplied_candidates_are_not_execution_records() {
    let (conn, ws) = setup();
    let empty = HashSet::new();
    let october = period("2026-10-01", "2026-10-31");
    assert!(prior_task_outcomes(&conn, ws, october, &empty)
        .unwrap()
        .is_empty());
    conn.execute("INSERT INTO task_candidates (candidate_generation_id,workspace_id,run_id,analysis_revision_no,analysis_input_fingerprint,generation_input_fingerprint,attempt_id,output_json,prompt_version,schema_version,created_at) VALUES(?1,?2,'synthetic-run',1,'synthetic-fingerprint','synthetic-fingerprint','synthetic-attempt',?3,'2.5','2','2026-11-01T00:00:00Z')",params![Uuid::new_v4().to_string(),ws.to_string(),json!({"tasks":[{"title":"尚未应用候选","target_period":{"from":"2026-10-01","to":"2026-10-31"}}]}).to_string()]).unwrap();
    assert!(prior_task_outcomes(&conn, ws, october, &empty)
        .unwrap()
        .is_empty());
    let task = add_task(
        &conn,
        ws,
        "十月分析后应用的十一月任务",
        Some(period("2026-11-01", "2026-11-30")),
        "not_started",
    );
    // 当前生成的下一期计划不能冒充本期执行结果；下次分析该目标期间时自动进入检视。
    assert!(prior_task_outcomes(&conn, ws, october, &empty)
        .unwrap()
        .is_empty());
    let next = prior_task_outcomes(&conn, ws, period("2026-11-01", "2026-11-30"), &empty).unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].task_id, task);
    assert_eq!(next[0].progress.to_string(), "0");
}

#[test]
fn review_scope_includes_overlap_and_unfinished_carryover_but_excludes_foreign_future_and_superseded(
) {
    let (conn, ws) = setup();
    let old = period("2026-09-01", "2026-09-30");
    let current = period("2026-10-01", "2026-10-31");
    for (title, status, range) in [
        ("本期已完成", "completed", current),
        (
            "跨期执行",
            "in_progress",
            period("2026-09-15", "2026-10-15"),
        ),
        ("遗留未开始", "not_started", old),
        ("遗留进行中", "in_progress", old),
        ("遗留阻塞", "blocked", old),
        ("早期已完成", "completed", old),
        (
            "未来计划",
            "not_started",
            period("2026-11-01", "2026-11-30"),
        ),
    ] {
        add_task(&conn, ws, title, Some(range), status);
    }
    let superseded = add_task(&conn, ws, "已被替代", Some(current), "blocked");
    conn.execute(
        "UPDATE tasks SET lifecycle='superseded' WHERE task_id=?1",
        [&superseded],
    )
    .unwrap();
    add_task(
        &conn,
        add_workspace(&conn),
        "另一企业",
        Some(current),
        "blocked",
    );
    add_task(&conn, ws, "没有期间依据", None, "blocked");
    let changes = conn.total_changes();
    let got = prior_task_outcomes(&conn, ws, current, &HashSet::new()).unwrap();
    assert_eq!(
        conn.total_changes(),
        changes,
        "读取不写任务、不建立自动关联"
    );
    let names: BTreeSet<_> = got.iter().map(|t| t.title.as_str()).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "本期已完成",
            "跨期执行",
            "遗留未开始",
            "遗留进行中",
            "遗留阻塞"
        ])
    );
    assert!(got
        .iter()
        .find(|t| t.title == "遗留阻塞")
        .unwrap()
        .outcome_note
        .as_ref()
        .unwrap()
        .contains("较早期间遗留"));
}

#[test]
fn explicit_target_wins_and_legacy_period_links_do_not_duplicate_tasks() {
    let (conn, ws) = setup();
    let current = period("2026-10-01", "2026-10-31");
    let future = period("2026-11-01", "2026-11-30");
    let legacy = add_task(&conn, ws, "仅关联表有期间", None, "in_progress");
    link(&conn, ws, &legacy, current, "plan");
    link(&conn, ws, &legacy, current, "review");
    let future_task = add_task(&conn, ws, "已调整为未来目标", Some(future), "in_progress");
    link(&conn, ws, &future_task, current, "review");
    let foreign_link = add_task(&conn, ws, "非法跨企业关联", None, "blocked");
    link(
        &conn,
        add_workspace(&conn),
        &foreign_link,
        current,
        "review",
    );
    let got = prior_task_outcomes(&conn, ws, current, &HashSet::new()).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].task_id, legacy);
    link(&conn, ws, &legacy, future, "plan");
    assert!(prior_task_outcomes(&conn, ws, current, &HashSet::new())
        .unwrap()
        .is_empty());
}

#[test]
fn current_execution_metadata_and_exact_progress_are_preserved_without_out_of_scope_evidence() {
    let (conn, ws) = setup();
    let current = period("2026-10-01", "2026-10-31");
    let task = add_task(&conn, ws, "本地执行记录", Some(current), "blocked");
    conn.execute("UPDATE tasks SET task_revision_no=7,progress='33.33',review_note='尚缺回单',blocked_reason='等待确认',actual_value='10000000000000000.125',unit='元',completed_at='2026-11-02T00:00:00Z' WHERE task_id=?1",[&task]).unwrap();
    let step = Uuid::new_v4().to_string();
    conn.execute("INSERT INTO task_steps (step_id,task_id,step_order,action,status,blocked_reason,created_at,updated_at) VALUES(?1,?2,1,'核对回单','blocked','回单待补','2026-10-01T00:00:00Z','2026-11-01T00:00:00Z')",params![step,task]).unwrap();
    for (id, revision) in [
        ("current-evidence", 1),
        ("current-evidence", 2),
        ("old-evidence", 1),
    ] {
        conn.execute("INSERT INTO task_evidence(task_id,source_id,analysis_revision_no,evidence_id,summary_json,created_at) VALUES(?1,'synthetic-source',?2,?3,'{}','2026-10-01T00:00:00Z')",params![task,revision,id]).unwrap();
    }
    conn.execute("INSERT INTO task_step_evidence(step_id,source_id,analysis_revision_no,evidence_id,summary_json,created_at) VALUES(?1,'synthetic-source',1,'step-evidence','{}','2026-10-01T00:00:00Z')",[&step]).unwrap();
    let allowed = HashSet::from(["current-evidence".to_string(), "step-evidence".to_string()]);
    let got = prior_task_outcomes(&conn, ws, current, &allowed).unwrap();
    let note = got[0].outcome_note.as_ref().unwrap();
    assert_eq!(got[0].progress.to_string(), "33.33");
    assert_eq!(serde_json::to_value(got[0].status).unwrap(), "blocked");
    for expected in [
        "当前执行记录",
        "非期末快照",
        "r7",
        "尚缺回单",
        "等待确认",
        "10000000000000000.125",
        "核对回单",
        "回单待补",
        "2026-11-02",
    ] {
        assert!(note.contains(expected), "未保留 {expected}");
    }
    assert_eq!(got[0].evidence_ids, ["current-evidence", "step-evidence"]);
    conn.execute(
        "UPDATE task_steps SET deleted_at='2026-11-02T00:00:00Z' WHERE step_id=?1",
        [&step],
    )
    .unwrap();
    let changed = prior_task_outcomes(&conn, ws, current, &allowed).unwrap();
    assert!(!changed[0]
        .outcome_note
        .as_ref()
        .unwrap()
        .contains("核对回单"));
    assert_eq!(changed[0].evidence_ids, ["current-evidence"]);
}
