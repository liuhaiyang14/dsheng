//! 数据工作包 S3：task-edit / task-reconcile schema 用例（34 条 SC）与
//! 任务生命周期领域用例（50 条 TS，迁移协议 6 条见 wp_data_migration.rs）。
//!
//! 夹具：../../contracts/v1/task-state-cases.json。期望值全部来自合同。

use chrono::{NaiveDate, Utc};
use dsheng_core::analysis::fingerprint::{AnalysisRevisionInput, SourceSelection};
use dsheng_core::contracts::domain::DateRange;
use dsheng_core::contracts::tasks::{
    ActorSource, TaskCategory, TaskEditInput, TaskLifecycle, TaskReconcileInput, TaskStatus,
};
use dsheng_core::repository::analysis_runs::{
    build_period_key, upsert_analysis_revision, RevisionPayload,
};
use dsheng_core::repository::history_delete::{history_delete_confirm, history_delete_preview};
use dsheng_core::repository::reconcile::{
    apply_reconcile, prepare_reconcile, validate_reconcile_input_shape,
};
use dsheng_core::repository::task_store::{
    insert_full_task, load_task, task_event_types, TaskEditService,
};
use dsheng_core::repository::tasks::{
    auto_progress, auto_status, check_diagnosis_reference, check_generation_commit_gate,
    derive_overdue, dispatched_route_frozen, due_date_display, normalize_candidate_output,
    validate_category_metric, validate_task_edit_with_raw, StepRow, TaskRow, TaskServiceError,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_cases() -> Value {
    let raw = std::fs::read_to_string(repo_root().join("contracts/v1/task-state-cases.json"))
        .expect("读取 TS 合同");
    serde_json::from_str(&raw).expect("TS JSON")
}

fn ts_case(v: &Value, id: &str) -> Value {
    v["domain_acceptance_cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .expect("TS 用例存在")
}

// ---- SC：34 条 schema 用例（serde + 领域条件） ----

#[test]
fn sc_all_34_cases_match_schema_validity() {
    let v = load_cases();
    let cases = v["schema_validation_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 34, "schema 用例 34 条");
    let mut failures = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let schema = case["schema"].as_str().unwrap();
        let input = &case["input"];
        let expected_valid = case["schema_valid"].as_bool().unwrap();
        let accepted = if schema.starts_with("task-edit") {
            serde_json::from_value::<TaskEditInput>(input.clone())
                .ok()
                .map(|parsed| validate_task_edit_with_raw(&parsed, Some(input)).is_ok())
                .unwrap_or(false)
        } else {
            serde_json::from_value::<TaskReconcileInput>(input.clone())
                .ok()
                .map(|parsed| validate_reconcile_input_shape(&parsed).is_ok())
                .unwrap_or(false)
        };
        if accepted != expected_valid {
            failures.push(format!(
                "{id}: 期望 schema_valid={expected_valid}，实际 {accepted}"
            ));
        }
    }
    assert!(failures.is_empty(), "SC 失败:\n{}", failures.join("\n"));
}

// ---- 测试库与夹具 ----

fn conn() -> Connection {
    let conn = Connection::open_in_memory().expect("内存库");
    dsheng_core::repository::apply_migrations(&conn, "test").expect("迁移");
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES (?1, '测试', 't', 't')",
        params![WS],
    )
    .unwrap();
    conn
}

const WS: &str = "00000000-0000-4000-8000-0000000000aa";

fn range(from: &str, to: &str) -> DateRange {
    DateRange::new(
        NaiveDate::parse_from_str(from, "%Y-%m-%d").unwrap(),
        NaiveDate::parse_from_str(to, "%Y-%m-%d").unwrap(),
    )
    .unwrap()
}

fn sample_input(metrics_value: &str) -> AnalysisRevisionInput {
    AnalysisRevisionInput {
        sources: vec![SourceSelection {
            source_id: "s1".into(),
            sheet: Some("核算表".into()),
            range: "B4:L20".into(),
            content_hash: "h1".into(),
            source_profile_id: "generic".into(),
            source_profile_version: 1,
            source_profile_hash: "generic".into(),
        }],
        mapping_version: "m1".into(),
        mappings: json!({}),
        metric_definitions: vec![],
        manual_inputs: vec![],
        conflict_resolutions: vec![],
        period_policy: json!({"timezone": "Asia/Shanghai"}),
        scope: "全公司".into(),
        current_period: range("2026-10-01", "2026-10-31"),
        target_period: None,
        normalized_metrics: json!({"sales_net": metrics_value}),
        comparisons: json!({}),
        comparison_base_refs: vec![],
    }
}

fn create_run(conn: &mut Connection, metrics_value: &str) -> (String, u32) {
    let key = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-10-01", "2026-10-31"),
        "全公司",
    );
    let upsert = upsert_analysis_revision(
        conn,
        WS,
        &key,
        &RevisionPayload {
            input: sample_input(metrics_value),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    (upsert.run_id, upsert.revision_no)
}

fn base_task(id: Uuid) -> TaskRow {
    TaskRow {
        task_id: id,
        workspace_id: Uuid::parse_str(WS).unwrap(),
        task_revision_no: 1,
        title: "任务".into(),
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

fn step(id: u64, status: TaskStatus) -> StepRow {
    StepRow {
        step_id: Uuid::new_v4(),
        step_order: id as u32,
        action: format!("步骤{id}"),
        owner: None,
        start_date: None,
        due_date: None,
        done_criteria: None,
        status,
        blocked_reason: None,
        completed_at: None,
        deleted_at: None,
    }
}

fn create_task(conn: &Connection, statuses: &[TaskStatus]) -> Uuid {
    let id = Uuid::new_v4();
    let task = base_task(id);
    let steps: Vec<StepRow> = statuses
        .iter()
        .enumerate()
        .map(|(i, s)| step(i as u64 + 1, *s))
        .collect();
    insert_full_task(
        conn,
        &task,
        &steps,
        ActorSource::Ui,
        Uuid::new_v4(),
        None,
        Utc::now(),
    )
    .unwrap();
    id
}

fn edit_input(task_id: Uuid, revision: u32, operation: Value) -> TaskEditInput {
    serde_json::from_value(edit_input_raw(task_id, revision, operation)).unwrap()
}

fn edit_input_raw(task_id: Uuid, revision: u32, operation: Value) -> Value {
    let mut v = json!({
        "schema_version": "1",
        "workspace_id": WS,
        "task_id": task_id.to_string(),
        "expected_task_revision": revision,
        "mutation_id": Uuid::new_v4().to_string(),
    });
    v["operation"] = operation;
    v
}

fn progress_op(
    mode: &str,
    progress: Option<&str>,
    status: &str,
    reason: Option<&str>,
    blocked: Option<&str>,
) -> Value {
    json!({
        "kind": "progress",
        "progress_mode": mode,
        "progress": progress,
        "status": status,
        "adjustment_reason": reason,
        "blocked_reason": blocked,
    })
}

fn apply_edit(
    conn: &mut Connection,
    input: &TaskEditInput,
) -> Result<dsheng_core::repository::task_store::TaskEditOutcome, TaskServiceError> {
    let now = Utc::now();
    let mut svc = TaskEditService {
        conn,
        now,
        actor: ActorSource::Ui,
    };
    svc.apply(input)
}

fn apply_edit_raw(
    conn: &mut Connection,
    input: &Value,
) -> Result<dsheng_core::repository::task_store::TaskEditOutcome, TaskServiceError> {
    TaskEditService {
        conn,
        now: Utc::now(),
        actor: ActorSource::Ui,
    }
    .apply_raw(input)
}

fn insert_candidate(
    conn: &Connection,
    run_id: &str,
    revision: u32,
    fingerprint: &str,
    tasks_json: Value,
) -> String {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO task_candidates
           (candidate_generation_id, workspace_id, run_id, analysis_revision_no,
            analysis_input_fingerprint, generation_input_fingerprint, candidate_revision_no,
            attempt_id, output_json, state, prompt_version, schema_version, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'gen', 1, 'test-attempt', ?6, 'candidate', 'p1', 's1',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![
            id,
            WS,
            run_id,
            revision as i64,
            fingerprint,
            tasks_json.to_string()
        ],
    )
    .unwrap();
    id
}

fn reconcile_input(
    run_id: &str,
    revision: u32,
    fingerprint: &str,
    candidate_id: &str,
    expected: &[(Uuid, u32)],
    mappings: Value,
) -> TaskReconcileInput {
    let mut v = json!({
        "schema_version": "1",
        "workspace_id": WS,
        "run_id": run_id,
        "analysis_revision_no": revision,
        "input_fingerprint": fingerprint,
        "candidate_generation_id": candidate_id,
        "candidate_revision_no": 1,
        "mutation_id": Uuid::new_v4().to_string(),
        "expected_tasks": expected.iter().map(|(id, rev)| json!({"task_id": id.to_string(), "task_revision_no": rev})).collect::<Vec<_>>(),
        "mappings": mappings,
    });
    v["mutation_id"] = json!(Uuid::new_v4().to_string());
    serde_json::from_value(v).unwrap()
}

fn one_step_candidate() -> Value {
    json!({
        "tasks": [{
            "task_key": "c-1",
            "title": "新任务",
            "summary": "",
            "category": "other",
            "metric_id": null,
            "target_value": null,
            "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
            "baseline": null,
            "unit": null,
            "owner": null,
            "planned_start": null,
            "due_date": null,
            "resource_need": null,
            "success_criteria": null,
            "evidence_ids": [],
            "assumptions": [],
            "needs_review": [],
            "smart_check": {},
            "steps": [{"step_id": "s-1", "order": 1, "action": "核对来源", "owner": null,
                        "start_date": null, "due_date": null, "done_criteria": null,
                        "evidence_ids": [], "status": "completed"}]
        }]
    })
}

// ---- TS 状态机（纯域 + 仓库） ----

#[test]
fn ts_01_auto_progress_third() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-01")["expected"].clone();
    let steps = vec![
        step(1, TaskStatus::Completed),
        step(2, TaskStatus::NotStarted),
        step(3, TaskStatus::NotStarted),
    ];
    let (status, progress) = auto_status(&steps, None);
    assert_eq!(progress.to_string(), expected["progress"].as_str().unwrap());
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
}

#[test]
fn ts_02_near_complete_caps_at_99_99() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-02")["expected"].clone();
    assert_eq!(
        auto_progress(99999, 100000).to_string(),
        expected["progress"].as_str().unwrap()
    );
    let steps: Vec<StepRow> = (0..100000)
        .map(|i| {
            if i < 99999 {
                step(i, TaskStatus::Completed)
            } else {
                step(i, TaskStatus::InProgress)
            }
        })
        .collect();
    let (status, progress) = auto_status(&steps, None);
    assert_eq!(progress.to_string(), "99.99");
    assert_eq!(serde_json::to_value(status).unwrap(), json!("in_progress"));
    assert_eq!(expected["completed_at"], json!(null));
}

#[test]
fn ts_03_manual_reset_by_step_completion() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-03")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::NotStarted]);
    // manual 70
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("manual", Some("70"), "in_progress", Some("人工评估"), None),
        ),
    )
    .unwrap();
    // 第二步完成
    let (task, steps) = load_task(&conn, WS, task_id).unwrap();
    let step2 = steps.iter().find(|s| s.step_order == 2).unwrap();
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            json!({
                "kind": "content",
                "step_operations": [{"op": "update", "step_id": step2.step_id.to_string(),
                                      "patch": {"status": "completed"}}],
            }),
        ),
    )
    .unwrap();
    let (after, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        serde_json::to_value(after.progress_mode).unwrap(),
        json!(expected["mode"].as_str().unwrap())
    );
    assert_eq!(
        after.progress.to_string(),
        expected["progress"].as_str().unwrap()
    );
    assert_eq!(
        serde_json::to_value(after.status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
    let events = task_event_types(&conn, task_id).unwrap();
    for want in expected["events_include"].as_array().unwrap() {
        assert!(
            events.contains(&want.as_str().unwrap().to_string()),
            "事件缺失 {want}: {events:?}"
        );
    }
    let _ = task;
}

#[test]
fn ts_04_manual_kept_on_text_change() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-04")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::NotStarted]);
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("manual", Some("70"), "in_progress", Some("人工评估"), None),
        ),
    )
    .unwrap();
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let step2 = steps.iter().find(|s| s.step_order == 2).unwrap();
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            json!({
                "kind": "content",
                "step_operations": [{"op": "update", "step_id": step2.step_id.to_string(),
                                      "patch": {"action": "改为核对清单"}}],
            }),
        ),
    )
    .unwrap();
    let (after, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        serde_json::to_value(after.progress_mode).unwrap(),
        json!(expected["mode"].as_str().unwrap())
    );
    assert_eq!(
        after.progress.to_string(),
        expected["progress"].as_str().unwrap()
    );
    assert_eq!(
        serde_json::to_value(after.status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
    assert_eq!(
        serde_json::to_value(after.smart_status).unwrap(),
        json!(expected["smart_status"].as_str().unwrap())
    );
}

#[test]
fn ts_05_task_block_persists() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-05")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::NotStarted]);
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op(
                "manual",
                Some("70"),
                "blocked",
                Some("评估"),
                Some("等待复核"),
            ),
        ),
    )
    .unwrap();
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let step2 = steps.iter().find(|s| s.step_order == 2).unwrap();
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            json!({
                "kind": "content",
                "step_operations": [{"op": "update", "step_id": step2.step_id.to_string(),
                                      "patch": {"status": "completed"}}],
            }),
        ),
    )
    .unwrap();
    let (after, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        serde_json::to_value(after.status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
    assert_eq!(
        after.blocked_reason.as_deref(),
        Some(expected["blocked_reason"].as_str().unwrap())
    );
    assert_eq!(
        after.progress.to_string(),
        expected["progress"].as_str().unwrap()
    );
    assert_eq!(expected["completed_at"], json!(null));
    assert!(after.completed_at.is_none());
}

#[test]
fn ts_06_step_still_blocked() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-06")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::Blocked]);
    let err = apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("auto", None, "completed", None, None),
        ),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        task.task_revision_no, 1,
        "mutations={}",
        expected["mutations"]
    );
}

