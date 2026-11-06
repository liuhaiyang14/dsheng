//! 已应用任务的当前执行记录 → 任务生成的历史检视输入（只读）。
//!
//! 检视窗口使用本次分析的来源期间：包含该期间任务及更早未完成事项，
//! 不把为下一期刚生成的候选/任务当作本期执行结果，也不冒充期末历史快照。

use crate::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::decimal::to_canonical_string;
use dsheng_core::contracts::domain::DateRange;
use dsheng_core::contracts::tasks::TaskStatus;
use dsheng_core::generation::input::{PriorTaskOutcome, TaskOutcomeStatus};
use dsheng_core::repository::task_store::load_task;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Number;
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

/// 读取同企业已应用任务；证据仅保留本次模型输入确实携带的 ID。
pub fn prior_task_outcomes(
    conn: &Connection,
    workspace_id: Uuid,
    source_period: DateRange,
    allowed_evidence_ids: &HashSet<String>,
) -> CliResult<Vec<PriorTaskOutcome>> {
    if source_period.from > source_period.to {
        return Err(CliError::invalid("任务检视期间无效"));
    }
    let mut stmt = conn.prepare(
        "SELECT task_id, target_period_json, status FROM tasks
         WHERE workspace_id = ?1 AND lifecycle = 'active'
         ORDER BY created_at, task_id",
    )?;
    let rows = stmt.query_map([workspace_id.to_string()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut outcomes = Vec::new();
    for row in rows {
        let (task_id, raw_period, status) = row?;
        let period = match raw_period {
            Some(raw) => Some(
                serde_json::from_str::<DateRange>(&raw)
                    .map_err(|_| invalid_history("任务目标期间格式无效"))?,
            ),
            // 老记录可能仅在关联表保存明确期间；以最近的目标期间为准，
            // 不把多条 plan/review 关联重复计为多项任务。
            None => linked_target_period(conn, workspace_id, &task_id)?,
        };
        let Some(period) = period else { continue };
        if period.from > period.to {
            return Err(invalid_history("任务目标期间起止顺序无效"));
        }
        if period.from > source_period.to
            || (period.to < source_period.from && status == "completed")
        {
            continue;
        }
        let id = Uuid::parse_str(&task_id).map_err(|_| invalid_history("历史任务 ID 无效"))?;
        let (task, steps) = load_task(conn, &workspace_id.to_string(), id)?;
        let status = match task.status {
            TaskStatus::NotStarted => TaskOutcomeStatus::NotStarted,
            TaskStatus::InProgress => TaskOutcomeStatus::InProgress,
            TaskStatus::Completed => TaskOutcomeStatus::Completed,
            TaskStatus::Blocked => TaskOutcomeStatus::Blocked,
        };
        let mut note = vec![format!(
            "本地已应用任务的当前执行记录（任务版本 r{}），非期末快照；执行状态不等于经营成效已核实。目标期间：{} 至 {}。",
            task.task_revision_no, period.from, period.to
        )];
        if period.to < source_period.from {
            note.push("较早期间遗留的未完成事项。".into());
        }
        push_note(&mut note, "任务说明", Some(&task.summary));
        push_note(&mut note, "责任人记录", task.owner.as_deref());
        push_note(&mut note, "目标值记录", task.target_value.as_deref());
        push_note(&mut note, "计划值记录", task.planned_value.as_deref());
        push_note(&mut note, "实绩值记录", task.actual_value.as_deref());
        push_note(&mut note, "当前值记录", task.current_value.as_deref());
        push_note(&mut note, "数值单位", task.unit.as_deref());
        push_note(&mut note, "复盘记录", task.review_note.as_deref());
        push_note(&mut note, "阻塞原因记录", task.blocked_reason.as_deref());
        push_note(
            &mut note,
            "进度调整依据记录",
            task.adjustment_reason.as_deref(),
        );
        if let Some(completed_at) = task.completed_at {
            note.push(format!("记录完成时间：{}。", completed_at.to_rfc3339()));
        }
        for step in steps.iter().filter(|step| step.deleted_at.is_none()) {
            note.push(format!(
                "步骤 {}：{}；记录状态：{}。",
                step.step_order,
                step.action,
                status_label(step.status)
            ));
            push_note(&mut note, "步骤责任人记录", step.owner.as_deref());
            push_note(
                &mut note,
                "步骤阻塞原因记录",
                step.blocked_reason.as_deref(),
            );
            push_note(&mut note, "步骤验收标准", step.done_criteria.as_deref());
            if let Some(completed_at) = step.completed_at {
                note.push(format!("步骤记录完成时间：{}。", completed_at.to_rfc3339()));
            }
        }
        let mut evidence = conn.prepare(
            "SELECT evidence_id FROM task_evidence WHERE task_id = ?1
             UNION SELECT e.evidence_id FROM task_step_evidence e
             JOIN task_steps s ON s.step_id = e.step_id
             WHERE s.task_id = ?1 AND s.deleted_at IS NULL",
        )?;
        let ids = evidence.query_map([&task_id], |row| row.get::<_, String>(0))?;
        let mut evidence_ids = BTreeSet::new();
        for id in ids {
            let id = id?;
            if allowed_evidence_ids.contains(&id) {
                evidence_ids.insert(id);
            }
        }
        let progress = to_canonical_string(task.progress)
            .parse::<Number>()
            .map_err(|_| invalid_history("任务进度格式无效"))?;
        outcomes.push(PriorTaskOutcome {
            task_id,
            title: task.title,
            status,
            progress,
            outcome_note: Some(note.join("\n")),
            evidence_ids: evidence_ids.into_iter().collect(),
        });
    }
    Ok(outcomes)
}

fn linked_target_period(
    conn: &Connection,
    workspace_id: Uuid,
    task_id: &str,
) -> CliResult<Option<DateRange>> {
    let range: Option<(String, String)> = conn
        .query_row(
            "SELECT target_period_from, target_period_to FROM task_period_links
             WHERE workspace_id = ?1 AND task_id = ?2
             ORDER BY target_period_to DESC, target_period_from DESC LIMIT 1",
            params![workspace_id.to_string(), task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    range
        .map(|(from, to)| {
            Ok(DateRange {
                from: from
                    .parse()
                    .map_err(|_| invalid_history("任务关联起始日无效"))?,
                to: to
                    .parse()
                    .map_err(|_| invalid_history("任务关联结束日无效"))?,
            })
        })
        .transpose()
}

fn push_note(note: &mut Vec<String>, label: &str, text: Option<&str>) {
    if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
        note.push(format!("{label}：{text}"));
    }
}

fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::NotStarted => "待开始",
        TaskStatus::InProgress => "进行中",
        TaskStatus::Completed => "已完成",
        TaskStatus::Blocked => "有阻塞",
    }
}

fn invalid_history(message: &str) -> CliError {
    CliError::new(CliErrorCode::DataValidationFailed, message)
}
