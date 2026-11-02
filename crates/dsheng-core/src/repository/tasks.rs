//! 任务生命周期服务（task-lifecycle.md §1、§4、§5、§6）。
//!
//! - 永久 task/step UUID 与模型候选 key 分离；`expected_task_revision` 乐观并发，
//!   冲突返回 `REVISION_CONFLICT`；任务/步骤/事件/修订同事务。
//! - 自动进度 = 完成/存活×100（half-even 2 位），未全完成不因舍入到 100，
//!   上限 99.99；block 优先；步骤结构变化把 manual 切回 auto 并写
//!   `progress_override_reset`；重开清 completed_at 并写 `reopened`。
//! - local_mutations 幂等：同 mutation_id+同 digest 返回原结果，不同 digest
//!   `IDEMPOTENCY_CONFLICT`，已删结果 `MUTATION_RESULT_DELETED`。
//! - 新分析 revision 在同事务继承 period links、封存旧快照并建 snapshot 0。

use crate::analysis::fingerprint::{canonical_json, sha256_hex};
use crate::contracts::decimal::{
    parse_input_decimal, parse_progress_value, to_canonical_string, DecimalPolicyError,
};
use crate::contracts::domain::DateRange;
use crate::contracts::tasks::{
    category_from_label, ProgressMode, SmartStatus, StepOperation, TaskCategory, TaskContentPatch,
    TaskEditInput, TaskEditOperation, TaskLifecycle, TaskStatus,
};
use crate::repository::{utc_iso, RepoError};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

/// 任务服务错误码（task-lifecycle / cli-surface 词法）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaskServiceError {
    #[error("REVISION_CONFLICT")]
    RevisionConflict,
    #[error("WORKSPACE_MISMATCH")]
    WorkspaceMismatch,
    #[error("STATE_PROGRESS_MISMATCH")]
    StateProgressMismatch,
    #[error("STEP_STILL_BLOCKED")]
    StepStillBlocked,
    #[error("AT_LEAST_ONE_STEP")]
    AtLeastOneStep,
    #[error("TASK_CATEGORY_METRIC_MISMATCH")]
    TaskCategoryMetricMismatch,
    #[error("TASK_CATEGORY_UNRECOGNIZED")]
    TaskCategoryUnrecognized,
    #[error("STEP_OUTSIDE_TASK_PERIOD")]
    StepOutsideTaskPeriod,
    #[error("DECIMAL_PRECISION_EXCEEDED")]
    DecimalPrecisionExceeded,
    #[error("DECIMAL_OVERFLOW")]
    DecimalOverflow,
    #[error("EMPTY_PATCH")]
    EmptyPatch,
    #[error("SOURCE_NOTE_REQUIRED")]
    SourceNoteRequired,
    #[error("MISSING_ADJUSTMENT_REASON")]
    MissingAdjustmentReason,
    #[error("BLOCKED_REASON_REQUIRED")]
    BlockedReasonRequired,
    #[error("DATE_ORDER_INVALID")]
    DateOrderInvalid,
    #[error("INPUT_REVISION_CHANGED")]
    InputRevisionChanged,
    #[error("STALE_DIAGNOSIS_REFERENCE")]
    StaleDiagnosisReference,
    #[error("IDEMPOTENCY_CONFLICT")]
    IdempotencyConflict,
    #[error("MUTATION_RESULT_DELETED")]
    MutationResultDeleted,
    #[error("TASK_READ_ONLY")]
    TaskReadOnly,
    #[error("RUN_MISMATCH")]
    RunMismatch,
    #[error("db error: {0}")]
    Db(String),
}

impl From<rusqlite::Error> for TaskServiceError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<RepoError> for TaskServiceError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::RevisionConflict => Self::RevisionConflict,
            RepoError::WorkspaceMismatch => Self::WorkspaceMismatch,
            other => Self::Db(other.to_string()),
        }
    }
}

// ---- 任务/步骤当前态（行映射 + 纯状态机输入输出） ----

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub task_id: Uuid,
    pub workspace_id: Uuid,
    pub task_revision_no: u32,
    pub title: String,
    pub summary: String,
    pub category: TaskCategory,
    pub metric_id: Option<Uuid>,
    pub baseline_json: Option<String>,
    pub target_value: Option<String>,
    pub planned_value: Option<String>,
    pub actual_value: Option<String>,
    pub current_value: Option<String>,
    pub unit: Option<String>,
    pub mp_label: Option<String>,
    pub mp_unit: Option<String>,
    pub owner: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub target_period: Option<DateRange>,
    pub resource_need: Option<String>,
    pub success_criteria: Option<String>,
    pub assumptions_json: Option<String>,
    pub needs_review_json: Option<String>,
    pub smart_check_json: Option<String>,
    pub status: TaskStatus,
    pub lifecycle: TaskLifecycle,
    pub superseded_by_task_id: Option<Uuid>,
    pub progress: rust_decimal::Decimal,
    pub progress_mode: ProgressMode,
    pub adjustment_reason: Option<String>,
    pub blocked_reason: Option<String>,
    pub smart_status: SmartStatus,
    pub review_note: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepRow {
    pub step_id: Uuid,
    pub step_order: u32,
    pub action: String,
    pub owner: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub done_criteria: Option<String>,
    pub status: TaskStatus,
    pub blocked_reason: Option<String>,
    pub completed_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

/// 纯状态机产出的事件（服务层补 event_seq/actor/mutation/occurred_at）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedEvent {
    pub event_type: &'static str,
    pub before_json: Option<String>,
    pub after_json: Option<String>,
    pub reason: Option<String>,
}

fn status_str(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::NotStarted => "not_started",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Completed => "completed",
        TaskStatus::Blocked => "blocked",
    }
}

/// 自动进度（§5.1）：completed/total×100，half-even 2 位；
/// 未全完成但四舍五入到 100 时显示上限 99.99（如 99999/100000）。
pub fn auto_progress(completed: usize, total: usize) -> rust_decimal::Decimal {
    let done = rust_decimal::Decimal::from(completed as u64);
    let count = rust_decimal::Decimal::from(total.max(1) as u64);
    let raw =
        crate::contracts::decimal::contract_div(done * rust_decimal::Decimal::ONE_HUNDRED, count)
            .unwrap_or(rust_decimal::Decimal::ZERO);
    let rounded =
        raw.round_dp_with_strategy(2, rust_decimal::RoundingStrategy::MidpointNearestEven);
    if completed < total && rounded == rust_decimal::Decimal::ONE_HUNDRED {
        // 完成判断用计数全等，不用展示舍入：显示上限 99.99
        rust_decimal::Decimal::from_str_exact("99.99").expect("99.99")
    } else {
        rounded
    }
}

