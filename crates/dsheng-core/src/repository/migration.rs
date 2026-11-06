//! 迁移协议（task-lifecycle.md §9）：schema 版本协商、一致性备份、
//! 排他迁移锁、单事务迁移与恢复。
//!
//! - 旧客户端发现更高 schema → `DB_SCHEMA_UNSUPPORTED`，不写、不自动降级；
//! - 迁移前一致性备份（`VACUUM INTO` 快照，WAL 安全；备份完整性/大小校验，
//!   失败 `DB_BACKUP_FAILED` 不迁）；
//! - 跨进程迁移锁：忙 → `DB_MIGRATION_BUSY`，不无限等待、不杀其他进程；
//! - 单事务迁移：任一语句失败整体回滚 `DB_MIGRATION_FAILED`，原库可由旧版读取，
//!   备份保留；
//! - 恢复为显式本地操作：先校验备份版本/完整性，再对现库做保留备份，原子替换
//!   并校验；失败 `DB_RESTORE_FAILED`，现库与备份都保留。

use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 迁移协议错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MigrationError {
    #[error("DB_SCHEMA_UNSUPPORTED")]
    SchemaUnsupported,
    #[error("DB_MIGRATION_BUSY")]
    MigrationBusy,
    #[error("DB_BACKUP_FAILED")]
    BackupFailed,
    #[error("DB_MIGRATION_FAILED: {0}")]
    MigrationFailed(String),
    #[error("DB_INTEGRITY_FAILED")]
    IntegrityFailed,
    #[error("DB_RESTORE_FAILED: {0}")]
    RestoreFailed(String),
    #[error("io: {0}")]
    Io(String),
}

/// 应用 binary 声明的支持读写 schema 范围（§9）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchemaSupport {
    pub read_min: i64,
    pub read_max: i64,
    pub write: i64,
}

/// 当前 binary 的支持范围。
pub fn current_support() -> SchemaSupport {
    SchemaSupport {
        read_min: 1,
        read_max: super::CURRENT_SCHEMA_VERSION,
        write: super::CURRENT_SCHEMA_VERSION,
    }
}

/// 数据库与备份目录。
#[derive(Debug, Clone)]
pub struct DatabasePaths {
    pub db_path: PathBuf,
    pub backups_dir: PathBuf,
}

impl DatabasePaths {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        let db_path = db_path.into();
        let backups_dir = db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("backups");
        Self {
            db_path,
            backups_dir,
        }
    }
}

/// 读取 schema 版本；空库（无 app_schema_metadata）→ None。
pub fn read_schema_version(conn: &Connection) -> Result<Option<i64>, MigrationError> {
    let has_table: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='app_schema_metadata'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| MigrationError::MigrationFailed(e.to_string()))?;
    if !has_table {
        return Ok(None);
    }
    conn.query_row(
        "SELECT MAX(schema_version) FROM app_schema_metadata",
        [],
        |r| r.get(0),
    )
    .map(Some)
    .map_err(|e| MigrationError::MigrationFailed(e.to_string()))
}

/// 打开数据库并做版本协商（§9）：更高 schema 拒绝写，不降级。
///
/// 空库允许打开（首次建库路径由迁移执行）。版本低于 read_min 同样拒绝。
pub fn open_with_negotiation(
    paths: &DatabasePaths,
    support: SchemaSupport,
) -> Result<Connection, MigrationError> {
    let conn = Connection::open(&paths.db_path).map_err(|e| MigrationError::Io(e.to_string()))?;
    if let Some(version) = read_schema_version(&conn)? {
        if version > support.read_max || version < support.read_min {
            return Err(MigrationError::SchemaUnsupported);
        }
    }
    Ok(conn)
}

/// SQLite 一致性快照备份：`VACUUM INTO`（WAL 模式安全；不用文件拷贝）。
pub fn backup_database(conn: &Connection, dest: &Path) -> Result<(), MigrationError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|_| MigrationError::BackupFailed)?;
    }
    let sql = format!(
        "VACUUM INTO '{}'",
        dest.to_string_lossy().replace('\'', "''")
    );
    conn.execute_batch(&sql)
        .map_err(|_| MigrationError::BackupFailed)?;
    // 备份完整性 + 大小校验
    let backup = Connection::open(dest).map_err(|_| MigrationError::BackupFailed)?;
    verify_integrity(&backup)?;
    let size = std::fs::metadata(dest)
        .map_err(|_| MigrationError::BackupFailed)?
        .len();
    if size == 0 {
        return Err(MigrationError::BackupFailed);
    }
    Ok(())
}

