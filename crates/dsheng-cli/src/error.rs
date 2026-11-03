//! CLI 错误类型：携带 envelope 所需的 code/message/field/details，
//! 并把 core 服务层错误映射到 cli-surface.json 错误目录（不改 core）。

use dsheng_core::contracts::cli::{CliErrorCode, ErrorEnvelope};
use dsheng_core::repository::migration::MigrationError;
use dsheng_core::repository::tasks::TaskServiceError;
use dsheng_core::repository::RepoError;

/// CLI 执行错误（单条命令内失败；退出码取 code）。
#[derive(Debug, Clone)]
pub struct CliError {
    pub code: CliErrorCode,
    pub message: String,
    pub field: Option<String>,
    pub details: serde_json::Value,
}

impl CliError {
    pub fn new(code: CliErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
            details: serde_json::Value::Null,
        }
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = details;
        self
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(CliErrorCode::InvalidArgument, message)
    }

    pub fn envelope(&self) -> ErrorEnvelope {
        ErrorEnvelope {
            schema_version: dsheng_core::contracts::cli::RESPONSE_SCHEMA_VERSION,
            ok: false,
            error: dsheng_core::contracts::cli::ErrorBody {
                code: self.code,
                message: self.message.clone(),
                field: self.field.clone(),
                details: self.details.clone(),
            },
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.code.exit_code()
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for CliError {}

impl From<RepoError> for CliError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::RevisionConflict => CliError::new(
                CliErrorCode::RevisionConflict,
                "expected revision 与当前不一致",
            ),
            RepoError::WorkspaceMismatch => {
                CliError::new(CliErrorCode::WorkspaceMismatch, "对象不属于所选 workspace")
            }
            RepoError::PolicyOverlapRejected => CliError::new(
                CliErrorCode::DataValidationFailed,
                "策略版本 [from,to) 区间与现存版本重叠",
            ),
            RepoError::PolicySelectionRequired => CliError::new(
                CliErrorCode::ContextInvalid,
                "分析范围跨策略版本且未显式选择（--policy）",
            )
            .with_field("policy"),
            RepoError::IdempotencyConflict => CliError::new(
                CliErrorCode::IdempotencyConflict,
                "同 mutation_id 不同请求内容",
            ),
            RepoError::MutationResultDeleted => CliError::new(
                CliErrorCode::MutationResultDeleted,
                "幂等结果已随业务对象删除",
            ),
            RepoError::Db(inner) => CliError::new(CliErrorCode::LocalSaveFailed, inner.to_string()),
            RepoError::TaskService(inner) => CliError::from_task_service_text(&inner),
        }
    }
}

impl CliError {
    /// TaskService 以 String 透传的错误按词法映射（core 未导出全部变体构造）。
    pub fn from_task_service_text(text: &str) -> Self {
        let known: &[(&str, CliErrorCode, &str)] = &[
            (
                "REVISION_CONFLICT",
                CliErrorCode::RevisionConflict,
                "expected revision 与当前不一致",
            ),
            (
                "WORKSPACE_MISMATCH",
                CliErrorCode::WorkspaceMismatch,
                "对象不属于所选 workspace",
            ),
            (
                "STATE_PROGRESS_MISMATCH",
                CliErrorCode::StateProgressMismatch,
                "任务状态与进度不一致",
            ),
            (
                "STEP_STILL_BLOCKED",
                CliErrorCode::StepStillBlocked,
                "存在未解决阻塞步骤",
            ),
            (
                "AT_LEAST_ONE_STEP",
                CliErrorCode::AtLeastOneStep,
                "任务必须保留至少一个有效步骤",
            ),
            (
                "TASK_CATEGORY_METRIC_MISMATCH",
                CliErrorCode::TaskCategoryMetricMismatch,
                "非 other 类别必须绑定有效指标",
            ),
            (
                "TASK_CATEGORY_UNRECOGNIZED",
                CliErrorCode::TaskCategoryUnrecognized,
                "未知任务类别",
            ),
            (
                "STEP_OUTSIDE_TASK_PERIOD",
                CliErrorCode::StepOutsideTaskPeriod,
                "步骤日期超出任务目标期",
            ),
            (
                "IDEMPOTENCY_CONFLICT",
                CliErrorCode::IdempotencyConflict,
                "同 mutation_id 不同请求内容",
            ),
            (
                "MUTATION_RESULT_DELETED",
                CliErrorCode::MutationResultDeleted,
                "幂等结果已随业务对象删除",
            ),
            (
                "INPUT_REVISION_CHANGED",
                CliErrorCode::InputRevisionChanged,
                "分析/报告/输入 revision 已变化",
            ),
            (
                "STALE_DIAGNOSIS_REFERENCE",
                CliErrorCode::StaleDiagnosisReference,
                "诊断引用与当前 revision 不匹配",
            ),
            (
                "DUPLICATE_CANDIDATE_KEY",
                CliErrorCode::DuplicateCandidateKey,
                "候选任务键重复",
            ),
            (
                "DUPLICATE_RECONCILE_TARGET",
                CliErrorCode::DuplicateReconcileTarget,
                "多候选指向同一持久任务",
            ),
            (
                "DELETE_DEPENDENCY_BLOCKED",
                CliErrorCode::DeleteDependencyBlocked,
                "删除被存活依赖阻断",
            ),
            (
                "DELETE_PREVIEW_STALE",
                CliErrorCode::DeletePreviewStale,
                "删除依赖在预览后已变化",
            ),
        ];
        for (lex, code, msg) in known {
            if text.starts_with(lex) {
                return CliError::new(*code, *msg).with_details(serde_json::json!({"lex": lex}));
            }
        }
        CliError::new(CliErrorCode::LocalSaveFailed, text.to_string())
    }
}

impl From<TaskServiceError> for CliError {
    fn from(e: TaskServiceError) -> Self {
        // thiserror Display 输出词法码（REVISION_CONFLICT 等），统一走词法映射。
        CliError::from_task_service_text(&e.to_string())
    }
}

impl From<MigrationError> for CliError {
    fn from(e: MigrationError) -> Self {
        match e {
            MigrationError::SchemaUnsupported => CliError::new(
                CliErrorCode::DbSchemaUnsupported,
                "客户端不支持该数据库版本",
            ),
            MigrationError::MigrationBusy => {
                CliError::new(CliErrorCode::DbMigrationBusy, "迁移或活动写入者阻止操作")
            }
            MigrationError::MigrationFailed(_) => {
                CliError::new(CliErrorCode::DbMigrationFailed, "迁移失败；原数据库受保护")
            }
            MigrationError::BackupFailed => {
                CliError::new(CliErrorCode::DbBackupFailed, "一致性备份失败；不迁移")
            }
            MigrationError::IntegrityFailed => {
                CliError::new(CliErrorCode::DbIntegrityFailed, "数据库完整性检查失败")
            }
            MigrationError::RestoreFailed(_) => {
                CliError::new(CliErrorCode::DbRestoreFailed, "本地恢复失败")
            }
            MigrationError::Io(inner) => CliError::new(
                CliErrorCode::DataDirectoryUnavailable,
                format!("无法解析/创建/打开数据目录: {inner}"),
            ),
        }
    }
}

impl From<rusqlite::Error> for CliError {
    fn from(e: rusqlite::Error) -> Self {
        CliError::new(CliErrorCode::LocalSaveFailed, format!("db error: {e}"))
    }
}

/// 便捷 Result 别名。
pub type CliResult<T> = Result<T, CliError>;