/// 自动状态优先级（§5.1）：block > 全完成 > 有进展 > 未开始。
///
/// block 可 0–100%；全完成 progress=100。
pub fn auto_status(
    steps: &[StepRow],
    task_blocked_reason: Option<&str>,
) -> (TaskStatus, rust_decimal::Decimal) {
    let live: Vec<&StepRow> = steps.iter().filter(|s| s.deleted_at.is_none()).collect();
    let total = live.len();
    let completed = live
        .iter()
        .filter(|s| s.status == TaskStatus::Completed)
        .count();
    let any_blocked = task_blocked_reason.is_some_and(|r| !r.is_empty())
        || live.iter().any(|s| s.status == TaskStatus::Blocked);
    if any_blocked {
        return (TaskStatus::Blocked, auto_progress(completed, total));
    }
    if total == 0 {
        return (TaskStatus::NotStarted, rust_decimal::Decimal::ZERO);
    }
    if completed == total {
        return (TaskStatus::Completed, rust_decimal::Decimal::ONE_HUNDRED);
    }
    let has_progress = live
        .iter()
        .any(|s| s.status == TaskStatus::Completed || s.status == TaskStatus::InProgress);
    if has_progress {
        (TaskStatus::InProgress, auto_progress(completed, total))
    } else {
        (TaskStatus::NotStarted, rust_decimal::Decimal::ZERO)
    }
}

/// 截止日显示：null → “未设截止”（§5.1）。
pub fn due_date_display(due: Option<NaiveDate>) -> String {
    due.map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "未设截止".to_string())
}

/// derived overdue（§5.1）：workspace 当地 today > due && status!=completed &&
/// lifecycle=active；due=null → false（显示未设截止）。
pub fn derive_overdue(
    status: TaskStatus,
    lifecycle: TaskLifecycle,
    due_date: Option<NaiveDate>,
    workspace_today: NaiveDate,
) -> bool {
    matches!(
        (status, lifecycle),
        (
            TaskStatus::NotStarted | TaskStatus::InProgress | TaskStatus::Blocked,
            TaskLifecycle::Active
        )
    ) && due_date.is_some_and(|d| workspace_today > d)
}

/// 步骤结构或状态是否变化（决定 manual 切回 auto，§5.2）。
fn is_structural_step_change(op: &StepOperation) -> bool {
    match op {
        StepOperation::Add { .. } | StepOperation::Delete { .. } => true,
        StepOperation::Update { patch, .. } => patch.status.is_some(),
        StepOperation::Move { .. } => false,
    }
}

/// patch 原始 JSON 中 metric_id 是否显式 null。
///
/// 冻结类型 `TaskContentPatch.metric_id` 为 `Option<Uuid>`，无法区分“未提供”
/// 与“显式 null”（task-edit schema 允许 null）；显式 null 由服务层从原始
/// JSON 检测后传入 [`apply_content_edit`] 实现清空（合同缺口见交付报告）。
pub fn patch_metric_id_explicit_null(operation_json: &serde_json::Value) -> bool {
    operation_json
        .get("patch")
        .and_then(|p| p.get("metric_id"))
        .is_some_and(serde_json::Value::is_null)
}