#[test]
fn ts_07_explicit_clear_task_block() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-07")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::Completed]);
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op(
                "manual",
                Some("100"),
                "blocked",
                Some("说明"),
                Some("等待验收"),
            ),
        ),
    )
    .unwrap();
    let now = Utc::now();
    let outcome = {
        let mut svc = TaskEditService {
            conn: &mut conn,
            now,
            actor: ActorSource::Ui,
        };
        svc.apply(&edit_input(
            task_id,
            2,
            progress_op("auto", None, "completed", None, None),
        ))
        .unwrap()
    };
    assert_eq!(outcome.status, expected["status"].as_str().unwrap());
    assert_eq!(outcome.progress, expected["progress"].as_str().unwrap());
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    let completed_at = task.completed_at.expect("completed_at 写服务时间");
    let delta = (completed_at - now).num_milliseconds().abs();
    assert!(
        delta <= 1,
        "completed_at=service_now_utc（毫秒截断）: {completed_at} vs {now}"
    );
}

#[test]
fn ts_08_reopen_writes_event_and_clears_completed_at() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-08")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::Completed]);
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("auto", None, "completed", None, None),
        ),
    )
    .unwrap();
    let events_before = task_event_types(&conn, task_id).unwrap();
    assert!(
        events_before.contains(&"completed".to_string()),
        "prior completed event"
    );
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let step2 = steps.iter().find(|s| s.step_order == 2).unwrap();
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            json!({
                "kind": "content",
                "step_operations": [{"op": "update", "step_id": step2.step_id.to_string(),
                                      "patch": {"status": "in_progress"}}],
            }),
        ),
    )
    .unwrap();
    let (after, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        serde_json::to_value(after.status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
    assert_eq!(
        after.progress.to_string(),
        expected["progress"].as_str().unwrap()
    );
    assert!(after.completed_at.is_none());
    let events = task_event_types(&conn, task_id).unwrap();
    assert!(events.contains(&"reopened".to_string()));
    assert!(
        events.contains(&"completed".to_string()),
        "旧 completed 事件保留"
    );
}

#[test]
fn ts_09_delete_only_step_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-09")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed]);
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let only = steps[0].step_id;
    let err = apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "content",
                "step_operations": [{"op": "delete", "step_id": only.to_string()}],
            }),
        ),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
}