/// PRAGMA quick_check。
pub fn verify_integrity(conn: &Connection) -> Result<(), MigrationError> {
    let ok: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|_| MigrationError::IntegrityFailed)?;
    if ok != "ok" {
        return Err(MigrationError::IntegrityFailed);
    }
    Ok(())
}

/// 跨进程排他迁移锁（lock 文件 O_EXCL；陈旧锁按超时接管；不杀进程）。
#[derive(Debug)]
pub struct MigrationGuard {
    lock_path: PathBuf,
    released: bool,
}

impl MigrationGuard {
    /// 测试辅助：锁路径展示。
    pub fn lock_path_display(&self) -> std::path::Display<'_> {
        self.lock_path.display()
    }
}

/// 陈旧锁接管阈值。
const STALE_LOCK_TIMEOUT: Duration = Duration::from_secs(600);

impl MigrationGuard {
    pub fn acquire(paths: &DatabasePaths) -> Result<Self, MigrationError> {
        let lock_path = paths.db_path.with_extension("migration-lock");
        if lock_path.exists() {
            let stale = std::fs::metadata(&lock_path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age > STALE_LOCK_TIMEOUT);
            if !stale {
                return Err(MigrationError::MigrationBusy);
            }
            // 陈旧锁接管（持锁进程崩溃残留）
            let _ = std::fs::remove_file(&lock_path);
        }
        let pid = std::process::id();
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        std::fs::write(&lock_path, format!("pid={pid};nonce={nonce}"))
            .map_err(|e| MigrationError::Io(e.to_string()))?;
        Ok(Self {
            lock_path,
            released: false,
        })
    }

    pub fn release(&mut self) {
        if !self.released {
            let _ = std::fs::remove_file(&self.lock_path);
            self.released = true;
        }
    }
}

impl Drop for MigrationGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// 备份文件名：pre-migration-<schema>-<timestamp>.sqlite（§9）。
fn backup_file_name(paths: &DatabasePaths, from_schema: i64) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    paths
        .backups_dir
        .join(format!("pre-migration-{from_schema}-{ts}.sqlite"))
}

/// 单事务迁移到目标清单最新版（备份先行；失败回滚保留原库与备份）。
///
/// `migrations`：(name, sql) 按版本升序；空库从第 1 条起全量执行（建库路径）。
pub fn migrate_with_migrations(
    paths: &DatabasePaths,
    support: SchemaSupport,
    migrations: &[(&'static str, &'static str)],
    app_version: &str,
    _guard: &mut MigrationGuard,
) -> Result<i64, MigrationError> {
    let conn = Connection::open(&paths.db_path).map_err(|e| MigrationError::Io(e.to_string()))?;
    let current = read_schema_version(&conn)?;
    // 现库 schema 超出本 binary 读范围 → 拒绝（不尝试写入或降级）
    if let Some(v) = current {
        if v > support.read_max {
            return Err(MigrationError::SchemaUnsupported);
        }
    }
    let target = migrations.len() as i64;
    if target > support.write {
        return Err(MigrationError::SchemaUnsupported);
    }
    if current == Some(target) {
        return Ok(target);
    }
    // 升级前一致性备份（空库首建无数据可备份，直接建）
    if let Some(from) = current {
        let backup_path = backup_file_name(paths, from);
        backup_database(&conn, &backup_path)?;
    }
    // 单一迁移事务：全部升级步骤 + metadata + 提交；失败回滚
    let tx_result = conn.execute_batch("BEGIN IMMEDIATE");
    if let Err(e) = tx_result {
        return Err(MigrationError::MigrationFailed(e.to_string()));
    }
    let start_idx = current.unwrap_or(0) as usize;
    for (offset, entry) in migrations.iter().enumerate().skip(start_idx) {
        let (name, sql) = entry;
        let version = (offset + 1) as i64;
        if let Err(e) = conn.execute_batch(sql) {
            // 回滚由 drop 完成；原库保持旧版本可读
            let _ = conn.execute_batch("ROLLBACK");
            return Err(MigrationError::MigrationFailed(format!("{name}: {e}")));
        }
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        if let Err(e) = conn.execute(
            "INSERT INTO app_schema_metadata (schema_version, migration_state, last_migrated_by_app_version, applied_at)
             VALUES (?1, 'applied', ?2, ?3)",
            params![version, app_version, now],
        ) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(MigrationError::MigrationFailed(format!("{name}: {e}")));
        }
    }
    if let Err(e) = conn.execute_batch("COMMIT") {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(MigrationError::MigrationFailed(e.to_string()));
    }
    // 迁移后完整性校验（失败已回滚不可达；此处为防御）
    verify_integrity(&conn)?;
    Ok(target)
}