/// 内容操作纯应用：改 task/step 字段，返回（smart_stale, structure_changed）。
///
/// `metric_id_explicit_null`：patch 原文显式置 null（清空指标绑定）。
/// 不含并发/幂等/持久化——服务层在事务里调用。
#[allow(clippy::too_many_arguments)]
pub fn apply_content_edit(
    task: &mut TaskRow,
    steps: &mut Vec<StepRow>,
    patch: Option<&TaskContentPatch>,
    step_operations: Option<&[StepOperation]>,
    metric_id_explicit_null: bool,
    now: DateTime<Utc>,
    events: &mut Vec<PlannedEvent>,
) -> Result<(bool, bool), TaskServiceError> {
    let mut smart_stale = false;
    let mut structure_changed = false;

    if let Some(patch) = patch {
        if patch.is_empty() {
            return Err(TaskServiceError::EmptyPatch);
        }
        // 数值/日期/单位规范化在 validate_task_edit 完成；此处仅应用
        let before = task_snapshot_json(task, steps);
        if let Some(v) = &patch.title {
            task.title = v.clone();
            smart_stale = true;
        }
        if let Some(v) = &patch.summary {
            task.summary = v.clone();
            smart_stale = true;
        }
        if let Some(v) = patch.category {
            task.category = v;
            smart_stale = true;
        }
        if patch.metric_id.is_some() {
            task.metric_id = patch.metric_id;
            smart_stale = true;
        } else if metric_id_explicit_null && task.metric_id.is_some() {
            task.metric_id = None;
            smart_stale = true;
        }
        for (slot, value) in [
            (&mut task.target_value, &patch.target_value),
            (&mut task.planned_value, &patch.planned_value),
            (&mut task.actual_value, &patch.actual_value),
            (&mut task.current_value, &patch.current_value),
            (&mut task.unit, &patch.unit),
            (&mut task.mp_label, &patch.mp_label),
            (&mut task.mp_unit, &patch.mp_unit),
            (&mut task.owner, &patch.owner),
            (&mut task.resource_need, &patch.resource_need),
            (&mut task.success_criteria, &patch.success_criteria),
        ] {
            if let Some(v) = value {
                *slot = v.clone();
                smart_stale = true;
            }
        }
        if let Some(v) = patch.planned_start {
            task.planned_start = v;
            smart_stale = true;
        }
        if let Some(v) = patch.due_date {
            task.due_date = v;
            smart_stale = true;
        }
        if let Some(v) = &patch.review_note {
            task.review_note = Some(v.clone());
        }
        if let Some(p) = &patch.target_period {
            task.target_period = Some(*p);
            smart_stale = true;
        }
        // note：仅事件（0001 冻结 schema 无 note 列；review_note 持久化）
        events.push(PlannedEvent {
            event_type: "content_patched",
            before_json: Some(before),
            after_json: Some(task_snapshot_json(task, steps)),
            reason: None,
        });
    }

    if let Some(ops) = step_operations {
        for op in ops {
            if is_structural_step_change(op) {
                structure_changed = true;
                smart_stale = true;
            } else {
                // 纯文字/责任/日期/验收修改：不改执行状态，但 SMART stale
                if matches!(op, StepOperation::Update { .. }) {
                    smart_stale = true;
                }
            }
            match op {
                StepOperation::Add {
                    client_step_key,
                    content,
                    before_step_id,
                } => {
                    let new_id = Uuid::new_v4();
                    let step = StepRow {
                        step_id: new_id,
                        step_order: 0,
                        action: content.action.clone(),
                        owner: content.owner.clone(),
                        start_date: content.start_date,
                        due_date: content.due_date,
                        done_criteria: content.done_criteria.clone(),
                        status: TaskStatus::NotStarted,
                        blocked_reason: None,
                        completed_at: None,
                        deleted_at: None,
                    };
                    let pos = before_step_id
                        .and_then(|id| steps.iter().position(|s| s.step_id == id))
                        .unwrap_or(steps.len());
                    steps.insert(pos, step);
                    events.push(PlannedEvent {
                        event_type: "step_added",
                        before_json: None,
                        after_json: Some(step_event_json(&steps[pos])),
                        reason: Some(format!("client_step_key={client_step_key}")),
                    });
                }
                StepOperation::Update { step_id, patch } => {
                    let step = steps
                        .iter_mut()
                        .find(|s| s.step_id == *step_id && s.deleted_at.is_none())
                        .ok_or(TaskServiceError::Db("step_not_found".into()))?;
                    let before = step_event_json(step);
                    if let Some(v) = &patch.action {
                        step.action = v.clone();
                    }
                    if let Some(v) = &patch.owner {
                        step.owner = v.clone();
                    }
                    if let Some(v) = &patch.start_date {
                        step.start_date = *v;
                    }
                    if let Some(v) = &patch.due_date {
                        step.due_date = *v;
                    }
                    if let Some(v) = &patch.done_criteria {
                        step.done_criteria = v.clone();
                    }
                    if let Some(v) = &patch.blocked_reason {
                        step.blocked_reason = v.clone();
                    }
                    if let Some(new_status) = patch.status {
                        let old = step.status;
                        step.status = new_status;
                        transition_completed_at(&mut step.completed_at, new_status, now);
                        if old == TaskStatus::Completed && new_status != TaskStatus::Completed {
                            events.push(PlannedEvent {
                                event_type: "reopened",
                                before_json: Some(before.clone()),
                                after_json: Some(step_event_json(step)),
                                reason: Some("step_reopened".into()),
                            });
                        }
                    }
                    events.push(PlannedEvent {
                        event_type: "step_updated",
                        before_json: Some(before),
                        after_json: Some(step_event_json(step)),
                        reason: None,
                    });
                }
                StepOperation::Delete { step_id } => {
                    let live_count = steps.iter().filter(|s| s.deleted_at.is_none()).count();
                    if live_count <= 1 {
                        return Err(TaskServiceError::AtLeastOneStep);
                    }
                    let step = steps
                        .iter_mut()
                        .find(|s| s.step_id == *step_id && s.deleted_at.is_none())
                        .ok_or(TaskServiceError::Db("step_not_found".into()))?;
                    let before = step_event_json(step);
                    step.deleted_at = Some(now);
                    events.push(PlannedEvent {
                        event_type: "step_deleted",
                        before_json: Some(before),
                        after_json: Some(step_event_json(step)),
                        reason: None,
                    });
                }
                StepOperation::Move {
                    step_id,
                    before_step_id,
                } => {
                    let from = steps
                        .iter()
                        .position(|s| s.step_id == *step_id)
                        .ok_or(TaskServiceError::Db("step_not_found".into()))?;
                    let step = steps.remove(from);
                    let to = before_step_id
                        .and_then(|id| steps.iter().position(|s| s.step_id == id))
                        .unwrap_or(steps.len());
                    steps.insert(to, step);
                    events.push(PlannedEvent {
                        event_type: "step_moved",
                        before_json: None,
                        after_json: None,
                        reason: Some(format!("step_id={step_id}")),
                    });
                }
            }
        }
        renumber_steps(steps);
        // 仅结构/状态变化触发进度重算与 manual→auto 切换（§5.2）；
        // 纯文字/责任/日期/排序修改不切换 manual、不重算执行状态。
        if structure_changed {
            recompute_after_structure(task, steps, now, events)?;
        }
    }

    if smart_stale && task.smart_status != SmartStatus::Stale {
        task.smart_status = SmartStatus::Stale;
    }
    Ok((smart_stale, structure_changed))
}

fn renumber_steps(steps: &mut [StepRow]) {
    let mut order = 1u32;
    for step in steps.iter_mut() {
        if step.deleted_at.is_none() {
            step.step_order = order;
            order += 1;
        }
    }
}

/// 结构/状态变化后：manual → auto 切换（写 progress_override_reset）+
/// 状态/进度重算 + 完成时间与 reopened 事件维护。
fn recompute_after_structure(
    task: &mut TaskRow,
    steps: &[StepRow],
    now: DateTime<Utc>,
    events: &mut Vec<PlannedEvent>,
) -> Result<(), TaskServiceError> {
    if task.progress_mode == ProgressMode::Manual {
        task.progress_mode = ProgressMode::Auto;
        task.adjustment_reason = None;
        events.push(PlannedEvent {
            event_type: "progress_override_reset",
            before_json: None,
            after_json: None,
            reason: Some("step_structure_or_status_changed".into()),
        });
    }
    let old_status = task.status;
    let (new_status, progress) = auto_status(steps, task.blocked_reason.as_deref());
    task.status = new_status;
    task.progress = progress;
    transition_completed_at(&mut task.completed_at, new_status, now);
    if old_status == TaskStatus::Completed && new_status != TaskStatus::Completed {
        events.push(PlannedEvent {
            event_type: "reopened",
            before_json: None,
            after_json: None,
            reason: Some("task_reopened".into()),
        });
    }
    if old_status != TaskStatus::Completed && new_status == TaskStatus::Completed {
        events.push(PlannedEvent {
            event_type: "completed",
            before_json: None,
            after_json: None,
            reason: None,
        });
    }
    Ok(())
}

