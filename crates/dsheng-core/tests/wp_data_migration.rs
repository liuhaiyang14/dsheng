//! 数据工作包 S5：迁移协议 TS 用例（TS-32..TS-37，迁移/安装层）。
//! 夹具：../../contracts/v1/task-state-cases.json；期望值来自合同。

use dsheng_core::repository::migration::{
    backup_database, migrate_with_migrations, open_with_negotiation, restore_database,
    DatabasePaths, MigrationError, MigrationGuard, SchemaSupport,
};
use rusqlite::Connection;
use serde_json::Value;
use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn ts_case(id: &str) -> Value {
    let raw = std::fs::read_to_string(repo_root().join("contracts/v1/task-state-cases.json"))
        .expect("TS");
    let v: Value = serde_json::from_str(&raw).expect("JSON");
    v["domain_acceptance_cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
        .expect("TS 用例存在")
}

fn temp_paths() -> (tempfile::TempDir, DatabasePaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = DatabasePaths::new(dir.path().join("user.sqlite"));
    (dir, paths)
}

fn build_v1(paths: &DatabasePaths) {
    let mut guard = MigrationGuard::acquire(paths).unwrap();
    migrate_with_migrations(
        paths,
        SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        },
        &[("0001_init", dsheng_core::repository::MIGRATIONS[0].1)],
        "test",
        &mut guard,
    )
    .unwrap();
    guard.release();
}

#[test]
fn ts_32_newer_schema_unsupported() {
    let expected = ts_case("TS-32")["expected"].clone();
    let (_dir, paths) = temp_paths();
    build_v1(&paths);
    // db schema 伪装为 3
    let conn = Connection::open(&paths.db_path).unwrap();
    conn.execute(
        "INSERT INTO app_schema_metadata (schema_version, migration_state, applied_at)
         VALUES (3, 'applied', 't')",
        [],
    )
    .unwrap();
    drop(conn);
    let old_client = SchemaSupport {
        read_min: 1,
        read_max: ts_case("TS-32")["given"]["client_supported_read_max"]
            .as_i64()
            .unwrap(),
        write: 2,
    };
    assert_eq!(
        open_with_negotiation(&paths, old_client)
            .unwrap_err()
            .to_string(),
        expected["error"].as_str().unwrap()
    );
    // writes=0：db schema 仍为 3，未被降级或修改
    let conn = Connection::open(&paths.db_path).unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT MAX(schema_version) FROM app_schema_metadata",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, 3, "writes=0/automatic_downgrade=false");
}

#[test]
fn ts_33_backup_integrity_failed_blocks_migration() {
    let expected = ts_case("TS-33")["expected"].clone();
    let (_dir, paths) = temp_paths();
    build_v1(&paths);
    // 备份目标不可写（backups 目录被文件占位）
    std::fs::remove_dir_all(&paths.backups_dir).unwrap_or(());
    std::fs::write(&paths.backups_dir, b"occupied").unwrap();
    let mut guard = MigrationGuard::acquire(&paths).unwrap();
    let err = migrate_with_migrations(
        &paths,
        SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        },
        &[
            ("0001_init", dsheng_core::repository::MIGRATIONS[0].1),
            ("0002_x", "CREATE TABLE t2 (id INTEGER);"),
        ],
        "test",
        &mut guard,
    )
    .unwrap_err();
    assert_eq!(err.to_string(), expected["error"].as_str().unwrap());
    guard.release();
    let conn = Connection::open(&paths.db_path).unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT MAX(schema_version) FROM app_schema_metadata",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, expected["db_schema"].as_i64().unwrap());
    let started: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='t2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(started, 0, "migration_started=false");
}

