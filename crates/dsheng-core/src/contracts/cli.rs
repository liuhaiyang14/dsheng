//! CLI 错误目录与响应外层（cli-surface.json response_contract + errors）。
//!
//! 退出码：成功 0；参数/配置/确认/版本冲突 2；数据/本地存储 3；
//! Provider/发送后失败 4；导出 5。`--json` 的 stdout 只含一个最终 JSON envelope。

use serde::{Deserialize, Serialize};

/// JSON envelope schema_version（固定 "1"）。
pub const RESPONSE_SCHEMA_VERSION: &str = "1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CliErrorCode {
    // ---- exit 2：参数/配置/确认/版本冲突 ----
    #[serde(rename = "INVALID_ARGUMENT")]
    InvalidArgument,
    #[serde(rename = "WORKSPACE_REQUIRED")]
    WorkspaceRequired,
    #[serde(rename = "WORKSPACE_MISMATCH")]
    WorkspaceMismatch,
    #[serde(rename = "REVISION_CONFLICT")]
    RevisionConflict,
    #[serde(rename = "PROVIDER_REQUIRED")]
    ProviderRequired,
    #[serde(rename = "PROVIDER_KEY_MISSING")]
    ProviderKeyMissing,
    #[serde(rename = "PROVIDER_URL_INVALID")]
    ProviderUrlInvalid,
    #[serde(rename = "PROVIDER_IN_USE")]
    ProviderInUse,
    #[serde(rename = "CONFIRMATION_REQUIRED")]
    ConfirmationRequired,
    #[serde(rename = "CONFIRMATION_STALE")]
    ConfirmationStale,
    #[serde(rename = "CONFIRMATION_CANCELLED")]
    ConfirmationCancelled,
    #[serde(rename = "REQUEST_TOO_LARGE")]
    RequestTooLarge,
    #[serde(rename = "USER_MESSAGE_TOO_LARGE")]
    UserMessageTooLarge,
    #[serde(rename = "CONTEXT_BUDGET_EXCEEDED")]
    ContextBudgetExceeded,
    #[serde(rename = "INVALID_TIMEOUT_BUDGET")]
    InvalidTimeoutBudget,
    #[serde(rename = "INVALID_CONTEXT_BUDGET")]
    InvalidContextBudget,
    #[serde(rename = "RUNTIME_MISSING")]
    RuntimeMissing,
    #[serde(rename = "INPUT_REVISION_CHANGED")]
    InputRevisionChanged,
    #[serde(rename = "IDEMPOTENCY_CONFLICT")]
    IdempotencyConflict,
    #[serde(rename = "STALE_DIAGNOSIS_REFERENCE")]
    StaleDiagnosisReference,
    #[serde(rename = "CONTEXT_INVALID")]
    ContextInvalid,
    #[serde(rename = "SCOPE_SELECTION_REQUIRED")]
    ScopeSelectionRequired,
    #[serde(rename = "CLARIFICATION_RESTATEMENT_REQUIRED")]
    ClarificationRestatementRequired,
    #[serde(rename = "MUTATION_RESULT_DELETED")]
    MutationResultDeleted,
    #[serde(rename = "DELETE_PREVIEW_STALE")]
    DeletePreviewStale,
    #[serde(rename = "INPUT_CONTRACT_INVALID")]
    InputContractInvalid,
    // ---- exit 3：数据/本地存储 ----
    #[serde(rename = "DATA_VALIDATION_FAILED")]
    DataValidationFailed,
    #[serde(rename = "DATA_DIRECTORY_UNAVAILABLE")]
    DataDirectoryUnavailable,
    #[serde(rename = "LOCAL_SAVE_FAILED")]
    LocalSaveFailed,
    #[serde(rename = "SOURCE_CHANGED")]
    SourceChanged,
    #[serde(rename = "DB_SCHEMA_UNSUPPORTED")]
    DbSchemaUnsupported,
    #[serde(rename = "DB_MIGRATION_BUSY")]
    DbMigrationBusy,
    #[serde(rename = "DB_MIGRATION_FAILED")]
    DbMigrationFailed,
    #[serde(rename = "DELETE_DEPENDENCY_BLOCKED")]
    DeleteDependencyBlocked,
    #[serde(rename = "UNIT_MISMATCH")]
    UnitMismatch,
    #[serde(rename = "STEP_STILL_BLOCKED")]
    StepStillBlocked,
    #[serde(rename = "DB_BACKUP_FAILED")]
    DbBackupFailed,
    #[serde(rename = "DB_INTEGRITY_FAILED")]
    DbIntegrityFailed,
    #[serde(rename = "DB_RESTORE_FAILED")]
    DbRestoreFailed,
    #[serde(rename = "TASK_CATEGORY_METRIC_MISMATCH")]
    TaskCategoryMetricMismatch,
    #[serde(rename = "TASK_CATEGORY_UNRECOGNIZED")]
    TaskCategoryUnrecognized,
    #[serde(rename = "DUPLICATE_CANDIDATE_KEY")]
    DuplicateCandidateKey,
    #[serde(rename = "DUPLICATE_RECONCILE_TARGET")]
    DuplicateReconcileTarget,
    #[serde(rename = "STATE_PROGRESS_MISMATCH")]
    StateProgressMismatch,
    #[serde(rename = "STEP_OUTSIDE_TASK_PERIOD")]
    StepOutsideTaskPeriod,
    #[serde(rename = "AT_LEAST_ONE_STEP")]
    AtLeastOneStep,
    #[serde(rename = "SOURCE_NOTE_REQUIRED")]
    SourceNoteRequired,
    #[serde(rename = "DATE_ORDER_INVALID")]
    DateOrderInvalid,
    // ---- exit 4：Provider/发送后失败 ----
    #[serde(rename = "NETWORK_ERROR")]
    NetworkError,
    #[serde(rename = "TLS_ERROR")]
    TlsError,
    #[serde(rename = "REDIRECT_REJECTED")]
    RedirectRejected,
    #[serde(rename = "CONNECT_TIMEOUT")]
    ConnectTimeout,
    #[serde(rename = "RESPONSE_TIMEOUT")]
    ResponseTimeout,
    #[serde(rename = "TOTAL_TIMEOUT")]
    TotalTimeout,
    #[serde(rename = "AUTH_FAILED")]
    AuthFailed,
    #[serde(rename = "ACCESS_DENIED")]
    AccessDenied,
    #[serde(rename = "ENDPOINT_OR_MODEL_NOT_FOUND")]
    EndpointOrModelNotFound,
    #[serde(rename = "RATE_LIMITED")]
    RateLimited,
    #[serde(rename = "QUOTA_EXHAUSTED")]
    QuotaExhausted,
    #[serde(rename = "PROVIDER_REJECTED_UNKNOWN")]
    ProviderRejectedUnknown,
    #[serde(rename = "PROVIDER_SERVER_ERROR")]
    ProviderServerError,
    #[serde(rename = "CONTEXT_REJECTED")]
    ContextRejected,
    #[serde(rename = "RESPONSE_TOO_LARGE")]
    ResponseTooLarge,
    #[serde(rename = "INCOMPLETE_OUTPUT")]
    IncompleteOutput,
    #[serde(rename = "PROTOCOL_ERROR")]
    ProtocolError,
    #[serde(rename = "INVALID_JSON")]
    InvalidJson,
    #[serde(rename = "SCHEMA_VALIDATION_FAILED")]
    SchemaValidationFailed,
    #[serde(rename = "SEMANTIC_VALIDATION_FAILED")]
    SemanticValidationFailed,
    #[serde(rename = "CANCELLED")]
    Cancelled,
    #[serde(rename = "INTERRUPTED")]
    Interrupted,
    // ---- exit 5：导出 ----
    #[serde(rename = "EXPORT_FAILED")]
    ExportFailed,
}

