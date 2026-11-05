//! read-views.schema.json 视图构造（字段名/空值语义逐一对齐冻结合同）。
//!
//! - decimal 全部字符串（`to_canonical_string`）；日期 `YYYY-MM-DD`；
//!   时间戳 RFC3339 毫秒 UTC。
//! - `overdue` 派生：当地 today > due && status != completed && lifecycle=active
//!   （当地 today 经 `analysis::periods::local_calendar_date`）。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::decimal::to_canonical_string;
use dsheng_core::contracts::domain::DateRange;
use dsheng_core::contracts::tasks::ActorSource;
use dsheng_core::repository::task_store::load_task;
use dsheng_core::repository::tasks::{derive_overdue, StepRow, TaskRow};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn opt_date(d: Option<chrono::NaiveDate>) -> Value {
    match d {
        Some(d) => json!(date_str(d)),
        None => json!(null),
    }
}

fn opt_dt(d: Option<chrono::DateTime<chrono::Utc>>) -> Value {
    match d {
        Some(d) => json!(dsheng_core::repository::utc_iso(d)),
        None => json!(null),
    }
}

fn dec(v: &Option<String>) -> Value {
    match v {
        Some(raw) => match dsheng_core::contracts::decimal::canonical_decimal(raw) {
            Ok(d) => json!(to_canonical_string(d)),
            Err(_) => json!(raw.as_str()),
        },
        None => json!(null),
    }
}

fn range_json(r: Option<DateRange>) -> Value {
    match r {
        Some(r) => json!({"from": date_str(r.from), "to": date_str(r.to)}),
        None => json!(null),
    }
}

/// workspace 当地今日（当前策略时区；失败兜底 Asia/Shanghai）。
pub fn workspace_today(conn: &Connection, workspace_id: Uuid) -> chrono::NaiveDate {
    let tz: String = conn
        .query_row(
            "SELECT COALESCE(p.timezone_id, 'Asia/Shanghai') FROM workspaces w
             LEFT JOIN period_policies p ON p.period_policy_id = w.current_period_policy_id
             WHERE w.workspace_id = ?1",
            params![workspace_id.to_string()],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| "Asia/Shanghai".into());
    let now = chrono::Utc::now();
    dsheng_core::analysis::periods::local_calendar_date(now, &tz)
        .unwrap_or_else(|_| now.date_naive())
}

fn smart_check_view(task: &TaskRow) -> Value {
    if let Some(raw) = &task.smart_check_json {
        if let Ok(v) = serde_json::from_str::<Value>(raw) {
            return v;
        }
    }
    json!({
        "smart_status": serde_json::to_value(task.smart_status).unwrap_or(json!("stale")),
        "assessed_task_revision": task.task_revision_no,
        "checks": [],
    })
}

fn steps_view(steps: &[StepRow]) -> Vec<Value> {
    steps
        .iter()
        .filter(|s| s.deleted_at.is_none())
        .enumerate()
        .map(|(idx, s)| {
            json!({
                "step_id": s.step_id.to_string(),
                "step_order": idx + 1,
                "action": s.action,
                "owner": s.owner,
                "start_date": opt_date(s.start_date),
                "due_date": opt_date(s.due_date),
                "done_criteria": s.done_criteria,
                "status": serde_json::to_value(s.status).unwrap_or(json!("not_started")),
                "blocked_reason": s.blocked_reason,
                "completed_at": opt_dt(s.completed_at),
            })
        })
        .collect()
}

