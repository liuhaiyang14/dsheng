//! 人工创建任务：同编辑规则、同本地事务，不经过候选生成或 Provider。
use super::task_store::{
    append_task_snapshot, check_mutation, check_step_dates_in_task_period, insert_full_task,
    record_mutation, MutationCheck, TaskEditOutcome,
};
use super::tasks::{
    apply_content_edit, mutation_digest, validate_category_metric, validate_task_edit_with_raw,
    TaskRow, TaskServiceError,
};
use crate::contracts::domain::DateRange;
use crate::contracts::tasks::{
    ActorSource, CycleRole, ProgressMode, SmartStatus, StepOperation, TaskCreateInput,
    TaskEditInput, TaskEditOperation, TaskLifecycle, TaskStatus,
};
use crate::repository::utc_iso;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

pub struct TaskCreateService<'a> {
    pub conn: &'a mut Connection,
    pub now: DateTime<Utc>,
    pub actor: ActorSource,
}

impl TaskCreateService<'_> {
    pub fn apply_raw(&mut self, raw: &Value) -> Result<TaskEditOutcome, TaskServiceError> {
        let mut input: TaskCreateInput = serde_json::from_value(raw.clone())
            .map_err(|e| invalid(format!("人工任务输入无效：{e}")))?;
        if input.schema_version != "1"
            || input.analysis_revision_no == 0
            || raw["content"].get("target_period").is_some()
        {
            return Err(invalid("人工任务期间必须使用所选分析"));
        }
        // 新任务中 null 数值表示未填写，不是清空既有数值，不需要虚构依据。
        for value in [
            &mut input.content.target_value,
            &mut input.content.planned_value,
            &mut input.content.actual_value,
            &mut input.content.current_value,
        ] {
            if matches!(value, Some(None)) {
                *value = None;
            }
        }
        let title = input
            .content
            .title
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| invalid("请填写任务名称"))?;
        let category = input
            .content
            .category
            .ok_or_else(|| invalid("请选择任务类别"))?;
        if input.steps.is_empty() {
            return Err(TaskServiceError::AtLeastOneStep);
        }
        for step in &input.steps {
            if step.action.trim().is_empty() {
                return Err(invalid("请填写每项行动内容"));
            }
            if matches!((step.start_date,step.due_date),(Some(start),Some(end)) if start>end) {
                return Err(TaskServiceError::DateOrderInvalid);
            }
        }
        let digest = mutation_digest(raw);
        let ws = input.workspace_id.to_string();
        let run = input.run_id.to_string();
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        match check_mutation(&tx, &ws, &input.mutation_id.to_string(), &digest)? {
            MutationCheck::Replay(value) => {
                let mut result: TaskEditOutcome = serde_json::from_str(&value)
                    .map_err(|_| invalid("人工任务幂等结果无法读取"))?;
                result.replayed = true;
                return Ok(result);
            }
            MutationCheck::Conflict => return Err(TaskServiceError::IdempotencyConflict),
            MutationCheck::Deleted => return Err(TaskServiceError::MutationResultDeleted),
            MutationCheck::Proceed => {}
        }
        let binding: Option<(String,u32,String)> = tx.query_row("SELECT workspace_id,current_revision_no,analysis_input_fingerprint FROM analysis_runs WHERE run_id=?1",[&run],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        let (owner, revision, fingerprint) = binding.ok_or(TaskServiceError::RunMismatch)?;
        if owner != ws {
            return Err(TaskServiceError::WorkspaceMismatch);
        }
        if revision != input.analysis_revision_no || fingerprint != input.input_fingerprint {
            return Err(TaskServiceError::InputRevisionChanged);
        }
        let snapshot_fp:String = tx.query_row("SELECT analysis_input_fingerprint FROM run_revisions WHERE run_id=?1 AND revision_no=?2",params![run,revision],|row| row.get(0))?;
        if snapshot_fp != fingerprint {
            return Err(TaskServiceError::InputRevisionChanged);
        }
        let (source, plan) = saved_task_periods(&tx, &run, revision)?;
        let target = match input.cycle_role {
            CycleRole::Review => source,
            CycleRole::Plan => plan,
        };
        validate_category_metric(category, input.content.metric_id)?;
        super::task_store::validate_metric_workspace(&tx, &ws, input.content.metric_id)?;
        let mut task = TaskRow {
            task_id: Uuid::new_v4(),
            workspace_id: input.workspace_id,
            task_revision_no: 1,
            title: title.to_string(),
            summary: String::new(),
            category,
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
            target_period: Some(target),
            resource_need: None,
            success_criteria: None,
            assumptions_json: Some("[]".into()),
            needs_review_json: Some("[]".into()),
            smart_check_json: None,
            status: TaskStatus::NotStarted,
            lifecycle: TaskLifecycle::Active,
            superseded_by_task_id: None,
            progress: rust_decimal::Decimal::ZERO,
            progress_mode: ProgressMode::Auto,
            adjustment_reason: None,
            blocked_reason: None,
            smart_status: SmartStatus::Stale,
            review_note: None,
            completed_at: None,
        };
        let ops: Vec<StepOperation> = input
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| StepOperation::Add {
                client_step_key: format!("manual-{index}"),
                content: step.clone(),
                before_step_id: None,
            })
            .collect();
        let edit = TaskEditInput {
            schema_version: "1".into(),
            workspace_id: input.workspace_id,
            task_id: task.task_id,
            expected_task_revision: 1,
            mutation_id: input.mutation_id,
            operation: TaskEditOperation::Content {
                patch: Some(Box::new(input.content.clone())),
                step_operations: Some(ops.clone()),
                source_note: input.source_note.clone(),
            },
        };
        let edit_raw = json!({"operation":{"kind":"content","patch":raw["content"],"step_operations":ops,"source_note":input.source_note}});
        validate_task_edit_with_raw(&edit, Some(&edit_raw))?;
        let mut steps = Vec::new();
        apply_content_edit(
            &mut task,
            &mut steps,
            Some(&input.content),
            Some(&ops),
            false,
            self.now,
            &mut Vec::new(),
        )?;
        if matches!((task.planned_start,task.due_date),(Some(start),Some(end)) if start>end) {
            return Err(TaskServiceError::DateOrderInvalid);
        }
        check_step_dates_in_task_period(&task, &steps)?;
        insert_full_task(
            &tx,
            &task,
            &steps,
            self.actor,
            input.mutation_id,
            Some((&run, revision as i64)),
            self.now,
        )?;
        let reason = [
            Some("人工新建任务"),
            input.source_note.as_deref(),
            input.content.note.as_ref().and_then(|note| note.as_deref()),
        ]
        .into_iter()
        .flatten()
        .filter(|v| !v.trim().is_empty())
        .collect::<Vec<_>>()
        .join("；");
        tx.execute("UPDATE task_events SET reason=?1 WHERE task_id=?2 AND mutation_id=?3 AND event_type='task_created'",params![reason,task.task_id.to_string(),input.mutation_id.to_string()])?;
        if let Some(source) = input
            .source_note
            .as_deref()
            .filter(|text| !text.trim().is_empty())
        {
            tx.execute("UPDATE task_events SET after_json=json_set(after_json,'$.source_note',?1) WHERE task_id=?2 AND mutation_id=?3 AND event_type='task_created'",params![source,task.task_id.to_string(),input.mutation_id.to_string()])?;
        }
        if let Some(note) = &input.content.note {
            tx.execute("UPDATE task_events SET after_json=json_set(after_json,'$.note',?1) WHERE task_id=?2 AND mutation_id=?3 AND event_type='task_created'",params![note,task.task_id.to_string(),input.mutation_id.to_string()])?;
        }
        tx.execute("INSERT INTO task_period_links (id,workspace_id,task_id,run_id,analysis_revision_no,cycle_role,source_period_from,source_period_to,target_period_from,target_period_to,inherited_from_link_id,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,NULL,?11)",params![Uuid::new_v4().to_string(),ws,task.task_id.to_string(),run,revision,match input.cycle_role{CycleRole::Review=>"review",CycleRole::Plan=>"plan"},source.from.to_string(),source.to.to_string(),target.from.to_string(),target.to.to_string(),utc_iso(self.now)])?;
        append_task_snapshot(&tx, &run, revision as i64, self.now)?;
        let result = TaskEditOutcome {
            task_id: task.task_id.to_string(),
            task_revision_no: 1,
            status: "not_started".into(),
            progress: "0".into(),
            replayed: false,
        };
        record_mutation(
            &tx,
            &ws,
            &input.mutation_id.to_string(),
            &digest,
            &serde_json::to_string(&result).expect("task result"),
            &[run],
            &[task.task_id.to_string()],
            self.now,
        )?;
        tx.commit()?;
        Ok(result)
    }
}
fn invalid(message: impl Into<String>) -> TaskServiceError {
    TaskServiceError::InvalidInput(message.into())
}