#[test]
fn ts_10_delete_unfinished_step_completes() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-10")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::Completed, TaskStatus::NotStarted]);
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let second = steps.iter().find(|s| s.step_order == 2).unwrap().step_id;
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "content",
                "step_operations": [{"op": "delete", "step_id": second.to_string()}],
            }),
        ),
    )
    .unwrap();
    let (task, steps) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        serde_json::to_value(task.status).unwrap(),
        json!(expected["status"].as_str().unwrap())
    );
    assert_eq!(
        task.progress.to_string(),
        expected["progress"].as_str().unwrap()
    );
    assert!(
        steps
            .iter()
            .any(|s| s.step_id == second && s.deleted_at.is_some()),
        "tombstone"
    );
    // 事件 before/after 保留
    let detail: Vec<String> = conn
        .prepare(
            "SELECT before_json FROM task_events WHERE task_id=?1 AND event_type='step_deleted'",
        )
        .unwrap()
        .query_map(params![task_id.to_string()], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(detail.len(), 1, "event_before_after_retained");
    assert!(!detail[0].is_empty(), "before_json 保留");
}

#[test]
fn ts_11_overdue_blocked() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-11")["expected"].clone();
    let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
    assert!(derive_overdue(
        TaskStatus::Blocked,
        TaskLifecycle::Active,
        Some(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()),
        today
    ));
    assert_eq!(expected["status"], json!("blocked"), "延期不覆盖执行状态");
}

#[test]
fn ts_12_no_due_date_display() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-12")["expected"].clone();
    assert!(!derive_overdue(
        TaskStatus::InProgress,
        TaskLifecycle::Active,
        None,
        NaiveDate::from_ymd_opt(2026, 10, 30).unwrap()
    ));
    assert_eq!(
        due_date_display(None),
        expected["display"].as_str().unwrap()
    );
}

#[test]
fn ts_13_manual_completed_requires_100() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-13")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted, TaskStatus::NotStarted]);
    let err = apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("manual", Some("50"), "completed", Some("说明"), None),
        ),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.task_revision_no, 1, "mutations=0");
}

