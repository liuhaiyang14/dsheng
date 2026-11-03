//! S1 桌面数据目录解析（docs/specs/provider-config-delivery.md §5）。
//!
//! application identifier 固定 `com.dsheng.desktop`（与 tauri.conf.json 一致）；
//! 基目录用系统 API 解析（macOS `~/Library/Application Support`、
//! Windows `%LOCALAPPDATA%` Known Folder、Linux XDG），不依赖 cwd/解压路径/
//! 程序文件名；缺失即报 `DATA_DIRECTORY_UNAVAILABLE`，不回退 cwd。
//!
//! 目录创建与数据库打开复用 [`dsheng_cli::ctx::open_db_at`]（CLI `--db`
//! 同一实现：创建失败 → `DATA_DIRECTORY_UNAVAILABLE`，版本协商/首启迁移一致）。
//! CLI 与桌面共享同一 SQLite 的方式：CLI 侧 `--db <桌面库路径>` 或
//! `DSHENG_DB`（dev/测试注入优先级相同，见 [`DB_OVERRIDE_ENV`]）。

use dsheng_cli::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 桌面 application identifier（合同固定值；与 tauri.conf.json 一致）。
pub const APP_IDENTIFIER: &str = "com.dsheng.desktop";

/// SQLite 文件名（与 CLI 默认一致）。
pub const DB_FILE_NAME: &str = "dsheng.sqlite3";

/// dev/测试显式库路径注入（与 CLI 的 `DSHENG_DB` 同名同义；优先于平台目录）。
pub const DB_OVERRIDE_ENV: &str = "DSHENG_DB";

/// 平台用户数据基目录（系统 API；macOS=Application Support、
/// Windows=LocalAppData Known Folder、Linux=XDG data home）。不回退 cwd。
pub fn platform_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
}

/// 纯函数：基目录 → 数据目录（`<base>/com.dsheng.desktop`）。
pub fn data_dir_under(base: &Path) -> PathBuf {
    base.join(APP_IDENTIFIER)
}

/// 纯函数：显式注入 > 平台目录 + `<identifier>/dsheng.sqlite3`；
/// 基目录缺失 → `DATA_DIRECTORY_UNAVAILABLE`。
pub fn resolve_db_path(explicit: Option<OsString>, base: Option<PathBuf>) -> CliResult<PathBuf> {
    if let Some(value) = explicit {
        if !value.is_empty() {
            return Ok(PathBuf::from(value));
        }
    }
    let base = base.ok_or_else(|| {
        CliError::new(
            CliErrorCode::DataDirectoryUnavailable,
            "无法解析用户数据目录（系统 Known Folder/XDG 缺失；不回退 cwd）",
        )
    })?;
    Ok(data_dir_under(&base).join(DB_FILE_NAME))
}

/// 桌面数据库路径：`DSHENG_DB` 显式注入 > 平台数据目录。
pub fn desktop_db_path() -> CliResult<PathBuf> {
    resolve_db_path(std::env::var_os(DB_OVERRIDE_ENV), platform_data_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s1_explicit_override_wins() {
        let path = resolve_db_path(
            Some(OsString::from("/tmp/injected/dsheng.sqlite3")),
            Some(PathBuf::from("/base")),
        )
        .expect("显式路径优先");
        assert_eq!(path, PathBuf::from("/tmp/injected/dsheng.sqlite3"));
    }

    #[test]
    fn s1_platform_dir_shape_per_contract() {
        let path =
            resolve_db_path(None, Some(PathBuf::from("/UserDataBase"))).expect("平台目录解析");
        assert_eq!(
            path,
            PathBuf::from("/UserDataBase/com.dsheng.desktop/dsheng.sqlite3")
        );
    }

    #[test]
    fn s1_missing_base_dir_is_data_directory_unavailable() {
        let err = resolve_db_path(None, None).expect_err("基目录缺失必须报错");
        assert_eq!(err.code, CliErrorCode::DataDirectoryUnavailable);
    }

    #[test]
    fn s1_identifier_matches_tauri_conf() {
        let conf = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
            .expect("tauri.conf.json 可读");
        assert!(
            conf.contains(&format!("\"identifier\": \"{APP_IDENTIFIER}\"")),
            "tauri.conf.json identifier 必须与合同一致"
        );
    }
}
