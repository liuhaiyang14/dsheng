//! Provider 请求/确认/attempt 共享类型（provider-config-delivery.md §2–§3、
//! prompt-context-validation.md §6 C 方案）。

use crate::contracts::settings::{Budget, Timeouts};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 生成工作流。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workflow {
    Task,
    Diagnosis,
}

/// 诊断框架。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Framework {
    Why5,
    W5,
    W5h2,
}

impl Framework {
    pub fn as_bundle_key(self) -> &'static str {
        match self {
            Framework::Why5 => "diagnosis_5why",
            Framework::W5 => "diagnosis_5w",
            Framework::W5h2 => "diagnosis_5w2h",
        }
    }
}

/// 生成操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenerationOperation {
    TaskGenerate,
    DiagnosisGenerate,
    DiagnosisContinue,
    GenerationRetry,
}

/// 请求体/响应体硬上限（固定，不可配置）。
pub const REQUEST_BODY_MAX_BYTES: usize = 1_048_576; // 1 MiB
/// 非流式响应体上限（spec §3 固定硬上限）。
pub const RESPONSE_BODY_MAX_BYTES: usize = 2_097_152; // 2 MiB
/// 流式响应不设字节上限（2026-11-05 用户决策：模型会自行停止，
/// 本地程序负责完整接收；失控保护由总超时兜底）。
/// 非流式仍维持 2MiB 硬上限（RESPONSE_BODY_MAX_BYTES）。
pub const STREAMING_BODY_MAX_BYTES: usize = usize::MAX;
pub const USER_MESSAGE_MAX_SCALARS: usize = 8_000; // Unicode scalar values
/// 保守输入估算固定消息开销。
pub const CONSERVATIVE_OVERHEAD_BYTES: u64 = 256;
/// 确认 TTL（秒）。
pub const CONFIRMATION_TTL_SECONDS: i64 = 600;
/// 每动作最多 HTTP 请求数。
pub const REQUEST_COUNT_MAXIMUM: u32 = 1;

/// 保守输入 token 估算：最终 system/user content UTF-8 字节和 + 256。
pub fn conservative_input_tokens(system_bytes: u64, user_bytes: u64) -> u64 {
    system_bytes + user_bytes + CONSERVATIVE_OVERHEAD_BYTES
}

/// 预算检查：估算 + 输出预留 > 上下文窗口时拒绝（CONTEXT_BUDGET_EXCEEDED）。
pub fn context_budget_exceeded(budget: &Budget, conservative_input_tokens: u64) -> bool {
    conservative_input_tokens + budget.output_reserve_tokens as u64
        > budget.context_window_tokens as u64
}

/// 客户端可观察阶段（非流式；无远端进度臆测）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStage {
    Preparing,
    Dispatched,
    WaitingResponse,
    ResponseReceived,
    Parsing,
    SchemaValidation,
    SemanticValidation,
    Saving,
    Terminal,
}

/// attempt 终态（不可回退）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalState {
    Success,
    Failed,
    Cancelled,
    Interrupted,
    TimedOut,
}

/// 发送确认绑定清单（provider-config-delivery.md §2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmationBinding {
    pub confirmation_id: uuid::Uuid,
    pub workflow: Workflow,
    pub operation: GenerationOperation,
    pub workspace_id: uuid::Uuid,
    pub provider_id: uuid::Uuid,
    pub provider_revision: u32,
    pub run_id: Option<uuid::Uuid>,
    pub analysis_revision_no: Option<u32>,
    pub analysis_input_fingerprint: String,
    pub generation_input_fingerprint: String,
    pub report_id: Option<uuid::Uuid>,
    pub report_revision_no: Option<u32>,
    pub prompt_version: String,
    pub schema_version: String,
    pub context_policy_version: Option<String>,
    pub source_manifest_hash: String,
    pub input_hash: String,
    pub request_bytes: u64,
    pub conservative_input_tokens: u64,
    pub budget: Budget,
    pub timeouts: Timeouts,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl ConfirmationBinding {
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        now > self.expires_at
    }
}

/// Provider 响应提取阶段（C 方案：严格优先 + 唯一裸任务对象本地包装根信封）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionStage {
    ContractMatched,
    RootShapeButInvalid,
    WrappedBareTask,
    AmbiguousBareTasks,
    NoContractCandidate,
    NoJsonObject,
}

/// 提取结果。`wrapped_bare_task` 时调用方必须在所有存储与验收记录中
/// 持久化 `envelope_locally_reconstructed=true`。
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractionOutcome {
    pub stage: ExtractionStage,
    pub object: Option<serde_json::Value>,
    pub envelope_locally_reconstructed: bool,
}

/// Chat Completions 请求 wire（共同兼容子集：仅 model/messages；非流式）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<WireMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireMessage {
    pub role: WireRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireRole {
    System,
    User,
}

/// 只读取 choices[0].message.content 作为正文。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatCompletionsResponse {
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub message: ResponseMessage,
    /// finish_reason 缺失或 null 允许继续；非 stop 显式值拒绝为 INCOMPLETE_OUTPUT。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub content: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::settings::DEFAULT_BUDGET;

    #[test]
    fn budget_check_math() {
        let b = DEFAULT_BUDGET;
        assert!(!context_budget_exceeded(
            &b,
            b.context_window_tokens as u64 - b.output_reserve_tokens as u64
        ));
        assert!(context_budget_exceeded(&b, b.context_window_tokens as u64));
        assert_eq!(conservative_input_tokens(100, 200), 556);
    }
}