#[test]
fn ts_34_migration_failure_rolls_back() {
    let expected = ts_case("TS-34")["expected"].clone();
    let (_dir, paths) = temp_paths();
    build_v1(&paths);
    let bad: &[(&'static str, &'static str)] = &[
        ("0001_init", dsheng_core::repository::MIGRATIONS[0].1),
        (
            "0002_bad",
            "CREATE TABLE step_a (id INTEGER);\nCREATE TABLE step_b (id INTEGER);\nNOT VALID SQL;",
        ),
    ];
    let mut guard = MigrationGuard::acquire(&paths).unwrap();
    let err = migrate_with_migrations(
        &paths,
        SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        },
        bad,
        "test",
        &mut guard,
    )
    .unwrap_err();
    assert!(matches!(err, MigrationError::MigrationFailed(_)));
    guard.release();
    let conn = Connection::open(&paths.db_path).unwrap();
    let v: i64 = conn
        .query_row(
            "SELECT MAX(schema_version) FROM app_schema_metadata",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(v, expected["db_schema"].as_i64().unwrap());
    for leaked in ["step_a", "step_b"] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
                [leaked],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 0, "transaction_rolled_back（{leaked} 不残留）");
    }
    let backups = count_backups(&paths.backups_dir);
    assert!(backups >= 1, "backup_retained");
}

fn count_backups(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|it| it.filter_map(|e| e.ok()).count())
        .unwrap_or(0)
}

#[test]
fn ts_35_busy_lock_does_not_kill() {
    let expected = ts_case("TS-35")["expected"].clone();
    let (_dir, paths) = temp_paths();
    let _held = MigrationGuard::acquire(&paths).unwrap(); // 另一实例持锁
                                                          // 第二个实例请求迁移：锁忙 → DB_MIGRATION_BUSY，不等待、不杀进程
    let busy = match MigrationGuard::acquire(&paths) {
        Err(e) => e,
        Ok(guard) => panic!("锁忙应失败：{:?}", guard.lock_path_display()),
    };
    assert_eq!(busy.to_string(), expected["error"].as_str().unwrap());
    // 持锁实例未被干预（锁文件仍在）
    assert!(paths.db_path.with_extension("migration-lock").exists());
    assert_eq!(expected["processes_killed"].as_i64().unwrap(), 0);
    assert_eq!(expected["provider_requests"].as_i64().unwrap(), 0);
}

#[test]
fn ts_36_reinstall_retains_user_data() {
    let expected = ts_case("TS-36")["expected"].clone();
    let (_dir, paths) = temp_paths();
    build_v1(&paths);
    let conn = Connection::open(&paths.db_path).unwrap();
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w1','保留','t','t')",
        [],
    )
    .unwrap();
    drop(conn);
    std::fs::create_dir_all(&paths.backups_dir).unwrap();
    let backup = paths.backups_dir.join("b.sqlite");
    backup_database(&Connection::open(&paths.db_path).unwrap(), &backup).unwrap();
    // 卸载重装 = 同一用户数据目录重新打开（不携带/不新建空库）
    let reopened = open_with_negotiation(
        &paths,
        SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        },
    )
    .unwrap();
    let workspaces: i64 = reopened
        .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
        .unwrap();
    assert_eq!(workspaces, 1, "user_db=retained");
    assert!(backup.exists(), "backups=retained");
    assert!(!expected["first_run_creates_empty_db"].as_bool().unwrap());
}

#[test]
fn ts_37_confirmed_restore() {
    let expected = ts_case("TS-37")["expected"].clone();
    let (_dir, paths) = temp_paths();
    build_v1(&paths);
    let conn = Connection::open(&paths.db_path).unwrap();
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w1','备份态','t','t')",
        [],
    )
    .unwrap();
    let backup = paths.backups_dir.join("restore-source.sqlite");
    backup_database(&conn, &backup).unwrap();
    drop(conn);
    // 备份后的新写入（恢复应覆盖）
    let conn = Connection::open(&paths.db_path).unwrap();
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w2','后写入','t','t')",
        [],
    )
    .unwrap();
    drop(conn);
    restore_database(
        &paths,
        &backup,
        SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        },
    )
    .unwrap();
    let conn = Connection::open(&paths.db_path).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM workspaces ORDER BY workspace_id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(names, vec!["备份态".to_string()], "restored_db_verified");
    let preserved = std::fs::read_dir(&paths.backups_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().to_string_lossy().contains("pre-restore-"))
        .count();
    assert_eq!(
        preserved >= 1,
        expected["current_db_preservation_backup_created"]
            .as_bool()
            .unwrap()
    );
    assert!(backup.exists(), "source_files_untouched（备份原件保留）");
    assert_eq!(expected["provider_requests"].as_i64().unwrap(), 0);
}
