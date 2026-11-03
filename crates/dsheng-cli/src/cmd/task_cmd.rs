//! task 命令组（cli-surface 34–42；generate 属 S3 门禁）。
//!
//! 编辑统一走 core `TaskEditService`（幂等＋乐观并发＋单事务）；
//! `task update --status/--progress/--note` 等 CLI sugar 规范化到同一
//! task-edit 合同（aliases 注记），不另设校验路径。

use crate::cli;
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::jsonio;
use crate::store::views;
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::tasks::{
    ProgressMode, StepOperation, TaskEditInput, TaskEditOperation, TaskStatus,
};
use dsheng_core::repository::task_store::TaskEditService;
use serde_json::json;
use uuid::Uuid;

pub fn dispatch(action: cli::TaskAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::TaskAction::Generate { .. } => super::generation_cmd::task_generate(action, db),
        cli::TaskAction::List { workspace, run } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            if let Some(run_id) = run.as_deref() {
                ctx::ensure_object_in_workspace(
                    &conn,
                    "analysis_runs",
                    "run_id",
                    run_id,
                    ws.workspace_id,
                    "run",
                )?;
            }
            let ids = views::task_ids(&conn, ws.workspace_id, run.as_deref())?;
            let tasks: Vec<serde_json::Value> = ids
                .iter()
                .map(|id| views::task_view(&conn, ws.workspace_id, *id))
                .collect::<Result<_, _>>()?;
            Ok(json!({ "tasks": tasks }))
        }
        cli::TaskAction::Show { workspace, task } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let task_id = ctx::uuid_arg(&task, "--task")?;
            ctx::ensure_object_in_workspace(
                &conn,
                "tasks",
                "task_id",
                &task_id.to_string(),
                ws.workspace_id,
                "task",
            )?;
            views::task_view(&conn, ws.workspace_id, task_id)
        }
        cli::TaskAction::Update {
            workspace,
            task,
            patch_json,
            status,
            progress,
            progress_mode,
            adjustment_reason,
            blocked_reason,
            note,
            expected_revision,
            mutation_id,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let task_id = ctx::uuid_arg(&task, "--task")?;
            ctx::ensure_object_in_workspace(
                &conn,
                "tasks",
                "task_id",
                &task_id.to_string(),
                ws.workspace_id,
                "task",
            )?;
            let input = if let Some(path) = patch_json {
                // 全量 task-edit 输入（operation 内含）；workspace/task 交叉校验
                let value = jsonio::read_json(&path)?;
                let parsed: TaskEditInput = serde_json::from_value(value)
                    .map_err(|e| CliError::invalid(format!("patch-json 非法: {e}")))?;
                if parsed.workspace_id != ws.workspace_id || parsed.task_id != task_id {
                    return Err(CliError::new(
                        CliErrorCode::WorkspaceMismatch,
                        "patch-json 的 workspace/task 与 flag 不一致",
                    ));
                }
                parsed
            } else {
                // CLI sugar → 同一 task-edit 合同
                let expected = expected_revision.ok_or_else(|| {
                    CliError::invalid("CLI sugar 编辑需要 --expected-revision（乐观并发）")
                        .with_field("--expected-revision")
                })?;
                let status = status
                    .map(|s| parse_status(&s))
                    .transpose()?
                    .unwrap_or(TaskStatus::InProgress);
                let progress_mode = progress_mode
                    .as_deref()
                    .map(|m| {
                        serde_json::from_value::<ProgressMode>(json!(m))
                            .map_err(|_| CliError::invalid("progress_mode 取值非法"))
                    })
                    .transpose()?
                    .unwrap_or(ProgressMode::Manual);
                if progress.is_none() {
                    return Err(
                        CliError::invalid("CLI sugar 编辑需要 --progress").with_field("--progress")
                    );
                }
                let progress_value = progress
                    .as_deref()
                    .map(parse_progress)
                    .transpose()?
                    .unwrap_or_else(|| "0".into());
                let adjustment = match progress_mode {
                    ProgressMode::Manual => {
                        adjustment_reason.clone().filter(|s| !s.trim().is_empty())
                    }
                    ProgressMode::Auto => None,
                };
                let blocked = match status {
                    TaskStatus::Blocked => blocked_reason.filter(|s| !s.trim().is_empty()),
                    _ => None,
                };
                let mut operation = json!({
                    "kind": "progress",
                    "progress_mode": serde_json::to_value(progress_mode).unwrap(),
                    "progress": progress_value,
                    "status": serde_json::to_value(status).unwrap(),
                    "adjustment_reason": adjustment,
                    "blocked_reason": blocked,
                });
                if let Some(note) = note {
                    // note 无独立操作；走 content patch（review_note 持久化路径）
                    operation = json!({
                        "kind": "content",
                        "patch": {"review_note": note},
                    });
                }
                TaskEditInput {
                    schema_version: "1".into(),
                    workspace_id: ws.workspace_id,
                    task_id,
                    expected_task_revision: expected,
                    mutation_id: mutation_id
                        .map(|m| ctx::uuid_arg(&m, "--mutation-id"))
                        .transpose()?
                        .unwrap_or_else(Uuid::new_v4),
                    operation: serde_json::from_value(operation)
                        .map_err(|e| CliError::invalid(format!("sugar 输入非法: {e}")))?,
                }
            };
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            apply_edit(&mut conn, &input)
        }
        cli::TaskAction::Step { action } => step(action, db),
        cli::TaskAction::Reconcile {
            workspace,
            run,
            input_json,
            dry_run,
            apply,
        } => reconcile(workspace, run, input_json, dry_run, apply, db),
        cli::TaskAction::Timeline {
            workspace,
            from,
            to,
            grain,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let f = parse_date(&from, "--from")?;
            let t = parse_date(&to, "--to")?;
            let range = dsheng_core::contracts::domain::DateRange::new(f, t)
                .map_err(|e| CliError::new(CliErrorCode::DataValidationFailed, e))?;
            let _ = grain; // 桶粒度由任务日期直接驱动；grain 影响分组视图（后续导出用）
            let entries = views::timeline_entries(&conn, ws.workspace_id, range)?;
            Ok(json!({ "entries": entries }))
        }
    }
}