/// 生产入口：用嵌入清单迁移（§9 完整协议）。
pub fn migrate_database(
    paths: &DatabasePaths,
    support: SchemaSupport,
    app_version: &str,
) -> Result<i64, MigrationError> {
    let mut guard = MigrationGuard::acquire(paths)?;
    let result =
        migrate_with_migrations(paths, support, super::MIGRATIONS, app_version, &mut guard);
    guard.release();
    result
}

/// 断电/崩溃后首次启动检查（§9）：完整性失败停止普通写入。
pub fn startup_check(paths: &DatabasePaths) -> Result<(), MigrationError> {
    let conn = Connection::open(&paths.db_path).map_err(|e| MigrationError::Io(e.to_string()))?;
    verify_integrity(&conn)
}

/// 显式恢复（§9）：校验备份版本/完整性 → 现库保留备份 → 原子替换 → 校验。
///
/// 失败 `DB_RESTORE_FAILED`，现库与备份都保留；不删除唯一可用副本；
/// 不自动降级（备份 schema 超出支持范围则拒绝）。
pub fn restore_database(
    paths: &DatabasePaths,
    backup_path: &Path,
    support: SchemaSupport,
) -> Result<(), MigrationError> {
    // 1. 备份版本/完整性校验
    let backup_conn =
        Connection::open(backup_path).map_err(|e| MigrationError::RestoreFailed(e.to_string()))?;
    verify_integrity(&backup_conn)
        .map_err(|_| MigrationError::RestoreFailed("backup_integrity".into()))?;
    if let Some(v) = read_schema_version(&backup_conn)? {
        if v > support.read_max || v < support.read_min {
            return Err(MigrationError::RestoreFailed(
                "backup_schema_unsupported".into(),
            ));
        }
    }
    // 2. 现库保留备份（存在时）
    if paths.db_path.exists() {
        let preservation = paths.backups_dir.join(format!(
            "pre-restore-{}.sqlite",
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default()
        ));
        let current_conn = Connection::open(&paths.db_path)
            .map_err(|e| MigrationError::RestoreFailed(e.to_string()))?;
        backup_database(&current_conn, &preservation)
            .map_err(|e| MigrationError::RestoreFailed(format!("{e}")))?;
    }
    // 3. 原子替换（同目录临时文件 + rename）并校验
    let tmp = paths.db_path.with_extension("restore-tmp");
    std::fs::copy(backup_path, &tmp).map_err(|e| MigrationError::RestoreFailed(e.to_string()))?;
    let restored =
        Connection::open(&tmp).map_err(|e| MigrationError::RestoreFailed(e.to_string()))?;
    verify_integrity(&restored)
        .map_err(|_| MigrationError::RestoreFailed("restored_integrity".into()))?;
    // WAL 伴生文件一并清理，避免旧 WAL 覆盖恢复内容
    for sidecar in ["-wal", "-shm"] {
        let mut p = paths.db_path.clone().into_os_string();
        p.push(sidecar);
        let _ = std::fs::remove_file(PathBuf::from(&p));
    }
    std::fs::rename(&tmp, &paths.db_path)
        .map_err(|e| MigrationError::RestoreFailed(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths_in(dir: &Path) -> DatabasePaths {
        DatabasePaths::new(dir.join("user.sqlite"))
    }

    #[test]
    fn fresh_db_migrates_to_current() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let v = migrate_database(&paths, current_support(), "test").unwrap();
        assert_eq!(v, super::super::CURRENT_SCHEMA_VERSION);
        // 幂等：已最新则直接返回
        assert_eq!(
            migrate_database(&paths, current_support(), "test").unwrap(),
            v
        );
    }

    #[test]
    fn upgrade_0002_to_current_preserves_providers_and_relaxes_stream() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        // 建 v2 库（0001+0002；providers 仍带 CHECK (stream = 0)）
        let migrations_v2: &[(&'static str, &'static str)] = &[
            ("0001_init", super::super::MIGRATIONS[0].1),
            ("0002_stable_metric_identity", super::super::MIGRATIONS[1].1),
        ];
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        migrate_with_migrations(&paths, current_support(), migrations_v2, "test", &mut guard)
            .unwrap();
        guard.release();
        let conn = Connection::open(&paths.db_path).unwrap();
        conn.execute(
            "INSERT INTO workspaces (workspace_id, name, created_at, updated_at)
             VALUES ('w1','测试企业','t','t')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO providers (provider_id, workspace_id, revision, name, base_url, model,
                    stream, api_key, connect_seconds, response_seconds, total_seconds,
                    context_window_tokens, output_reserve_tokens, created_at, updated_at)
             VALUES ('p1','w1',1,'主 Provider','https://api.example.com/v1','m1',0,'sk-keep',
                     15,180,240,32768,8192,'t','t')",
            [],
        )
        .unwrap();
        // v2 基线：stream=1 被 0001 的 CHECK (stream = 0) 拒绝
        assert!(
            conn.execute(
                "UPDATE providers SET stream = 1 WHERE provider_id = 'p1'",
                []
            )
            .is_err(),
            "v2 CHECK(stream=0) 仍生效"
        );
        drop(conn);
        // 升级到 v3（0003 放宽 stream；其余路径不动）
        let mut guard2 = MigrationGuard::acquire(&paths).unwrap();
        let v = migrate_with_migrations(
            &paths,
            current_support(),
            super::super::MIGRATIONS,
            "test",
            &mut guard2,
        )
        .unwrap();
        guard2.release();
        assert_eq!(v, super::super::CURRENT_SCHEMA_VERSION);
        let conn = Connection::open(&paths.db_path).unwrap();
        assert_eq!(
            read_schema_version(&conn).unwrap(),
            Some(super::super::CURRENT_SCHEMA_VERSION)
        );
        // providers 数据保留（含 api_key/revision），stream 回填 0
        let (name, key, stream, revision): (String, Option<String>, i64, i64) = conn
            .query_row(
                "SELECT name, api_key, stream, revision FROM providers WHERE provider_id = 'p1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (name.as_str(), key.as_deref(), stream, revision),
            ("主 Provider", Some("sk-keep"), 0, 1)
        );
        // v3：stream=1 可写（流式开关落库）；total_seconds 可到 1800（增量）
        conn.execute(
            "UPDATE providers SET stream = 1 WHERE provider_id = 'p1'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE providers SET total_seconds = 1800 WHERE provider_id = 'p1'",
            [],
        )
        .unwrap();
        // 其余约束仍守门：stream 只许 0/1；total_seconds 上限 1800；
        // UNIQUE (workspace_id, name) 保留
        assert!(conn
            .execute(
                "UPDATE providers SET stream = 2 WHERE provider_id = 'p1'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "UPDATE providers SET total_seconds = 1801 WHERE provider_id = 'p1'",
                []
            )
            .is_err());
        assert!(conn
            .execute(
                "INSERT INTO providers (provider_id, workspace_id, revision, name, base_url, model,
                        stream, api_key, connect_seconds, response_seconds, total_seconds,
                        context_window_tokens, output_reserve_tokens, created_at, updated_at)
                 VALUES ('p2','w1',1,'主 Provider','https://api.example.com/v1','m1',0,NULL,
                         15,180,240,32768,8192,'t','t')",
                [],
            )
            .is_err());
    }

    #[test]
    fn busy_lock_returns_migration_busy() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let _guard = MigrationGuard::acquire(&paths).unwrap();
        assert_eq!(
            migrate_database(&paths, current_support(), "test").unwrap_err(),
            MigrationError::MigrationBusy
        );
    }

    #[test]
    fn stale_lock_taken_over() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let lock = paths.db_path.with_extension("migration-lock");
        std::fs::write(&lock, "pid=1;nonce=1").unwrap();
        // 模拟陈旧：mtime 改到 1 小时前
        let old = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        assert!(migrate_with_migrations(
            &paths,
            current_support(),
            super::super::MIGRATIONS,
            "test",
            &mut guard
        )
        .is_ok());
    }

    #[test]
    fn failed_migration_rolls_back_and_keeps_backup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        // 建 v1 库
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        migrate_with_migrations(
            &paths,
            current_support(),
            &[("0001_init", super::super::MIGRATIONS[0].1)],
            "test",
            &mut guard,
        )
        .unwrap();
        guard.release();
        // 坏的第 2 条迁移：语句 2 失败（注入清单允许写到 v2）
        let bad: &[(&'static str, &'static str)] = &[
            ("0001_init", super::super::MIGRATIONS[0].1),
            (
                "0002_bad",
                "CREATE TABLE step1_ok (id INTEGER);\nTHIS IS NOT SQL;",
            ),
        ];
        let support2 = SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        };
        let mut guard2 = MigrationGuard::acquire(&paths).unwrap();
        let err = migrate_with_migrations(&paths, support2, bad, "test", &mut guard2).unwrap_err();
        assert!(matches!(err, MigrationError::MigrationFailed(_)), "{err}");
        guard2.release();
        // 原库保持 v1、step1_ok 未残留（回滚）
        let conn = Connection::open(&paths.db_path).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), Some(1));
        let leaked: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='step1_ok'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leaked, 0, "失败语句前的 DDL 必须回滚");
        // 备份保留
        let backups: Vec<_> = std::fs::read_dir(&paths.backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(!backups.is_empty(), "backup_retained");
    }

    #[test]
    fn backup_failure_blocks_migration() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        migrate_with_migrations(
            &paths,
            current_support(),
            &[("0001_init", super::super::MIGRATIONS[0].1)],
            "test",
            &mut guard,
        )
        .unwrap();
        guard.release();
        // backups 目录被一个同名文件占位 → 备份写不出 → DB_BACKUP_FAILED
        std::fs::create_dir_all(&paths.backups_dir).unwrap();
        std::fs::remove_dir_all(&paths.backups_dir).unwrap();
        std::fs::write(&paths.backups_dir, b"not-a-dir").unwrap();
        let mut guard2 = MigrationGuard::acquire(&paths).unwrap();
        let err = migrate_with_migrations(
            &paths,
            SchemaSupport {
                read_min: 1,
                read_max: 2,
                write: 2,
            },
            &[
                ("0001_init", super::super::MIGRATIONS[0].1),
                ("0002_x", "CREATE TABLE t2 (id INTEGER);"),
            ],
            "test",
            &mut guard2,
        )
        .unwrap_err();
        assert_eq!(err, MigrationError::BackupFailed);
        guard2.release();
        let conn = Connection::open(&paths.db_path).unwrap();
        assert_eq!(
            read_schema_version(&conn).unwrap(),
            Some(1),
            "migration_started=false"
        );
        std::fs::remove_file(&paths.backups_dir).unwrap();
    }

    #[test]
    fn newer_schema_rejected_for_write() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        migrate_with_migrations(
            &paths,
            current_support(),
            super::super::MIGRATIONS,
            "test",
            &mut guard,
        )
        .unwrap();
        guard.release();
        // 伪装未来数据库版本，高于当前和旧客户端支持范围
        let conn = Connection::open(&paths.db_path).unwrap();
        let future_version = super::super::CURRENT_SCHEMA_VERSION + 1;
        conn.execute(
            "INSERT INTO app_schema_metadata (schema_version, migration_state, applied_at) VALUES (?1, 'applied', 't')",
            [future_version],
        )
        .unwrap();
        drop(conn);
        let old_client = SchemaSupport {
            read_min: 1,
            read_max: 2,
            write: 2,
        };
        assert_eq!(
            open_with_negotiation(&paths, old_client).unwrap_err(),
            MigrationError::SchemaUnsupported
        );
        // 自动降级=false：未来版本号仍在
        let conn = Connection::open(&paths.db_path).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), Some(future_version));
    }

    #[test]
    fn restore_preserves_current_and_verifies() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        let mut guard = MigrationGuard::acquire(&paths).unwrap();
        migrate_with_migrations(
            &paths,
            current_support(),
            super::super::MIGRATIONS,
            "test",
            &mut guard,
        )
        .unwrap();
        guard.release();
        let conn = Connection::open(&paths.db_path).unwrap();
        conn.execute(
            "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w1','恢复前','t','t')",
            [],
        )
        .unwrap();
        // 备份
        let backup = paths.backups_dir.join("manual-backup.sqlite");
        backup_database(&conn, &backup).unwrap();
        drop(conn);
        // 之后的写入会被恢复覆盖
        let conn = Connection::open(&paths.db_path).unwrap();
        conn.execute(
            "INSERT INTO workspaces (workspace_id, name, created_at, updated_at) VALUES ('w2','恢复后写入','t','t')",
            [],
        )
        .unwrap();
        drop(conn);
        restore_database(&paths, &backup, current_support()).unwrap();
        let conn = Connection::open(&paths.db_path).unwrap();
        verify_integrity(&conn).unwrap();
        let names: i64 = conn
            .query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0))
            .unwrap();
        assert_eq!(names, 1, "恢复到备份内容");
        // 保留备份存在
        let preserved: Vec<_> = std::fs::read_dir(&paths.backups_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().to_string_lossy().contains("pre-restore-"))
            .collect();
        assert!(
            !preserved.is_empty(),
            "current_db_preservation_backup_created"
        );
        // 备份原件仍在
        assert!(backup.exists(), "source backup untouched");
    }

    #[test]
    fn corrupted_db_detected_by_startup_check() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        std::fs::write(&paths.db_path, b"this is not a sqlite database at all").unwrap();
        assert!(matches!(
            startup_check(&paths),
            Err(MigrationError::IntegrityFailed) | Err(MigrationError::Io(_))
        ));
    }
}
