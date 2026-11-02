//! 历史删除：预览（依赖阻断 + 10 分钟 preview token）与确认（最终检查 +
//! 单事务物理删除 + local_mutations 墓碑化；task-lifecycle.md §8）。

use super::task_store::load_task;
use super::tasks::{mutation_digest, TaskServiceError};
use crate::repository::utc_iso;
use chrono::{DateTime, Duration, Utc};
use rusqlite::{params, Connection};
use serde_json::json;
use uuid::Uuid;

/// 删除预览有效期（§8：10 分钟）。
pub const DELETE_PREVIEW_VALID_MINUTES: i64 = 10;

/// 删除范围计数。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeleteScope {
    pub run_revisions: usize,
    pub period_links: usize,
    pub snapshots: usize,
    pub candidates: usize,
    pub candidate_applications: usize,
    pub reports: usize,
    pub sessions: usize,
    pub deletable_tasks: usize,
    pub deletable_steps: usize,
    pub deletable_task_revisions: usize,
    pub deletable_events: usize,
}

/// 预览结果。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DeletePreview {
    pub delete_preview_id: String,
    pub run_id: String,
    pub blocked: Option<String>,
    pub scope: DeleteScope,
    pub dependency_digest: String,
    pub expires_at: String,
    /// 说明：从当前数据库删除，不清除既有备份/导出（§8）。
    pub backup_warning: String,
}

pub const BACKUP_WARNING: &str = "从当前数据库删除，不清除既有备份/导出";