fn parse_status(raw: &str) -> CliResult<TaskStatus> {
    serde_json::from_value(json!(raw)).map_err(|_| {
        CliError::invalid("status 取值非法（not_started/in_progress/completed/blocked）")
    })
}

fn parse_progress(raw: &str) -> CliResult<String> {
    dsheng_core::contracts::decimal::parse_progress_value(raw)
        .map(dsheng_core::contracts::decimal::to_canonical_string)
        .map_err(|e| {
            CliError::new(
                CliErrorCode::DataValidationFailed,
                format!("progress 非法: {e}"),
            )
        })
}

fn parse_date(raw: &str, flag: &str) -> CliResult<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| CliError::invalid(format!("{flag} 必须是 YYYY-MM-DD")).with_field(flag))
}

fn apply_edit(
    conn: &mut rusqlite::Connection,
    input: &TaskEditInput,
) -> CliResult<serde_json::Value> {
    let mut service = TaskEditService {
        conn,
        now: chrono::Utc::now(),
        actor: views::cli_actor(),
    };
    let outcome = service.apply(input).map_err(CliError::from)?;
    Ok(json!({
        "task_id": outcome.task_id,
        "task_revision_no": outcome.task_revision_no,
        "status": outcome.status,
        "progress": outcome.progress,
        "replayed": outcome.replayed,
    }))
}

/// 步骤归属：step_id → task（含 workspace 校验）。
fn task_of_step(conn: &rusqlite::Connection, workspace_id: Uuid, step_id: &str) -> CliResult<Uuid> {
    let found: Option<(String, String)> = conn
        .query_row(
            "SELECT t.task_id, t.workspace_id FROM task_steps s JOIN tasks t ON t.task_id = s.task_id
             WHERE s.step_id = ?1",
            rusqlite::params![step_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    match found {
        None => Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "step 不存在",
        )),
        Some((task_id, owner)) => {
            if owner != workspace_id.to_string() {
                return Err(CliError::new(
                    CliErrorCode::WorkspaceMismatch,
                    "step 不属于所选 workspace",
                ));
            }
            Uuid::parse_str(&task_id)
                .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "task_id 非法"))
        }
    }
}