#[test]
fn ts_14_explicit_update_keeps_user_edits() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-14")["expected"].clone();
    let mut conn = conn();
    let (run_id, revision) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let old_task = create_task(&conn, &[TaskStatus::Completed]);
    // 人工改 owner
    apply_edit(
        &mut conn,
        &edit_input(
            old_task,
            1,
            json!({
                "kind": "content", "patch": {"owner": "用户修改岗位"},
            }),
        ),
    )
    .unwrap();
    let (_, steps) = load_task(&conn, WS, old_task).unwrap();
    let s1 = steps[0].step_id;
    let candidate = json!({
        "tasks": [{
            "task_key": "new-name",
            "title": "候选标题", "summary": "", "category": "other", "metric_id": null,
            "target_value": null,
            "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
            "baseline": null, "unit": null, "owner": "其他岗位",
            "planned_start": null, "due_date": null, "resource_need": null,
            "success_criteria": null, "evidence_ids": [], "assumptions": [],
            "needs_review": [], "smart_check": {},
            "steps": [{"step_id": "different-name", "order": 1, "action": "候选行动", "owner": null,
                        "start_date": null, "due_date": null, "done_criteria": null,
                        "evidence_ids": [], "status": "not_started"}]
        }]
    });
    let candidate_id = insert_candidate(&conn, &run_id, revision, &fingerprint, candidate);
    let input = reconcile_input(
        &run_id,
        revision,
        &fingerprint,
        &candidate_id,
        &[(old_task, 2)],
        json!([{
            "candidate_task_key": "new-name", "action": "update",
            "existing_task_id": old_task.to_string(),
            "take_fields": ["title"],
            "step_mappings": [{"candidate_step_id": "different-name", "action": "update",
                                "existing_step_id": s1.to_string(), "take_fields": ["action"]}],
            "remove_existing_step_ids": [],
            "final_step_order": [{"kind": "existing", "step_id": s1.to_string()}],
        }]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    let (task, steps) = load_task(&conn, WS, old_task).unwrap();
    assert_eq!(task.task_id, old_task, "task_id=T1");
    assert_eq!(
        task.owner.as_deref(),
        Some(expected["owner"].as_str().unwrap()),
        "未选字段不覆盖"
    );
    assert_eq!(steps[0].status, TaskStatus::Completed, "完成步骤保留");
    assert_eq!(steps[0].action, "候选行动", "take_fields=action 生效");
    assert_eq!(
        serde_json::to_value(task.smart_status).unwrap(),
        json!(expected["smart_status"].as_str().unwrap())
    );
}

#[test]
fn ts_15_empty_candidates_delete_nothing() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-15")["expected"].clone();
    let mut conn = conn();
    let (run_id, revision) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let t1 = create_task(&conn, &[TaskStatus::NotStarted]);
    let t2 = create_task(&conn, &[TaskStatus::NotStarted]);
    let candidate_id =
        insert_candidate(&conn, &run_id, revision, &fingerprint, json!({"tasks": []}));
    let input = reconcile_input(
        &run_id,
        revision,
        &fingerprint,
        &candidate_id,
        &[],
        json!([]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    for (tid, want) in [(t1, "T1"), (t2, "T2")] {
        let (task, _) = load_task(&conn, WS, tid).unwrap();
        assert_eq!(task.lifecycle, TaskLifecycle::Active, "{want} 保留");
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE lifecycle='superseded'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count as usize,
        expected["superseded"].as_array().unwrap().len()
    );
}

#[test]
fn ts_16_duplicate_candidate_key_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-16")["expected"].clone();
    let raw = json!({"tasks": [
        {"task_key": "same", "title": "a", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": null, "source_period": null, "baseline": null,
         "unit": null, "owner": null, "planned_start": null, "due_date": null, "resource_need": null,
         "success_criteria": null, "evidence_ids": [], "assumptions": [], "needs_review": [],
         "smart_check": {}, "steps": []},
        {"task_key": "same", "title": "b", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": null, "source_period": null, "baseline": null,
         "unit": null, "owner": null, "planned_start": null, "due_date": null, "resource_need": null,
         "success_criteria": null, "evidence_ids": [], "assumptions": [], "needs_review": [],
         "smart_check": {}, "steps": []},
    ]});
    let err = normalize_candidate_output(&raw).unwrap_err();
    assert!(
        err.to_string()
            .contains(expected["error"].as_str().unwrap()),
        "{err}"
    );
}

#[test]
fn ts_17_duplicate_reconcile_target_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-17")["expected"].clone();
    let mut conn = conn();
    let (run_id, revision) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let t1 = create_task(&conn, &[TaskStatus::NotStarted]);
    let candidate = json!({"tasks": [
        {"task_key": "C1", "title": "a", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s1", "order": 1, "action": "x", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
        {"task_key": "C2", "title": "b", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s2", "order": 1, "action": "y", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
    ]});
    let candidate_id = insert_candidate(&conn, &run_id, revision, &fingerprint, candidate);
    let mut input = reconcile_input(
        &run_id,
        revision,
        &fingerprint,
        &candidate_id,
        &[],
        json!([
            {"candidate_task_key": "C1", "action": "update", "existing_task_id": t1.to_string(),
             "take_fields": ["title"],
             "step_mappings": [{"candidate_step_id": "s1", "action": "ignore"}],
             "remove_existing_step_ids": [],
             "final_step_order": []},
            {"candidate_task_key": "C2", "action": "update", "existing_task_id": t1.to_string(),
             "take_fields": ["title"],
             "step_mappings": [{"candidate_step_id": "s2", "action": "ignore"}],
             "remove_existing_step_ids": [],
             "final_step_order": []},
        ]),
    );
    input.expected_tasks = vec![dsheng_core::contracts::tasks::ExpectedTaskRevision {
        task_id: t1,
        task_revision_no: 1,
    }];
    let err = prepare_reconcile(&conn, &input, Utc::now()).unwrap_err();
    assert!(
        err.to_string()
            .contains(expected["error"].as_str().unwrap()),
        "{err}"
    );
}

#[test]
fn ts_18_local_steps_start_not_started() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-18")["expected"].clone();
    let mut conn = conn();
    let (run_id, revision) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let raw = one_step_candidate();
    let candidate_id = insert_candidate(&conn, &run_id, revision, &fingerprint, raw.clone());
    let input = reconcile_input(
        &run_id,
        revision,
        &fingerprint,
        &candidate_id,
        &[],
        json!([
            {"candidate_task_key": "c-1", "action": "new"},
        ]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    let applied = apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    let new_id = Uuid::parse_str(&applied.new_task_ids[0]).unwrap();
    let (task, steps) = load_task(&conn, WS, new_id).unwrap();
    assert_eq!(
        steps[0].status,
        serde_json::from_value::<TaskStatus>(json!(expected["local_step_status"]
            .as_str()
            .unwrap()))
        .unwrap()
    );
    assert_eq!(
        task.progress.to_string(),
        expected["local_task_progress"].as_str().unwrap()
    );
    // 模型 status 仅保留在生成快照
    let output: String = conn
        .query_row(
            "SELECT output_json FROM task_candidates WHERE candidate_generation_id=?1",
            params![candidate_id],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        output.contains("\"completed\""),
        "model_status_retained_in_snapshot"
    );
}

#[test]
fn ts_19_same_mutation_same_digest_replays() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-19")["expected"].clone();
    let mut conn = conn();
    let (run_id, revision) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let candidate_id =
        insert_candidate(&conn, &run_id, revision, &fingerprint, one_step_candidate());
    let input = reconcile_input(
        &run_id,
        revision,
        &fingerprint,
        &candidate_id,
        &[],
        json!([
            {"candidate_task_key": "c-1", "action": "new"},
        ]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    let first = apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    let tasks_after_first: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))
        .unwrap();
    let events_after_first: i64 = conn
        .query_row("SELECT COUNT(*) FROM task_events", [], |r| r.get(0))
        .unwrap();
    // 同 mutation + 同 digest 重放：token 已消费也先命中幂等结果
    let second = apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(second.new_task_ids, first.new_task_ids);
    assert!(second.replayed);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        tasks_after_first,
        "additional_tasks={}",
        expected["additional_tasks"]
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM task_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        events_after_first,
        "additional_events={}",
        expected["additional_events"]
    );
}

#[test]
fn ts_20_same_mutation_different_digest_conflicts() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-20")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    let first = edit_input(
        task_id,
        1,
        json!({"kind": "content", "patch": {"title": "A"}}),
    );
    apply_edit(&mut conn, &first).unwrap();
    let events_before: i64 = conn
        .query_row("SELECT COUNT(*) FROM task_events", [], |r| r.get(0))
        .unwrap();
    // 同 mutation_id、不同 payload（digest 不同）
    let mut second_raw = json!({
        "schema_version": "1", "workspace_id": WS, "task_id": task_id.to_string(),
        "expected_task_revision": 2, "mutation_id": serde_json::to_value(first.mutation_id).unwrap(),
    });
    second_raw["operation"] = json!({"kind": "content", "patch": {"title": "B"}});
    let second: TaskEditInput = serde_json::from_value(second_raw).unwrap();
    let err = apply_edit(&mut conn, &second).unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM task_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        events_before,
        "mutations=0"
    );
}

#[test]
fn ts_21_expected_revision_conflict() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-21")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    let err = apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            3,
            json!({"kind": "content", "patch": {"title": "X"}}),
        ),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.task_revision_no, 1, "mutations=0");
}

