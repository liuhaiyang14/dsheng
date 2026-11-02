//! 候选合并 reconcile：dry-run（diff + apply_token 10 分钟）与 apply
//! （revision/digest/token 校验、映射唯一性、final_step_order、tombstone、
//! 单事务原子；task-lifecycle.md §3.2/§3.3）。
//!
//! dry-run 输出按 contracts/v1/read-views.schema.json reconcilePreviewView。

use super::task_store::{
    append_events, append_snapshots_for_task, append_task_revision, check_mutation,
    insert_full_task, load_task, record_mutation, save_task_state, MutationCheck,
};
use super::tasks::{
    mutation_digest, normalize_candidate_output, CandidateStep, CandidateTask, PlannedEvent,
    StepRow, TaskRow, TaskServiceError,
};
use crate::contracts::domain::DateRange;
use crate::contracts::tasks::{
    ActorSource, CandidateMapping, FinalStepOrderEntry, SmartStatus, StepMapping, StepTakeField,
    TaskLifecycle, TaskStatus, TaskTakeField,
};
use crate::repository::utc_iso;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde_json::json;
use std::collections::BTreeSet;
use uuid::Uuid;

/// apply_token / preview 有效期（§3.3：10 分钟）。
pub const TOKEN_VALID_MINUTES: i64 = 10;

/// dry-run 结果（reconcilePreviewView 对齐）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReconcilePreview {
    pub apply_token: String,
    pub expires_at: String,
    pub affected: Vec<AffectedTask>,
    pub created_new: Vec<String>,
    pub ignored: Vec<String>,
    pub replaced: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AffectedTask {
    pub task_id: String,
    pub task_revision_no: u32,
    pub action: String,
}

/// apply 结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReconcileApplied {
    pub candidate_generation_id: String,
    pub new_task_ids: Vec<String>,
    pub updated_task_ids: Vec<String>,
    pub superseded_task_ids: Vec<String>,
    pub replayed: bool,
}

/// 候选读取（task_candidates.output_json 原样保存的模型输出）。
fn load_candidate(
    conn: &Connection,
    workspace_id: &str,
    candidate_generation_id: &Uuid,
) -> Result<(Vec<CandidateTask>, String, String), TaskServiceError> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT workspace_id, output_json, state, analysis_input_fingerprint
             FROM task_candidates WHERE candidate_generation_id = ?1",
            params![candidate_generation_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    let Some((ws, output_json, state, fingerprint)) = row else {
        return Err(TaskServiceError::Db("candidate_not_found".into()));
    };
    if ws != workspace_id {
        return Err(TaskServiceError::WorkspaceMismatch);
    }
    if state == "applied" {
        return Err(TaskServiceError::Db("candidate_already_applied".into()));
    }
    let raw: serde_json::Value = serde_json::from_str(&output_json)
        .map_err(|_| TaskServiceError::Db("candidate_json".into()))?;
    Ok((normalize_candidate_output(&raw)?, fingerprint, state))
}

/// 输入结构校验：映射唯一性、expected_tasks 精确覆盖（§3.2/§3.3）。
fn validate_reconcile_input(
    input: &crate::contracts::tasks::TaskReconcileInput,
    candidates: &[CandidateTask],
) -> Result<(), TaskServiceError> {
    let candidate_keys: BTreeSet<&str> = candidates.iter().map(|c| c.task_key.as_str()).collect();
    let mut mapped_keys = BTreeSet::new();
    let mut mapped_old_tasks: BTreeSet<&Uuid> = BTreeSet::new();
    let mut update_replace_ids: BTreeSet<&Uuid> = BTreeSet::new();
    // 第一遍：候选 key 唯一 + 旧任务不被多候选写入/替代（优先报 DUPLICATE_RECONCILE_TARGET）
    for m in &input.mappings {
        let key = m.candidate_task_key();
        if !mapped_keys.insert(key) {
            return Err(TaskServiceError::Db("DUPLICATE_CANDIDATE_KEY".into()));
        }
        if !candidate_keys.contains(key) {
            return Err(TaskServiceError::Db("candidate_key_not_found".into()));
        }
        let old_target = match m {
            CandidateMapping::Update {
                existing_task_id, ..
            }
            | CandidateMapping::Replace {
                existing_task_id, ..
            } => Some(existing_task_id),
            _ => None,
        };
        if let Some(id) = old_target {
            if !mapped_old_tasks.insert(id) {
                return Err(TaskServiceError::Db("DUPLICATE_RECONCILE_TARGET".into()));
            }
            update_replace_ids.insert(id);
        }
    }
    // 第二遍：update 映射形状
    for m in &input.mappings {
        if let CandidateMapping::Update {
            step_mappings,
            final_step_order,
            ..
        } = m
        {
            validate_update_mapping_shape(step_mappings, final_step_order)?;
        }
    }
    // 每个候选任务恰好映射一次（未列候选不允许——schema 输入映射缺省即拒绝）
    for key in &candidate_keys {
        if !mapped_keys.contains(key) {
            return Err(TaskServiceError::Db("candidate_not_mapped".into()));
        }
    }
    // expected_tasks 必须精确覆盖全部 update/replace 旧任务
    let expected: BTreeSet<&Uuid> = input.expected_tasks.iter().map(|t| &t.task_id).collect();
    if expected != update_replace_ids {
        return Err(TaskServiceError::Db("expected_tasks_mismatch".into()));
    }
    Ok(())
}