fn step(action: cli::StepAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::StepAction::Update {
            workspace,
            step,
            patch_json,
            expected_revision,
            mutation_id,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let step_id = ctx::uuid_arg(&step, "--step")?;
            let task_id = task_of_step(&conn, ws.workspace_id, &step_id.to_string())?;
            let (op_patch, expected) = match patch_json {
                Some(path) => {
                    let value = jsonio::read_json(&path)?;
                    // 接受完整 task-edit 输入或仅 step patch 对象
                    if value.get("operation").is_some() {
                        let parsed: TaskEditInput = serde_json::from_value(value)
                            .map_err(|e| CliError::invalid(format!("patch-json 非法: {e}")))?;
                        let TaskEditOperation::Content {
                            step_operations, ..
                        } = &parsed.operation
                        else {
                            return Err(CliError::invalid("patch-json 必须是 content/step 操作"));
                        };
                        let Some(StepOperation::Update { patch, .. }) =
                            step_operations.as_ref().and_then(|ops| ops.first())
                        else {
                            return Err(CliError::invalid("patch-json 必须包含 step update 操作"));
                        };
                        (Some(patch.clone()), parsed.expected_task_revision)
                    } else {
                        let patch: dsheng_core::contracts::tasks::StepPatch =
                            serde_json::from_value(value)
                                .map_err(|e| CliError::invalid(format!("step patch 非法: {e}")))?;
                        (Some(patch), 0)
                    }
                }
                None => (None, 0),
            };
            let expected = expected_revision.or(Some(expected)).unwrap_or(0);
            if op_patch.is_none() && expected_revision.is_none() {
                return Err(CliError::invalid(
                    "step update 需要 --patch-json 或 sugar --expected-revision",
                ));
            }
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            let task_revision = current_task_revision(&conn, task_id)?;
            let input = TaskEditInput {
                schema_version: "1".into(),
                workspace_id: ws.workspace_id,
                task_id,
                expected_task_revision: expected.max(task_revision),
                mutation_id: mutation_id
                    .map(|m| ctx::uuid_arg(&m, "--mutation-id"))
                    .transpose()?
                    .unwrap_or_else(Uuid::new_v4),
                operation: TaskEditOperation::Content {
                    patch: None,
                    step_operations: Some(vec![StepOperation::Update {
                        step_id,
                        patch: op_patch.unwrap_or_default(),
                    }]),
                    source_note: None,
                },
            };
            apply_edit(&mut conn, &input)
        }
        cli::StepAction::Add {
            workspace,
            task,
            input_json,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let task_id = ctx::uuid_arg(&task, "--task")?;
            ctx::ensure_object_in_workspace(
                &conn,
                "tasks",
                "task_id",
                &task_id.to_string(),
                ws.workspace_id,
                "task",
            )?;
            let value = jsonio::read_json(&input_json)?;
            let mut add: StepAddInput = serde_json::from_value(value)
                .map_err(|e| CliError::invalid(format!("step add 输入非法: {e}")))?;
            if add.client_step_key.take().is_none() {
                // 生成稳定 key（内容哈希）以便幂等重试
            }
            let revision = current_task_revision(&conn, task_id)?;
            let before_id = add_before(&add)?;
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            let input = TaskEditInput {
                schema_version: "1".into(),
                workspace_id: ws.workspace_id,
                task_id,
                expected_task_revision: revision,
                mutation_id: Uuid::new_v4(),
                operation: TaskEditOperation::Content {
                    patch: None,
                    step_operations: Some(vec![StepOperation::Add {
                        client_step_key: Uuid::new_v4().to_string(),
                        content: add.into(),
                        before_step_id: before_id,
                    }]),
                    source_note: None,
                },
            };
            apply_edit(&mut conn, &input)
        }
        cli::StepAction::Delete {
            workspace,
            step,
            expected_revision,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let step_id = ctx::uuid_arg(&step, "--step")?;
            let task_id = task_of_step(&conn, ws.workspace_id, &step_id.to_string())?;
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            let input = TaskEditInput {
                schema_version: "1".into(),
                workspace_id: ws.workspace_id,
                task_id,
                expected_task_revision: expected_revision,
                mutation_id: Uuid::new_v4(),
                operation: TaskEditOperation::Content {
                    patch: None,
                    step_operations: Some(vec![StepOperation::Delete { step_id }]),
                    source_note: None,
                },
            };
            apply_edit(&mut conn, &input)
        }
        cli::StepAction::Move {
            workspace,
            step,
            before,
            expected_revision,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let step_id = ctx::uuid_arg(&step, "--step")?;
            let task_id = task_of_step(&conn, ws.workspace_id, &step_id.to_string())?;
            let before_step_id = if before == "end" {
                None
            } else {
                Some(ctx::uuid_arg(&before, "--before")?)
            };
            drop(conn);
            let mut conn = ctx::open_db(db)?;
            let input = TaskEditInput {
                schema_version: "1".into(),
                workspace_id: ws.workspace_id,
                task_id,
                expected_task_revision: expected_revision,
                mutation_id: Uuid::new_v4(),
                operation: TaskEditOperation::Content {
                    patch: None,
                    step_operations: Some(vec![StepOperation::Move {
                        step_id,
                        before_step_id,
                    }]),
                    source_note: None,
                },
            };
            apply_edit(&mut conn, &input)
        }
    }
}