/// 依赖集合摘要：run 当前 revision + 阻断/可删对象计数（变化则预览失效）。
fn dependency_digest(
    conn: &Connection,
    workspace_id: &str,
    run_id: &Uuid,
) -> Result<String, TaskServiceError> {
    let (current_rev, attempts, active_tasks, external_refs, superseded_refs): (
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = conn
        .query_row(
            "SELECT
               (SELECT current_revision_no FROM analysis_runs WHERE run_id = ?2 AND workspace_id = ?1),
               (SELECT COUNT(*) FROM generation_attempts a
                  JOIN generation_sessions s ON s.session_id = a.session_id
                  WHERE s.run_id = ?2 AND a.terminal_state IS NULL),
               (SELECT COUNT(*) FROM task_period_links l JOIN tasks t ON t.task_id = l.task_id
                  WHERE l.run_id = ?2 AND t.lifecycle = 'active' AND t.status != 'completed'),
               (SELECT COUNT(*) FROM run_revisions r
                  WHERE r.run_id != ?2 AND r.comparison_base_refs_json LIKE '%' || ?2 || '%'),
               (SELECT COUNT(*) FROM tasks t
                  WHERE t.origin_run_id = ?2 AND t.lifecycle = 'superseded'
                    AND EXISTS (SELECT 1 FROM tasks u
                                WHERE u.superseded_by_task_id = t.task_id AND u.origin_run_id != ?2))",
            params![workspace_id, run_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(TaskServiceError::from)?;
    let value = json!({
        "active_attempts": attempts,
        "active_tasks": active_tasks,
        "current_revision": current_rev,
        "external_refs": external_refs,
        "superseded_refs": superseded_refs,
    });
    Ok(mutation_digest(&value))
}

/// 阻断检查（§8）：活动请求 / 未完成任务 / 跨 run 结构化引用。
fn blocked_reason(
    conn: &Connection,
    workspace_id: &str,
    run_id: &Uuid,
) -> Result<Option<&'static str>, TaskServiceError> {
    let run_ok: Option<String> = conn
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
    if run_ok.as_deref() != Some(workspace_id) {
        return Err(TaskServiceError::WorkspaceMismatch);
    }
    let active_attempts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM generation_attempts a
             JOIN generation_sessions s ON s.session_id = a.session_id
             WHERE s.run_id = ?1 AND a.terminal_state IS NULL",
            params![run_id.to_string()],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    if active_attempts > 0 {
        return Ok(Some("DELETE_DEPENDENCY_BLOCKED:active_generation_attempt"));
    }
    let active_tasks: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks t
             WHERE t.lifecycle = 'active' AND t.status != 'completed'
               AND (t.origin_run_id = ?1 OR EXISTS (
                    SELECT 1 FROM task_period_links l WHERE l.task_id = t.task_id AND l.run_id = ?1))",
            params![run_id.to_string()],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    if active_tasks > 0 {
        return Ok(Some("DELETE_DEPENDENCY_BLOCKED:active_task"));
    }
    let external_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM run_revisions r
             WHERE r.run_id != ?1 AND r.comparison_base_refs_json LIKE '%' || ?1 || '%'",
            params![run_id.to_string()],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    if external_refs > 0 {
        return Ok(Some("DELETE_DEPENDENCY_BLOCKED:cross_run_reference"));
    }
    let superseded_refs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tasks t
             WHERE t.origin_run_id = ?1 AND t.lifecycle = 'superseded'
               AND EXISTS (SELECT 1 FROM tasks u
                           WHERE u.superseded_by_task_id = t.task_id AND u.origin_run_id != ?1)",
            params![run_id.to_string()],
            |r| r.get(0),
        )
        .map_err(TaskServiceError::from)?;
    if superseded_refs > 0 {
        return Ok(Some("DELETE_DEPENDENCY_BLOCKED:superseded_reference"));
    }
    Ok(None)
}

/// 可随 run 删除的任务：只属于目标 run、已完成或已替代、无存活外部引用。
fn deletable_task_ids(conn: &Connection, run_id: &Uuid) -> Result<Vec<String>, TaskServiceError> {
    let mut stmt = conn
        .prepare(
            "SELECT t.task_id FROM tasks t
             WHERE (t.origin_run_id = ?1 OR EXISTS (
                    SELECT 1 FROM task_period_links l WHERE l.task_id = t.task_id AND l.run_id = ?1))
               AND (t.lifecycle = 'superseded' OR t.status = 'completed')
               AND NOT EXISTS (SELECT 1 FROM task_period_links l2
                               WHERE l2.task_id = t.task_id AND l2.run_id != ?1)
               AND NOT EXISTS (SELECT 1 FROM tasks u
                               WHERE u.superseded_by_task_id = t.task_id AND u.origin_run_id != ?1)",
        )
        .map_err(TaskServiceError::from)?;
    let ids = stmt
        .query_map(params![run_id.to_string()], |r| r.get(0))
        .map_err(TaskServiceError::from)?
        .collect::<Result<Vec<String>, _>>()
        .map_err(TaskServiceError::from)?;
    Ok(ids)
}

fn count_scope(conn: &Connection, run_id: &Uuid) -> Result<DeleteScope, TaskServiceError> {
    let count = |sql: &str, p: Vec<String>| -> Result<usize, TaskServiceError> {
        let n: i64 = conn
            .query_row(sql, rusqlite::params_from_iter(p), |r| r.get(0))
            .map_err(TaskServiceError::from)?;
        Ok(n as usize)
    };
    let rid = run_id.to_string();
    let tasks = deletable_task_ids(conn, run_id)?;
    let mut scope = DeleteScope {
        run_revisions: count(
            "SELECT COUNT(*) FROM run_revisions WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        period_links: count(
            "SELECT COUNT(*) FROM task_period_links WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        snapshots: count(
            "SELECT COUNT(*) FROM run_task_snapshots WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        candidates: count(
            "SELECT COUNT(*) FROM task_candidates WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        candidate_applications: count(
            "SELECT COUNT(*) FROM task_candidate_applications a
             JOIN task_candidates c ON c.candidate_generation_id = a.candidate_generation_id
             WHERE c.run_id = ?1",
            vec![rid.clone()],
        )?,
        reports: count(
            "SELECT COUNT(*) FROM diagnosis_reports WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        sessions: count(
            "SELECT COUNT(*) FROM generation_sessions WHERE run_id = ?1",
            vec![rid.clone()],
        )?,
        deletable_tasks: tasks.len(),
        deletable_steps: count(
            "SELECT COUNT(*) FROM task_steps WHERE task_id IN (SELECT json_each.value FROM json_each(?1))",
            vec![serde_json::to_string(&tasks).expect("json")],
        )?,
        deletable_task_revisions: 0,
        deletable_events: 0,
    };
    for tid in &tasks {
        let uuid = Uuid::parse_str(tid).map_err(|_| TaskServiceError::Db("uuid".into()))?;
        scope.deletable_task_revisions += count(
            "SELECT COUNT(*) FROM task_revisions WHERE task_id = ?1",
            vec![tid.clone()],
        )?;
        scope.deletable_events += count(
            "SELECT COUNT(*) FROM task_events WHERE task_id = ?1",
            vec![tid.clone()],
        )?;
        let _ = uuid;
    }
    Ok(scope)
}

/// 预览：返回 delete_preview_id/范围/阻断依赖（不删除任何记录）。
pub fn history_delete_preview(
    conn: &Connection,
    workspace_id: &str,
    run_id: &Uuid,
    now: DateTime<Utc>,
) -> Result<DeletePreview, TaskServiceError> {
    let blocked = blocked_reason(conn, workspace_id, run_id)?;
    let digest = dependency_digest(conn, workspace_id, run_id)?;
    let scope = if blocked.is_none() {
        count_scope(conn, run_id)?
    } else {
        DeleteScope::default()
    };
    let preview_id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO local_action_tokens
           (token_id, kind, workspace_id, binding_digest, created_at, expires_at)
         VALUES (?1, 'history_delete', ?2, ?3, ?4, ?5)",
        params![
            preview_id,
            workspace_id,
            digest,
            utc_iso(now),
            utc_iso(now + Duration::minutes(DELETE_PREVIEW_VALID_MINUTES)),
        ],
    )
    .map_err(TaskServiceError::from)?;
    Ok(DeletePreview {
        delete_preview_id: preview_id,
        run_id: run_id.to_string(),
        blocked: blocked.map(str::to_string),
        scope,
        dependency_digest: digest,
        expires_at: utc_iso(now + Duration::minutes(DELETE_PREVIEW_VALID_MINUTES)),
        backup_warning: BACKUP_WARNING.to_string(),
    })
}

