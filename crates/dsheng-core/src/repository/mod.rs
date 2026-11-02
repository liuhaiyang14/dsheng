//! SQLite repository：持久化、事务、迁移、任务生命周期（数据与 SQLite 工作包所有权）。
//!
//! 合同：docs/specs/task-lifecycle.md §7（表/索引）、§8（删除）、§9（升级恢复）；
//! migrations/0001_init.sql 为 Phase 0 冻结基线。数据工作包实现 repository 层；
//! 修改冻结唯一键/事务语义须经主 Agent 批准并以新 migration 版本提交。
//!
//! - [`analysis_runs`]：period_key / 策略版本化 / 不可变 run revision / 输入指纹；
//! - [`tasks`]：任务内容/进度/period_link 编辑、状态机、事件、快照、幂等；
//! - [`reconcile`]：候选合并 dry-run/apply 与 apply_token；
//! - [`history_delete`]：run 历史删除预览/确认；
//! - [`migration`]：schema 版本协商、一致性备份、排他锁、单事务迁移。
//!
//! Phase 0 提供：嵌入 migration 清单 + 最小应用器（供测试/首启建库骨架）；
//! 完整迁移协议在 [`migration`]。

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::Connection;

pub mod analysis_runs;
pub mod history_delete;
pub mod reconcile;
pub mod task_store;
pub mod tasks;

/// repository 层错误码（CLI cli-surface 词法对齐）。
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// expected revision 与当前不一致（lifecycle §1）。
    #[error("REVISION_CONFLICT")]
    RevisionConflict,
    /// 跨 workspace 访问（lifecycle §1）。
    #[error("WORKSPACE_MISMATCH")]
    WorkspaceMismatch,
    /// 策略版本 [from,to) 区间重叠（data-period §3.1）。
    #[error("POLICY_OVERLAP_REJECTED")]
    PolicyOverlapRejected,
    /// 分析范围跨策略版本且未显式选择。
    #[error("POLICY_SELECTION_REQUIRED")]
    PolicySelectionRequired,
    /// 同 mutation_id 不同 digest。
    #[error("IDEMPOTENCY_CONFLICT")]
    IdempotencyConflict,
    /// 已删除 mutation 的结果重放。
    #[error("MUTATION_RESULT_DELETED")]
    MutationResultDeleted,
    /// 底层 SQLite 错误。
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    /// 任务服务层错误（跨模块传递时保留词法）。
    #[error("{0}")]
    TaskService(String),
}

/// UTC ISO-8601 毫秒（时间戳存储格式）。
pub fn utc_iso(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// 测试/引导用：确保 workspace 存在。
pub fn ensure_workspace(
    conn: &Connection,
    workspace_id: &str,
    name: &str,
) -> Result<(), RepoError> {
    conn.execute(
        "INSERT OR IGNORE INTO workspaces (workspace_id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)",
        rusqlite::params![workspace_id, name, utc_iso(Utc::now())],
    )?;
    Ok(())
}

/// 嵌入 migration 清单（顺序即版本；发布包含同一清单）。
pub const MIGRATIONS: &[(&str, &str)] =
    &[("0001_init", include_str!("../../migrations/0001_init.sql"))];

/// 当前 schema 版本（= MIGRATIONS 长度）。
pub const CURRENT_SCHEMA_VERSION: i64 = MIGRATIONS.len() as i64;

/// 最小应用器：空库按序执行全部 migration 并写入 app_schema_metadata。
///
/// 注意：这不是完整迁移协议（无备份/锁/协商）；生产入口由数据工作包提供。
pub fn apply_migrations(conn: &Connection, app_version: &str) -> rusqlite::Result<i64> {
    for (idx, (_name, sql)) in MIGRATIONS.iter().enumerate() {
        let target = (idx + 1) as i64;
        let done: Option<i64> = conn
            .query_row(
                "SELECT schema_version FROM app_schema_metadata ORDER BY schema_version DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .ok();
        if done.map(|v| v >= target).unwrap_or(false) {
            continue;
        }
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT INTO app_schema_metadata (schema_version, migration_state, last_migrated_by_app_version, applied_at)
             VALUES (?1, 'applied', ?2, strftime('%Y-%m-%dT%H:%M:%fZ','now'))",
            rusqlite::params![target, app_version],
        )?;
    }
    Ok(CURRENT_SCHEMA_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_on_empty_db() {
        let conn = Connection::open_in_memory().expect("内存库");
        let v = apply_migrations(&conn, env!("CARGO_PKG_VERSION")).expect("迁移成功");
        assert_eq!(v, CURRENT_SCHEMA_VERSION);
        // 关键唯一约束存在性抽样
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN
                 ('workspaces','providers','analysis_runs','run_revisions','tasks','task_steps',
                  'task_revisions','task_period_links','task_events','run_task_snapshots',
                  'task_candidates','task_candidate_applications','local_mutations',
                  'confirmations','generation_sessions','generation_attempts',
                  'diagnosis_reports','diagnosis_turns','app_schema_metadata')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 19, "冻结表全部建立");
        // 同 workspace + period_key 唯一
        conn.execute(
            "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w1','测试','t','t')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO analysis_runs (run_id, workspace_id, period_key, analysis_input_fingerprint, created_at, updated_at)
             VALUES ('r1','w1','pk1','fp','t','t')",
            [],
        )
        .unwrap();
        let dup = conn.execute(
            "INSERT INTO analysis_runs (run_id, workspace_id, period_key, analysis_input_fingerprint, created_at, updated_at)
             VALUES ('r2','w1','pk1','fp2','t','t')",
            [],
        );
        assert!(dup.is_err(), "同 workspace+period_key 必须唯一");
    }
}