/// update 映射结构校验（schema minItems 的领域等价物；SC-20 等）。
pub fn validate_update_mapping_shape(
    step_mappings: &[StepMapping],
    final_step_order: &[FinalStepOrderEntry],
) -> Result<(), TaskServiceError> {
    if step_mappings.is_empty() {
        return Err(TaskServiceError::Db("step_mapping_required".into()));
    }
    if final_step_order.is_empty() {
        return Err(TaskServiceError::AtLeastOneStep);
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for sm in step_mappings {
        if !seen.insert(sm.candidate_step_id()) {
            return Err(TaskServiceError::Db("DUPLICATE_CANDIDATE_KEY".into()));
        }
    }
    Ok(())
}

/// reconcile 输入形状校验（无候选库依赖；SC 用例驱动）。
pub fn validate_reconcile_input_shape(
    input: &crate::contracts::tasks::TaskReconcileInput,
) -> Result<(), TaskServiceError> {
    let mut keys = BTreeSet::new();
    for m in &input.mappings {
        if !keys.insert(m.candidate_task_key().to_string()) {
            return Err(TaskServiceError::Db("DUPLICATE_CANDIDATE_KEY".into()));
        }
        if let CandidateMapping::Update {
            step_mappings,
            final_step_order,
            ..
        } = m
        {
            validate_update_mapping_shape(step_mappings, final_step_order)?;
        }
    }
    Ok(())
}

trait MappingKey {
    fn candidate_task_key(&self) -> &str;
}

impl MappingKey for CandidateMapping {
    fn candidate_task_key(&self) -> &str {
        match self {
            CandidateMapping::New {
                candidate_task_key, ..
            }
            | CandidateMapping::Ignore {
                candidate_task_key, ..
            }
            | CandidateMapping::Replace {
                candidate_task_key, ..
            }
            | CandidateMapping::Update {
                candidate_task_key, ..
            } => candidate_task_key,
        }
    }
}

trait StepMappingKey {
    fn candidate_step_id(&self) -> &str;
}

impl StepMappingKey for StepMapping {
    fn candidate_step_id(&self) -> &str {
        match self {
            StepMapping::New {
                candidate_step_id, ..
            }
            | StepMapping::Ignore {
                candidate_step_id, ..
            }
            | StepMapping::Update {
                candidate_step_id, ..
            } => candidate_step_id,
        }
    }
}

/// 分析 revision/指纹门禁（§2.2：预览与提交都校验）。
fn check_analysis_gate(
    conn: &Connection,
    input: &crate::contracts::tasks::TaskReconcileInput,
    candidate_fingerprint: &str,
) -> Result<(), TaskServiceError> {
    let (current_rev, current_fp): (i64, String) = conn
        .query_row(
            "SELECT current_revision_no, analysis_input_fingerprint FROM analysis_runs
             WHERE run_id = ?1 AND workspace_id = ?2",
            params![input.run_id.to_string(), input.workspace_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| TaskServiceError::RunMismatch)?;
    if current_rev != input.analysis_revision_no as i64
        || current_fp != input.input_fingerprint
        || candidate_fingerprint != input.input_fingerprint
    {
        return Err(TaskServiceError::InputRevisionChanged);
    }
    Ok(())
}

fn token_binding_digest(
    input: &crate::contracts::tasks::TaskReconcileInput,
    current_revisions: &[(Uuid, u32)],
) -> String {
    let mut value = serde_json::to_value(input).expect("json");
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "__current_task_revisions".into(),
            json!(current_revisions
                .iter()
                .map(|(id, rev)| json!({"task_id": id.to_string(), "task_revision_no": rev}))
                .collect::<Vec<_>>()),
        );
    }
    mutation_digest(&value)
}