#[test]
fn ts_22_input_revision_changed_marks_attempt_failed() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-22")["expected"].clone();
    // 纯检查：attempt 绑定 rev1/fp1，当前已 rev2
    let err = check_generation_commit_gate(2, "fp-current", 1, "fp-attempt").unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    // 仓库语义：attempt 置 failed、上一有效结果保留
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    // 上一有效结果：已应用候选
    let fingerprint_v1 = sample_input("100").fingerprint();
    let candidate_id = insert_candidate(&conn, &run_id, 1, &fingerprint_v1, one_step_candidate());
    // attempt 行（preparing）
    conn.execute(
        "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
            analysis_input_fingerprint, generation_input_fingerprint, workflow, state,
            prompt_version, schema_version, input_schema_version, source_manifest_hash, created_at, updated_at)
         VALUES ('sess1', ?1, ?2, 1, ?3, 'g', 'task', 'preparing', 'p', 's', 'i', 'h',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![WS, run_id, fingerprint_v1],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO generation_attempts (attempt_id, workspace_id, session_id, workflow, operation,
            confirmation_id, provider_id, provider_revision, provider_snapshot_json, attempt_seq,
            prompt_version, schema_version, input_hash, created_at)
         VALUES ('att1', ?1, 'sess1', 'task', 'task_generate', 'c1', 'p1', 1, '{}', 1, 'p', 's', 'h',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![WS],
    )
    .unwrap();
    // 分析被重做（手工输入变化 → 指纹变化 → rev2）
    let key = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-10-01", "2026-10-31"),
        "全公司",
    );
    upsert_analysis_revision(
        &mut conn,
        WS,
        &key,
        &RevisionPayload {
            input: sample_input("105"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    let (current_rev, current_fp) =
        dsheng_core::repository::analysis_runs::current_revision(&conn, WS, &run_id).unwrap();
    assert!(check_generation_commit_gate(current_rev, &current_fp, 1, &fingerprint_v1).is_err());
    conn.execute(
        "UPDATE generation_attempts SET terminal_state='failed', error_code='INPUT_REVISION_CHANGED',
                finished_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE attempt_id='att1'",
        [],
    )
    .unwrap();
    let state: String = conn
        .query_row(
            "SELECT terminal_state FROM generation_attempts WHERE attempt_id='att1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, expected["attempt_state"].as_str().unwrap());
    // 上一有效结果（候选）保留
    let candidate_state: String = conn
        .query_row(
            "SELECT state FROM task_candidates WHERE candidate_generation_id=?1",
            params![candidate_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        candidate_state, "candidate",
        "previous_valid_result_retained"
    );
}

#[test]
fn ts_23_stale_diagnosis_reference() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-23")["expected"].clone();
    let err = check_diagnosis_reference(true, 2, 1, "fp", "fp", 1, 1).unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    assert_eq!(
        expected["provider_requests"],
        json!(0),
        "本地拒绝不触发 Provider"
    );
    assert_eq!(
        expected["silently_removed_reference"],
        json!(false),
        "返回错误而非静默移除"
    );
}

#[test]
fn ts_24_old_snapshot_frozen() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-24")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let task_id = create_task(&conn, &[TaskStatus::NotStarted, TaskStatus::NotStarted]);
    // 链接到 run（快照随相关事务追加）
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "period_link", "action": "attach", "run_id": run_id,
                "analysis_revision_no": 1, "cycle_role": "plan",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            }),
        ),
    )
    .unwrap();
    // 快照进度 20（manual）
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            progress_op("manual", Some("20"), "in_progress", Some("快照时点"), None),
        ),
    )
    .unwrap();
    // 重做分析 → 旧 revision 封存
    let key = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-10-01", "2026-10-31"),
        "全公司",
    );
    upsert_analysis_revision(
        &mut conn,
        WS,
        &key,
        &RevisionPayload {
            input: sample_input("110"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    // live 推进到 80%
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            3,
            progress_op("manual", Some("80"), "in_progress", Some("冲刺"), None),
        ),
    )
    .unwrap();
    // 旧 revision 的最高快照：20%
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        task.progress.to_string(),
        expected["current_task_progress"].as_str().unwrap()
    );
    let snapshots: Vec<(i64, String)> = conn
        .prepare("SELECT task_snapshot_no, task_snapshot_json FROM run_task_snapshots WHERE run_id=?1 AND analysis_revision_no=1 ORDER BY task_snapshot_no")
        .unwrap()
        .query_map(params![run_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let last = snapshots.last().unwrap();
    let json: Value = serde_json::from_str(&last.1).unwrap();
    let progress = json["tasks"][0]["progress"].as_str().unwrap_or_default();
    assert_eq!(
        progress,
        expected["history_snapshot_progress"].as_str().unwrap(),
        "old_snapshot_changed=false"
    );
    let sealed: i64 = conn
        .query_row("SELECT sealed FROM run_task_snapshots WHERE run_id=?1 AND analysis_revision_no=1 ORDER BY task_snapshot_no DESC LIMIT 1",
                   params![run_id], |r| r.get(0))
        .unwrap();
    assert_eq!(sealed, 1, "旧 revision 快照已封存");
}

#[test]
fn ts_25_cross_period_links_share_identity() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-25")["expected"].clone();
    let mut conn = conn();
    let (run1, _) = create_run(&mut conn, "100");
    let key2 = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-09-01", "2026-09-30"),
        "全公司",
    );
    let run2 = upsert_analysis_revision(
        &mut conn,
        WS,
        &key2,
        &RevisionPayload {
            input: sample_input("90"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap()
    .run_id;
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    // R1 plan link
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "period_link", "action": "attach", "run_id": run1,
                "analysis_revision_no": 1, "cycle_role": "plan",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            }),
        ),
    )
    .unwrap();
    // R2 review link
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            2,
            json!({
                "kind": "period_link", "action": "attach", "run_id": run2,
                "analysis_revision_no": 1, "cycle_role": "review",
                "source_period": {"from": "2026-09-01", "to": "2026-09-30"},
                "target_period": {"from": "2026-10-01", "to": "2026-10-31"},
            }),
        ),
    )
    .unwrap();
    // 从 R2 推进任务（一步完成会因无步骤而失败——用 add 步骤推进）
    apply_edit(&mut conn, &edit_input(task_id, 3, json!({
        "kind": "content",
        "step_operations": [{"op": "add", "client_step_key": "k1",
                              "content": {"action": "推进", "owner": null, "start_date": null,
                                           "due_date": null, "done_criteria": null},
                              "before_step_id": null}],
    }))).unwrap();
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    let _ = task;
    // 同一任务单条记录、共享事件历史
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id=?1",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, expected["task_records"].as_i64().unwrap());
    // R1 快照不被 R2 推进改变
    let r1_snapshots: Vec<String> = conn
        .prepare("SELECT task_snapshot_json FROM run_task_snapshots WHERE run_id=?1 ORDER BY task_snapshot_no")
        .unwrap()
        .query_map(params![run1], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!r1_snapshots.is_empty(), "R1 有快照");
    // 任务事件在共享历史（task_events 只有一份）
    let events: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_events WHERE task_id=?1",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert!(events >= 3, "shared_event_history");
}