/// 确认删除：单事务内最终依赖检查 + 物理删除 + local_mutations 墓碑化。
pub fn history_delete_confirm(
    conn: &mut Connection,
    workspace_id: &str,
    run_id: &Uuid,
    delete_preview_id: &str,
    now: DateTime<Utc>,
) -> Result<DeleteScope, TaskServiceError> {
    let tx = conn.transaction().map_err(TaskServiceError::from)?;
    // token 校验（过期/绑定变化 → 预览失效）
    let row: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT binding_digest, expires_at, consumed_at FROM local_action_tokens
             WHERE token_id = ?1 AND kind = 'history_delete'",
            params![delete_preview_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(TaskServiceError::from)?;
    let Some((binding, expires_at, consumed)) = row else {
        return Err(TaskServiceError::Db(
            "DELETE_PREVIEW_STALE:token_missing".into(),
        ));
    };
    if consumed.is_some() {
        return Err(TaskServiceError::Db("DELETE_PREVIEW_STALE:consumed".into()));
    }
    let expires = DateTime::parse_from_rfc3339(&expires_at)
        .map_err(|_| TaskServiceError::Db("token_expiry".into()))?
        .with_timezone(&Utc);
    if now > expires {
        return Err(TaskServiceError::Db("DELETE_PREVIEW_STALE:expired".into()));
    }
    // 先核对预览绑定（依赖集合变化 → DELETE_PREVIEW_STALE，TS-31），
    // 再做删除前一次最终依赖检查（同事务）。
    let current_digest = dependency_digest(&tx, workspace_id, run_id)?;
    if current_digest != binding {
        return Err(TaskServiceError::Db(
            "DELETE_PREVIEW_STALE:dependency_changed".into(),
        ));
    }
    if let Some(reason) = blocked_reason(&tx, workspace_id, run_id)? {
        return Err(TaskServiceError::Db(reason.to_string()));
    }
    let scope = count_scope(&tx, run_id)?;
    let tasks = deletable_task_ids(&tx, run_id)?;
    // 先删 run 的 period links（FK：task_period_links.task_id → tasks）
    tx.execute(
        "DELETE FROM task_period_links WHERE run_id = ?1",
        params![run_id.to_string()],
    )
    .map_err(TaskServiceError::from)?;
    // 物理删除：run-local 数据
    for table in [
        "DELETE FROM task_step_evidence WHERE step_id IN (SELECT step_id FROM task_steps WHERE task_id IN (SELECT value FROM json_each(?1)))",
        "DELETE FROM task_evidence WHERE task_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM task_events WHERE task_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM task_revisions WHERE task_id IN (SELECT value FROM json_each(?1))",
        "DELETE FROM task_steps WHERE task_id IN (SELECT value FROM json_each(?1))",
    ] {
        tx.execute(table, params![serde_json::to_string(&tasks).expect("json")])
            .map_err(TaskServiceError::from)?;
    }
    tx.execute(
        "DELETE FROM tasks WHERE task_id IN (SELECT value FROM json_each(?1))",
        params![serde_json::to_string(&tasks).expect("json")],
    )
    .map_err(TaskServiceError::from)?;
    for sql in [
        "DELETE FROM task_candidate_applications WHERE candidate_generation_id IN
            (SELECT candidate_generation_id FROM task_candidates WHERE run_id = ?1)",
        "DELETE FROM task_candidates WHERE run_id = ?1",
        "DELETE FROM run_task_snapshots WHERE run_id = ?1",
        "DELETE FROM diagnosis_turns WHERE session_id IN
            (SELECT session_id FROM generation_sessions WHERE run_id = ?1)",
        "DELETE FROM diagnosis_reports WHERE run_id = ?1",
        "DELETE FROM generation_attempts WHERE session_id IN
            (SELECT session_id FROM generation_sessions WHERE run_id = ?1)",
        "DELETE FROM generation_sessions WHERE run_id = ?1",
        "DELETE FROM run_revisions WHERE run_id = ?1",
        "DELETE FROM analysis_runs WHERE run_id = ?1",
    ] {
        tx.execute(sql, params![run_id.to_string()])
            .map_err(TaskServiceError::from)?;
    }
    // local_mutations：结果清除、保留最小防重放元数据（§8）
    // （LIKE 为预筛；affected_*_json 为 JSON 数组，元素为 UUID 字符串）
    for tid in &tasks {
        tx.execute(
            "UPDATE local_mutations SET result_json = NULL, result_deleted = 1
             WHERE workspace_id = ?1 AND affected_task_ids_json LIKE ?2",
            params![workspace_id, format!("%{tid}%")],
        )
        .map_err(TaskServiceError::from)?;
    }
    tx.execute(
        "UPDATE local_mutations SET result_json = NULL, result_deleted = 1
         WHERE workspace_id = ?1 AND affected_run_ids_json LIKE ?2",
        params![workspace_id, format!("%{}%", run_id)],
    )
    .map_err(TaskServiceError::from)?;
    tx.execute(
        "UPDATE local_action_tokens SET consumed_at = ?2 WHERE token_id = ?1",
        params![delete_preview_id, utc_iso(now)],
    )
    .map_err(TaskServiceError::from)?;
    tx.commit().map_err(TaskServiceError::from)?;
    Ok(scope)
}

/// TS 用例辅助：读取任务的 workspace（跨 workspace 校验）。
pub fn task_workspace_or_mismatch(
    conn: &Connection,
    request_workspace: &str,
    task_id: &Uuid,
) -> Result<(), TaskServiceError> {
    let _ = load_task(conn, request_workspace, *task_id)?;
    Ok(())
}