fn load_current_revisions(
    conn: &Connection,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, u32)>, TaskServiceError> {
    let mut out = Vec::new();
    for id in ids {
        let rev: i64 = conn
            .query_row(
                "SELECT task_revision_no FROM tasks WHERE task_id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )
            .map_err(TaskServiceError::from)?;
        out.push((*id, rev as u32));
    }
    Ok(out)
}

/// dry-run：校验 + diff + apply_token（10 分钟；不修改业务记录）。
#[allow(clippy::needless_pass_by_value)]
pub fn prepare_reconcile(
    conn: &Connection,
    input: &crate::contracts::tasks::TaskReconcileInput,
    now: DateTime<Utc>,
) -> Result<ReconcilePreview, TaskServiceError> {
    let (candidates, fingerprint, _state) = load_candidate(
        conn,
        &input.workspace_id.to_string(),
        &input.candidate_generation_id,
    )?;
    validate_reconcile_input(input, &candidates)?;
    check_analysis_gate(conn, input, &fingerprint)?;
    let mut affected = Vec::new();
    let mut created_new = Vec::new();
    let mut ignored = Vec::new();
    let mut replaced = Vec::new();
    let mut expected_ids = Vec::new();
    for t in &input.expected_tasks {
        expected_ids.push(t.task_id);
    }
    for mapping in &input.mappings {
        match mapping {
            CandidateMapping::New {
                candidate_task_key, ..
            } => {
                created_new.push(candidate_task_key.clone());
            }
            CandidateMapping::Ignore { candidate_task_key } => {
                ignored.push(candidate_task_key.clone())
            }
            CandidateMapping::Replace {
                candidate_task_key,
                existing_task_id,
                ..
            } => {
                replaced.push(candidate_task_key.clone());
                let rev = input
                    .expected_tasks
                    .iter()
                    .find(|t| t.task_id == *existing_task_id)
                    .map(|t| t.task_revision_no)
                    .ok_or(TaskServiceError::RevisionConflict)?;
                affected.push(AffectedTask {
                    task_id: existing_task_id.to_string(),
                    task_revision_no: rev,
                    action: "replace".into(),
                });
            }
            CandidateMapping::Update {
                existing_task_id, ..
            } => {
                let rev = input
                    .expected_tasks
                    .iter()
                    .find(|t| t.task_id == *existing_task_id)
                    .map(|t| t.task_revision_no)
                    .ok_or(TaskServiceError::RevisionConflict)?;
                affected.push(AffectedTask {
                    task_id: existing_task_id.to_string(),
                    task_revision_no: rev,
                    action: "update".into(),
                });
            }
        }
    }
    // 精确 expected revision 校验（prepare 阶段即失败）
    let current = load_current_revisions(conn, &expected_ids)?;
    for ((id, rev), expected) in current.iter().zip(&input.expected_tasks) {
        if id != &expected.task_id || *rev != expected.task_revision_no {
            return Err(TaskServiceError::RevisionConflict);
        }
    }
    let token = Uuid::new_v4().to_string();
    let binding = token_binding_digest(input, &current);
    conn.execute(
        "INSERT INTO local_action_tokens
           (token_id, kind, workspace_id, binding_digest, created_at, expires_at)
         VALUES (?1, 'reconcile_apply', ?2, ?3, ?4, ?5)",
        params![
            token,
            input.workspace_id.to_string(),
            binding,
            utc_iso(now),
            utc_iso(now + Duration::minutes(TOKEN_VALID_MINUTES)),
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(ReconcilePreview {
        apply_token: token.clone(),
        expires_at: utc_iso(now + Duration::minutes(TOKEN_VALID_MINUTES)),
        affected,
        created_new,
        ignored,
        replaced,
    })
}

fn consume_token(
    conn: &Connection,
    _workspace_id: &str,
    token: &str,
    binding: &str,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let row: Option<(String, String, Option<String>)> = conn
        .query_row(
            "SELECT binding_digest, expires_at, consumed_at FROM local_action_tokens
             WHERE token_id = ?1 AND kind = 'reconcile_apply'",
            params![token],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    let Some((stored_binding, expires_at, consumed)) = row else {
        return Err(TaskServiceError::Db("token_invalid".into()));
    };
    if consumed.is_some() {
        return Err(TaskServiceError::Db("token_consumed".into()));
    }
    let expires = DateTime::parse_from_rfc3339(&expires_at)
        .map_err(|_| TaskServiceError::Db("token_expiry".into()))?
        .with_timezone(&Utc);
    if now > expires {
        return Err(TaskServiceError::Db("token_expired".into()));
    }
    if stored_binding != binding {
        return Err(TaskServiceError::Db("token_binding_mismatch".into()));
    }
    conn.execute(
        "UPDATE local_action_tokens SET consumed_at = ?2 WHERE token_id = ?1",
        params![token, utc_iso(now)],
    )
    .map_err(TaskServiceError::from)?;
    Ok(())
}

/// apply：一个事务完成任务/步骤/links/events/候选标记/幂等结果（§3.3）。
pub fn apply_reconcile(
    conn: &mut Connection,
    input: &crate::contracts::tasks::TaskReconcileInput,
    apply_token: &str,
    actor: ActorSource,
    now: DateTime<Utc>,
) -> Result<ReconcileApplied, TaskServiceError> {
    let input_json = serde_json::to_value(input).expect("json");
    let digest = mutation_digest(&input_json);
    let ws = input.workspace_id.to_string();
    let tx = conn.transaction().map_err(TaskServiceError::from)?;
    // 幂等先行：同 mutation_id 先命中已完成结果（即使 token 已消费）
    match check_mutation(&tx, &ws, &input.mutation_id.to_string(), &digest)? {
        MutationCheck::Replay(result) => {
            let mut outcome: ReconcileApplied =
                serde_json::from_str(&result).map_err(|_| TaskServiceError::Db("result".into()))?;
            outcome.replayed = true;
            return Ok(outcome);
        }
        MutationCheck::Conflict => return Err(TaskServiceError::IdempotencyConflict),
        MutationCheck::Deleted => return Err(TaskServiceError::MutationResultDeleted),
        MutationCheck::Proceed => {}
    }
    let (candidates, fingerprint, state) =
        load_candidate(&tx, &ws, &input.candidate_generation_id)?;
    if state == "applied" {
        return Err(TaskServiceError::Db("candidate_already_applied".into()));
    }
    validate_reconcile_input(input, &candidates)?;
    check_analysis_gate(&tx, input, &fingerprint)?;
    let mut expected_ids = Vec::new();
    for t in &input.expected_tasks {
        expected_ids.push(t.task_id);
    }
    let current = load_current_revisions(&tx, &expected_ids)?;
    for ((id, rev), expected) in current.iter().zip(&input.expected_tasks) {
        if id != &expected.task_id || *rev != expected.task_revision_no {
            return Err(TaskServiceError::RevisionConflict);
        }
    }
    // token：本地短期凭据；计划/并发/来源变化使其失效
    consume_token(
        &tx,
        &ws,
        apply_token,
        &token_binding_digest(input, &current),
        now,
    )?;

    let mut new_task_ids = Vec::new();
    let mut updated_task_ids = Vec::new();
    let mut superseded_task_ids = Vec::new();
    let mut permanent_ids = Vec::new();
    let affected_run_ids = vec![input.run_id.to_string()];
    for mapping in &input.mappings {
        let candidate = candidates
            .iter()
            .find(|c| c.task_key == *mapping.candidate_task_key())
            .expect("已校验存在");
        match mapping {
            CandidateMapping::Ignore { .. } => {}
            CandidateMapping::New { .. } => {
                let (task_row, steps) = candidate_to_task_rows(candidate, input);
                let task_id = task_row.task_id;
                let source = candidate
                    .source_period
                    .or(candidate.target_period)
                    .ok_or(TaskServiceError::Db("candidate_period_missing".into()))?;
                insert_full_task(
                    &tx,
                    &task_row,
                    &steps,
                    actor,
                    input.mutation_id,
                    Some((&input.run_id.to_string(), input.analysis_revision_no as i64)),
                    now,
                )?;
                insert_plan_link(
                    &tx,
                    &task_row,
                    &input.run_id,
                    input.analysis_revision_no,
                    source,
                    now,
                )?;
                new_task_ids.push(task_id.to_string());
                permanent_ids.push(json!({
                    "candidate_task_key": candidate.task_key,
                    "task_id": task_id.to_string(),
                }));
            }
            CandidateMapping::Replace {
                existing_task_id,
                reason,
                ..
            } => {
                let (old, _) = load_task(&tx, &ws, *existing_task_id)?;
                let (mut task_row, steps) = candidate_to_task_rows(candidate, input);
                task_row.task_id = Uuid::new_v4();
                let new_id = task_row.task_id;
                let source = candidate
                    .source_period
                    .or(candidate.target_period)
                    .ok_or(TaskServiceError::Db("candidate_period_missing".into()))?;
                insert_full_task(
                    &tx,
                    &task_row,
                    &steps,
                    actor,
                    input.mutation_id,
                    Some((&input.run_id.to_string(), input.analysis_revision_no as i64)),
                    now,
                )?;
                // 旧任务 superseded、只读、历史保留
                tx.execute(
                    "UPDATE tasks SET lifecycle='superseded', superseded_by_task_id=?2,
                            task_revision_no=task_revision_no+1, updated_at=?3
                     WHERE task_id=?1",
                    params![
                        existing_task_id.to_string(),
                        new_id.to_string(),
                        utc_iso(now)
                    ],
                )
                .map_err(TaskServiceError::from)?;
                append_events(
                    &tx,
                    *existing_task_id,
                    old.task_revision_no + 1,
                    actor,
                    input.mutation_id,
                    &[PlannedEvent {
                        event_type: "task_superseded",
                        before_json: None,
                        after_json: Some(
                            serde_json::to_string(&json!({
                                "by_task_id": new_id.to_string(),
                            }))
                            .expect("json"),
                        ),
                        reason: Some(reason.clone()),
                    }],
                    now,
                )?;
                insert_plan_link(
                    &tx,
                    &task_row,
                    &input.run_id,
                    input.analysis_revision_no,
                    source,
                    now,
                )?;
                superseded_task_ids.push(existing_task_id.to_string());
                new_task_ids.push(new_id.to_string());
                permanent_ids.push(json!({
                    "candidate_task_key": candidate.task_key,
                    "replaced_task_id": existing_task_id.to_string(),
                    "task_id": new_id.to_string(),
                }));
            }
            CandidateMapping::Update {
                existing_task_id,
                take_fields,
                step_mappings,
                remove_existing_step_ids,
                final_step_order,
                ..
            } => {
                apply_update_mapping(
                    &tx,
                    &ws,
                    input,
                    candidate,
                    *existing_task_id,
                    take_fields,
                    step_mappings,
                    remove_existing_step_ids,
                    final_step_order,
                    actor,
                    now,
                )?;
                updated_task_ids.push(existing_task_id.to_string());
                permanent_ids.push(json!({
                    "candidate_task_key": candidate.task_key,
                    "task_id": existing_task_id.to_string(),
                }));
            }
        }
    }
    // 候选标记 applied + 应用记录
    tx.execute(
        "UPDATE task_candidates SET state='applied', applied_at=?2 WHERE candidate_generation_id=?1",
        params![input.candidate_generation_id.to_string(), utc_iso(now)],
    )
    .map_err(TaskServiceError::from)?;
    tx.execute(
        "INSERT INTO task_candidate_applications
           (candidate_generation_id, workspace_id, mutation_id, mappings_json,
            permanent_ids_json, applied_result_json, actor_source, applied_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            input.candidate_generation_id.to_string(),
            ws,
            input.mutation_id.to_string(),
            serde_json::to_string(&input.mappings).expect("json"),
            serde_json::to_string(&permanent_ids).expect("json"),
            serde_json::to_string(&json!({"new": new_task_ids, "updated": updated_task_ids, "superseded": superseded_task_ids})).expect("json"),
            serde_json::to_value(actor).expect("json").as_str().expect("snake_case"),
            utc_iso(now),
        ],
    )
    .map_err(TaskServiceError::from)?;
    let result = ReconcileApplied {
        candidate_generation_id: input.candidate_generation_id.to_string(),
        new_task_ids: new_task_ids.clone(),
        updated_task_ids: updated_task_ids.clone(),
        superseded_task_ids: superseded_task_ids.clone(),
        replayed: false,
    };
    let mut affected_tasks: Vec<String> = new_task_ids
        .iter()
        .chain(updated_task_ids.iter())
        .cloned()
        .collect();
    affected_tasks.extend(superseded_task_ids.iter().cloned());
    record_mutation(
        &tx,
        &ws,
        &input.mutation_id.to_string(),
        &digest,
        &serde_json::to_string(&result).expect("json"),
        &affected_run_ids,
        &affected_tasks,
        now,
    )?;
    tx.commit().map_err(TaskServiceError::from)?;
    Ok(result)
}

fn insert_plan_link(
    tx: &Connection,
    task: &TaskRow,
    run_id: &Uuid,
    analysis_revision_no: u32,
    source: DateRange,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let Some(target) = task.target_period else {
        return Err(TaskServiceError::Db(
            "candidate_target_period_missing".into(),
        ));
    };
    tx.execute(
        "INSERT OR IGNORE INTO task_period_links
           (id, workspace_id, task_id, run_id, analysis_revision_no, cycle_role,
            source_period_from, source_period_to, target_period_from, target_period_to,
            inherited_from_link_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'plan', ?6, ?7, ?8, ?9, NULL, ?10)",
        params![
            Uuid::new_v4().to_string(),
            task.workspace_id.to_string(),
            task.task_id.to_string(),
            run_id.to_string(),
            analysis_revision_no as i64,
            source.from.format("%Y-%m-%d").to_string(),
            source.to.format("%Y-%m-%d").to_string(),
            target.from.format("%Y-%m-%d").to_string(),
            target.to.format("%Y-%m-%d").to_string(),
            utc_iso(now),
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(())
}

/// 候选 → 新 TaskRow/StepRow（本地步骤一律 not_started；模型 status 仅留快照）。
fn candidate_to_task_rows(
    candidate: &CandidateTask,
    input: &crate::contracts::tasks::TaskReconcileInput,
) -> (TaskRow, Vec<StepRow>) {
    let task = TaskRow {
        task_id: Uuid::new_v4(),
        workspace_id: input.workspace_id,
        task_revision_no: 1,
        title: candidate.title.clone(),
        summary: candidate.summary.clone(),
        category: candidate.category,
        metric_id: candidate.metric_id,
        baseline_json: candidate.baseline_json.clone(),
        target_value: candidate.target_value.clone(),
        planned_value: None,
        actual_value: None,
        current_value: None,
        unit: candidate.unit.clone(),
        mp_label: None,
        mp_unit: None,
        owner: candidate.owner.clone(),
        planned_start: candidate.planned_start,
        due_date: candidate.due_date,
        target_period: candidate.target_period,
        resource_need: candidate.resource_need.clone(),
        success_criteria: candidate.success_criteria.clone(),
        assumptions_json: Some(candidate.assumptions_json.clone()),
        needs_review_json: Some(candidate.needs_review_json.clone()),
        smart_check_json: Some(candidate.smart_check_json.clone()),
        status: TaskStatus::NotStarted,
        lifecycle: TaskLifecycle::Active,
        superseded_by_task_id: None,
        progress: rust_decimal::Decimal::ZERO,
        progress_mode: crate::contracts::tasks::ProgressMode::Auto,
        adjustment_reason: None,
        blocked_reason: None,
        smart_status: SmartStatus::Generated,
        review_note: None,
        completed_at: None,
    };
    let steps = candidate
        .steps
        .iter()
        .map(|s| CandidateStep::to_step_row(s, task.task_id))
        .collect();
    (task, steps)
}

impl CandidateStep {
    fn to_step_row(&self, _task_id: Uuid) -> StepRow {
        StepRow {
            step_id: Uuid::new_v4(),
            step_order: self.order,
            action: self.action.clone(),
            owner: self.owner.clone(),
            start_date: self.start_date,
            due_date: self.due_date,
            done_criteria: self.done_criteria.clone(),
            // 模型 status 仅保留在生成快照；本地一律 not_started（§3.1）
            status: TaskStatus::NotStarted,
            blocked_reason: None,
            completed_at: None,
            deleted_at: None,
        }
    }
}

/// update 映射：take_fields 白名单、步骤映射/删除/排序、重算进度、SMART stale。
#[allow(clippy::too_many_arguments)]
fn apply_update_mapping(
    tx: &Connection,
    ws: &str,
    input: &crate::contracts::tasks::TaskReconcileInput,
    candidate: &CandidateTask,
    existing_task_id: Uuid,
    take_fields: &[TaskTakeField],
    step_mappings: &[StepMapping],
    remove_existing_step_ids: &[Uuid],
    final_step_order: &[FinalStepOrderEntry],
    actor: ActorSource,
    now: DateTime<Utc>,
) -> Result<(), TaskServiceError> {
    let (mut task, steps) = load_task(tx, ws, existing_task_id)?;
    let mut events = Vec::new();
    let mut structure_changed = false;
    for field in take_fields {
        match field {
            TaskTakeField::Title => task.title = candidate.title.clone(),
            TaskTakeField::Summary => task.summary = candidate.summary.clone(),
            TaskTakeField::Category => task.category = candidate.category,
            TaskTakeField::MetricId => task.metric_id = candidate.metric_id,
            TaskTakeField::Baseline => task.baseline_json = candidate.baseline_json.clone(),
            TaskTakeField::TargetPeriod => task.target_period = candidate.target_period,
            TaskTakeField::TargetValue => task.target_value = candidate.target_value.clone(),
            TaskTakeField::Unit => task.unit = candidate.unit.clone(),
            TaskTakeField::Owner => task.owner = candidate.owner.clone(),
            TaskTakeField::PlannedStart => task.planned_start = candidate.planned_start,
            TaskTakeField::DueDate => task.due_date = candidate.due_date,
            TaskTakeField::ResourceNeed => task.resource_need = candidate.resource_need.clone(),
            TaskTakeField::SuccessCriteria => {
                task.success_criteria = candidate.success_criteria.clone();
            }
            TaskTakeField::EvidenceIds
            | TaskTakeField::Assumptions
            | TaskTakeField::NeedsReview => {
                // evidence/assumptions/needs_review 需显式选取并保留原 revision 来源；
                // 当前候选规范化保留原文，直接替换（引用校验在发送层完成）。
                match field {
                    TaskTakeField::Assumptions => {
                        task.assumptions_json = Some(candidate.assumptions_json.clone());
                    }
                    TaskTakeField::NeedsReview => {
                        task.needs_review_json = Some(candidate.needs_review_json.clone());
                    }
                    _ => {}
                }
            }
        }
    }
    if !take_fields.is_empty() {
        task.smart_status = SmartStatus::Stale;
    }
    // 步骤映射
    let mut live_steps: Vec<StepRow> = steps
        .iter()
        .filter(|s| s.deleted_at.is_none())
        .cloned()
        .collect();
    for sm in step_mappings {
        match sm {
            StepMapping::New {
                candidate_step_id, ..
            } => {
                let cand = candidate
                    .steps
                    .iter()
                    .find(|s| &s.step_id == candidate_step_id)
                    .ok_or(TaskServiceError::Db("candidate_step_not_found".into()))?;
                live_steps.push(cand.to_step_row(existing_task_id));
                structure_changed = true;
            }
            StepMapping::Update {
                candidate_step_id,
                existing_step_id,
                take_fields,
            } => {
                let cand = candidate
                    .steps
                    .iter()
                    .find(|s| &s.step_id == candidate_step_id)
                    .ok_or(TaskServiceError::Db("candidate_step_not_found".into()))?;
                let step = live_steps
                    .iter_mut()
                    .find(|s| s.step_id == *existing_step_id)
                    .ok_or(TaskServiceError::Db("step_not_found".into()))?;
                for f in take_fields {
                    match f {
                        StepTakeField::Action => step.action = cand.action.clone(),
                        StepTakeField::Owner => step.owner = cand.owner.clone(),
                        StepTakeField::StartDate => step.start_date = cand.start_date,
                        StepTakeField::DueDate => step.due_date = cand.due_date,
                        StepTakeField::DoneCriteria => {
                            step.done_criteria = cand.done_criteria.clone();
                        }
                        StepTakeField::EvidenceIds => {}
                    }
                }
                // update 不允许通过候选改 status/progress/completed_at（§3.2）
                if !take_fields.is_empty() {
                    task.smart_status = SmartStatus::Stale;
                }
            }
            StepMapping::Ignore { .. } => {}
        }
    }
    // 显式删除：tombstone；最终至少一条存活步骤
    for id in remove_existing_step_ids {
        let step = live_steps
            .iter_mut()
            .find(|s| s.step_id == *id)
            .ok_or(TaskServiceError::Db("step_not_found".into()))?;
        step.deleted_at = Some(now);
        structure_changed = true;
    }
    let mut live_after: Vec<StepRow> = live_steps
        .iter()
        .filter(|s| s.deleted_at.is_none())
        .cloned()
        .collect();
    if live_after.is_empty() {
        return Err(TaskServiceError::AtLeastOneStep);
    }
    // final_step_order：无重无漏列出最终存活旧步骤 + 被选 new 候选步骤
    let mut order_keys = Vec::new();
    for entry in final_step_order {
        match entry {
            FinalStepOrderEntry::Existing { step_id } => {
                if !live_after.iter().any(|s| s.step_id == *step_id) {
                    return Err(TaskServiceError::Db("final_step_order_invalid".into()));
                }
                order_keys.push(format!("e:{step_id}"));
            }
            FinalStepOrderEntry::NewCandidate {
                candidate_step_id, ..
            } => {
                if !step_mappings.iter().any(|sm| match sm {
                    StepMapping::New {
                        candidate_step_id: cid,
                        ..
                    } => cid == candidate_step_id,
                    _ => false,
                }) {
                    return Err(TaskServiceError::Db("final_step_order_invalid".into()));
                }
                order_keys.push(format!("n:{candidate_step_id}"));
            }
        }
    }
    let mut sorted = order_keys.clone();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != order_keys.len() {
        return Err(TaskServiceError::Db("final_step_order_duplicate".into()));
    }
    let expected_total = live_after.len();
    if order_keys.len() != expected_total {
        return Err(TaskServiceError::Db("final_step_order_missing".into()));
    }
    // 依 final_step_order 重排存活步骤（existing 取现存行，new 取候选新行）
    let mut final_rows: Vec<StepRow> = Vec::new();
    for entry in final_step_order {
        match entry {
            FinalStepOrderEntry::Existing { step_id } => {
                if let Some(pos) = live_after.iter().position(|s| s.step_id == *step_id) {
                    final_rows.push(live_after.remove(pos));
                }
            }
            FinalStepOrderEntry::NewCandidate {
                candidate_step_id, ..
            } => {
                let cand = candidate
                    .steps
                    .iter()
                    .find(|s| s.step_id == *candidate_step_id)
                    .ok_or(TaskServiceError::Db("candidate_step_not_found".into()))?;
                let row = cand.to_step_row(existing_task_id);
                final_rows.push(row);
            }
        }
    }
    for (i, s) in final_rows.iter_mut().enumerate() {
        s.step_order = (i + 1) as u32;
    }
    // 合并 tombstone 行（保留历史）
    let tombstones: Vec<StepRow> = steps
        .iter()
        .filter(|s| s.deleted_at.is_some())
        .cloned()
        .chain(
            final_rows
                .iter()
                .filter(|s| s.deleted_at.is_some())
                .cloned(),
        )
        .collect();
    let mut all_steps = final_rows;
    all_steps.extend(tombstones);
    if structure_changed {
        if task.progress_mode == crate::contracts::tasks::ProgressMode::Manual {
            task.progress_mode = crate::contracts::tasks::ProgressMode::Auto;
            task.adjustment_reason = None;
            events.push(PlannedEvent {
                event_type: "progress_override_reset",
                before_json: None,
                after_json: None,
                reason: Some("reconcile_step_change".into()),
            });
        }
        let (status, progress) =
            super::tasks::auto_status(&all_steps, task.blocked_reason.as_deref());
        task.status = status;
        task.progress = progress;
        if status == TaskStatus::Completed && task.completed_at.is_none() {
            task.completed_at = Some(now);
        } else if status != TaskStatus::Completed {
            task.completed_at = None;
        }
    }
    task.task_revision_no += 1;
    save_task_state(tx, &task, &all_steps, now)?;
    append_task_revision(tx, &task, &all_steps, actor, input.mutation_id, now)?;
    events.push(PlannedEvent {
        event_type: "reconcile_updated",
        before_json: None,
        after_json: None,
        reason: Some(candidate.task_key.clone()),
    });
    append_events(
        tx,
        existing_task_id,
        task.task_revision_no,
        actor,
        input.mutation_id,
        &events,
        now,
    )?;
    // update 为被选任务创建或更新同一 run/revision 的 plan link（§3.2）：
    // link 的期间取候选 target/source period（任务整体目标期不因此暗改，TS-43）。
    if let (Some(target), Some(source)) = (candidate.target_period, candidate.source_period) {
        tx.execute(
            "INSERT INTO task_period_links
               (id, workspace_id, task_id, run_id, analysis_revision_no, cycle_role,
                source_period_from, source_period_to, target_period_from, target_period_to,
                inherited_from_link_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 'plan', ?6, ?7, ?8, ?9, NULL, ?10)
             ON CONFLICT(task_id, run_id, analysis_revision_no, cycle_role) DO UPDATE SET
                source_period_from=excluded.source_period_from,
                source_period_to=excluded.source_period_to,
                target_period_from=excluded.target_period_from,
                target_period_to=excluded.target_period_to",
            params![
                Uuid::new_v4().to_string(),
                ws,
                existing_task_id.to_string(),
                input.run_id.to_string(),
                input.analysis_revision_no as i64,
                source.from.format("%Y-%m-%d").to_string(),
                source.to.format("%Y-%m-%d").to_string(),
                target.from.format("%Y-%m-%d").to_string(),
                target.to.format("%Y-%m-%d").to_string(),
                utc_iso(now),
            ],
        )
        .map_err(TaskServiceError::from)?;
    }
    append_snapshots_for_task(tx, existing_task_id, now)?;
    Ok(())
}

/// 初次生成判定（§3.1）：本 run 当前 revision 从未应用过、无 active 任务关联、
/// 且无候选目标期任务时可直接应用；否则 review_required。
pub fn initial_apply_allowed(
    conn: &Connection,
    workspace_id: &str,
    run_id: &Uuid,
    analysis_revision_no: u32,
) -> Result<bool, TaskServiceError> {
    let applied: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_candidates
             WHERE run_id = ?1 AND analysis_revision_no = ?2 AND state = 'applied'",
            params![run_id.to_string(), analysis_revision_no as i64],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    let active_links: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM task_period_links l JOIN tasks t ON t.task_id = l.task_id
             WHERE l.run_id = ?1 AND l.analysis_revision_no = ?2
               AND t.lifecycle = 'active' AND t.workspace_id = ?3",
            params![
                run_id.to_string(),
                analysis_revision_no as i64,
                workspace_id
            ],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    Ok(applied == 0 && active_links == 0)
}