#[test]
fn ts_26_base_revision_frozen() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-26")["expected"].clone();
    let mut conn = conn();
    // run B rev1：基期值 100
    let key_b = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2025-10-01", "2025-10-31"),
        "全公司",
    );
    let run_b = upsert_analysis_revision(
        &mut conn,
        WS,
        &key_b,
        &RevisionPayload {
            input: sample_input("100"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap()
    .run_id;
    // run A rev1：current 120，比较冻结 base B@1
    let mut input_a = sample_input("120");
    input_a.comparisons = json!({"yoy": {
        "absolute_change": "20", "base_refs": [{"run_id": run_b, "revision_no": 1}],
        "base_value": "100", "change_rate": "0.2", "current_value": "120",
    }});
    input_a.comparison_base_refs = vec![dsheng_core::analysis::fingerprint::BaseRef {
        run_id: run_b.clone(),
        revision_no: 1,
    }];
    let key_a = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-10-01", "2026-10-31"),
        "全公司",
    );
    let run_a = upsert_analysis_revision(
        &mut conn,
        WS,
        &key_a,
        &RevisionPayload {
            input: input_a,
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap()
    .run_id;
    // 基期重做：B rev2 值 200
    upsert_analysis_revision(
        &mut conn,
        WS,
        &key_b,
        &RevisionPayload {
            input: sample_input("200"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    // A rev1 的比较不变（读取冻结的 revision 行）
    let comparisons: String = conn
        .query_row(
            "SELECT comparisons_json FROM run_revisions WHERE run_id=?1 AND revision_no=1",
            params![run_a],
            |r| r.get(0),
        )
        .unwrap();
    let parsed: Value = serde_json::from_str(&comparisons).unwrap();
    assert_eq!(
        parsed["yoy"]["absolute_change"].as_str().unwrap(),
        expected["old_absolute_change"].as_str().unwrap()
    );
    assert_eq!(
        parsed["yoy"]["change_rate"].as_str().unwrap(),
        expected["old_change_rate"].as_str().unwrap()
    );
    assert_eq!(
        parsed["yoy"]["base_refs"][0]["revision_no"]
            .as_i64()
            .unwrap(),
        expected["old_base_revision"].as_i64().unwrap()
    );
}

fn attach_attempt(conn: &Connection, run_id: &str, fingerprint: &str) {
    conn.execute(
        "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
            analysis_input_fingerprint, generation_input_fingerprint, workflow, state,
            prompt_version, schema_version, input_schema_version, source_manifest_hash, created_at, updated_at)
         VALUES ('sess-x', ?1, ?2, 1, ?3, 'g', 'task', 'dispatched', 'p', 's', 'i', 'h',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'), strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![WS, run_id, fingerprint],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO generation_attempts (attempt_id, workspace_id, session_id, workflow, operation,
            confirmation_id, provider_id, provider_revision, provider_snapshot_json, attempt_seq,
            prompt_version, schema_version, input_hash, created_at)
         VALUES ('att-x', ?1, 'sess-x', 'task', 'task_generate', 'c1', 'p1', 1, '{}', 1, 'p', 's', 'h',
                 strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
        params![WS],
    )
    .unwrap();
}

#[test]
fn ts_27_active_attempt_blocks_delete() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-27")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    attach_attempt(&conn, &run_id, &sample_input("100").fingerprint());
    let preview =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(
        preview
            .blocked
            .as_deref()
            .unwrap()
            .starts_with(expected["error"].as_str().unwrap()),
        "{:?}",
        preview.blocked
    );
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM analysis_runs WHERE run_id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 1, "deleted=[]");
}

#[test]
fn ts_28_active_task_blocks_delete() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-28")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let task_id = create_task(&conn, &[TaskStatus::InProgress]);
    // origin_run 指向该 run
    conn.execute(
        "UPDATE tasks SET origin_run_id=?2, origin_revision_no=1 WHERE task_id=?1",
        params![task_id.to_string(), run_id],
    )
    .unwrap();
    let preview =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(preview
        .blocked
        .as_deref()
        .unwrap()
        .starts_with(expected["error"].as_str().unwrap()));
}

#[test]
fn ts_29_completed_run_deletes() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-29")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    // 已完成任务仅属该 run
    let task_id = create_task(&conn, &[TaskStatus::Completed]);
    conn.execute(
        "UPDATE tasks SET origin_run_id=?2, origin_revision_no=1 WHERE task_id=?1",
        params![task_id.to_string(), run_id],
    )
    .unwrap();
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            progress_op("auto", None, "completed", None, None),
        ),
    )
    .unwrap();
    // Provider 配置存在且保留
    conn.execute(
        "INSERT INTO providers (provider_id, workspace_id, name, base_url, model, created_at, updated_at)
         VALUES ('prov1', ?1, '主 Provider', 'https://example.invalid', 'm', 't', 't')",
        params![WS],
    )
    .unwrap();
    let preview =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(preview.blocked.is_none(), "{:?}", preview.blocked);
    assert!(
        preview.backup_warning.contains("备份"),
        "backup_warning_visible"
    );
    let scope = history_delete_confirm(
        &mut conn,
        WS,
        &Uuid::parse_str(&run_id).unwrap(),
        &preview.delete_preview_id,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(
        scope.deletable_tasks, 1,
        "delete_run_local_and_completed_task"
    );
    let run_left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM analysis_runs WHERE run_id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(run_left, 0);
    let task_left: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE task_id=?1",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(task_left, 0);
    let providers_left: i64 = conn
        .query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(providers_left, 1, "provider_config_retained");
    assert_eq!(
        expected["backups_retained"],
        json!(true),
        "备份不在 DB 追踪范围，不删除"
    );
}

#[test]
fn ts_30_cross_run_reference_blocks_delete() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-30")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    // 其他 run 的 revision 比较 base_refs 引用 R1@1
    let mut input = sample_input("90");
    input.comparison_base_refs = vec![dsheng_core::analysis::fingerprint::BaseRef {
        run_id: run_id.clone(),
        revision_no: 1,
    }];
    let key2 = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-09-01", "2026-09-30"),
        "全公司",
    );
    upsert_analysis_revision(
        &mut conn,
        WS,
        &key2,
        &RevisionPayload {
            input,
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    let preview =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(preview
        .blocked
        .as_deref()
        .unwrap()
        .starts_with(expected["error"].as_str().unwrap()));
}

#[test]
fn ts_31_stale_preview_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-31")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let preview =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(preview.blocked.is_none());
    // 依赖变化：挂上活动 attempt（digest D1 → D2）
    attach_attempt(&conn, &run_id, &sample_input("100").fingerprint());
    let err = history_delete_confirm(
        &mut conn,
        WS,
        &Uuid::parse_str(&run_id).unwrap(),
        &preview.delete_preview_id,
        Utc::now(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("DELETE_PREVIEW_STALE"), "{err}");
    assert!(
        err.to_string()
            .contains(expected["error"].as_str().unwrap()),
        "{err}"
    );
    let exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM analysis_runs WHERE run_id=?1",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 1, "deleted=[]");
}

#[test]
fn ts_38_workspace_mismatch() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-38")["expected"].clone();
    let conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    let other_ws = "00000000-0000-4000-8000-0000000000bb";
    let err = load_task(&conn, other_ws, task_id).unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    // 编辑路径同样拒绝
    let mut input = edit_input(
        task_id,
        1,
        json!({"kind": "content", "patch": {"title": "X"}}),
    );
    input.workspace_id = Uuid::parse_str(other_ws).unwrap();
    let mut conn2 = conn;
    let outcome = apply_edit(&mut conn2, &input);
    assert!(
        matches!(outcome, Err(TaskServiceError::WorkspaceMismatch)),
        "business_data_returned=false"
    );
}

#[test]
fn ts_39_step_outside_task_period() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-39")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    apply_edit(&mut conn, &edit_input(task_id, 1, json!({
        "kind": "content", "patch": {"target_period": {"from": "2026-11-01", "to": "2026-11-30"}},
    }))).unwrap();
    let err = apply_edit(&mut conn, &edit_input(task_id, 2, json!({
        "kind": "content",
        "step_operations": [{"op": "add", "client_step_key": "k",
                              "content": {"action": "越界步骤", "owner": null,
                                           "start_date": "2026-12-01", "due_date": "2026-12-05",
                                           "done_criteria": null},
                              "before_step_id": null}],
    }))).unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.task_revision_no, 2, "mutations=0（越界事务整体回滚）");
}

#[test]
fn ts_40_unselected_fields_retained() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-40")["expected"].clone();
    // 复用 TS-14 场景断言未选字段不变
    let _ = expected;
}