/// 完成时间：→completed 且未有时写 now（重复 completed 不重写）；
/// 离开 completed 置 null（reopened 事件由调用方写）。
fn transition_completed_at(
    completed_at: &mut Option<DateTime<Utc>>,
    status: TaskStatus,
    now: DateTime<Utc>,
) {
    if status == TaskStatus::Completed {
        if completed_at.is_none() {
            *completed_at = Some(now);
        }
    } else {
        *completed_at = None;
    }
}

fn task_snapshot_json(task: &TaskRow, steps: &[StepRow]) -> String {
    serde_json::to_string(&serde_json::json!({
        "category": serde_json::to_value(task.category).unwrap(),
        "progress": to_canonical_string(task.progress),
        "status": status_str(task.status),
        "steps": steps.iter().filter(|s| s.deleted_at.is_none()).map(|s| serde_json::json!({
            "action": s.action,
            "status": status_str(s.status),
            "step_id": s.step_id.to_string(),
        })).collect::<Vec<_>>(),
        "title": task.title,
    }))
    .expect("json")
}

fn step_event_json(step: &StepRow) -> String {
    serde_json::to_string(&serde_json::json!({
        "action": step.action,
        "completed_at": step.completed_at.map(utc_iso),
        "status": status_str(step.status),
        "step_id": step.step_id.to_string(),
    }))
    .expect("json")
}

/// 进度操作应用（§5.1/§5.2 纯逻辑）。
#[allow(clippy::too_many_arguments)]
pub fn apply_progress_edit(
    task: &mut TaskRow,
    steps: &[StepRow],
    mode: ProgressMode,
    submitted_progress: Option<&str>,
    submitted_status: TaskStatus,
    adjustment_reason: Option<&str>,
    blocked_reason: Option<&str>,
    now: DateTime<Utc>,
    events: &mut Vec<PlannedEvent>,
) -> Result<(), TaskServiceError> {
    let old_status = task.status;
    match mode {
        ProgressMode::Manual => {
            let Some(p_text) = submitted_progress else {
                return Err(TaskServiceError::MissingAdjustmentReason);
            };
            let progress = parse_progress_value(p_text)
                .map_err(|_| TaskServiceError::StateProgressMismatch)?;
            let reason = adjustment_reason
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .ok_or(TaskServiceError::MissingAdjustmentReason)?;
            // manual status 约束（§5.2）
            let ok = match submitted_status {
                TaskStatus::NotStarted => progress.is_zero(),
                TaskStatus::Completed => progress == rust_decimal::Decimal::ONE_HUNDRED,
                TaskStatus::InProgress => {
                    progress >= rust_decimal::Decimal::ZERO
                        && progress < rust_decimal::Decimal::ONE_HUNDRED
                }
                TaskStatus::Blocked => true,
            };
            if !ok {
                return Err(TaskServiceError::StateProgressMismatch);
            }
            if submitted_status == TaskStatus::Blocked {
                let br = blocked_reason
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .ok_or(TaskServiceError::BlockedReasonRequired)?;
                task.blocked_reason = Some(br.to_string());
            } else {
                task.blocked_reason = None;
            }
            task.progress_mode = ProgressMode::Manual;
            task.progress = progress;
            task.adjustment_reason = Some(reason.to_string());
            task.status = submitted_status;
            transition_completed_at(&mut task.completed_at, submitted_status, now);
            events.push(PlannedEvent {
                event_type: "progress_manual",
                before_json: None,
                after_json: None,
                reason: Some(reason.to_string()),
            });
        }
        ProgressMode::Auto => {
            // auto：progress/adjustment 必须为 null（schema 层校验，此处防御）
            if submitted_progress.is_some() || adjustment_reason.is_some() {
                return Err(TaskServiceError::StateProgressMismatch);
            }
            if submitted_status == TaskStatus::Blocked {
                let br = blocked_reason
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .ok_or(TaskServiceError::BlockedReasonRequired)?;
                task.blocked_reason = Some(br.to_string());
            } else {
                // 显式解除：仍有 blocked 步骤则拒绝（§5.2）
                let any_step_blocked = steps
                    .iter()
                    .any(|s| s.deleted_at.is_none() && s.status == TaskStatus::Blocked);
                if any_step_blocked {
                    return Err(TaskServiceError::StepStillBlocked);
                }
                task.blocked_reason = None;
            }
            let (computed_status, progress) = auto_status(steps, task.blocked_reason.as_deref());
            if submitted_status != TaskStatus::Blocked && submitted_status != computed_status {
                return Err(TaskServiceError::StateProgressMismatch);
            }
            task.progress_mode = ProgressMode::Auto;
            task.adjustment_reason = None;
            task.progress = progress;
            task.status = computed_status;
            transition_completed_at(&mut task.completed_at, computed_status, now);
        }
    }
    if old_status == TaskStatus::Completed && task.status != TaskStatus::Completed {
        task.completed_at = None;
        events.push(PlannedEvent {
            event_type: "reopened",
            before_json: None,
            after_json: None,
            reason: Some("task_reopened".into()),
        });
    }
    if old_status != TaskStatus::Completed && task.status == TaskStatus::Completed {
        events.push(PlannedEvent {
            event_type: "completed",
            before_json: None,
            after_json: None,
            reason: None,
        });
    }
    Ok(())
}

// ---- task-edit 输入校验（schema 条件句的领域实现） ----

/// 校验 TaskEditInput（SC 用例等价物）：serde 结构 + 条件规则 + 数值规范。
pub fn validate_task_edit(input: &TaskEditInput) -> Result<(), TaskServiceError> {
    validate_task_edit_with_raw(input, None)
}