impl CliErrorCode {
    /// 退出码。`INPUT_REVISION_CHANGED` 发送前 2 / 发送后 4，由调用方按阶段覆盖。
    pub fn exit_code(self) -> i32 {
        use CliErrorCode::*;
        match self {
            // 2
            InvalidArgument
            | WorkspaceRequired
            | WorkspaceMismatch
            | RevisionConflict
            | ProviderRequired
            | ProviderKeyMissing
            | ProviderUrlInvalid
            | ProviderInUse
            | ConfirmationRequired
            | ConfirmationStale
            | ConfirmationCancelled
            | RequestTooLarge
            | UserMessageTooLarge
            | ContextBudgetExceeded
            | InvalidTimeoutBudget
            | InvalidContextBudget
            | RuntimeMissing
            | InputRevisionChanged
            | IdempotencyConflict
            | StaleDiagnosisReference
            | ContextInvalid
            | ScopeSelectionRequired
            | ClarificationRestatementRequired
            | MutationResultDeleted
            | DeletePreviewStale
            | InputContractInvalid => 2,
            // 3
            DataValidationFailed
            | DataDirectoryUnavailable
            | LocalSaveFailed
            | SourceChanged
            | DbSchemaUnsupported
            | DbMigrationBusy
            | DbMigrationFailed
            | DeleteDependencyBlocked
            | UnitMismatch
            | StepStillBlocked
            | DbBackupFailed
            | DbIntegrityFailed
            | DbRestoreFailed
            | TaskCategoryMetricMismatch
            | TaskCategoryUnrecognized
            | DuplicateCandidateKey
            | DuplicateReconcileTarget
            | StateProgressMismatch
            | StepOutsideTaskPeriod
            | AtLeastOneStep
            | SourceNoteRequired
            | DateOrderInvalid => 3,
            // 4
            NetworkError
            | TlsError
            | RedirectRejected
            | ConnectTimeout
            | ResponseTimeout
            | TotalTimeout
            | AuthFailed
            | AccessDenied
            | EndpointOrModelNotFound
            | RateLimited
            | QuotaExhausted
            | ProviderRejectedUnknown
            | ProviderServerError
            | ContextRejected
            | ResponseTooLarge
            | IncompleteOutput
            | ProtocolError
            | InvalidJson
            | SchemaValidationFailed
            | SemanticValidationFailed
            | Cancelled
            | Interrupted => 4,
            // 5
            ExportFailed => 5,
        }
    }