/// 读取不可变分析快照的本期/计划期；桌面只读详情与人工创建共用同一口径。
pub fn saved_task_periods(
    conn: &Connection,
    run: &str,
    revision: u32,
) -> Result<(DateRange, DateRange), TaskServiceError> {
    let (normalized,policy,action):(String,String,Option<String>) = conn.query_row("SELECT normalized_metrics_json,period_policy_snapshot_json,action_period_snapshot_json FROM run_revisions WHERE run_id=?1 AND revision_no=?2",params![run,revision],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let normalized: Value =
        serde_json::from_str(&normalized).map_err(|_| invalid("分析期间无法读取"))?;
    let source: DateRange = serde_json::from_value(normalized["period"].clone())
        .map_err(|_| invalid("分析期间无法读取"))?;
    let plan = match action {
        Some(value) => {
            serde_json::from_str::<DateRange>(&value).map_err(|_| invalid("计划期间无法读取"))?
        }
        None => {
            let policy: Value =
                serde_json::from_str(&policy).map_err(|_| invalid("期间策略无法读取"))?;
            let day = policy["month_start_day"]
                .as_u64()
                .filter(|v| (1..=31).contains(v))
                .unwrap_or(1) as u32;
            let current = crate::analysis::periods::monthly_period_span(source.to, day);
            let next = current
                .to
                .succ_opt()
                .ok_or(TaskServiceError::DateOrderInvalid)?;
            crate::analysis::periods::monthly_period_span(next, day)
        }
    };
    if source.from > source.to || plan.from > plan.to {
        return Err(TaskServiceError::DateOrderInvalid);
    }
    Ok((source, plan))
}