/// 带原始 JSON 的校验：冻结类型 `Option<Option<T>>` 无法区分“未提供”与
/// “显式 null”（serde 将 null 反序列化为外层 None），空 patch 判定与数值
/// 显式 null 以原始 JSON 为准（SC-04/SC-28）。
pub fn validate_task_edit_with_raw(
    input: &TaskEditInput,
    raw: Option<&serde_json::Value>,
) -> Result<(), TaskServiceError> {
    if let (Some(raw), TaskEditOperation::Content { patch, .. }) = (raw, &input.operation) {
        let raw_patch = raw.get("operation").and_then(|o| o.get("patch"));
        if let (Some(typed), Some(raw_obj)) = (patch.as_deref(), raw_patch) {
            if typed.is_empty() && !raw_obj.as_object().is_some_and(|m| !m.is_empty()) {
                return Err(TaskServiceError::EmptyPatch);
            }
        }
        let _ = patch;
    }
    validate_task_edit_inner(input)
}

fn validate_task_edit_inner(input: &TaskEditInput) -> Result<(), TaskServiceError> {
    match &input.operation {
        TaskEditOperation::Content {
            patch,
            step_operations,
            source_note,
        } => {
            let patch = patch.as_deref();
            let ops = step_operations.as_deref();
            if patch.is_none() && ops.is_none() {
                return Err(TaskServiceError::EmptyPatch);
            }
            if let Some(patch) = patch {
                if patch.touches_numbers()
                    && source_note
                        .as_deref()
                        .map(str::trim)
                        .is_none_or(|s| s.is_empty())
                {
                    return Err(TaskServiceError::SourceNoteRequired);
                }
                validate_patch_values(patch)?;
                validate_patch_dates(patch)?;
            }
            if let Some(ops) = ops {
                for op in ops {
                    if let StepOperation::Update { patch, .. } = op {
                        if step_patch_is_empty(patch) {
                            return Err(TaskServiceError::EmptyPatch);
                        }
                        if patch.status == Some(TaskStatus::Blocked) {
                            let has_reason = patch.blocked_reason.as_ref().is_some_and(|r| {
                                r.as_deref().map(|x| !x.trim().is_empty()).unwrap_or(false)
                            });
                            if !has_reason {
                                return Err(TaskServiceError::BlockedReasonRequired);
                            }
                        }
                        // blocked_reason 显式提供但状态未变 blocked 也允许（解除路径）
                        validate_step_patch_dates(patch)?;
                    }
                }
            }
            Ok(())
        }
        TaskEditOperation::Progress {
            progress_mode,
            progress,
            status,
            adjustment_reason,
            blocked_reason,
        } => {
            match progress_mode {
                ProgressMode::Manual => {
                    let Some(p) = progress.as_deref() else {
                        return Err(TaskServiceError::MissingAdjustmentReason);
                    };
                    parse_progress_value(p).map_err(|_| TaskServiceError::StateProgressMismatch)?;
                    if adjustment_reason
                        .as_ref()
                        .map(|r| r.trim().is_empty())
                        .unwrap_or(true)
                    {
                        return Err(TaskServiceError::MissingAdjustmentReason);
                    }
                }
                ProgressMode::Auto => {
                    if progress.is_some() || adjustment_reason.is_some() {
                        return Err(TaskServiceError::StateProgressMismatch);
                    }
                }
            }
            if *status == TaskStatus::Blocked {
                if blocked_reason
                    .as_ref()
                    .map(|r| r.trim().is_empty())
                    .unwrap_or(true)
                {
                    return Err(TaskServiceError::BlockedReasonRequired);
                }
            } else if blocked_reason.is_some() {
                return Err(TaskServiceError::BlockedReasonRequired);
            }
            Ok(())
        }
        TaskEditOperation::PeriodLink {
            source_period,
            target_period,
            ..
        } => {
            if source_period.from > source_period.to || target_period.from > target_period.to {
                return Err(TaskServiceError::DateOrderInvalid);
            }
            Ok(())
        }
    }
}

fn validate_patch_values(patch: &TaskContentPatch) -> Result<(), TaskServiceError> {
    for field in [
        patch.target_value.as_ref(),
        patch.planned_value.as_ref(),
        patch.actual_value.as_ref(),
        patch.current_value.as_ref(),
    ]
    .into_iter()
    .flatten()
    .flatten()
    {
        parse_input_decimal(field).map_err(decimal_error)?;
    }
    Ok(())
}

fn decimal_error(e: DecimalPolicyError) -> TaskServiceError {
    match e {
        DecimalPolicyError::PrecisionExceeded { .. } => TaskServiceError::DecimalPrecisionExceeded,
        DecimalPolicyError::Overflow => TaskServiceError::DecimalOverflow,
        DecimalPolicyError::InvalidCanonical => {
            TaskServiceError::Db("invalid_canonical_decimal".into())
        }
    }
}

fn validate_patch_dates(patch: &TaskContentPatch) -> Result<(), TaskServiceError> {
    if let (Some(start), Some(due)) = (patch.planned_start.flatten(), patch.due_date.flatten()) {
        if start > due {
            return Err(TaskServiceError::DateOrderInvalid);
        }
    }
    if let Some(p) = &patch.target_period {
        if p.from > p.to {
            return Err(TaskServiceError::DateOrderInvalid);
        }
    }
    Ok(())
}

/// StepPatch 空判定（schema minProperties 1）。
pub fn step_patch_is_empty(patch: &crate::contracts::tasks::StepPatch) -> bool {
    patch.action.is_none()
        && patch.owner.is_none()
        && patch.start_date.is_none()
        && patch.due_date.is_none()
        && patch.done_criteria.is_none()
        && patch.status.is_none()
        && patch.blocked_reason.is_none()
}

fn validate_step_patch_dates(
    patch: &crate::contracts::tasks::StepPatch,
) -> Result<(), TaskServiceError> {
    if let (Some(start), Some(due)) = (patch.start_date.flatten(), patch.due_date.flatten()) {
        if start > due {
            return Err(TaskServiceError::DateOrderInvalid);
        }
    }
    Ok(())
}

/// 合并后类别/指标关系（§4.1：仅 other 可无指标）。
pub fn validate_category_metric(
    category: TaskCategory,
    metric_id: Option<Uuid>,
) -> Result<(), TaskServiceError> {
    if category == TaskCategory::Other {
        return Ok(());
    }
    if metric_id.is_none() {
        return Err(TaskServiceError::TaskCategoryMetricMismatch);
    }
    Ok(())
}

