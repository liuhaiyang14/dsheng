//! 核心缺口修复 F2：stable metric identity（migration 0002）。
//!
//! data-period-import §1.4：模型输入的 metric_id 引用工作区内稳定的定义身份，
//! 版本变化保留身份。0001 每版本新 metric_id；0002 为 metric_definitions 增加
//! stable_metric_id（回填 = 各 (workspace, metric_key) 首版行 metric_id，全部
//! 版本行共用），保留全部版本行——冻结引用（run_revisions 快照、
//! tasks.metric_id、manual_inputs.metric_id）继续指向具体版本行。

use dsheng_core::repository::{apply_migrations, MIGRATIONS};
use rusqlite::{params, Connection};

const WS: &str = "00000000-0000-4000-8000-0000000000aa";

/// 仅建 0001 库（schema_version=1），模拟旧库升级路径。
fn v1_conn() -> Connection {
    let conn = Connection::open_in_memory().expect("内存库");
    conn.execute_batch(MIGRATIONS[0].1).expect("0001");
    conn.execute(
        "INSERT INTO app_schema_metadata (schema_version, migration_state, applied_at)
         VALUES (1, 'applied', '2026-01-01T00:00:00Z')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at)
         VALUES (?1, '旧库', 't', 't')",
        params![WS],
    )
    .unwrap();
    conn
}

/// 0001 形态的指标行（无 stable_metric_id 列）。
fn insert_v1_metric(conn: &Connection, metric_id: &str, key: &str, version: i64) {
    conn.execute(
        "INSERT INTO metric_definitions (metric_id, workspace_id, metric_key, version,
                semantic_hash, label, semantic_type, unit_signature_json, direction,
                target_constraint_scope, aggregation, comparison_policy_json, effective_from,
                created_at)
         VALUES (?1, ?2, ?3, ?4, 'h', '指标', 'flow', '{}', 'up', 'none', 'sum_disjoint',
                 '{}', '2026-01-01', 't')",
        params![metric_id, WS, key, version],
    )
    .unwrap();
}

#[test]
fn fresh_db_is_schema_2_with_stable_identity_column() {
    let conn = Connection::open_in_memory().unwrap();
    let version = apply_migrations(&conn, "test").unwrap();
    assert_eq!(version, 2, "0002 后 schema 版本自动跟进");
    // stable_metric_id 列存在且 NOT NULL（重建表生效）
    let (not_null, dflt): (i64, Option<String>) = conn
        .query_row(
            "SELECT [notnull], [dflt_value] FROM pragma_table_info('metric_definitions')
             WHERE name = 'stable_metric_id'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(not_null, 1, "stable_metric_id NOT NULL");
    assert!(dflt.is_none(), "无默认值（写入必须显式提供身份）");
    // 身份查询索引存在
    let idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index'
             AND name='idx_metric_definitions_stable'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1);
    // 省略 stable_metric_id 的 INSERT 被拒（NOT NULL 无默认）
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at)
         VALUES ('w2', 'w', 't', 't')",
        [],
    )
    .unwrap();
    let err = conn.execute(
        "INSERT INTO metric_definitions (metric_id, workspace_id, metric_key, version,
                semantic_hash, label, semantic_type, unit_signature_json, direction,
                target_constraint_scope, aggregation, comparison_policy_json, effective_from,
                created_at)
         VALUES ('m1', 'w2', 'k', 1, 'h', 'l', 'flow', '{}', 'up', 'none', 'sum_disjoint',
                 '{}', '2026-01-01', 't')",
        [],
    );
    assert!(err.is_err(), "缺 stable_metric_id 必须被 NOT NULL 拒绝");
}