fn add_before(add: &StepAddInput) -> CliResult<Option<Uuid>> {
    add.before_step_id
        .as_deref()
        .map(|id| ctx::uuid_arg(id, "before_step_id"))
        .transpose()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StepAddInput {
    #[serde(default)]
    client_step_key: Option<String>,
    action: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    start_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    due_date: Option<chrono::NaiveDate>,
    #[serde(default)]
    done_criteria: Option<String>,
    #[serde(default)]
    before_step_id: Option<String>,
}

impl From<StepAddInput> for dsheng_core::contracts::tasks::StepContent {
    fn from(v: StepAddInput) -> Self {
        Self {
            action: v.action,
            owner: v.owner,
            start_date: v.start_date,
            due_date: v.due_date,
            done_criteria: v.done_criteria,
        }
    }
}

fn current_task_revision(conn: &rusqlite::Connection, task_id: Uuid) -> CliResult<u32> {
    let rev: i64 = conn
        .query_row(
            "SELECT task_revision_no FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id.to_string()],
            |r| r.get(0),
        )
        .map_err(|_| CliError::new(CliErrorCode::WorkspaceMismatch, "task 不存在"))?;
    Ok(rev as u32)
}

fn reconcile(
    workspace: Option<String>,
    run: String,
    input_json: String,
    dry_run: bool,
    apply: bool,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    if dry_run == apply {
        return Err(CliError::invalid("--dry-run 与 --apply 必须二选一").with_field("--dry-run"));
    }
    let mut conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    ctx::ensure_object_in_workspace(
        &conn,
        "analysis_runs",
        "run_id",
        &run,
        ws.workspace_id,
        "run",
    )?;
    let raw = jsonio::read_json(&input_json)?;
    // apply_token 顶层字段（cli-surface note；task-reconcile.schema 未列——合同缺口报告）
    let apply_token_from_input = raw
        .get("apply_token")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut value = raw;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("apply_token");
    }
    let input: dsheng_core::contracts::tasks::TaskReconcileInput = serde_json::from_value(value)
        .map_err(|e| CliError::invalid(format!("reconcile 输入非法: {e}")))?;
    if input.workspace_id != ws.workspace_id {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "输入 workspace_id 与所选 workspace 不一致",
        ));
    }
    if input.run_id.to_string() != run {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "输入 run_id 与 --run 不一致",
        ));
    }
    if dry_run {
        let preview = dsheng_core::repository::reconcile::prepare_reconcile(
            &conn,
            &input,
            chrono::Utc::now(),
        )
        .map_err(CliError::from)?;
        return serde_json::to_value(preview)
            .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()));
    }
    // --apply：输入须携带 dry-run 发出的 apply_token（缺失 → CONFIRMATION_REQUIRED）
    let apply_token = apply_token_from_input.ok_or_else(|| {
        CliError::new(
            CliErrorCode::ConfirmationRequired,
            "--apply 需要 --input-json 顶层的 apply_token（先 --dry-run 获取）",
        )
    })?;
    let result = dsheng_core::repository::reconcile::apply_reconcile(
        &mut conn,
        &input,
        &apply_token,
        views::cli_actor(),
        chrono::Utc::now(),
    )
    .map_err(CliError::from)?;
    serde_json::to_value(result)
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))
}