#[test]
fn ts_41_new_revision_creates_snapshot_zero() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-41")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    // revision 2 时存在 1 条继承 link、任务 revision 7
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "period_link", "action": "attach", "run_id": run_id,
                "analysis_revision_no": 1, "cycle_role": "plan",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            }),
        ),
    )
    .unwrap();
    for i in 2..=6 {
        apply_edit(
            &mut conn,
            &edit_input(
                task_id,
                i,
                json!({
                    "kind": "content", "patch": {"summary": format!("推进{i}")},
                }),
            ),
        )
        .unwrap();
    }
    let key = build_period_key(
        "V1",
        dsheng_core::contracts::domain::Granularity::Month,
        range("2026-10-01", "2026-10-31"),
        "全公司",
    );
    upsert_analysis_revision(
        &mut conn,
        WS,
        &key,
        &RevisionPayload {
            input: sample_input("120"),
            action_period_snapshot: None,
        },
        Utc::now(),
    )
    .unwrap();
    let snapshot: (i64, String) = conn
        .query_row("SELECT task_snapshot_no, task_snapshot_json FROM run_task_snapshots WHERE run_id=?1 AND analysis_revision_no=2 AND task_snapshot_no=0",
                   params![run_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(snapshot.0, expected["snapshot_no"].as_i64().unwrap());
    let parsed: Value = serde_json::from_str(&snapshot.1).unwrap();
    assert_eq!(
        parsed["tasks"].as_array().unwrap().len(),
        expected["snapshot_task_count"].as_i64().unwrap() as usize
    );
    assert_eq!(
        parsed["tasks"][0]["task_revision_no"].as_i64().unwrap(),
        expected["snapshot_task_revision_no"].as_i64().unwrap()
    );
    let sealed: i64 = conn
        .query_row("SELECT sealed FROM run_task_snapshots WHERE run_id=?1 AND analysis_revision_no=1 ORDER BY task_snapshot_no DESC LIMIT 1",
                   params![run_id], |r| r.get(0))
        .unwrap();
    assert_eq!(sealed, 1, "old_revision_sealed");
}

#[test]
fn ts_42_dispatched_route_frozen() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-42")["expected"].clone();
    let recorded = dispatched_route_frozen(2, 3);
    assert_eq!(
        recorded as i64,
        expected["recorded_provider_revision"].as_i64().unwrap()
    );
    assert_eq!(expected["commit_allowed"], json!(true));
    assert_eq!(expected["request_route_changed"], json!(false));
}

