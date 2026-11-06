//! read-views.schema.json 视图构造（字段名/空值语义逐一对齐冻结合同）。
//!
//! - decimal 全部字符串（`to_canonical_string`）；日期 `YYYY-MM-DD`；
//!   时间戳 RFC3339 毫秒 UTC。
//! - `overdue` 派生：当地 today > due && status != completed && lifecycle=active
//!   （当地 today 经 `analysis::periods::local_calendar_date`）。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::decimal::to_canonical_string;
use dsheng_core::contracts::domain::DateRange;
use dsheng_core::contracts::tasks::{ActorSource, SmartStatus};
use dsheng_core::generation::task_candidate::{
    CandidateState, TaskCandidateView, TaskCandidateViewEntry, TaskCandidateViewStep,
};
use dsheng_core::repository::task_store::load_task;
use dsheng_core::repository::tasks::{
    derive_overdue, normalize_candidate_output, StepRow, TaskRow,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

/// 已保存的模型候选 → 冻结读视图；与 reconcile 使用同一规范化规则。
/// CLI 的生成成功收据不含 tasks，不能直接交给桌面候选列表。
pub fn task_candidate_view(
    conn: &Connection,
    workspace_id: Uuid,
    candidate_id: &str,
) -> CliResult<TaskCandidateView> {
    let candidate_generation_id = Uuid::parse_str(candidate_id)
        .map_err(|_| CliError::invalid("candidate_generation_id 非法"))?;
    let saved = conn
        .query_row(
            "SELECT candidate_revision_no, run_id, analysis_revision_no, state,
                envelope_locally_reconstructed, output_json
         FROM task_candidates WHERE candidate_generation_id = ?1 AND workspace_id = ?2",
            params![candidate_id, workspace_id.to_string()],
            |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?
        .ok_or_else(|| {
            CliError::new(
                CliErrorCode::WorkspaceMismatch,
                "候选不存在或不属于当前企业",
            )
        })?;
    let invalid = || {
        CliError::new(
            CliErrorCode::DataValidationFailed,
            "已保存的任务候选格式无效，无法显示；请勿重复生成",
        )
    };
    let raw: Value = serde_json::from_str(&saved.5).map_err(|_| invalid())?;
    let tasks = normalize_candidate_output(&raw).map_err(|_| invalid())?;
    let state: CandidateState = serde_json::from_value(json!(saved.3)).map_err(|_| invalid())?;
    Ok(TaskCandidateView {
        candidate_generation_id,
        candidate_revision_no: saved.0,
        run_id: saved.1,
        analysis_revision_no: saved.2,
        state,
        envelope_locally_reconstructed: saved.4,
        tasks: tasks
            .into_iter()
            .map(|task| TaskCandidateViewEntry {
                task_key: task.task_key,
                title: task.title,
                summary: task.summary,
                category: task.category,
                target_value: task.target_value,
                owner: task.owner,
                steps: task
                    .steps
                    .into_iter()
                    .map(|step| TaskCandidateViewStep {
                        step_id: step.step_id,
                        action: step.action,
                    })
                    .collect(),
            })
            .collect(),
    })
}

/// 只恢复当前分析版本仍可应用的最后一份候选，不发送模型请求、不改写候选。
/// 指纹作为恢复元数据放在 wrapper 中，不扩展冻结的 taskCandidateView。
pub fn latest_task_candidate(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision_no: u32,
) -> CliResult<Value> {
    let tx = conn.unchecked_transaction()?;
    let (current_revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        &tx,
        &workspace_id.to_string(),
        run_id,
    )?;
    if current_revision != revision_no {
        return Err(CliError::new(
            CliErrorCode::InputRevisionChanged,
            "所选分析版本已变化，请刷新当前期间后读取已生成结果",
        ));
    }
    let candidate_id: Option<String> = tx
        .query_row(
            "SELECT candidate_generation_id FROM task_candidates
         WHERE workspace_id = ?1 AND run_id = ?2 AND analysis_revision_no = ?3
           AND analysis_input_fingerprint = ?4 AND state = 'candidate'
         ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![workspace_id.to_string(), run_id, revision_no, fingerprint],
            |row| row.get(0),
        )
        .optional()?;
    let data = match candidate_id {
        Some(id) => json!({
            "candidate": task_candidate_view(&tx, workspace_id, &id)?,
            "analysis_input_fingerprint": fingerprint,
        }),
        None => Value::Null,
    };
    tx.commit()?;
    Ok(data)
}

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
    project_smart_check(
        task.smart_check_json.as_deref(),
        task.smart_status,
        task.task_revision_no,
    )
}

/// 持久化的是模型输出合同，不能直接当作桌面读视图。
/// SMART 在创建任务时绑定 revision 1，后续编辑只标 stale，不重算评价。
fn project_smart_check(raw: Option<&str>, smart_status: SmartStatus, revision: u32) -> Value {
    let saved = raw.and_then(|raw| serde_json::from_str::<Value>(raw).ok());
    let saved = saved.as_ref().unwrap_or(&Value::Null);
    let mut checks = Vec::new();
    let mut assessed_revision = 1;
    if let Some(legacy_checks) = saved["checks"].as_array() {
        // 兼容历史已投影数据；读视图外的字段不透传，stale 以任务当前态为准。
        assessed_revision = saved["assessed_task_revision"]
            .as_u64()
            .filter(|n| *n >= 1 && *n <= u64::from(revision))
            .unwrap_or(1);
        for check in legacy_checks {
            let Some(dimension @ ("S" | "M" | "A" | "R" | "T")) = check["dimension"].as_str()
            else {
                continue;
            };
            let Some(status) = smart_criterion_status(&check["status"]) else {
                continue;
            };
            checks.push(json!({
                "dimension": dimension,
                "status": status,
                "note": check["note"].as_str(),
            }));
        }
    } else if saved["overall_goal"]["criteria"].is_object() {
        // 每个总目标/子目标的每个维度独立投影，保留原评价；不合并或重算判断。
        let overall_label = match saved["overall_goal"]["goal_text"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            Some(text) => format!("总目标（{text}）"),
            None => "总目标".into(),
        };
        let mut goals = vec![(overall_label, &saved["overall_goal"]["criteria"])];
        if let Some(subgoals) = saved["subgoals"].as_array() {
            goals.extend(subgoals.iter().enumerate().map(|(index, subgoal)| {
                let label = match subgoal["goal_text"].as_str().filter(|s| !s.is_empty()) {
                    Some(text) => format!("子目标 {}（{}）", index + 1, text),
                    None => format!("子目标 {}", index + 1),
                };
                (label, &subgoal["criteria"])
            }));
        }
        for (label, criteria) in goals {
            for dimension in ["S", "M", "A", "R", "T"] {
                let criterion = &criteria[dimension];
                let Some(status) = smart_criterion_status(&criterion["status"]) else {
                    continue;
                };
                let note = match criterion["rationale"].as_str().filter(|s| !s.is_empty()) {
                    Some(reason) => format!("{label}：{reason}"),
                    None => label.clone(),
                };
                checks.push(json!({
                    "dimension": dimension,
                    "status": status,
                    "note": note,
                }));
            }
        }
    }
    json!({
        "smart_status": smart_status,
        "assessed_task_revision": assessed_revision,
        "checks": checks,
    })
}

fn smart_criterion_status(value: &Value) -> Option<&str> {
    match value.as_str()? {
        status @ ("met" | "needs_confirmation" | "not_applicable") => Some(status),
        _ => None,
    }
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
        "note": dsheng_core::repository::task_store::current_task_note(conn, task_id)?,
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

#[cfg(test)]
mod smart_check_tests {
    use super::*;

    fn criteria(status: &str) -> Value {
        ["S", "M", "A", "R", "T"]
            .into_iter()
            .map(|dimension| {
                (
                    dimension.to_string(),
                    json!({
                        "status": status,
                        "rationale": format!("{dimension} 原始评价"),
                        "evidence_ids": ["synthetic-evidence"],
                    }),
                )
            })
            .collect()
    }

    #[test]
    fn model_smart_goals_project_each_original_criterion_without_reassessment() {
        let raw = json!({
            "overall_goal": {"goal_text": "整体目标", "criteria": criteria("met")},
            "subgoals": [
                {"subgoal_id": "one", "goal_text": "确认依据", "criteria": criteria("needs_confirmation")},
                {"subgoal_id": "two", "goal_text": "补充步骤", "criteria": criteria("not_applicable")},
            ],
        }).to_string();
        let view = project_smart_check(Some(&raw), SmartStatus::Generated, 4);
        assert_eq!(view["smart_status"], "generated");
        assert_eq!(view["assessed_task_revision"], 1);
        let checks = view["checks"].as_array().unwrap();
        assert_eq!(checks.len(), 15);
        for (goal_index, status) in ["met", "needs_confirmation", "not_applicable"]
            .into_iter()
            .enumerate()
        {
            for (dimension_index, dimension) in ["S", "M", "A", "R", "T"].into_iter().enumerate() {
                let check = &checks[goal_index * 5 + dimension_index];
                assert_eq!(check["dimension"], dimension);
                assert_eq!(check["status"], status);
                assert!(check["note"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("{dimension} 原始评价")));
                assert_eq!(check.as_object().unwrap().len(), 3);
            }
        }
        assert_eq!(checks[0]["note"], "总目标（整体目标）：S 原始评价");
        assert_eq!(checks[5]["note"], "子目标 1（确认依据）：S 原始评价");
        assert_eq!(checks[10]["note"], "子目标 2（补充步骤）：S 原始评价");
        assert_eq!(view.as_object().unwrap().len(), 3);
    }

    #[test]
    fn model_smart_null_rationale_is_valid_and_content_edits_keep_original_revision() {
        let mut raw = json!({
            "overall_goal": {"goal_text": "目标", "criteria": criteria("not_applicable")},
            "subgoals": [],
        });
        raw["overall_goal"]["criteria"]["S"]["rationale"] = Value::Null;
        let view = project_smart_check(Some(&raw.to_string()), SmartStatus::Stale, 8);
        assert_eq!(view["smart_status"], "stale");
        assert_eq!(view["assessed_task_revision"], 1);
        assert_eq!(view["checks"][0]["status"], "not_applicable");
        assert_eq!(view["checks"][0]["note"], "总目标（目标）");
    }

    #[test]
    fn historical_read_view_preserves_repeated_dimensions_and_actual_task_status() {
        let raw = json!({
            "smart_status": "generated", "assessed_task_revision": 3,
            "checks": [
                {"dimension": "S", "status": "met", "note": "总目标"},
                {"dimension": "S", "status": "needs_confirmation", "note": "子目标"},
                {"dimension": "A", "status": "not_applicable", "note": null},
            ],
        });
        let view = project_smart_check(Some(&raw.to_string()), SmartStatus::Stale, 5);
        assert_eq!(view["checks"], raw["checks"]);
        assert_eq!(view["smart_status"], "stale");
        assert_eq!(view["assessed_task_revision"], 3);
    }

    #[test]
    fn absent_or_unrecognized_smart_data_never_invents_passed_checks() {
        for raw in [
            None,
            Some("null"),
            Some("{}"),
            Some("invalid json"),
            Some("[]"),
        ] {
            let view = project_smart_check(raw, SmartStatus::Stale, 3);
            assert_eq!(view["smart_status"], "stale");
            assert_eq!(view["checks"], json!([]));
        }
        let raw = json!({
            "assessed_task_revision": 99,
            "checks": [
                {"dimension": "S", "pass": true, "note": "not a contract status"},
                {"dimension": "unknown", "status": "met", "note": null},
            ],
        });
        let view = project_smart_check(Some(&raw.to_string()), SmartStatus::Stale, 3);
        assert_eq!(view["checks"], json!([]));
        assert_eq!(view["assessed_task_revision"], 1);
    }
}