    pub fn as_str(self) -> &'static str {
        use CliErrorCode::*;
        match self {
            InvalidArgument => "INVALID_ARGUMENT",
            WorkspaceRequired => "WORKSPACE_REQUIRED",
            WorkspaceMismatch => "WORKSPACE_MISMATCH",
            RevisionConflict => "REVISION_CONFLICT",
            ProviderRequired => "PROVIDER_REQUIRED",
            ProviderKeyMissing => "PROVIDER_KEY_MISSING",
            ProviderUrlInvalid => "PROVIDER_URL_INVALID",
            ProviderInUse => "PROVIDER_IN_USE",
            ConfirmationRequired => "CONFIRMATION_REQUIRED",
            ConfirmationStale => "CONFIRMATION_STALE",
            ConfirmationCancelled => "CONFIRMATION_CANCELLED",
            RequestTooLarge => "REQUEST_TOO_LARGE",
            UserMessageTooLarge => "USER_MESSAGE_TOO_LARGE",
            ContextBudgetExceeded => "CONTEXT_BUDGET_EXCEEDED",
            InvalidTimeoutBudget => "INVALID_TIMEOUT_BUDGET",
            InvalidContextBudget => "INVALID_CONTEXT_BUDGET",
            RuntimeMissing => "RUNTIME_MISSING",
            InputRevisionChanged => "INPUT_REVISION_CHANGED",
            IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
            StaleDiagnosisReference => "STALE_DIAGNOSIS_REFERENCE",
            ContextInvalid => "CONTEXT_INVALID",
            ScopeSelectionRequired => "SCOPE_SELECTION_REQUIRED",
            ClarificationRestatementRequired => "CLARIFICATION_RESTATEMENT_REQUIRED",
            MutationResultDeleted => "MUTATION_RESULT_DELETED",
            DeletePreviewStale => "DELETE_PREVIEW_STALE",
            InputContractInvalid => "INPUT_CONTRACT_INVALID",
            DataValidationFailed => "DATA_VALIDATION_FAILED",
            DataDirectoryUnavailable => "DATA_DIRECTORY_UNAVAILABLE",
            LocalSaveFailed => "LOCAL_SAVE_FAILED",
            SourceChanged => "SOURCE_CHANGED",
            DbSchemaUnsupported => "DB_SCHEMA_UNSUPPORTED",
            DbMigrationBusy => "DB_MIGRATION_BUSY",
            DbMigrationFailed => "DB_MIGRATION_FAILED",
            DeleteDependencyBlocked => "DELETE_DEPENDENCY_BLOCKED",
            UnitMismatch => "UNIT_MISMATCH",
            StepStillBlocked => "STEP_STILL_BLOCKED",
            DbBackupFailed => "DB_BACKUP_FAILED",
            DbIntegrityFailed => "DB_INTEGRITY_FAILED",
            DbRestoreFailed => "DB_RESTORE_FAILED",
            TaskCategoryMetricMismatch => "TASK_CATEGORY_METRIC_MISMATCH",
            TaskCategoryUnrecognized => "TASK_CATEGORY_UNRECOGNIZED",
            DuplicateCandidateKey => "DUPLICATE_CANDIDATE_KEY",
            DuplicateReconcileTarget => "DUPLICATE_RECONCILE_TARGET",
            StateProgressMismatch => "STATE_PROGRESS_MISMATCH",
            StepOutsideTaskPeriod => "STEP_OUTSIDE_TASK_PERIOD",
            AtLeastOneStep => "AT_LEAST_ONE_STEP",
            SourceNoteRequired => "SOURCE_NOTE_REQUIRED",
            DateOrderInvalid => "DATE_ORDER_INVALID",
            NetworkError => "NETWORK_ERROR",
            TlsError => "TLS_ERROR",
            RedirectRejected => "REDIRECT_REJECTED",
            ConnectTimeout => "CONNECT_TIMEOUT",
            ResponseTimeout => "RESPONSE_TIMEOUT",
            TotalTimeout => "TOTAL_TIMEOUT",
            AuthFailed => "AUTH_FAILED",
            AccessDenied => "ACCESS_DENIED",
            EndpointOrModelNotFound => "ENDPOINT_OR_MODEL_NOT_FOUND",
            RateLimited => "RATE_LIMITED",
            QuotaExhausted => "QUOTA_EXHAUSTED",
            ProviderRejectedUnknown => "PROVIDER_REJECTED_UNKNOWN",
            ProviderServerError => "PROVIDER_SERVER_ERROR",
            ContextRejected => "CONTEXT_REJECTED",
            ResponseTooLarge => "RESPONSE_TOO_LARGE",
            IncompleteOutput => "INCOMPLETE_OUTPUT",
            ProtocolError => "PROTOCOL_ERROR",
            InvalidJson => "INVALID_JSON",
            SchemaValidationFailed => "SCHEMA_VALIDATION_FAILED",
            SemanticValidationFailed => "SEMANTIC_VALIDATION_FAILED",
            Cancelled => "CANCELLED",
            Interrupted => "INTERRUPTED",
            ExportFailed => "EXPORT_FAILED",
        }
    }
}