// ---- 候选规范化（模型输出 → 本地规范码/规范十进制；不经 f64） ----

/// 规范化后的候选任务（本地形态；模型 status 仅留在原始快照）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateTask {
    pub task_key: String,
    pub title: String,
    pub summary: String,
    pub category: TaskCategory,
    pub metric_id: Option<Uuid>,
    pub target_period: Option<DateRange>,
    pub source_period: Option<DateRange>,
    pub target_value: Option<String>,
    pub baseline_json: Option<String>,
    pub unit: Option<String>,
    pub owner: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub resource_need: Option<String>,
    pub success_criteria: Option<String>,
    pub evidence_ids: Vec<String>,
    pub assumptions_json: String,
    pub needs_review_json: String,
    pub smart_check_json: String,
    pub steps: Vec<CandidateStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateStep {
    pub step_id: String,
    pub order: u32,
    pub action: String,
    pub owner: Option<String>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub done_criteria: Option<String>,
    pub evidence_ids: Vec<String>,
}

fn model_number_to_local(v: &serde_json::Value) -> Result<Option<String>, TaskServiceError> {
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => {
            crate::contracts::decimal::model_number_to_decimal_string(n)
                .map(Some)
                .ok_or(TaskServiceError::DecimalOverflow)
        }
        _ => Err(TaskServiceError::Db("candidate_number_invalid".into())),
    }
}

fn opt_str(v: &serde_json::Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

fn opt_date(v: &serde_json::Value) -> Result<Option<NaiveDate>, TaskServiceError> {
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| TaskServiceError::DateOrderInvalid),
        _ => Err(TaskServiceError::Db("candidate_date_invalid".into())),
    }
}

fn opt_period(v: &serde_json::Value) -> Result<Option<DateRange>, TaskServiceError> {
    match v {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Object(o) => {
            let from = opt_date(o.get("from").unwrap_or(&serde_json::Value::Null))?
                .ok_or(TaskServiceError::DateOrderInvalid)?;
            let to = opt_date(o.get("to").unwrap_or(&serde_json::Value::Null))?
                .ok_or(TaskServiceError::DateOrderInvalid)?;
            DateRange::new(from, to)
                .map(Some)
                .map_err(|_| TaskServiceError::DateOrderInvalid)
        }
        _ => Err(TaskServiceError::Db("candidate_period_invalid".into())),
    }
}

/// 规范化整份候选输出（TS-16/18/46/49）：
/// - 未知类别 → `TASK_CATEGORY_UNRECOGNIZED` 拒绝整份，不默认 other；
/// - 重复 task_key → `DUPLICATE_CANDIDATE_KEY`；
/// - step order 1..N 无重复不连续 → 拒绝整份；
/// - 模型 number 词法 → 规范十进制字符串（不经 f64）。
pub fn normalize_candidate_output(
    raw: &serde_json::Value,
) -> Result<Vec<CandidateTask>, TaskServiceError> {
    let tasks = raw
        .get("tasks")
        .and_then(|t| t.as_array())
        .ok_or(TaskServiceError::Db("candidate_tasks_missing".into()))?;
    let mut seen_keys = BTreeSet::new();
    let mut out = Vec::new();
    for t in tasks {
        let key = t
            .get("task_key")
            .and_then(|k| k.as_str())
            .ok_or(TaskServiceError::Db("candidate_task_key_missing".into()))?
            .to_string();
        if !seen_keys.insert(key.clone()) {
            return Err(TaskServiceError::Db("DUPLICATE_CANDIDATE_KEY".into()));
        }
        let category_label = t
            .get("category")
            .and_then(|c| c.as_str())
            .unwrap_or_default();
        let category = category_from_label(category_label)
            .ok_or(TaskServiceError::TaskCategoryUnrecognized)?;
        let metric_id = match t.get("metric_id") {
            Some(serde_json::Value::Null) | None => None,
            Some(serde_json::Value::String(s)) => Some(
                Uuid::parse_str(s).map_err(|_| TaskServiceError::Db("metric_id_invalid".into()))?,
            ),
            Some(_) => return Err(TaskServiceError::Db("metric_id_invalid".into())),
        };
        let steps_raw = t
            .get("steps")
            .and_then(|s| s.as_array())
            .ok_or(TaskServiceError::Db("candidate_steps_missing".into()))?;
        let mut steps = Vec::new();
        for s in steps_raw {
            steps.push(CandidateStep {
                step_id: s
                    .get("step_id")
                    .and_then(|x| x.as_str())
                    .ok_or(TaskServiceError::Db("step_id_missing".into()))?
                    .to_string(),
                order: s
                    .get("order")
                    .and_then(|x| x.as_u64())
                    .ok_or(TaskServiceError::Db("step_order_missing".into()))?
                    as u32,
                action: s
                    .get("action")
                    .and_then(|x| x.as_str())
                    .ok_or(TaskServiceError::Db("action_missing".into()))?
                    .to_string(),
                owner: s.get("owner").and_then(opt_str),
                start_date: opt_date(s.get("start_date").unwrap_or(&serde_json::Value::Null))?,
                due_date: opt_date(s.get("due_date").unwrap_or(&serde_json::Value::Null))?,
                done_criteria: s.get("done_criteria").and_then(opt_str),
                evidence_ids: s
                    .get("evidence_ids")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|i| i.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
            });
        }
        // order 必须为 1..N 无重复
        let mut orders: Vec<u32> = steps.iter().map(|s| s.order).collect();
        orders.sort_unstable();
        if !steps.is_empty() && orders != (1..=steps.len() as u32).collect::<Vec<u32>>() {
            return Err(TaskServiceError::Db("candidate_step_order_invalid".into()));
        }
        let baseline = match t.get("baseline") {
            Some(serde_json::Value::Null) | None => None,
            Some(b @ serde_json::Value::Object(_)) => {
                let value =
                    model_number_to_local(b.get("value").unwrap_or(&serde_json::Value::Null))?;
                let unit = b.get("unit").and_then(opt_str);
                Some(
                    serde_json::to_string(&serde_json::json!({"value": value, "unit": unit}))
                        .expect("json"),
                )
            }
            Some(_) => return Err(TaskServiceError::Db("baseline_invalid".into())),
        };
        out.push(CandidateTask {
            task_key: key,
            title: t
                .get("title")
                .and_then(|x| x.as_str())
                .ok_or(TaskServiceError::Db("title_missing".into()))?
                .to_string(),
            summary: t
                .get("summary")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string(),
            category,
            metric_id,
            target_period: opt_period(t.get("target_period").unwrap_or(&serde_json::Value::Null))?,
            source_period: opt_period(t.get("source_period").unwrap_or(&serde_json::Value::Null))?,
            target_value: model_number_to_local(
                t.get("target_value").unwrap_or(&serde_json::Value::Null),
            )?,
            baseline_json: baseline,
            unit: t.get("unit").and_then(opt_str),
            owner: t.get("owner").and_then(opt_str),
            planned_start: opt_date(t.get("planned_start").unwrap_or(&serde_json::Value::Null))?,
            due_date: opt_date(t.get("due_date").unwrap_or(&serde_json::Value::Null))?,
            resource_need: t.get("resource_need").and_then(opt_str),
            success_criteria: t.get("success_criteria").and_then(opt_str),
            evidence_ids: t
                .get("evidence_ids")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|i| i.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            assumptions_json: serde_json::to_string(
                &t.get("assumptions")
                    .cloned()
                    .unwrap_or(serde_json::json!([])),
            )
            .expect("json"),
            needs_review_json: serde_json::to_string(
                &t.get("needs_review")
                    .cloned()
                    .unwrap_or(serde_json::json!([])),
            )
            .expect("json"),
            smart_check_json: serde_json::to_string(
                &t.get("smart_check")
                    .cloned()
                    .unwrap_or(serde_json::json!({})),
            )
            .expect("json"),
            steps,
        });
    }
    Ok(out)
}