fn events_view(conn: &Connection, task_id: Uuid) -> CliResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT event_id, task_revision_no, event_seq, event_type, reason, actor_source, occurred_at
         FROM task_events WHERE task_id = ?1 ORDER BY event_seq",
    )?;
    let rows = stmt.query_map(params![task_id.to_string()], |r| {
        Ok(json!({
            "event_id": r.get::<_, String>(0)?,
            "task_id": task_id.to_string(),
            "task_revision_no": r.get::<_, i64>(1)?,
            "event_seq": r.get::<_, i64>(2)?,
            "kind": r.get::<_, String>(3)?,
            "reason": r.get::<_, Option<String>>(4)?,
            "actor_source": r.get::<_, String>(5)?,
            "occurred_at": r.get::<_, String>(6)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

fn evidence_view(conn: &Connection, task_id: Uuid) -> CliResult<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT evidence_id, source_id, analysis_revision_no, locator, summary_json
         FROM task_evidence WHERE task_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map(params![task_id.to_string()], |r| {
        let source_id: String = r.get(1)?;
        let summary: String = r.get(4)?;
        let label = serde_json::from_str::<Value>(&summary)
            .ok()
            .and_then(|v| {
                v.get("file_label")
                    .and_then(|l| l.as_str())
                    .map(str::to_string)
            })
            .unwrap_or(source_id);
        Ok(json!({
            "evidence_id": r.get::<_, String>(0)?,
            "source_label": label,
            "locator": r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            "analysis_revision_no": r.get::<_, i64>(2)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// taskView（read-views.taskView 全字段；origin_run_id 由 tasks 表直接读取）。
pub fn task_view(conn: &Connection, workspace_id: Uuid, task_id: Uuid) -> CliResult<Value> {
    let (task, steps) =
        load_task(conn, &workspace_id.to_string(), task_id).map_err(CliError::from)?;
    let today = workspace_today(conn, workspace_id);
    let metric_label: Option<String> = task
        .metric_id
        .map(|mid| {
            conn.query_row(
                "SELECT label FROM metric_definitions WHERE metric_id = ?1",
                params![mid.to_string()],
                |r| r.get(0),
            )
        })
        .transpose()
        .map_err(CliError::from)?;
    let origin_run: Option<String> = conn
        .query_row(
            "SELECT origin_run_id FROM tasks WHERE task_id = ?1",
            params![task_id.to_string()],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    Ok(json!({
        "workspace_id": workspace_id.to_string(),
        "task_id": task.task_id.to_string(),
        "task_revision_no": task.task_revision_no,
        "title": task.title,
        "summary": task.summary,
        "category": serde_json::to_value(task.category).unwrap_or(json!("other")),
        "metric_id": task.metric_id.map(|u| u.to_string()),
        "metric_label": metric_label,
        "target_value": dec(&task.target_value),
        "planned_value": dec(&task.planned_value),
        "actual_value": dec(&task.actual_value),
        "current_value": dec(&task.current_value),
        "unit": task.unit,
        "mp_label": task.mp_label,
        "mp_unit": task.mp_unit,
        "owner": task.owner,
        "planned_start": opt_date(task.planned_start),
        "due_date": opt_date(task.due_date),
        "resource_need": task.resource_need,
        "success_criteria": task.success_criteria,
        "note": Value::Null,
        "review_note": task.review_note,
        "target_period": range_json(task.target_period),
        "lifecycle": serde_json::to_value(task.lifecycle).unwrap_or(json!("active")),
        "status": serde_json::to_value(task.status).unwrap_or(json!("not_started")),
        "progress": to_canonical_string(task.progress),
        "progress_mode": serde_json::to_value(task.progress_mode).unwrap_or(json!("auto")),
        "adjustment_reason": task.adjustment_reason,
        "blocked_reason": task.blocked_reason,
        "completed_at": opt_dt(task.completed_at),
        "smart_check": smart_check_view(&task),
        "steps": steps_view(&steps),
        "evidence": evidence_view(conn, task_id)?,
        "events": events_view(conn, task_id)?,
        "origin_run_id": origin_run,
        "overdue": derive_overdue(task.status, task.lifecycle, task.due_date, today),
    }))
}

/// task list 查询（可选 run 过滤，经 task_period_links）。
pub fn task_ids(conn: &Connection, workspace_id: Uuid, run: Option<&str>) -> CliResult<Vec<Uuid>> {
    let ids: Vec<String> = match run {
        Some(run_id) => {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT t.task_id FROM tasks t
                 JOIN task_period_links l ON l.task_id = t.task_id
                 WHERE t.workspace_id = ?1 AND l.run_id = ?2
                 ORDER BY t.created_at",
            )?;
            let collected: Vec<String> = stmt
                .query_map(params![workspace_id.to_string(), run_id], |r| r.get(0))?
                .collect::<Result<_, _>>()
                .map_err(CliError::from)?;
            collected
        }
        None => {
            let mut stmt = conn
                .prepare("SELECT task_id FROM tasks WHERE workspace_id = ?1 ORDER BY created_at")?;
            let collected: Vec<String> = stmt
                .query_map(params![workspace_id.to_string()], |r| r.get(0))?
                .collect::<Result<_, _>>()
                .map_err(CliError::from)?;
            collected
        }
    };
    ids.into_iter()
        .map(|raw| {
            Uuid::parse_str(&raw).map_err(|_| {
                CliError::new(
                    dsheng_core::contracts::cli::CliErrorCode::LocalSaveFailed,
                    "task_id 非法",
                )
            })
        })
        .collect()
}

/// timelineEntryView（planned_start/due/completed/progress_event；date null 不参与排序）。
pub fn timeline_entries(
    conn: &Connection,
    workspace_id: Uuid,
    range: DateRange,
) -> CliResult<Vec<Value>> {
    let ids = task_ids(conn, workspace_id, None)?;
    let today = workspace_today(conn, workspace_id);
    let mut out = Vec::new();
    for task_id in ids {
        let (task, _) =
            load_task(conn, &workspace_id.to_string(), task_id).map_err(CliError::from)?;
        let title = task.title.clone();
        for (kind, date) in [
            ("planned_start", task.planned_start),
            ("due", task.due_date),
        ] {
            if let Some(d) = date {
                if d >= range.from && d <= range.to {
                    let detail = if kind == "due"
                        && derive_overdue(task.status, task.lifecycle, task.due_date, today)
                    {
                        "已逾期"
                    } else if kind == "due" {
                        "截止日"
                    } else {
                        "计划开始"
                    };
                    out.push(json!({
                        "task_id": task_id.to_string(),
                        "task_title": title,
                        "kind": kind,
                        "date": date_str(d),
                        "occurred_at": dsheng_core::repository::utc_iso(
                            d.and_hms_opt(0, 0, 0).unwrap().and_utc()),
                        "detail": detail,
                    }));
                }
            }
        }
        let mut stmt = conn.prepare(
            "SELECT event_type, occurred_at, reason FROM task_events
             WHERE task_id = ?1 AND (event_type LIKE '%completed%' OR event_type LIKE '%progress%'
                  OR event_type LIKE '%status%')
             ORDER BY event_seq",
        )?;
        let rows = stmt.query_map(params![task_id.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (event_type, occurred_at, reason) = row?;
            let ts = chrono::DateTime::parse_from_rfc3339(&occurred_at)
                .map(|t| t.with_timezone(&chrono::Utc))
                .ok();
            let Some(local_date) = ts.and_then(|t| {
                dsheng_core::analysis::periods::local_calendar_date(t, "Asia/Shanghai").ok()
            }) else {
                continue;
            };
            if local_date < range.from || local_date > range.to {
                continue;
            }
            let kind = if event_type.contains("completed") {
                "completed"
            } else {
                "progress_event"
            };
            out.push(json!({
                "task_id": task_id.to_string(),
                "task_title": title,
                "kind": kind,
                "date": date_str(local_date),
                "occurred_at": occurred_at,
                "detail": reason.unwrap_or_else(|| event_type.clone()),
            }));
        }
    }
    out.sort_by(|a, b| {
        let ka = (
            a["date"].as_str().unwrap_or("9999-12-31"),
            a["task_id"].as_str().unwrap_or(""),
        );
        let kb = (
            b["date"].as_str().unwrap_or("9999-12-31"),
            b["task_id"].as_str().unwrap_or(""),
        );
        ka.cmp(&kb)
    });
    Ok(out)
}

/// actor 来源（TaskEditService/insert_full_task 用）。
pub fn cli_actor() -> ActorSource {
    ActorSource::Cli
}