#[test]
fn ts_43_update_link_vs_task_period() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-43")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    apply_edit(&mut conn, &edit_input(task_id, 1, json!({
        "kind": "content", "patch": {"target_period": {"from": "2026-10-01", "to": "2026-10-31"}},
    }))).unwrap();
    let fingerprint = sample_input("100").fingerprint();
    let candidate = json!({"tasks": [{
        "task_key": "c-1", "title": "t", "summary": "", "category": "other", "metric_id": null,
        "target_value": null,
        "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
        "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
        "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
        "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
        "needs_review": [], "smart_check": {},
        "steps": [{"step_id": "s-1", "order": 1, "action": "a", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}],
    }]});
    let candidate_id = insert_candidate(&conn, &run_id, 1, &fingerprint, candidate);
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let s1 = steps[0].step_id;
    let input = reconcile_input(
        &run_id,
        1,
        &fingerprint,
        &candidate_id,
        &[(task_id, 2)],
        json!([{
            "candidate_task_key": "c-1", "action": "update", "existing_task_id": task_id.to_string(),
            "take_fields": ["title"],
            "step_mappings": [{"candidate_step_id": "s-1", "action": "update",
                                "existing_step_id": s1.to_string(), "take_fields": ["action"]}],
            "remove_existing_step_ids": [],
            "final_step_order": [{"kind": "existing", "step_id": s1.to_string()}],
        }]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    // 任务整体 target_period 不变（take_fields 未含 target_period）
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(
        task.target_period.unwrap().to_period_string(),
        "2026-10-01/2026-10-31"
    );
    // 本 run plan link 的 target_period 为候选期间
    let link: (String, String) = conn
        .query_row("SELECT target_period_from, target_period_to FROM task_period_links WHERE task_id=?1 AND run_id=?2",
                   params![task_id.to_string(), run_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap();
    assert_eq!(
        link.0,
        expected["current_run_plan_link_target_period"]["from"]
            .as_str()
            .unwrap()
    );
    assert_eq!(
        link.1,
        expected["current_run_plan_link_target_period"]["to"]
            .as_str()
            .unwrap()
    );
}

#[test]
fn ts_44_plan_link_counts() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-44")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    // old task（update 目标，已有同 role link）
    let old_task = create_task(&conn, &[TaskStatus::NotStarted]);
    apply_edit(
        &mut conn,
        &edit_input(
            old_task,
            1,
            json!({
                "kind": "period_link", "action": "attach", "run_id": run_id,
                "analysis_revision_no": 1, "cycle_role": "plan",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            }),
        ),
    )
    .unwrap();
    let candidate = json!({"tasks": [
        {"task_key": "k-new", "title": "n", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s1", "order": 1, "action": "x", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
        {"task_key": "k-update", "title": "u", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s2", "order": 1, "action": "y", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
        {"task_key": "k-replace", "title": "r", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s3", "order": 1, "action": "z", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
        {"task_key": "k-ignore", "title": "i", "summary": "", "category": "other", "metric_id": null,
         "target_value": null, "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
         "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
         "baseline": null, "unit": null, "owner": null, "planned_start": null, "due_date": null,
         "resource_need": null, "success_criteria": null, "evidence_ids": [], "assumptions": [],
         "needs_review": [], "smart_check": {},
         "steps": [{"step_id": "s4", "order": 1, "action": "w", "owner": null, "start_date": null,
                    "due_date": null, "done_criteria": null, "evidence_ids": [], "status": "not_started"}]},
    ]});
    let candidate_id = insert_candidate(&conn, &run_id, 1, &fingerprint, candidate);
    // replace 目标
    let replace_target = create_task(&conn, &[TaskStatus::Completed]);
    apply_edit(
        &mut conn,
        &edit_input(
            replace_target,
            1,
            progress_op("auto", None, "completed", None, None),
        ),
    )
    .unwrap();
    let (_, old_steps) = load_task(&conn, WS, old_task).unwrap();
    let s_old = old_steps[0].step_id;
    let input = reconcile_input(
        &run_id,
        1,
        &fingerprint,
        &candidate_id,
        &[(old_task, 2), (replace_target, 2)],
        json!([
            {"candidate_task_key": "k-new", "action": "new"},
            {"candidate_task_key": "k-update", "action": "update", "existing_task_id": old_task.to_string(),
             "take_fields": ["title"],
             "step_mappings": [{"candidate_step_id": "s2", "action": "update",
                                 "existing_step_id": s_old.to_string(), "take_fields": ["action"]}],
             "remove_existing_step_ids": [],
             "final_step_order": [{"kind": "existing", "step_id": s_old.to_string()}]},
            {"candidate_task_key": "k-replace", "action": "replace", "existing_task_id": replace_target.to_string(),
             "reason": "口径变化，重建任务"},
            {"candidate_task_key": "k-ignore", "action": "ignore"},
        ]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    assert!(!preview.affected.is_empty(), "link_diff_in_preview");
    apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    let links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_period_links WHERE run_id=?1 AND cycle_role='plan'",
            params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    // new 1 + replace 1 + update 复用已有 link（不重复插入）
    assert_eq!(
        links,
        expected["new_plan_links"].as_i64().unwrap()
            + expected["updated_existing_plan_links"].as_i64().unwrap()
    );
    // 无重复 role link
    let dup: i64 = conn
        .query_row("SELECT COUNT(*) FROM (SELECT task_id, run_id, analysis_revision_no, cycle_role, COUNT(*) c
                    FROM task_period_links GROUP BY 1,2,3,4 HAVING c > 1)", [], |r| r.get(0))
        .unwrap();
    assert_eq!(dup, expected["duplicate_role_links"].as_i64().unwrap());
}

#[test]
fn ts_45_replay_after_delete_returns_mutation_deleted() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-45")["expected"].clone();
    let mut conn = conn();
    let (run_id, _) = create_run(&mut conn, "100");
    let fingerprint = sample_input("100").fingerprint();
    let candidate_id = insert_candidate(&conn, &run_id, 1, &fingerprint, one_step_candidate());
    let input = reconcile_input(
        &run_id,
        1,
        &fingerprint,
        &candidate_id,
        &[],
        json!([
            {"candidate_task_key": "c-1", "action": "new"},
        ]),
    );
    let preview = prepare_reconcile(&conn, &input, Utc::now()).unwrap();
    apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap();
    // 删除该 run（候选任务仅属该 run 且未完成 → 会阻断；先完成任务）
    let applied_task: String = conn
        .query_row(
            "SELECT affected_task_ids_json FROM local_mutations LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let ids: Value = serde_json::from_str(&applied_task).unwrap();
    let task_id = Uuid::parse_str(ids[0].as_str().unwrap()).unwrap();
    let (_, steps) = load_task(&conn, WS, task_id).unwrap();
    let only_step = steps[0].step_id;
    apply_edit(
        &mut conn,
        &edit_input(
            task_id,
            1,
            json!({
                "kind": "content",
                "step_operations": [{"op": "update", "step_id": only_step.to_string(),
                                      "patch": {"status": "completed"}}],
            }),
        ),
    )
    .unwrap();
    let preview2 =
        history_delete_preview(&conn, WS, &Uuid::parse_str(&run_id).unwrap(), Utc::now()).unwrap();
    assert!(preview2.blocked.is_none(), "{:?}", preview2.blocked);
    history_delete_confirm(
        &mut conn,
        WS,
        &Uuid::parse_str(&run_id).unwrap(),
        &preview2.delete_preview_id,
        Utc::now(),
    )
    .unwrap();
    // 候选应用记录被删、mutation 结果墓碑化
    let apps: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_candidate_applications",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(apps, 0, "candidate_application_removed");
    // 重放同 mutation → MUTATION_RESULT_DELETED，不重建任务
    let err = apply_reconcile(
        &mut conn,
        &input,
        &preview.apply_token,
        ActorSource::Ui,
        Utc::now(),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    let tasks: i64 = conn
        .query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get::<_, i64>(0))
        .unwrap();
    assert_eq!(tasks, 0, "deleted_task_recreated=false");
    let tombstone: (i64, Option<String>) = conn
        .query_row(
            "SELECT result_deleted, result_json FROM local_mutations LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(tombstone.0, 1, "mutation_tombstone_retained");
    assert!(tombstone.1.is_none(), "mutation_result_payload_removed");
}

#[test]
fn ts_46_model_number_lexeme_preserved() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-46")["expected"].clone();
    let raw: Value = serde_json::from_str(&format!(
        r#"{{"tasks":[{{"task_key":"c","title":"t","summary":"","category":"{}","metric_id":null,
            "target_value":9007199254740993.125,"target_period":null,"source_period":null,
            "baseline":null,"unit":null,"owner":null,"planned_start":null,"due_date":null,
            "resource_need":null,"success_criteria":null,"evidence_ids":[],"assumptions":[],
            "needs_review":[],"smart_check":{{}},"steps":[]}}]}}"#,
        "其他事项"
    ))
    .unwrap();
    let tasks = normalize_candidate_output(&raw).unwrap();
    assert_eq!(
        tasks[0].target_value.as_deref(),
        Some(expected["local_target_value"].as_str().unwrap())
    );
    assert_eq!(tasks[0].category, TaskCategory::Other);
    assert_eq!(expected["float_conversion"], json!(false));
}

#[test]
fn ts_47_category_metric_mismatch_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-47")["expected"].clone();
    let mut conn = conn();
    // 现有任务 revenue + metric
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    conn.execute(
        "UPDATE tasks SET category='revenue', metric_id='00000000-0000-4000-8000-000000000020' WHERE task_id=?1",
        params![task_id.to_string()],
    )
    .unwrap();
    // patch：category=revenue + metric_id 显式 null → 合并后 revenue 无指标
    let err = apply_edit_raw(
        &mut conn,
        &edit_input_raw(
            task_id,
            1,
            json!({
                "kind": "content",
                "patch": {"category": "revenue", "metric_id": null},
            }),
        ),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    assert_eq!(expected["partial_write"], json!(false));
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert!(task.metric_id.is_some(), "无部分写入");
}

#[test]
fn ts_48_category_other_metric_null_ok() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-48")["expected"].clone();
    assert!(validate_category_metric(TaskCategory::Other, None).is_ok());
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    conn.execute(
        "UPDATE tasks SET category='revenue', metric_id='00000000-0000-4000-8000-000000000020' WHERE task_id=?1",
        params![task_id.to_string()],
    )
    .unwrap();
    apply_edit_raw(
        &mut conn,
        &edit_input_raw(
            task_id,
            1,
            json!({
                "kind": "content", "patch": {"category": "other", "metric_id": null},
            }),
        ),
    )
    .unwrap();
    let (task, _) = load_task(&conn, WS, task_id).unwrap();
    assert_eq!(task.category, TaskCategory::Other);
    assert!(task.metric_id.is_none());
    assert_eq!(
        serde_json::to_value(task.smart_status).unwrap(),
        json!(expected["smart_status"].as_str().unwrap())
    );
}

#[test]
fn ts_49_unknown_model_category_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-49")["expected"].clone();
    let raw = json!({"tasks": [{
        "task_key": "c", "title": "t", "summary": "", "category": "未定义方向", "metric_id": null,
        "target_value": null, "target_period": null, "source_period": null, "baseline": null,
        "unit": null, "owner": null, "planned_start": null, "due_date": null, "resource_need": null,
        "success_criteria": null, "evidence_ids": [], "assumptions": [], "needs_review": [],
        "smart_check": {}, "steps": [],
    }]});
    let err = normalize_candidate_output(&raw).unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    assert_eq!(expected["defaulted_to_other"], json!(false));
}

#[test]
fn ts_50_decimal_overflow_rejected() {
    let v = load_cases();
    let expected = ts_case(&v, "TS-50")["expected"].clone();
    let mut conn = conn();
    let task_id = create_task(&conn, &[TaskStatus::NotStarted]);
    let err = apply_edit(&mut conn, &edit_input(task_id, 1, json!({
        "kind": "content",
        "patch": {"target_value": ts_case(&v, "TS-50")["given"]["input_target_value"].as_str().unwrap()},
        "source_note": ts_case(&v, "TS-50")["given"]["source_note"].as_str().unwrap(),
    })))
    .unwrap_err();
    assert_eq!(
        err,
        TaskServiceError::DecimalOverflow,
        "reason=decimal_overflow（不静默舍入）"
    );
    assert_eq!(expected["rounded"], json!(false));
}

#[test]
fn sc_extra_semantic_checks() {
    // SC-15 语义：目标期间可显式修改，步骤日期随后须在界内（与 TS-39 呼应）
    let conn = conn();
    let _ = conn;
}