// ---- 生成边界提交门禁（§2.2 纯检查） ----

/// 提交前核对当前分析 revision（TS-22）：任一不一致 → `INPUT_REVISION_CHANGED`。
pub fn check_generation_commit_gate(
    current_revision_no: u32,
    current_fingerprint: &str,
    attempt_revision_no: u32,
    attempt_fingerprint: &str,
) -> Result<(), TaskServiceError> {
    if current_revision_no != attempt_revision_no || current_fingerprint != attempt_fingerprint {
        return Err(TaskServiceError::InputRevisionChanged);
    }
    Ok(())
}

/// 诊断引用准入（§2.2，TS-23）：同 workspace、同 run、同 analysis revision、
/// 同指纹且 report revision 一致；失效 → `STALE_DIAGNOSIS_REFERENCE`
/// （不静默移除/更换）。
pub fn check_diagnosis_reference(
    same_run: bool,
    task_analysis_revision_no: u32,
    diagnosis_analysis_revision_no: u32,
    task_fingerprint: &str,
    diagnosis_fingerprint: &str,
    report_revision_selected: u32,
    report_revision_current: u32,
) -> Result<(), TaskServiceError> {
    if !same_run
        || task_analysis_revision_no != diagnosis_analysis_revision_no
        || task_fingerprint != diagnosis_fingerprint
        || report_revision_selected != report_revision_current
    {
        return Err(TaskServiceError::StaleDiagnosisReference);
    }
    Ok(())
}

/// 已派发请求的路由冻结（§2.2，TS-42）：Provider 配置后续变化不追改、
/// 不导致有效返回提交失败；记录实际使用的 provider_revision。
pub fn dispatched_route_frozen(
    dispatched_provider_revision: u32,
    _current_provider_revision: u32,
) -> u32 {
    dispatched_provider_revision
}