/// 稳定 JSON 外层：成功。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OkEnvelope<T> {
    pub schema_version: &'static str,
    pub ok: bool,
    pub data: T,
}

/// 稳定 JSON 外层：失败。details 只允许脱敏 ID/revision/预算/计数。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorEnvelope {
    pub schema_version: &'static str,
    pub ok: bool,
    pub error: ErrorBody,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub code: CliErrorCode,
    /// 安全中文文本；不含 Key/URL 凭据/完整请求或失败响应。
    pub message: String,
    /// nullable JSON-pointer 或 flag 名。
    pub field: Option<String>,
    /// 脱敏白名单元数据。
    pub details: serde_json::Value,
}

impl<T> OkEnvelope<T> {
    pub fn new(data: T) -> Self {
        Self {
            schema_version: RESPONSE_SCHEMA_VERSION,
            ok: true,
            data,
        }
    }
}

impl ErrorEnvelope {
    pub fn new(code: CliErrorCode, message: impl Into<String>) -> Self {
        Self {
            schema_version: RESPONSE_SCHEMA_VERSION,
            ok: false,
            error: ErrorBody {
                code,
                message: message.into(),
                field: None,
                details: serde_json::Value::Null,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_codes_have_exit_codes() {
        // 抽样核对目录关键映射
        assert_eq!(CliErrorCode::InvalidArgument.exit_code(), 2);
        assert_eq!(CliErrorCode::InputRevisionChanged.exit_code(), 2);
        assert_eq!(CliErrorCode::LocalSaveFailed.exit_code(), 3);
        assert_eq!(CliErrorCode::NetworkError.exit_code(), 4);
        assert_eq!(CliErrorCode::Interrupted.exit_code(), 4);
        assert_eq!(CliErrorCode::ExportFailed.exit_code(), 5);
    }

    #[test]
    fn code_strings_match_catalog() {
        assert_eq!(
            CliErrorCode::ContextBudgetExceeded.as_str(),
            "CONTEXT_BUDGET_EXCEEDED"
        );
        assert_eq!(
            CliErrorCode::DbSchemaUnsupported.as_str(),
            "DB_SCHEMA_UNSUPPORTED"
        );
    }
}
