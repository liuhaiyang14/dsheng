//! 任务 SQLite 服务：编辑事务、修订/事件、幂等、周期链接与 run 快照
//! （task-lifecycle.md §1/§2/§3.3/§6）。

use super::tasks::{
    apply_content_edit, apply_progress_edit, mutation_digest, patch_metric_id_explicit_null,
    PlannedEvent, StepRow, TaskRow, TaskServiceError,
};
use crate::contracts::decimal::to_canonical_string;
use crate::contracts::domain::DateRange;
use crate::contracts::tasks::{
    ActorSource, PeriodLinkAction, TaskEditInput, TaskEditOperation, TaskLifecycle, TaskStatus,
};
use crate::repository::utc_iso;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

/// 编辑操作结果（local_mutations 的 result_json 载荷）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskEditOutcome {
    pub task_id: String,
    pub task_revision_no: u32,
    pub status: String,
    pub progress: String,
    /// 幂等重放（未产生新变更）。
    pub replayed: bool,
}

pub(crate) enum MutationCheck {
    Proceed,
    Replay(String),
    Conflict,
    Deleted,
}

/// 幂等检查：同 mutation_id 同 digest 重放、不同 digest 冲突、已删结果返回墓碑。
pub(crate) fn check_mutation(
    conn: &Connection,
    workspace_id: &str,
    mutation_id: &str,
    digest: &str,
) -> Result<MutationCheck, TaskServiceError> {
    let row: Option<(String, Option<String>, i64)> = conn
        .query_row(
            "SELECT request_digest, result_json, result_deleted FROM local_mutations
             WHERE workspace_id = ?1 AND mutation_id = ?2",
            params![workspace_id, mutation_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    match row {
        None => Ok(MutationCheck::Proceed),
        Some((stored_digest, result, deleted)) => {
            if deleted == 1 {
                Ok(MutationCheck::Deleted)
            } else if stored_digest != digest {
                Ok(MutationCheck::Conflict)
            } else {
                match result {
                    Some(json) => Ok(MutationCheck::Replay(json)),
                    None => Ok(MutationCheck::Conflict),
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_mutation(
    conn: &Connection,
    workspace_id: &str,
    mutation_id: &str,
    digest: &str,
    result_json: &str,
    affected_run_ids: &[String],
    affected_task_ids: &[String],
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    conn.execute(
        "INSERT INTO local_mutations
           (workspace_id, mutation_id, request_digest, result_json, result_deleted,
            affected_run_ids_json, affected_task_ids_json, committed_at)
         VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6, ?7)",
        params![
            workspace_id,
            mutation_id,
            digest,
            result_json,
            serde_json::to_string(affected_run_ids).expect("json"),
            serde_json::to_string(affected_task_ids).expect("json"),
            utc_iso(now)
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(())
}

fn date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn opt_date(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

fn period_json(p: Option<DateRange>) -> Option<String> {
    p.map(|r| {
        serde_json::to_string(&json!({"from": date_str(r.from), "to": date_str(r.to)}))
            .expect("json")
    })
}

/// 读取任务当前态 + 步骤（含 tombstone）；跨 workspace → WORKSPACE_MISMATCH。
pub fn load_task(
    conn: &Connection,
    workspace_id: &str,
    task_id: Uuid,
) -> Result<(TaskRow, Vec<StepRow>), TaskServiceError> {
    let found: Option<String> = conn
        .query_row(
            "SELECT workspace_id FROM tasks WHERE task_id = ?1",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    let Some(ws) = found else {
        return Err(TaskServiceError::Db("task_not_found".into()));
    };
    if ws != workspace_id {
        return Err(TaskServiceError::WorkspaceMismatch);
    }
    let task: TaskRow = conn
        .query_row(
            "SELECT task_id, workspace_id, task_revision_no, title, summary, category,
                    metric_id, baseline_json, target_value, planned_value, actual_value,
                    current_value, unit, mp_label, mp_unit, owner, planned_start, due_date,
                    target_period_json, resource_need, success_criteria, assumptions_json,
                    needs_review_json, smart_check_json, status, lifecycle,
                    superseded_by_task_id, progress, progress_mode, adjustment_reason,
                    blocked_reason, smart_status, review_note, completed_at
             FROM tasks WHERE task_id = ?1",
            params![task_id.to_string()],
            map_task_row,
        )
        .map_err(TaskServiceError::from)?;
    let mut stmt = conn
        .prepare(
            "SELECT step_id, step_order, action, owner, start_date, due_date, done_criteria,
                    status, blocked_reason, completed_at, deleted_at
             FROM task_steps WHERE task_id = ?1 ORDER BY step_order",
        )
        .map_err(TaskServiceError::from)?;
    let rows = stmt
        .query_map(params![task_id.to_string()], map_step_row)
        .map_err(TaskServiceError::from)?;
    let mut steps = Vec::new();
    for row in rows {
        steps.push(row.map_err(TaskServiceError::from)?);
    }
    Ok((task, steps))
}

/// tasks 行映射（列序与 load_task 的 SELECT 一致）。
fn map_task_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRow> {
    let ws: String = r.get(1)?;
    let progress: String = r.get(27)?;
    Ok(TaskRow {
        task_id: Uuid::parse_str(&r.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("task_id".into()))?,
        workspace_id: Uuid::parse_str(&ws)
            .map_err(|_| rusqlite::Error::InvalidColumnName("workspace_id".into()))?,
        task_revision_no: r.get::<_, i64>(2)? as u32,
        title: r.get(3)?,
        summary: r.get(4)?,
        category: serde_json::from_value(json!(r.get::<_, String>(5)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("category".into()))?,
        metric_id: r
            .get::<_, Option<String>>(6)?
            .as_deref()
            .and_then(|s| Uuid::parse_str(s).ok()),
        baseline_json: r.get(7)?,
        target_value: r.get(8)?,
        planned_value: r.get(9)?,
        actual_value: r.get(10)?,
        current_value: r.get(11)?,
        unit: r.get(12)?,
        mp_label: r.get(13)?,
        mp_unit: r.get(14)?,
        owner: r.get(15)?,
        planned_start: r
            .get::<_, Option<String>>(16)?
            .as_deref()
            .and_then(opt_date),
        due_date: r
            .get::<_, Option<String>>(17)?
            .as_deref()
            .and_then(opt_date),
        target_period: r
            .get::<_, Option<String>>(18)?
            .as_deref()
            .and_then(parse_period_json),
        resource_need: r.get(19)?,
        success_criteria: r.get(20)?,
        assumptions_json: r.get(21)?,
        needs_review_json: r.get(22)?,
        smart_check_json: r.get(23)?,
        status: serde_json::from_value(json!(r.get::<_, String>(24)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("status".into()))?,
        lifecycle: serde_json::from_value(json!(r.get::<_, String>(25)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("lifecycle".into()))?,
        superseded_by_task_id: r.get::<_, Option<String>>(26)?.as_deref().and_then(|s| {
            if s.is_empty() {
                None
            } else {
                Uuid::parse_str(s).ok()
            }
        }),
        progress: crate::contracts::decimal::canonical_decimal(&progress)
            .map_err(|_| rusqlite::Error::InvalidColumnName("progress".into()))?,
        progress_mode: serde_json::from_value(json!(r.get::<_, String>(28)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("progress_mode".into()))?,
        adjustment_reason: r.get(29)?,
        blocked_reason: r.get(30)?,
        smart_status: serde_json::from_value(json!(r.get::<_, String>(31)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("smart_status".into()))?,
        review_note: r.get(32)?,
        completed_at: r.get::<_, Option<String>>(33)?.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
    })
}

fn map_step_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StepRow> {
    Ok(StepRow {
        step_id: Uuid::parse_str(&r.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("step_id".into()))?,
        step_order: r.get::<_, i64>(1)? as u32,
        action: r.get(2)?,
        owner: r.get(3)?,
        start_date: r.get::<_, Option<String>>(4)?.as_deref().and_then(opt_date),
        due_date: r.get::<_, Option<String>>(5)?.as_deref().and_then(opt_date),
        done_criteria: r.get(6)?,
        status: serde_json::from_value(json!(r.get::<_, String>(7)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("status".into()))?,
        blocked_reason: r.get(8)?,
        completed_at: r.get::<_, Option<String>>(9)?.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
        deleted_at: r.get::<_, Option<String>>(10)?.as_deref().and_then(|s| {
            DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }),
    })
}

fn parse_period_json(s: &str) -> Option<DateRange> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    let from = chrono::NaiveDate::parse_from_str(v.get("from")?.as_str()?, "%Y-%m-%d").ok()?;
    let to = chrono::NaiveDate::parse_from_str(v.get("to")?.as_str()?, "%Y-%m-%d").ok()?;
    DateRange::new(from, to).ok()
}

/// 写回任务当前态（不改 revision——由调用方统一 +1）。
///
/// `steps` 必须是该任务的完整步骤集（`load_task` 载入后原地修改），含 tombstone。
///
/// 两阶段重排：`idx_task_steps_live_order` 对存活行 (task_id, step_order) 唯一，
/// 逐行 upsert 写非恒等重排（如 1,2,3→3,1,2）会在中途撞唯一键；因此先在
/// 同一事务内把该任务全部存活行的 step_order 平移出最终序区间（平移量 ≥
/// 最终序最大值，中间序永不与最终序重叠），再逐行写最终序。恒等/局部修改
/// 同样安全；失败随整个事务回滚，不残留中间序。
pub(crate) fn save_task_state(
    conn: &Connection,
    task: &TaskRow,
    steps: &[StepRow],
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let category = serde_json::to_value(task.category).expect("json");
    let category = category.as_str().expect("snake_case");
    let status = serde_json::to_value(task.status).expect("json");
    let status = status.as_str().expect("snake_case");
    let lifecycle = serde_json::to_value(task.lifecycle).expect("json");
    let lifecycle = lifecycle.as_str().expect("snake_case");
    let progress_mode = serde_json::to_value(task.progress_mode).expect("json");
    let progress_mode = progress_mode.as_str().expect("snake_case");
    let smart_status = serde_json::to_value(task.smart_status).expect("json");
    let smart_status = smart_status.as_str().expect("snake_case");
    conn.execute(
        "UPDATE tasks SET task_revision_no=?2, title=?3, summary=?4, category=?5, metric_id=?6,
                baseline_json=?7, target_value=?8, planned_value=?9, actual_value=?10,
                current_value=?11, unit=?12, mp_label=?13, mp_unit=?14, owner=?15,
                planned_start=?16, due_date=?17, target_period_json=?18, resource_need=?19,
                success_criteria=?20, status=?21, lifecycle=?22, superseded_by_task_id=?23,
                progress=?24, progress_mode=?25, adjustment_reason=?26, blocked_reason=?27,
                smart_status=?28, review_note=?29, completed_at=?30, updated_at=?31
         WHERE task_id=?1",
        params![
            task.task_id.to_string(),
            task.task_revision_no as i64,
            task.title,
            task.summary,
            category,
            task.metric_id.map(|u| u.to_string()),
            task.baseline_json,
            task.target_value,
            task.planned_value,
            task.actual_value,
            task.current_value,
            task.unit,
            task.mp_label,
            task.mp_unit,
            task.owner,
            task.planned_start.map(date_str),
            task.due_date.map(date_str),
            period_json(task.target_period),
            task.resource_need,
            task.success_criteria,
            status,
            lifecycle,
            task.superseded_by_task_id.map(|u| u.to_string()),
            to_canonical_string(task.progress),
            progress_mode,
            task.adjustment_reason,
            task.blocked_reason,
            smart_status,
            task.review_note,
            task.completed_at.map(utc_iso),
            utc_iso(now),
        ],
    )
    .map_err(TaskServiceError::from)?;
    // 阶段 1：存活行 step_order 整体平移出最终序区间（唯一索引中间态安全）
    let max_db_order: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(step_order), 0) FROM task_steps
             WHERE task_id = ?1 AND deleted_at IS NULL",
            params![task.task_id.to_string()],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    let max_final_order: i64 = steps.iter().map(|s| s.step_order as i64).max().unwrap_or(0);
    let offset = max_final_order.max(max_db_order);
    if offset > 0 {
        conn.execute(
            "UPDATE task_steps SET step_order = step_order + ?2, updated_at = ?3
             WHERE task_id = ?1 AND deleted_at IS NULL",
            params![task.task_id.to_string(), offset, utc_iso(now)],
        )
        .map_err(TaskServiceError::from)?;
    }
    // 阶段 2：逐行写最终序（含新增/删除标记），最终序严格小于任一平移序
    for step in steps {
        let st = serde_json::to_value(step.status).expect("json");
        let st = st.as_str().expect("snake_case");
        conn.execute(
            "INSERT INTO task_steps
               (step_id, task_id, step_order, action, owner, start_date, due_date,
                done_criteria, status, blocked_reason, completed_at, deleted_at,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
             ON CONFLICT(step_id) DO UPDATE SET
                step_order=excluded.step_order, action=excluded.action,
                owner=excluded.owner, start_date=excluded.start_date,
                due_date=excluded.due_date, done_criteria=excluded.done_criteria,
                status=excluded.status, blocked_reason=excluded.blocked_reason,
                completed_at=excluded.completed_at, deleted_at=excluded.deleted_at,
                updated_at=excluded.updated_at",
            params![
                step.step_id.to_string(),
                task.task_id.to_string(),
                step.step_order as i64,
                step.action,
                step.owner,
                step.start_date.map(date_str),
                step.due_date.map(date_str),
                step.done_criteria,
                st,
                step.blocked_reason,
                step.completed_at.map(utc_iso),
                step.deleted_at.map(utc_iso),
                utc_iso(now),
            ],
        )
        .map_err(TaskServiceError::from)?;
    }
    Ok(())
}

fn next_event_seq(conn: &Connection) -> Result<i64, TaskServiceError> {
    conn.query_row(
        "SELECT COALESCE(MAX(event_seq), 0) + 1 FROM task_events",
        [],
        |r| r.get(0),
    )
    .map_err(TaskServiceError::from)
}

#[allow(clippy::explicit_counter_loop)]
pub(crate) fn append_events(
    conn: &Connection,
    task_id: Uuid,
    revision_no: u32,
    actor: ActorSource,
    mutation_id: Uuid,
    events: &[PlannedEvent],
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let actor_str = serde_json::to_value(actor).expect("json");
    let actor_str = actor_str.as_str().expect("snake_case");
    let mut seq = next_event_seq(conn)?;
    for e in events.iter() {
        conn.execute(
            "INSERT INTO task_events
               (event_id, task_id, event_seq, task_revision_no, event_type, actor_source,
                mutation_id, before_json, after_json, reason, occurred_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                Uuid::new_v4().to_string(),
                task_id.to_string(),
                seq,
                revision_no as i64,
                e.event_type,
                actor_str,
                mutation_id.to_string(),
                e.before_json,
                e.after_json,
                e.reason,
                utc_iso(now),
            ],
        )
        .map_err(TaskServiceError::from)?;
        seq += 1;
    }
    Ok(())
}

pub(crate) fn append_task_revision(
    conn: &Connection,
    task: &TaskRow,
    steps: &[StepRow],
    actor: ActorSource,
    mutation_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let snapshot = task_snapshot_full(task, steps);
    let actor_str = serde_json::to_value(actor).expect("json");
    let actor_str = actor_str.as_str().expect("snake_case");
    conn.execute(
        "INSERT INTO task_revisions (task_id, revision_no, fields_snapshot_json, actor_source, mutation_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            task.task_id.to_string(),
            task.task_revision_no as i64,
            snapshot,
            actor_str,
            mutation_id.to_string(),
            utc_iso(now)
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(())
}

pub(crate) fn task_snapshot_full(task: &TaskRow, steps: &[StepRow]) -> String {
    serde_json::to_string(&json!({
        "progress": to_canonical_string(task.progress),
        "progress_mode": serde_json::to_value(task.progress_mode).unwrap(),
        "smart_status": serde_json::to_value(task.smart_status).unwrap(),
        "status": serde_json::to_value(task.status).unwrap(),
        "steps": steps.iter().filter(|s| s.deleted_at.is_none()).map(|s| json!({
            "action": s.action,
            "status": serde_json::to_value(s.status).unwrap(),
            "step_id": s.step_id.to_string(),
        })).collect::<Vec<_>>(),
        "task_revision_no": task.task_revision_no,
        "title": task.title,
    }))
    .expect("json")
}

/// 任务关联 run 的当前 revision 链接（供快照）。
fn current_links_for_task(
    conn: &Connection,
    task_id: Uuid,
) -> Result<Vec<(String, i64)>, TaskServiceError> {
    let mut stmt = conn
        .prepare(
            "SELECT l.run_id, l.analysis_revision_no FROM task_period_links l
             JOIN analysis_runs r ON r.run_id = l.run_id
             WHERE l.task_id = ?1 AND r.current_revision_no = l.analysis_revision_no",
        )
        .map_err(TaskServiceError::from)?;
    let rows = stmt
        .query_map(params![task_id.to_string()], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(TaskServiceError::from)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(TaskServiceError::from)?);
    }
    Ok(out)
}

/// 影响该 run 当前链接任务的事务后追加 run_task_snapshots（§6）。
pub(crate) fn append_snapshots_for_task(
    conn: &Connection,
    task_id: Uuid,
    now: DateTime<Utc>,
) -> Result<Vec<(String, i64, i64)>, TaskServiceError> {
    let mut created = Vec::new();
    for (run_id, revision) in current_links_for_task(conn, task_id)? {
        created.push(append_task_snapshot(conn, &run_id, revision, now)?);
    }
    Ok(created)
}

/// 追加一版 run 任务快照（run 当前 revision；no 递增；含全部链接任务当前态）。
pub fn append_task_snapshot(
    conn: &Connection,
    run_id: &str,
    analysis_revision_no: i64,
    now: DateTime<Utc>,
) -> Result<(String, i64, i64), TaskServiceError> {
    let current: i64 = conn
        .query_row(
            "SELECT current_revision_no FROM analysis_runs WHERE run_id = ?1",
            params![run_id],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    if current != analysis_revision_no {
        // 只更新当前关联 run 的快照，不追加到已封存旧 revision（§6）
        return Ok((run_id.to_string(), analysis_revision_no, -1));
    }
    let cutoff: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(event_seq), 0) FROM task_events",
            [],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    let mut stmt = conn
        .prepare(
            "SELECT task_id FROM task_period_links WHERE run_id = ?1 AND analysis_revision_no = ?2",
        )
        .map_err(TaskServiceError::from)?;
    let task_ids: Vec<String> = stmt
        .query_map(params![run_id, analysis_revision_no], |r| r.get(0))
        .map_err(TaskServiceError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(TaskServiceError::from)?;
    let mut tasks_json = Vec::new();
    for tid in &task_ids {
        if let Ok(uuid) = Uuid::parse_str(tid) {
            let (task, steps) = load_task(conn, &task_workspace(conn, &uuid)?, uuid)?;
            tasks_json.push(json!({
                "task_id": tid,
                "task_revision_no": task.task_revision_no,
                "progress": to_canonical_string(task.progress),
                "status": serde_json::to_value(task.status).unwrap(),
                "steps": steps.iter().filter(|s| s.deleted_at.is_none()).map(|s| json!({
                    "step_id": s.step_id.to_string(),
                    "status": serde_json::to_value(s.status).unwrap(),
                })).collect::<Vec<_>>(),
            }));
        }
    }
    let snapshot_json = serde_json::to_string(&json!({"tasks": tasks_json})).expect("json");
    conn.execute(
        "INSERT INTO run_task_snapshots
           (run_id, analysis_revision_no, task_snapshot_no, task_snapshot_json,
            event_cutoff_seq, captured_at, sealed)
         VALUES (?1, ?2, (SELECT COALESCE(MAX(t.task_snapshot_no), -1) + 1
                          FROM run_task_snapshots t
                          WHERE t.run_id = ?1 AND t.analysis_revision_no = ?2),
                 ?3, ?4, ?5, 0)",
        params![
            run_id,
            analysis_revision_no,
            snapshot_json,
            cutoff,
            utc_iso(now)
        ],
    )
    .map_err(TaskServiceError::from)?;
    let no: i64 = conn
        .query_row(
            "SELECT MAX(task_snapshot_no) FROM run_task_snapshots
             WHERE run_id = ?1 AND analysis_revision_no = ?2",
            params![run_id, analysis_revision_no],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    Ok((run_id.to_string(), analysis_revision_no, no))
}

fn task_workspace(conn: &Connection, task_id: &Uuid) -> Result<String, TaskServiceError> {
    conn.query_row(
        "SELECT workspace_id FROM tasks WHERE task_id = ?1",
        params![task_id.to_string()],
        |r| r.get(0),
    )
    .map_err(TaskServiceError::from)
}

/// 新分析 revision：继承上一 revision 的 links、封存旧快照、建 snapshot 0（§6）。
pub(crate) fn adopt_revision_tasks(
    conn: &Connection,
    run_id: &str,
    previous_revision: Option<i64>,
    new_revision: i64,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    if let Some(prev) = previous_revision {
        conn.execute(
            "INSERT INTO task_period_links
               (id, workspace_id, task_id, run_id, analysis_revision_no, cycle_role,
                source_period_from, source_period_to, target_period_from, target_period_to,
                inherited_from_link_id, created_at)
             SELECT 'inherited-' || lower(hex(randomblob(16))), l.workspace_id, l.task_id,
                    l.run_id, ?2, l.cycle_role, l.source_period_from, l.source_period_to,
                    l.target_period_from, l.target_period_to, l.id, ?3
             FROM task_period_links l WHERE l.run_id = ?1 AND l.analysis_revision_no = ?4",
            params![run_id, new_revision, utc_iso(now), prev],
        )
        .map_err(TaskServiceError::from)?;
        // 旧 revision 退出当前：封存其最后一个快照
        conn.execute(
            "UPDATE run_task_snapshots SET sealed = 1
             WHERE run_id = ?1 AND analysis_revision_no = ?2 AND sealed = 0",
            params![run_id, prev],
        )
        .map_err(TaskServiceError::from)?;
    }
    // snapshot 0：继承后完整 links 与任务当前态；无任务时显式空快照
    conn.execute(
        "INSERT INTO run_task_snapshots
           (run_id, analysis_revision_no, task_snapshot_no, task_snapshot_json,
            event_cutoff_seq, captured_at, sealed)
         VALUES (?1, ?2, 0, ?3, 0, ?4, 0)",
        params![
            run_id,
            new_revision,
            serde_json::to_string(&json!({"tasks": []})).expect("json"),
            utc_iso(now)
        ],
    )
    .map_err(TaskServiceError::from)?;
    refresh_snapshot_zero(conn, run_id, new_revision)?;
    Ok(())
}

fn refresh_snapshot_zero(
    conn: &Connection,
    run_id: &str,
    revision: i64,
) -> Result<(), TaskServiceError> {
    let mut stmt = conn
        .prepare(
            "SELECT task_id FROM task_period_links WHERE run_id = ?1 AND analysis_revision_no = ?2",
        )
        .map_err(TaskServiceError::from)?;
    let task_ids: Vec<String> = stmt
        .query_map(params![run_id, revision], |r| r.get(0))
        .map_err(TaskServiceError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(TaskServiceError::from)?;
    let mut tasks_json = Vec::new();
    for tid in &task_ids {
        if let Ok(uuid) = Uuid::parse_str(tid) {
            let (task, _steps) = load_task(conn, &task_workspace(conn, &uuid)?, uuid)?;
            tasks_json.push(json!({
                "task_id": tid,
                "task_revision_no": task.task_revision_no,
                "progress": to_canonical_string(task.progress),
                "status": serde_json::to_value(task.status).unwrap(),
            }));
        }
    }
    conn.execute(
        "UPDATE run_task_snapshots SET task_snapshot_json = ?3
         WHERE run_id = ?1 AND analysis_revision_no = ?2 AND task_snapshot_no = 0",
        params![
            run_id,
            revision,
            serde_json::to_string(&json!({"tasks": tasks_json})).expect("json")
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(())
}

/// 任务编辑服务入口（task-edit 合同；content/progress/period_link）。
pub struct TaskEditService<'a> {
    pub conn: &'a mut Connection,
    pub now: DateTime<Utc>,
    pub actor: ActorSource,
}

impl<'a> TaskEditService<'a> {
    /// 应用一次 task-edit（幂等 + 乐观并发 + 单事务）。
    pub fn apply(&mut self, input: &TaskEditInput) -> Result<TaskEditOutcome, TaskServiceError> {
        let mut input_json = serde_json::to_value(input).expect("json");
        // 类型调用的 None 表示未提供；不能把序列化补出的 null 当成清空指标。
        // 需要显式清空的 JSON 调用走 apply_raw，保留原始字段存在性。
        if let TaskEditOperation::Content { patch, .. } = &input.operation {
            if patch.as_deref().is_some_and(|p| p.metric_id.is_none()) {
                if let Some(patch) = input_json["operation"]["patch"].as_object_mut() {
                    patch.remove("metric_id");
                }
            }
        }
        self.apply_input(input, &input_json)
    }

    /// JSON 边界保留省略字段与显式 null 的区别；类型由同一原文产生，避免双输入不一致。
    pub fn apply_raw(
        &mut self,
        input_json: &serde_json::Value,
    ) -> Result<TaskEditOutcome, TaskServiceError> {
        let input: TaskEditInput = serde_json::from_value(input_json.clone())
            .map_err(|e| TaskServiceError::Db(format!("task_edit_input_invalid: {e}")))?;
        self.apply_input(&input, input_json)
    }

    fn apply_input(
        &mut self,
        input: &TaskEditInput,
        input_json: &serde_json::Value,
    ) -> Result<TaskEditOutcome, TaskServiceError> {
        let digest = mutation_digest(input_json);
        let ws = input.workspace_id.to_string();
        let tx = self.conn.transaction().map_err(TaskServiceError::from)?;
        match check_mutation(&tx, &ws, &input.mutation_id.to_string(), &digest)? {
            MutationCheck::Replay(result) => {
                let mut outcome: TaskEditOutcome = serde_json::from_str(&result)
                    .map_err(|_| TaskServiceError::Db("result".into()))?;
                outcome.replayed = true;
                return Ok(outcome);
            }
            MutationCheck::Conflict => return Err(TaskServiceError::IdempotencyConflict),
            MutationCheck::Deleted => return Err(TaskServiceError::MutationResultDeleted),
            MutationCheck::Proceed => {}
        }
        let (mut task, mut steps) = load_task(&tx, &ws, input.task_id)?;
        if task.lifecycle == TaskLifecycle::Superseded {
            return Err(TaskServiceError::TaskReadOnly);
        }
        if input.expected_task_revision != task.task_revision_no {
            return Err(TaskServiceError::RevisionConflict);
        }
        super::tasks::validate_task_edit_with_raw(input, Some(input_json))?;
        let mut events: Vec<PlannedEvent> = Vec::new();
        match &input.operation {
            TaskEditOperation::Content {
                patch,
                step_operations,
                source_note,
            } => {
                let explicit_null = patch_metric_id_explicit_null(&input_json["operation"]);
                // 合并后类别/指标关系（§4.1）
                let merged_category = patch
                    .as_deref()
                    .and_then(|p| p.category)
                    .unwrap_or(task.category);
                let merged_metric = if patch.as_deref().and_then(|p| p.metric_id).is_some() {
                    patch.as_deref().and_then(|p| p.metric_id)
                } else if explicit_null {
                    None
                } else {
                    task.metric_id
                };
                super::tasks::validate_category_metric(merged_category, merged_metric)?;
                if patch
                    .as_deref()
                    .is_some_and(|patch| patch.metric_id.is_some())
                {
                    validate_metric_workspace(&tx, &ws, merged_metric)?;
                }
                apply_content_edit(
                    &mut task,
                    &mut steps,
                    patch.as_deref(),
                    step_operations.as_deref(),
                    explicit_null,
                    self.now,
                    &mut events,
                )?;
                for event in events
                    .iter_mut()
                    .filter(|event| event.event_type == "content_patched")
                {
                    let reason = [
                        source_note.as_deref(),
                        patch
                            .as_deref()
                            .and_then(|p| p.note.as_ref().and_then(|note| note.as_deref())),
                    ]
                    .into_iter()
                    .flatten()
                    .filter(|value| !value.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("；");
                    if !reason.is_empty() {
                        event.reason = Some(reason);
                    }
                    if let Some(source) = source_note
                        .as_deref()
                        .filter(|text| !text.trim().is_empty())
                    {
                        let mut after: serde_json::Value =
                            serde_json::from_str(event.after_json.as_deref().unwrap_or("{}"))
                                .map_err(|_| TaskServiceError::Db("任务修改记录无法读取".into()))?;
                        after["source_note"] = json!(source);
                        event.after_json = Some(after.to_string());
                    }
                    if let Some(note) = patch.as_deref().and_then(|p| p.note.as_ref()) {
                        let mut after: serde_json::Value =
                            serde_json::from_str(event.after_json.as_deref().unwrap_or("{}"))
                                .map_err(|_| TaskServiceError::Db("任务修改记录无法读取".into()))?;
                        after["note"] = json!(note);
                        event.after_json = Some(after.to_string());
                    }
                }
                // 目标期边界：步骤日期须在整体目标范围内（TS-39）
                check_step_dates_in_task_period(&task, &steps)?;
            }
            TaskEditOperation::Progress {
                progress_mode,
                progress,
                status,
                adjustment_reason,
                blocked_reason,
            } => {
                apply_progress_edit(
                    &mut task,
                    &steps,
                    *progress_mode,
                    progress.as_deref(),
                    *status,
                    adjustment_reason.as_deref(),
                    blocked_reason.as_deref(),
                    self.now,
                    &mut events,
                )?;
            }
            TaskEditOperation::PeriodLink {
                action,
                run_id,
                analysis_revision_no,
                cycle_role,
                source_period,
                target_period,
            } => {
                apply_period_link(
                    &tx,
                    &task,
                    *action,
                    run_id,
                    *analysis_revision_no,
                    cycle_role,
                    source_period,
                    target_period,
                    self.now,
                    &mut events,
                )?;
            }
        }
        task.task_revision_no += 1;
        save_task_state(&tx, &task, &steps, self.now)?;
        append_task_revision(&tx, &task, &steps, self.actor, input.mutation_id, self.now)?;
        append_events(
            &tx,
            task.task_id,
            task.task_revision_no,
            self.actor,
            input.mutation_id,
            &events,
            self.now,
        )?;
        let snapshot_runs = append_snapshots_for_task(&tx, task.task_id, self.now)?;
        let outcome = TaskEditOutcome {
            task_id: task.task_id.to_string(),
            task_revision_no: task.task_revision_no,
            status: serde_json::to_value(task.status)
                .expect("json")
                .as_str()
                .expect("snake_case")
                .to_string(),
            progress: to_canonical_string(task.progress),
            replayed: false,
        };
        let result_json = serde_json::to_string(&outcome).expect("json");
        let affected_runs: Vec<String> = snapshot_runs.iter().map(|(r, _, _)| r.clone()).collect();
        record_mutation(
            &tx,
            &ws,
            &input.mutation_id.to_string(),
            &digest,
            &result_json,
            &affected_runs,
            &[task.task_id.to_string()],
            self.now,
        )?;
        tx.commit().map_err(TaskServiceError::from)?;
        Ok(outcome)
    }
}

/// period_link 操作（attach/update/detach；run 须属同 workspace）。
#[allow(clippy::too_many_arguments)]
fn apply_period_link(
    tx: &Connection,
    task: &TaskRow,
    action: crate::contracts::tasks::PeriodLinkAction,
    run_id: &Uuid,
    analysis_revision_no: u32,
    cycle_role: &crate::contracts::tasks::CycleRole,
    source_period: &DateRange,
    target_period: &DateRange,
    now: DateTime<Utc>,
    events: &mut Vec<PlannedEvent>,
) -> Result<(), TaskServiceError> {
    let run_ws: Option<String> = tx
        .query_row(
            "SELECT workspace_id FROM analysis_runs WHERE run_id = ?1",
            params![run_id.to_string()],
            |r| r.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    if run_ws.as_deref() != Some(task.workspace_id.to_string().as_str()) {
        return Err(TaskServiceError::RunMismatch);
    }
    let role = serde_json::to_value(cycle_role).expect("json");
    let role = role.as_str().expect("snake_case");
    let event_type = match action {
        PeriodLinkAction::Attach => "period_link_attached",
        PeriodLinkAction::Update => "period_link_updated",
        PeriodLinkAction::Detach => "period_link_detached",
    };
    match action {
        PeriodLinkAction::Attach => {
            tx.execute(
                "INSERT OR IGNORE INTO task_period_links
                       (id, workspace_id, task_id, run_id, analysis_revision_no, cycle_role,
                        source_period_from, source_period_to, target_period_from, target_period_to,
                        inherited_from_link_id, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, ?11)",
                params![
                    Uuid::new_v4().to_string(),
                    task.workspace_id.to_string(),
                    task.task_id.to_string(),
                    run_id.to_string(),
                    analysis_revision_no as i64,
                    role,
                    date_str(source_period.from),
                    date_str(source_period.to),
                    date_str(target_period.from),
                    date_str(target_period.to),
                    utc_iso(now),
                ],
            )
            .map_err(TaskServiceError::from)?;
        }
        PeriodLinkAction::Update => {
            tx.execute(
                "UPDATE task_period_links SET
                        source_period_from=?3, source_period_to=?4,
                        target_period_from=?5, target_period_to=?6
                     WHERE task_id=?1 AND run_id=?2 AND analysis_revision_no=?7 AND cycle_role=?8",
                params![
                    task.task_id.to_string(),
                    run_id.to_string(),
                    date_str(source_period.from),
                    date_str(source_period.to),
                    date_str(target_period.from),
                    date_str(target_period.to),
                    analysis_revision_no as i64,
                    role,
                ],
            )
            .map_err(TaskServiceError::from)?;
        }
        PeriodLinkAction::Detach => {
            tx.execute(
                "DELETE FROM task_period_links
                     WHERE task_id=?1 AND run_id=?2 AND analysis_revision_no=?3 AND cycle_role=?4",
                params![
                    task.task_id.to_string(),
                    run_id.to_string(),
                    analysis_revision_no as i64,
                    role,
                ],
            )
            .map_err(TaskServiceError::from)?;
        }
    }
    events.push(PlannedEvent {
        event_type,
        before_json: None,
        after_json: Some(
            serde_json::to_string(&json!({
                "analysis_revision_no": analysis_revision_no,
                "cycle_role": role,
                "run_id": run_id.to_string(),
            }))
            .expect("json"),
        ),
        reason: None,
    });
    Ok(())
}

/// 已给任务整体边界时，步骤日期须在边界内（§4.2；TS-39）。
pub fn check_step_dates_in_task_period(
    task: &TaskRow,
    steps: &[StepRow],
) -> Result<(), TaskServiceError> {
    let Some(period) = task.target_period else {
        return Ok(());
    };
    for step in steps.iter().filter(|s| s.deleted_at.is_none()) {
        if let Some(d) = step.start_date {
            if d < period.from || d > period.to {
                return Err(TaskServiceError::StepOutsideTaskPeriod);
            }
        }
        if let Some(d) = step.due_date {
            if d < period.from || d > period.to {
                return Err(TaskServiceError::StepOutsideTaskPeriod);
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_metric_workspace(
    conn: &Connection,
    workspace_id: &str,
    metric_id: Option<Uuid>,
) -> Result<(), TaskServiceError> {
    if let Some(metric) = metric_id {
        let owned: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM metric_definitions WHERE workspace_id=?1 AND (metric_id=?2 OR stable_metric_id=?2))", params![workspace_id, metric.to_string()], |row| row.get(0))?;
        if !owned {
            return Err(TaskServiceError::TaskCategoryMetricMismatch);
        }
    }
    Ok(())
}

/// 旧表无 note 列，人工备注按事件保存；显式空字符串同样保留为清空操作。
pub fn current_task_note(
    conn: &Connection,
    task_id: Uuid,
) -> Result<Option<String>, TaskServiceError> {
    current_task_text(conn, task_id, "note")
}

/// 最近一次人工数值依据；只读、不把它当作来源已核实事实。
pub fn current_task_source_note(
    conn: &Connection,
    task_id: Uuid,
) -> Result<Option<String>, TaskServiceError> {
    current_task_text(conn, task_id, "source_note")
}

fn current_task_text(
    conn: &Connection,
    task_id: Uuid,
    field: &str,
) -> Result<Option<String>, TaskServiceError> {
    use rusqlite::OptionalExtension;
    let path = format!("$.{field}");
    let value: Option<Option<String>> = conn.query_row("SELECT json_extract(after_json,?2) FROM task_events WHERE task_id=?1 AND json_valid(after_json) AND json_type(after_json,?2) IN ('text','null') ORDER BY event_seq DESC LIMIT 1", params![task_id.to_string(),path], |row| row.get(0)).optional()?;
    Ok(value.flatten())
}

/// 创建完整任务（reconcile new/replace 与测试；revision 1 起步）。
#[allow(clippy::too_many_arguments)]
pub fn insert_full_task(
    conn: &Connection,
    task: &TaskRow,
    steps: &[StepRow],
    actor: ActorSource,
    mutation_id: Uuid,
    origin_run: Option<(&str, i64)>,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let category = serde_json::to_value(task.category).expect("json");
    let category = category.as_str().expect("snake_case");
    let status = serde_json::to_value(task.status).expect("json");
    let status = status.as_str().expect("snake_case");
    let lifecycle = serde_json::to_value(task.lifecycle).expect("json");
    let lifecycle = lifecycle.as_str().expect("snake_case");
    let progress_mode = serde_json::to_value(task.progress_mode).expect("json");
    let progress_mode = progress_mode.as_str().expect("snake_case");
    let smart_status = serde_json::to_value(task.smart_status).expect("json");
    let smart_status = smart_status.as_str().expect("snake_case");
    conn.execute(
        "INSERT INTO tasks
           (task_id, workspace_id, origin_run_id, origin_revision_no, task_revision_no, title,
            summary, category, metric_id, baseline_json, target_value, planned_value,
            actual_value, current_value, unit, mp_label, mp_unit, owner, planned_start,
            due_date, target_period_json, resource_need, success_criteria, assumptions_json,
            needs_review_json, smart_check_json, smart_status, status, lifecycle,
            superseded_by_task_id, progress, progress_mode, completed_at, review_note, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32,
                 ?33, ?34, ?35, ?35)",
        params![
            task.task_id.to_string(),
            task.workspace_id.to_string(),
            origin_run.map(|(r, _)| r.to_string()),
            origin_run.map(|(_, rev)| rev),
            task.task_revision_no as i64,
            task.title,
            task.summary,
            category,
            task.metric_id.map(|u| u.to_string()),
            task.baseline_json,
            task.target_value,
            task.planned_value,
            task.actual_value,
            task.current_value,
            task.unit,
            task.mp_label,
            task.mp_unit,
            task.owner,
            task.planned_start.map(date_str),
            task.due_date.map(date_str),
            period_json(task.target_period),
            task.resource_need,
            task.success_criteria,
            task.assumptions_json.clone().unwrap_or_else(|| "[]".into()),
            task.needs_review_json
                .clone()
                .unwrap_or_else(|| "[]".into()),
            task.smart_check_json.clone(),
            smart_status,
            status,
            lifecycle,
            task.superseded_by_task_id.map(|u| u.to_string()),
            to_canonical_string(task.progress),
            progress_mode,
            task.completed_at.map(utc_iso),
            task.review_note,
            utc_iso(now),
        ],
    )
    .map_err(TaskServiceError::from)?;
    for step in steps {
        let st = serde_json::to_value(step.status).expect("json");
        let st = st.as_str().expect("snake_case");
        conn.execute(
            "INSERT INTO task_steps
               (step_id, task_id, step_order, action, owner, start_date, due_date,
                done_criteria, status, blocked_reason, completed_at, deleted_at,
                created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)",
            params![
                step.step_id.to_string(),
                task.task_id.to_string(),
                step.step_order as i64,
                step.action,
                step.owner,
                step.start_date.map(date_str),
                step.due_date.map(date_str),
                step.done_criteria,
                st,
                step.blocked_reason,
                step.completed_at.map(utc_iso),
                step.deleted_at.map(utc_iso),
                utc_iso(now),
            ],
        )
        .map_err(TaskServiceError::from)?;
    }
    let events = vec![PlannedEvent {
        event_type: "task_created",
        before_json: None,
        after_json: Some(task_snapshot_full(task, steps)),
        reason: None,
    }];
    append_events(
        conn,
        task.task_id,
        task.task_revision_no,
        actor,
        mutation_id,
        &events,
        now,
    )?;
    append_task_revision(conn, task, steps, actor, mutation_id, now)?;
    Ok(())
}

/// 派生 overdue 的查询辅助（workspace 当地 today 由调用方给）。
pub fn task_overdue(
    conn: &Connection,
    workspace_id: &str,
    task_id: Uuid,
    workspace_today: chrono::NaiveDate,
) -> Result<bool, TaskServiceError> {
    let (task, _) = load_task(conn, workspace_id, task_id)?;
    Ok(super::tasks::derive_overdue(
        task.status,
        task.lifecycle,
        task.due_date,
        workspace_today,
    ))
}

/// 任务状态快照查询（列表/详情共用）。
pub fn task_status_view(
    conn: &Connection,
    workspace_id: &str,
    task_id: Uuid,
) -> Result<serde_json::Value, TaskServiceError> {
    let (task, steps) = load_task(conn, workspace_id, task_id)?;
    Ok(json!({
        "progress": to_canonical_string(task.progress),
        "status": serde_json::to_value(task.status).unwrap(),
        "steps": steps.iter().filter(|s| s.deleted_at.is_none()).map(|s| json!({
            "status": serde_json::to_value(s.status).unwrap(),
            "step_id": s.step_id.to_string(),
        })).collect::<Vec<_>>(),
        "task_revision_no": task.task_revision_no,
    }))
}

/// 事件类型列表（时间线）。
pub fn task_event_types(conn: &Connection, task_id: Uuid) -> Result<Vec<String>, TaskServiceError> {
    let mut stmt = conn
        .prepare("SELECT event_type FROM task_events WHERE task_id = ?1 ORDER BY event_seq")
        .map_err(TaskServiceError::from)?;
    let rows = stmt
        .query_map(params![task_id.to_string()], |r| r.get(0))
        .map_err(TaskServiceError::from)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row.map_err(TaskServiceError::from)?);
    }
    Ok(out)
}

/// 当前任务状态查询（TaskStatus）。
pub fn task_status(
    conn: &Connection,
    workspace_id: &str,
    task_id: Uuid,
) -> Result<TaskStatus, TaskServiceError> {
    let (task, _) = load_task(conn, workspace_id, task_id)?;
    Ok(task.status)
}