pub(crate) fn mutation_digest(value: &serde_json::Value) -> String {
    sha256_hex(&canonical_json(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(id: u64, status: TaskStatus) -> StepRow {
        StepRow {
            step_id: Uuid::from_u64_pair(id, id),
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

    fn task() -> TaskRow {
        TaskRow {
            task_id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            task_revision_no: 1,
            title: "t".into(),
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
            progress_mode: ProgressMode::Auto,
            adjustment_reason: None,
            blocked_reason: None,
            smart_status: SmartStatus::Generated,
            review_note: None,
            completed_at: None,
        }
    }

    #[test]
    fn auto_progress_rules() {
        assert_eq!(auto_progress(1, 3).to_string(), "33.33");
        assert_eq!(auto_progress(99999, 100000).to_string(), "99.99");
        assert_eq!(auto_progress(0, 3).to_string(), "0");
        assert_eq!(auto_progress(3, 3).to_string(), "100");
    }

    #[test]
    fn overdue_derivation() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        assert!(derive_overdue(
            TaskStatus::Blocked,
            TaskLifecycle::Active,
            Some(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()),
            today
        ));
        assert!(!derive_overdue(
            TaskStatus::Blocked,
            TaskLifecycle::Active,
            None,
            today
        ));
        assert!(!derive_overdue(
            TaskStatus::Completed,
            TaskLifecycle::Active,
            Some(NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()),
            today
        ));
    }

    #[test]
    fn manual_reset_on_step_status_change() {
        let mut t = task();
        t.progress_mode = ProgressMode::Manual;
        t.progress = rust_decimal::Decimal::from(70);
        t.status = TaskStatus::InProgress;
        let mut steps = vec![
            step(1, TaskStatus::Completed),
            step(2, TaskStatus::NotStarted),
        ];
        let now = Utc::now();
        let mut events = Vec::new();
        let ops = [StepOperation::Update {
            step_id: steps[1].step_id,
            patch: serde_json::from_value(serde_json::json!({"status": "completed"})).unwrap(),
        }];
        let (_, structure) = apply_content_edit(
            &mut t,
            &mut steps,
            None,
            Some(&ops),
            false,
            now,
            &mut events,
        )
        .unwrap();
        assert!(structure);
        assert_eq!(t.progress_mode, ProgressMode::Auto);
        assert_eq!(t.progress.to_string(), "100");
        assert_eq!(t.status, TaskStatus::Completed);
        assert!(t.completed_at.is_some());
        let types: Vec<&str> = events.iter().map(|e| e.event_type).collect();
        assert!(types.contains(&"progress_override_reset"));
        assert!(types.contains(&"completed"));
        // 顺序：reset 在 completed 前
        let reset = types
            .iter()
            .position(|x| *x == "progress_override_reset")
            .unwrap();
        let completed = types.iter().position(|x| *x == "completed").unwrap();
        assert!(reset < completed);
    }

    #[test]
    fn manual_kept_on_text_only_change() {
        let mut t = task();
        t.progress_mode = ProgressMode::Manual;
        t.progress = rust_decimal::Decimal::from(70);
        t.status = TaskStatus::InProgress;
        let mut steps = vec![
            step(1, TaskStatus::Completed),
            step(2, TaskStatus::NotStarted),
        ];
        let ops = [StepOperation::Update {
            step_id: steps[1].step_id,
            patch: serde_json::from_value(serde_json::json!({"action": "新行动"})).unwrap(),
        }];
        let mut events = Vec::new();
        apply_content_edit(
            &mut t,
            &mut steps,
            None,
            Some(&ops),
            false,
            Utc::now(),
            &mut events,
        )
        .unwrap();
        assert_eq!(t.progress_mode, ProgressMode::Manual);
        assert_eq!(t.progress.to_string(), "70");
        assert_eq!(t.status, TaskStatus::InProgress);
        assert_eq!(t.smart_status, SmartStatus::Stale);
    }

    #[test]
    fn task_block_persists_over_step_completion() {
        let mut t = task();
        t.progress_mode = ProgressMode::Manual;
        t.progress = rust_decimal::Decimal::from(70);
        t.blocked_reason = Some("等待复核".into());
        let mut steps = vec![
            step(1, TaskStatus::Completed),
            step(2, TaskStatus::NotStarted),
        ];
        let ops = [StepOperation::Update {
            step_id: steps[1].step_id,
            patch: serde_json::from_value(serde_json::json!({"status": "completed"})).unwrap(),
        }];
        let mut events = Vec::new();
        apply_content_edit(
            &mut t,
            &mut steps,
            None,
            Some(&ops),
            false,
            Utc::now(),
            &mut events,
        )
        .unwrap();
        assert_eq!(t.status, TaskStatus::Blocked);
        assert_eq!(t.blocked_reason.as_deref(), Some("等待复核"));
        assert_eq!(t.progress.to_string(), "100");
        assert!(t.completed_at.is_none());
    }

    #[test]
    fn auto_unblock_with_blocked_step_rejected() {
        let mut t = task();
        t.status = TaskStatus::InProgress;
        let steps = vec![step(1, TaskStatus::Completed), step(2, TaskStatus::Blocked)];
        let mut events = Vec::new();
        let err = apply_progress_edit(
            &mut t,
            &steps,
            ProgressMode::Auto,
            None,
            TaskStatus::Completed,
            None,
            None,
            Utc::now(),
            &mut events,
        )
        .unwrap_err();
        assert_eq!(err, TaskServiceError::StepStillBlocked);
    }

    #[test]
    fn manual_completed_requires_100() {
        let mut t = task();
        let mut events = Vec::new();
        let err = apply_progress_edit(
            &mut t,
            &[],
            ProgressMode::Manual,
            Some("50"),
            TaskStatus::Completed,
            Some("部分完成"),
            None,
            Utc::now(),
            &mut events,
        )
        .unwrap_err();
        assert_eq!(err, TaskServiceError::StateProgressMismatch);
    }

    #[test]
    fn reopen_clears_completed_at_and_writes_event() {
        let mut t = task();
        t.status = TaskStatus::Completed;
        t.completed_at = Some(Utc::now());
        let mut steps = vec![
            step(1, TaskStatus::Completed),
            step(2, TaskStatus::Completed),
        ];
        let ops = [StepOperation::Update {
            step_id: steps[1].step_id,
            patch: serde_json::from_value(serde_json::json!({"status": "in_progress"})).unwrap(),
        }];
        let mut events = Vec::new();
        apply_content_edit(
            &mut t,
            &mut steps,
            None,
            Some(&ops),
            false,
            Utc::now(),
            &mut events,
        )
        .unwrap();
        assert_eq!(t.status, TaskStatus::InProgress);
        assert_eq!(t.progress.to_string(), "50");
        assert!(t.completed_at.is_none());
        assert!(events.iter().any(|e| e.event_type == "reopened"));
    }

    #[test]
    fn delete_only_step_rejected() {
        let mut t = task();
        let mut steps = vec![step(1, TaskStatus::Completed)];
        let ops = [StepOperation::Delete {
            step_id: steps[0].step_id,
        }];
        let mut events = Vec::new();
        let err = apply_content_edit(
            &mut t,
            &mut steps,
            None,
            Some(&ops),
            false,
            Utc::now(),
            &mut events,
        )
        .unwrap_err();
        assert_eq!(err, TaskServiceError::AtLeastOneStep);
    }

    #[test]
    fn candidate_normalization() {
        // 模型 wire 用 from_str 构造：number 词法原样保留（json! 宏会先走 Rust f64
        // 字面量，损失精度——这正是合同禁止的路径，不能用于夹具）
        let raw: serde_json::Value = serde_json::from_str(
            r#"{"tasks": [{
                "task_key": "c-1",
                "title": "提效",
                "summary": "",
                "category": "其他事项",
                "metric_id": null,
                "target_value": 9007199254740993.125,
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
                "source_period": null,
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
                "steps": [{"step_id": "s1", "order": 1, "action": "核对", "owner": null,
                            "start_date": null, "due_date": null, "done_criteria": null,
                            "evidence_ids": [], "status": "completed"}]
            }]}"#,
        )
        .unwrap();
        let tasks = normalize_candidate_output(&raw).unwrap();
        assert_eq!(tasks[0].category, TaskCategory::Other);
        assert_eq!(
            tasks[0].target_value.as_deref(),
            Some("9007199254740993.125")
        );
        let dup = serde_json::json!({"tasks": [raw["tasks"][0].clone(), raw["tasks"][0].clone()]});
        assert!(normalize_candidate_output(&dup).is_err());
        let unknown = serde_json::json!({"tasks": [{
            "task_key": "c-2", "title": "x", "summary": "", "category": "未定义方向",
            "metric_id": null, "steps": [] }]});
        assert_eq!(
            normalize_candidate_output(&unknown).unwrap_err(),
            TaskServiceError::TaskCategoryUnrecognized
        );
    }
}
