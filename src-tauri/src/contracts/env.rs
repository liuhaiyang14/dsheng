//! 桌面 command 执行环境：数据库路径解析结果 + 生成确认备忘。
//!
//! 无连接池（取舍）：每个 command 经 [`dsheng_cli::ctx::open_db_at`] 打开
//! 短连接——与 CLI 完全一致（版本协商/首启迁移/错误映射同一实现），
//! 避免 WAL 下常驻多连接的忙等待与迁移互斥问题；桌面命令为毫秒级
//! 本地 SQLite 操作，逐次打开的代价可接受。生成发送类命令阻塞在
//! async runtime 线程（#[tauri::command(async)]），不冻结主线程。
//!
//! 生成确认备忘（壳层状态，非业务数据）：UI 的 confirm* 只携带
//! confirmation_id，而服务层确认发送按 CLI 协议需以原始参数重建请求
//! 快照（逐项重验指纹后原子消费）。preview* 成功时把原始请求按
//! confirmation_id 记入进程内备忘；confirm* 取回重建。门禁本身（TTL/
//! 指纹重验/原子消费）全部仍在服务层；应用重启后备忘丢失 → 明确
//! INVALID_ARGUMENT（须重新预览）。Key 绝不进入备忘。

use crate::contracts::paths;
use dsheng_cli::error::{CliError, CliResult};
use dsheng_core::contracts::cli::CliErrorCode;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 备忘条目防过期清理上限（服务层确认 TTL 600s，留一倍余量）。
const MEMO_TTL: Duration = Duration::from_secs(1200);

/// 备忘的确认种类（confirm 命令按种类取回，跨种类复用 id → 拒绝）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    DiagnosisGenerate,
    DiagnosisContinue,
    TaskGenerate,
}

#[derive(Debug, Clone)]
pub struct PendingConfirmation {
    pub kind: ConfirmationKind,
    /// preview 时的完整原始请求（仅重建快照用；不含 Key）。
    pub request: Value,
    pub created_at: Instant,
}

type Memo = Arc<Mutex<HashMap<String, PendingConfirmation>>>;

/// 桌面执行环境（tauri State 管理；测试直接构造）。
#[derive(Clone, Default)]
pub struct DesktopEnv {
    db_path: Option<PathBuf>,
    /// 数据目录解析失败（基目录缺失等）：逐命令返回同一错误 envelope。
    db_unavailable: Option<Arc<CliError>>,
    memo: Memo,
}

impl DesktopEnv {
    /// 从 `DSHENG_DB`/平台数据目录解析（启动时调用一次）。
    pub fn resolve() -> Self {
        match paths::desktop_db_path() {
            Ok(path) => Self {
                db_path: Some(path),
                db_unavailable: None,
                memo: Default::default(),
            },
            Err(e) => Self {
                db_path: None,
                db_unavailable: Some(Arc::new(e)),
                memo: Default::default(),
            },
        }
    }

    /// 指定库路径构造（测试/临时目录注入）。
    pub fn with_db(path: impl Into<PathBuf>) -> Self {
        Self {
            db_path: Some(path.into()),
            db_unavailable: None,
            memo: Default::default(),
        }
    }

    /// 库路径（`--db` 字符串形式，供 CLI 命令层 dispatch）。
    pub fn db_arg(&self) -> CliResult<String> {
        if let Some(e) = &self.db_unavailable {
            return Err(CliError::new(e.code, e.message.clone()));
        }
        self.db_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .ok_or_else(|| {
                CliError::new(
                    CliErrorCode::DataDirectoryUnavailable,
                    "桌面数据库路径未解析",
                )
            })
    }

    /// preview 成功且返回 confirmation_id 时记录原始请求。
    pub fn remember_confirmation(&self, kind: ConfirmationKind, request: &Value, data: &Value) {
        let Some(id) = data.get("confirmation_id").and_then(Value::as_str) else {
            return;
        };
        let mut memo = self.memo.lock().expect("确认备忘锁");
        let now = Instant::now();
        memo.retain(|_, entry| now.duration_since(entry.created_at) < MEMO_TTL);
        memo.insert(
            id.to_string(),
            PendingConfirmation {
                kind,
                request: request.clone(),
                created_at: now,
            },
        );
    }

    /// confirm 时取回原始请求（种类必须匹配；未命中即拒绝，不静默重试）。
    pub fn lookup_confirmation(
        &self,
        kind: ConfirmationKind,
        confirmation_id: &str,
    ) -> CliResult<Value> {
        let memo = self.memo.lock().expect("确认备忘锁");
        match memo.get(confirmation_id) {
            None => Err(CliError::new(
                CliErrorCode::InvalidArgument,
                "confirmation_id 不在本会话预览记录中（应用重启或未预览；须重新预览）",
            )
            .with_field("confirmationId")),
            Some(entry) if entry.kind != kind => Err(CliError::new(
                CliErrorCode::InvalidArgument,
                "confirmation_id 属于其他生成操作（preview/confirm 种类不匹配）",
            )
            .with_field("confirmationId")),
            Some(entry) => Ok(entry.request.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn memo_roundtrip_and_kind_mismatch() {
        let env = DesktopEnv::with_db("/tmp/none.sqlite3");
        let request = json!({"problem": "p"});
        let data = json!({"confirmation_id": "11111111-1111-4111-8111-111111111111"});
        env.remember_confirmation(ConfirmationKind::TaskGenerate, &request, &data);
        let got = env
            .lookup_confirmation(
                ConfirmationKind::TaskGenerate,
                "11111111-1111-4111-8111-111111111111",
            )
            .expect("同种类取回");
        assert_eq!(got, request);
        let err = env
            .lookup_confirmation(
                ConfirmationKind::DiagnosisGenerate,
                "11111111-1111-4111-8111-111111111111",
            )
            .expect_err("跨种类必须拒绝");
        assert_eq!(err.code, CliErrorCode::InvalidArgument);
    }

    #[test]
    fn memo_miss_rejected() {
        let env = DesktopEnv::with_db("/tmp/none.sqlite3");
        let err = env
            .lookup_confirmation(
                ConfirmationKind::TaskGenerate,
                "00000000-0000-4000-8000-000000000000",
            )
            .expect_err("未预览的 confirm 必须拒绝");
        assert_eq!(err.code, CliErrorCode::InvalidArgument);
        assert_eq!(err.field.as_deref(), Some("confirmationId"));
    }

    #[test]
    fn unavailable_env_reports_data_directory_unavailable() {
        let err = DesktopEnv {
            db_path: None,
            db_unavailable: Some(Arc::new(CliError::new(
                CliErrorCode::DataDirectoryUnavailable,
                "无法解析用户数据目录（系统 Known Folder/XDG 缺失；不回退 cwd）",
            ))),
            memo: Default::default(),
        }
        .db_arg()
        .expect_err("解析失败必须传递");
        assert_eq!(err.code, CliErrorCode::DataDirectoryUnavailable);
    }
}