#[test]
fn upgrade_0001_to_0002_merges_versions_into_first_version_id() {
    let conn = v1_conn();
    // key_a 三版本 + key_b 单版本（首版 id 即自身）
    insert_v1_metric(&conn, "a-v1", "key_a", 1);
    insert_v1_metric(&conn, "a-v2", "key_a", 2);
    insert_v1_metric(&conn, "a-v3", "key_a", 3);
    insert_v1_metric(&conn, "b-v1", "key_b", 1);
    // 升级：apply_migrations 只补 0002（0001 已在 metadata 标记）
    let version = apply_migrations(&conn, "test").unwrap();
    assert_eq!(version, 2);
    // 全部版本行保留（冻结引用不变）
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM metric_definitions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 4, "同键旧多版本行保留、不折叠");
    // 同键全部版本行共用首版 metric_id 作为 stable_metric_id
    let stable_a: Vec<(String, String)> = conn
        .prepare(
            "SELECT metric_id, stable_metric_id FROM metric_definitions
             WHERE metric_key = 'key_a' ORDER BY version",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stable_a.len(), 3);
    let mut row_ids: Vec<&String> = stable_a.iter().map(|(id, _)| id).collect();
    row_ids.sort();
    assert_eq!(
        row_ids,
        vec!["a-v1", "a-v2", "a-v3"],
        "版本行自身 id 保留原值"
    );
    for (_, stable) in &stable_a {
        assert_eq!(
            stable, "a-v1",
            "同键全部版本行合并为同一 stable id（首版行 id）"
        );
    }
    let stable_b: String = conn
        .query_row(
            "SELECT stable_metric_id FROM metric_definitions WHERE metric_key = 'key_b'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stable_b, "b-v1", "单版本键 stable id = 首版行 id");
    assert_ne!(stable_b, "a-v1", "跨键身份互不相同");
    // 0001 唯一约束保留（workspace+key+version）
    let err = conn.execute(
        "INSERT INTO metric_definitions (metric_id, workspace_id, metric_key, version,
                stable_metric_id, semantic_hash, label, semantic_type, unit_signature_json,
                direction, target_constraint_scope, aggregation, comparison_policy_json,
                effective_from, created_at)
         VALUES ('dup', ?1, 'key_a', 3, 'a-v1', 'h', 'l', 'flow', '{}', 'up', 'none',
                 'sum_disjoint', '{}', '2026-01-01', 't')",
        params![WS],
    );
    assert!(err.is_err(), "UNIQUE(workspace, metric_key, version) 保留");
    // 跨键稳定身份可用索引检索（跨版本检索入口）
    let a_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM metric_definitions
             WHERE workspace_id = ?1 AND stable_metric_id = 'a-v1'",
            params![WS],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(a_rows, 3, "按 stable id 跨版本检索命中全部版本行");
}

#[test]
fn upgrade_preserves_frozen_version_row_references() {
    let conn = v1_conn();
    insert_v1_metric(&conn, "a-v1", "key_a", 1);
    insert_v1_metric(&conn, "a-v2", "key_a", 2);
    // 冻结引用：任务/手工输入指向 v2 具体版本行
    conn.execute(
        "INSERT INTO tasks (task_id, workspace_id, metric_id, title, category, status, progress,
                progress_mode, smart_status, created_at, updated_at)
         VALUES ('t1', ?1, 'a-v2', '旧任务', 'other', 'not_started', '0', 'auto', 'generated',
                 't', 't')",
        params![WS],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO task_steps (step_id, task_id, step_order, action, status, created_at, updated_at)
         VALUES ('s1', 't1', 1, '步骤', 'not_started', 't', 't')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO manual_inputs (input_id, workspace_id, metric_id, period_key, measure_role,
                scope_key, confirmed, source_note, input_revision, raw_text, created_at, updated_at)
         VALUES ('mi1', ?1, 'a-v2', '2026-10', 'actual', '全公司', 1, 'n', 1, 'r', 't', 't')",
        params![WS],
    )
    .unwrap();
    apply_migrations(&conn, "test").unwrap();
    // 迁移不重写引用列；版本行仍在，引用可解析
    let task_metric: String = conn
        .query_row(
            "SELECT metric_id FROM tasks WHERE task_id = 't1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(task_metric, "a-v2", "task.metric_id 原样保留");
    let v2_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM metric_definitions WHERE metric_id = 'a-v2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v2_exists, 1, "被引用的版本行保留");
    // 手工输入行 id 经 JOIN 解析到稳定身份（CLI 分析路径语义）
    let stable: String = conn
        .query_row(
            "SELECT md.stable_metric_id FROM manual_inputs mi
             JOIN metric_definitions md ON md.metric_id = mi.metric_id
             WHERE mi.input_id = 'mi1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(stable, "a-v1", "v2 引用归并到首版稳定身份");
}
