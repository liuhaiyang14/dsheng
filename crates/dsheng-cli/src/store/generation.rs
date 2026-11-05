//! 生成门禁接线（cli-surface 23/24/30/31/32/33）：
//!
//! - `SqliteGenerationStore`：core `GenerationStore` trait 的 SQLite 生产实现
//!   （core 仅有内存实现——服务层缺口，报告主 Agent；桌面复用建议上移 core）。
//! - 预览：组装完整输入投影（core `generation::input` 类型）→ `assemble_request`
//!   预算/字节检查 → `ConfirmationGate.create`（TTL 600s）→ confirmationPreview。
//! - 发送：双门禁（--confirmation-id＋--confirm-send）→ `verify_before_send`
//!   逐项重验 → 原子消费建 attempt → `mark_dispatched` → **注入的** dispatcher
//!   执行一次 `provider::http::send_once`（生产实现；测试注入记录器断言未确认
//!   绝不派发）→ 提取/合同校验 → 本地落库 → attempt 终态。
//! - 无 Mock Provider/假 endpoint/自动重试；真实请求仅经 provider::http。

use crate::error::{CliError, CliResult};
use chrono::{DateTime, Utc};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::provider::Framework;
use dsheng_core::contracts::provider::{
    conservative_input_tokens, ChatCompletionsRequest, ChatCompletionsResponse,
    ConfirmationBinding, GenerationOperation, TerminalState, Workflow, CONFIRMATION_TTL_SECONDS,
    REQUEST_COUNT_MAXIMUM,
};
use dsheng_core::contracts::settings::{Budget, Timeouts};
use dsheng_core::generation::input::*;
use dsheng_core::provider::confirm::{
    AttemptRecord, ConfirmationGate, ConsumeOutcome as GateConsume, GenerationStore, SendSnapshot,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::{json, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// SQLite GenerationStore
// ---------------------------------------------------------------------------

pub struct SqliteGenerationStore<'a> {
    pub conn: &'a mut Connection,
}

fn parse_ts(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .expect("DB 时间戳为 RFC3339")
        .with_timezone(&Utc)
}

fn now_iso() -> String {
    dsheng_core::repository::utc_iso(Utc::now())
}

/// DB/合同字符串形式（"5why"/"5w"/"5w2h"；core Framework 的 serde 名为 why5 等）。
pub fn framework_db_str(f: Framework) -> &'static str {
    match f {
        Framework::Why5 => "5why",
        Framework::W5 => "5w",
        Framework::W5h2 => "5w2h",
    }
}

impl GenerationStore for SqliteGenerationStore<'_> {
    type Error = CliError;

    fn insert_confirmation(&mut self, binding: &ConfirmationBinding) -> Result<(), Self::Error> {
        self.conn
            .execute(
                "INSERT INTO confirmations (confirmation_id, workspace_id, workflow, operation,
                        provider_id, provider_revision, run_id, analysis_revision_no,
                        analysis_input_fingerprint, generation_input_fingerprint, report_id,
                        report_revision_no, prompt_version, schema_version,
                        context_policy_version, source_manifest_hash, input_hash, request_bytes,
                        conservative_input_tokens, budget_json, timeouts_json, binding_hash,
                        created_at, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
                params![
                    binding.confirmation_id.to_string(),
                    binding.workspace_id.to_string(),
                    serde_json::to_value(binding.workflow)
                        .unwrap()
                        .as_str()
                        .unwrap(),
                    serde_json::to_value(binding.operation)
                        .unwrap()
                        .as_str()
                        .unwrap(),
                    binding.provider_id.to_string(),
                    binding.provider_revision as i64,
                    binding.run_id.map(|u| u.to_string()),
                    binding.analysis_revision_no.map(|v| v as i64),
                    binding.analysis_input_fingerprint,
                    binding.generation_input_fingerprint,
                    binding.report_id.map(|u| u.to_string()),
                    binding.report_revision_no.map(|v| v as i64),
                    binding.prompt_version,
                    binding.schema_version,
                    binding.context_policy_version,
                    binding.source_manifest_hash,
                    binding.input_hash,
                    binding.request_bytes as i64,
                    binding.conservative_input_tokens as i64,
                    serde_json::to_string(&binding.budget).unwrap(),
                    serde_json::to_string(&binding.timeouts).unwrap(),
                    dsheng_core::analysis::fingerprint::sha256_hex(
                        &serde_json::to_string(&snapshot_json_of(binding)).unwrap()
                    ),
                    dsheng_core::repository::utc_iso(binding.created_at),
                    dsheng_core::repository::utc_iso(binding.expires_at),
                ],
            )
            .map_err(CliError::from)?;
        Ok(())
    }

    fn load_confirmation(&self, id: Uuid) -> Option<ConfirmationBinding> {
        let row = self.conn.query_row(
            "SELECT confirmation_id, workspace_id, workflow, operation, provider_id,
                    provider_revision, run_id, analysis_revision_no, analysis_input_fingerprint,
                    generation_input_fingerprint, report_id, report_revision_no, prompt_version,
                    schema_version, context_policy_version, source_manifest_hash, input_hash,
                    request_bytes, conservative_input_tokens, budget_json, timeouts_json,
                    created_at, expires_at
             FROM confirmations WHERE confirmation_id = ?1",
            params![id.to_string()],
            map_binding_row,
        );
        row.ok()
    }

    fn consume_confirmation(
        &mut self,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<GateConsume, Self::Error> {
        let binding = self
            .load_confirmation(id)
            .ok_or_else(|| CliError::new(CliErrorCode::ConfirmationRequired, "确认不存在"))?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = consume_confirmation_record(&tx, &binding, now)?;
        tx.commit()?;
        Ok(outcome)
    }

    fn load_attempt(&self, attempt_id: Uuid) -> Option<AttemptRecord> {
        let row = self.conn.query_row(
            "SELECT attempt_id, confirmation_id, workflow, operation, workspace_id, provider_id,
                    provider_revision, request_stage, request_count, terminal_state,
                    created_at, COALESCE(finished_at, dispatched_at, created_at)
             FROM generation_attempts WHERE attempt_id = ?1",
            params![attempt_id.to_string()],
            map_attempt_row,
        );
        row.ok()
    }

    fn load_attempt_by_confirmation(&self, confirmation_id: Uuid) -> Option<AttemptRecord> {
        let row = self.conn.query_row(
            "SELECT attempt_id, confirmation_id, workflow, operation, workspace_id, provider_id,
                    provider_revision, request_stage, request_count, terminal_state,
                    created_at, COALESCE(finished_at, dispatched_at, created_at)
             FROM generation_attempts WHERE confirmation_id = ?1",
            params![confirmation_id.to_string()],
            map_attempt_row,
        );
        row.ok()
    }

    fn update_attempt(&mut self, attempt: &AttemptRecord) -> Result<(), Self::Error> {
        let changed = self
            .conn
            .execute(
                "UPDATE generation_attempts SET request_stage = ?2, request_count = ?3,
                        terminal_state = ?4,
                        dispatched_at = CASE WHEN ?2 = 'dispatched' AND dispatched_at IS NULL
                                             THEN ?5 ELSE dispatched_at END,
                        finished_at = CASE WHEN ?4 IS NOT NULL THEN ?5 ELSE finished_at END
                 WHERE attempt_id = ?1 AND terminal_state IS NULL
                   AND ((?2 = 'dispatched' AND request_count = 0)
                        OR (?2 != 'dispatched' AND request_count = ?3))",
                params![
                    attempt.attempt_id.to_string(),
                    serde_json::to_value(attempt.stage)
                        .unwrap()
                        .as_str()
                        .unwrap(),
                    attempt.request_count as i64,
                    attempt.terminal.map(|t| serde_json::to_value(t)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_string()),
                    dsheng_core::repository::utc_iso(attempt.updated_at),
                ],
            )
            .map_err(CliError::from)?;
        if changed != 1 {
            return Err(CliError::new(
                CliErrorCode::IdempotencyConflict,
                "attempt 已终止或已派发，拒绝覆盖并发状态",
            ));
        }
        Ok(())
    }
}

/// Caller holds an IMMEDIATE transaction: consumption and pre-dispatch cancellation
/// use the same lock, including the check for an already-created attempt.
fn consume_confirmation_record(
    conn: &Connection,
    binding: &ConfirmationBinding,
    now: DateTime<Utc>,
) -> CliResult<GateConsume> {
    let existing = conn
        .query_row(
            "SELECT attempt_id, confirmation_id, workflow, operation, workspace_id, provider_id,
            provider_revision, request_stage, request_count, terminal_state,
            created_at, COALESCE(finished_at, dispatched_at, created_at)
         FROM generation_attempts WHERE confirmation_id = ?1",
            [binding.confirmation_id.to_string()],
            map_attempt_row,
        )
        .optional()?;
    if let Some(attempt) = existing {
        return Ok(GateConsume::Existing(attempt));
    }
    // 会话：按 generation_input_fingerprint 定位/创建（预览时已建）
    let session_id: String = {
        let existing: Option<String> = conn
            .query_row(
                "SELECT session_id FROM generation_sessions
                     WHERE workspace_id = ?1 AND generation_input_fingerprint = ?2",
                params![
                    binding.workspace_id.to_string(),
                    binding.generation_input_fingerprint
                ],
                |r| r.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(CliError::from)?;
        match existing {
            Some(id) => id,
            None => {
                // 预览应已建会话；此兜底仅防御异常路径。diagnosis 的 CHECK
                // 要求非空 framework，故从最近同指纹确认的快照推导不出时
                // 直接报错而非写入 NULL。
                return Err(CliError::new(
                    CliErrorCode::LocalSaveFailed,
                    "generation session 缺失（预览未建立会话；须重新预览）",
                ));
            }
        }
    };
    let attempt_id = Uuid::new_v4();
    let attempt_seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(attempt_seq), 0) + 1 FROM generation_attempts WHERE session_id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .map_err(CliError::from)?;
    let provider_snapshot = json!({
        "provider_id": binding.provider_id.to_string(),
        "revision": binding.provider_revision,
        "timeouts": binding.timeouts,
        "budget": binding.budget,
    });
    conn.execute(
        "INSERT INTO generation_attempts (attempt_id, workspace_id, session_id, workflow,
                    operation, confirmation_id, provider_id, provider_revision,
                    provider_snapshot_json, request_stage, request_count, prompt_version,
                    schema_version, input_hash, request_bytes, conservative_input_tokens,
                    attempt_seq, retry_of_attempt_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'preparing', 0, ?10, ?11, ?12, ?13,
                     ?14, ?15, NULL, ?16)",
        params![
            attempt_id.to_string(),
            binding.workspace_id.to_string(),
            session_id,
            serde_json::to_value(binding.workflow)
                .unwrap()
                .as_str()
                .unwrap(),
            serde_json::to_value(binding.operation)
                .unwrap()
                .as_str()
                .unwrap(),
            binding.confirmation_id.to_string(),
            binding.provider_id.to_string(),
            binding.provider_revision as i64,
            serde_json::to_string(&provider_snapshot).unwrap(),
            binding.prompt_version,
            binding.schema_version,
            binding.input_hash,
            binding.request_bytes as i64,
            binding.conservative_input_tokens as i64,
            attempt_seq,
            now_iso()
        ],
    )
    .map_err(CliError::from)?;
    conn.execute(
        "UPDATE confirmations SET consumed_at = ?2, consumed_by_attempt_id = ?3
             WHERE confirmation_id = ?1",
        params![
            binding.confirmation_id.to_string(),
            dsheng_core::repository::utc_iso(now),
            attempt_id.to_string()
        ],
    )
    .map_err(CliError::from)?;

    let attempt = conn.query_row(
        "SELECT attempt_id, confirmation_id, workflow, operation, workspace_id, provider_id,
            provider_revision, request_stage, request_count, terminal_state,
            created_at, COALESCE(finished_at, dispatched_at, created_at)
         FROM generation_attempts WHERE attempt_id = ?1",
        [attempt_id.to_string()],
        map_attempt_row,
    )?;
    Ok(GateConsume::Created(attempt))
}

fn snapshot_json_of(binding: &ConfirmationBinding) -> Value {
    json!({
        "workspace_id": binding.workspace_id.to_string(),
        "provider_id": binding.provider_id.to_string(),
        "provider_revision": binding.provider_revision,
        "run_id": binding.run_id.map(|u| u.to_string()),
        "analysis_revision_no": binding.analysis_revision_no,
        "analysis_input_fingerprint": binding.analysis_input_fingerprint,
        "generation_input_fingerprint": binding.generation_input_fingerprint,
        "report_id": binding.report_id.map(|u| u.to_string()),
        "report_revision_no": binding.report_revision_no,
        "prompt_version": binding.prompt_version,
        "schema_version": binding.schema_version,
        "context_policy_version": binding.context_policy_version,
        "source_manifest_hash": binding.source_manifest_hash,
        "input_hash": binding.input_hash,
        "request_bytes": binding.request_bytes,
        "conservative_input_tokens": binding.conservative_input_tokens,
        "budget": binding.budget,
        "timeouts": binding.timeouts,
    })
}

fn map_binding_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ConfirmationBinding> {
    let budget_raw: String = r.get(19)?;
    let timeouts_raw: String = r.get(20)?;
    Ok(ConfirmationBinding {
        confirmation_id: Uuid::parse_str(&r.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("confirmation_id".into()))?,
        workflow: serde_json::from_value(json!(r.get::<_, String>(2)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("workflow".into()))?,
        operation: serde_json::from_value(json!(r.get::<_, String>(3)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("operation".into()))?,
        workspace_id: Uuid::parse_str(&r.get::<_, String>(1)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("workspace_id".into()))?,
        provider_id: Uuid::parse_str(&r.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("provider_id".into()))?,
        provider_revision: r.get::<_, i64>(5)? as u32,
        run_id: r
            .get::<_, Option<String>>(6)?
            .and_then(|s| Uuid::parse_str(&s).ok()),
        analysis_revision_no: r.get::<_, Option<i64>>(7)?.map(|v| v as u32),
        analysis_input_fingerprint: r.get(8)?,
        generation_input_fingerprint: r.get(9)?,
        report_id: r
            .get::<_, Option<String>>(10)?
            .and_then(|s| Uuid::parse_str(&s).ok()),
        report_revision_no: r.get::<_, Option<i64>>(11)?.map(|v| v as u32),
        prompt_version: r.get(12)?,
        schema_version: r.get(13)?,
        context_policy_version: r.get::<_, Option<String>>(14)?,
        source_manifest_hash: r.get(15)?,
        input_hash: r.get(16)?,
        request_bytes: r.get::<_, i64>(17)? as u64,
        conservative_input_tokens: r.get::<_, i64>(18)? as u64,
        budget: serde_json::from_str(&budget_raw)
            .map_err(|_| rusqlite::Error::InvalidColumnName("budget".into()))?,
        timeouts: serde_json::from_str(&timeouts_raw)
            .map_err(|_| rusqlite::Error::InvalidColumnName("timeouts".into()))?,
        created_at: parse_ts(&r.get::<_, String>(21)?),
        expires_at: parse_ts(&r.get::<_, String>(22)?),
    })
}

fn map_attempt_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AttemptRecord> {
    Ok(AttemptRecord {
        attempt_id: Uuid::parse_str(&r.get::<_, String>(0)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("attempt_id".into()))?,
        confirmation_id: Uuid::parse_str(&r.get::<_, String>(1)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("confirmation_id".into()))?,
        workflow: serde_json::from_value(json!(r.get::<_, String>(2)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("workflow".into()))?,
        operation: serde_json::from_value(json!(r.get::<_, String>(3)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("operation".into()))?,
        workspace_id: Uuid::parse_str(&r.get::<_, String>(4)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("workspace_id".into()))?,
        provider_id: Uuid::parse_str(&r.get::<_, String>(5)?)
            .map_err(|_| rusqlite::Error::InvalidColumnName("provider_id".into()))?,
        provider_revision: r.get::<_, i64>(6)? as u32,
        stage: serde_json::from_value(json!(r.get::<_, String>(7)?))
            .map_err(|_| rusqlite::Error::InvalidColumnName("stage".into()))?,
        request_count: r.get::<_, i64>(8)? as u32,
        terminal: r
            .get::<_, Option<String>>(9)?
            .and_then(|s| serde_json::from_value(json!(s)).ok()),
        created_at: parse_ts(&r.get::<_, String>(10)?),
        updated_at: parse_ts(&r.get::<_, String>(11)?),
    })
}

// ---------------------------------------------------------------------------
// 请求派发注入点
// ---------------------------------------------------------------------------

/// 派发运行时（无 Key 字段暴露；api_key 仅在 dispatch 内部读取使用）。
pub struct ProviderRuntime {
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// Provider 快照 stream 标志（§1 增量）：true=流式（SSE）路径。
    pub stream: bool,
    pub timeouts: Timeouts,
    pub budget: Budget,
    pub api_key: String,
}

/// 请求派发抽象：生产＝provider::http 一次 send_once / send_once_streaming
/// （按 runtime.stream 选择）；测试＝记录器（断言未确认不派发；无网络、
/// 无 Mock Provider）。`on_chunk` 为流式进度回调
/// `(chunk_seq, total_bytes, elapsed_ms)`（非流式实现忽略）。
pub trait RequestDispatch {
    fn dispatch(
        &mut self,
        runtime: &ProviderRuntime,
        request: &ChatCompletionsRequest,
        on_chunk: &mut dyn FnMut(u64, u64, i64),
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)>;

    fn dispatch_cancellable(
        &mut self,
        runtime: &ProviderRuntime,
        request: &ChatCompletionsRequest,
        on_chunk: &mut dyn FnMut(u64, u64, i64),
        should_cancel: &dyn Fn() -> bool,
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        if should_cancel() {
            return Err((CliErrorCode::Cancelled, "生成已手动停止".into()));
        }
        self.dispatch(runtime, request, on_chunk)
    }
}

/// 派发路径选择（纯函数；DI 测试断言用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchPath {
    /// 流式：请求带 "stream": true，SSE 逐块累积（§1 增量）。
    Streaming,
    /// 非流式：原 send_once。
    NonStreaming,
}

/// stream 标志 → 派发路径（HttpDispatch 分支依据；测试断言选择正确路径）。
pub fn dispatch_path(stream: bool) -> DispatchPath {
    if stream {
        DispatchPath::Streaming
    } else {
        DispatchPath::NonStreaming
    }
}

/// 生产派发（唯一网络路径；每 attempt 一条请求、无自动重试）。
pub struct HttpDispatch;

impl RequestDispatch for HttpDispatch {
    fn dispatch(
        &mut self,
        runtime: &ProviderRuntime,
        request: &ChatCompletionsRequest,
        on_chunk: &mut dyn FnMut(u64, u64, i64),
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        match dispatch_path(runtime.stream) {
            DispatchPath::Streaming => {
                let (client, endpoint) = dsheng_core::provider::http::build_streaming_client(
                    &runtime.base_url,
                    &runtime.timeouts,
                    &runtime.budget,
                )?;
                // total 起点＝本次派发开始（preparing→终态的近似锚点；
                // 确认消费/校验耗时不计入，见已知限制）
                let started_at = dsheng_core::provider::deadline::MonotonicClock::now_millis(
                    &dsheng_core::provider::deadline::SystemClock,
                );
                dsheng_core::provider::http::send_once_streaming(
                    &client,
                    &endpoint,
                    &runtime.api_key,
                    request,
                    &runtime.timeouts,
                    started_at,
                    on_chunk,
                )
            }
            DispatchPath::NonStreaming => {
                let (client, endpoint) = dsheng_core::provider::http::build_client(
                    &runtime.base_url,
                    &runtime.timeouts,
                    &runtime.budget,
                )?;
                dsheng_core::provider::http::send_once(
                    &client,
                    &endpoint,
                    &runtime.api_key,
                    request,
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 流式进度事件与消费端（CLI stderr / 桌面事件桥）
// ---------------------------------------------------------------------------

/// 面向消费端的进度事件（原始 chunk 三元组 ＋ attempt 关联与阶段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamProgressEvent {
    pub attempt_id: Uuid,
    pub confirmation_id: Uuid,
    pub workspace_id: Uuid,
    pub chunk_seq: u64,
    pub bytes: u64,
    pub elapsed_ms: i64,
    /// waiting / receiving / validating / saving（不估算模型进度）。
    pub stage: &'static str,
}

/// 进度消费端（S4 接线）：CLI＝stderr 节流；桌面＝事件桥回调。
pub enum ProgressSink {
    /// CLI stderr：每 30 秒一条「已接收 N 字节/第 M 块/耗时 Xs」
    /// （首条立即；stdout 保持纯 JSON 输出，符合 §5.2 约定）。
    Stderr { last_emit_ms: i64 },
    /// 桌面壳/嵌入方回调（tauri Emitter 发 generation://progress 等）。
    Callback(Box<dyn FnMut(StreamProgressEvent) + Send>),
    /// 静默（预览/非发送路径）。
    Null,
}

impl ProgressSink {
    /// stderr 节流 sink（last_emit 初始为极小值 → 首条立即输出）。
    pub fn stderr() -> Self {
        ProgressSink::Stderr {
            last_emit_ms: i64::MIN,
        }
    }

    /// stderr 节流判定（纯函数，测试断言）：首条立即，其后每满 30s 一条。
    pub fn stderr_emit_due(last_emit_ms: i64, elapsed_ms: i64) -> bool {
        elapsed_ms.saturating_sub(last_emit_ms) >= 30_000
    }

    pub fn on_event(&mut self, event: StreamProgressEvent) {
        match self {
            ProgressSink::Stderr { last_emit_ms } => {
                if Self::stderr_emit_due(*last_emit_ms, event.elapsed_ms) {
                    eprintln!(
                        "[generation] 已接收 {} 字节/第 {} 块/耗时 {}s（attempt {}，{}）",
                        event.bytes,
                        event.chunk_seq,
                        event.elapsed_ms / 1_000,
                        event.attempt_id,
                        event.stage
                    );
                    *last_emit_ms = event.elapsed_ms;
                }
            }
            ProgressSink::Callback(f) => f(event),
            ProgressSink::Null => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 输入投影构建（run revision → 模型输入合同）
// ---------------------------------------------------------------------------

pub struct GenerationMaterial {
    pub payload_json: String,
    pub user_supplied_text: String,
    pub framework: Option<Framework>,
    pub workflow: Workflow,
    pub input_hash: String,
    pub source_manifest_hash: String,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
    pub run_id: Uuid,
    pub report_id: Option<Uuid>,
    pub report_revision_no: Option<u32>,
    /// 预览 source_scope_summary（裁剪后范围如实反映；settings.schema 字段集）。
    pub scope_rows: Vec<Value>,
}

/// 从 run revision 组装诊断输入投影（验证过 dsheng-diagnosis-input-v2）。
#[allow(clippy::too_many_arguments)]
pub fn build_diagnosis_payload(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    framework: Framework,
    problem: &str,
    focus_metric_ids: &[String],
    evidence_ids: &[String],
    session_state: SessionStateProjection,
    continuation: Option<ContinuationInputProjection>,
    user_message: &str,
) -> CliResult<GenerationMaterial> {
    let (revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let selection = trimmed_source_selection(conn, workspace_id, run_id, revision, evidence_ids)?;
    let snapshot = load_snapshot(conn, run_id, revision)?;
    let defs = load_definition_projections(conn, workspace_id)?;
    let used_defs =
        used_definition_closure(defs, &observed_metric_keys(&snapshot), focus_metric_ids);
    let metrics = metric_projections(conn, workspace_id, &snapshot)?;
    let conflicts = conflict_projections(conn, workspace_id, run_id)?;
    let manual = manual_projections(conn, workspace_id)?;
    let anomalies: Vec<AnomalyProjection> = conflicts
        .iter()
        .map(|c| AnomalyProjection {
            code: "source_conflict".into(),
            metric_id: Some(c.metric_id.to_string()),
            message: format!("来源冲突未解决（unit={}）", c.unit),
            evidence_ids: c
                .candidate_values
                .iter()
                .map(|v| v.evidence_id.clone())
                .collect(),
        })
        .collect();
    let payload = DiagnosisInputProjection {
        request: DiagnosisRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: run_id.to_string(),
            analysis_revision_no: revision,
            analysis_input_fingerprint: fingerprint.clone(),
            prompt_version: DiagnosisPromptVersion::V20,
            workflow: DiagnosisWorkflowMarker::Diagnosis,
            report_id: None,
            generation_session_id: session_id_of(conn, workspace_id, run_id),
            round_no: 1,
            framework: framework.into(),
            schema_version: DiagnosisSchemaVersion::V3,
            user_confirmed_continue: continuation.is_some(),
            context_policy_version: ContextPolicyVersion::V2,
        },
        workspace: workspace_projection(conn, workspace_id, used_defs)?,
        metrics,
        source_manifest: selection.manifest.clone(),
        source_records: selection.records.clone(),
        manual_inputs: manual,
        missing_or_anomalous_inputs: anomalies,
        source_conflicts: conflicts,
        period: DiagnosisPeriodProjection {
            source_from: snapshot.period_from,
            source_to: snapshot.period_to,
            period_policy_version: snapshot.policy_version,
        },
        scope: ScopeProjection {
            problem_statement: problem.to_string(),
            focus_metric_ids: focus_metric_ids.to_vec(),
            framework: framework.into(),
            open_question_ids: vec![],
        },
        user_message: user_message.to_string(),
        continuation_input: continuation,
        session_state,
        output_contract: DiagnosisOutputContract {
            schema: framework.into(),
            schema_version: DiagnosisSchemaVersion::V3,
        },
    };
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let value: Value = serde_json::from_str(&payload_json).expect("serialize roundtrip");
    dsheng_core::provider::schema::validate_diagnosis_input(&value)
        .map_err(|errors| CliError::new(CliErrorCode::InputContractInvalid, errors.join("; ")))?;
    Ok(material_of(
        payload_json,
        user_message,
        Workflow::Diagnosis,
        Some(framework),
        revision,
        fingerprint,
        Uuid::parse_str(run_id).map_err(|_| CliError::invalid("run_id 非法"))?,
        selection,
    ))
}

/// 从 run revision 组装任务输入投影（验证过 dsheng-task-input-v2）。
pub fn build_task_payload(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    user_context: &str,
    optional_report: Option<&Value>,
) -> CliResult<GenerationMaterial> {
    let (revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let selection = trimmed_source_selection(conn, workspace_id, run_id, revision, &[])?;
    let snapshot = load_snapshot(conn, run_id, revision)?;
    let defs = load_definition_projections(conn, workspace_id)?;
    let metrics = metric_projections(conn, workspace_id, &snapshot)?;
    let conflicts = conflict_projections(conn, workspace_id, run_id)?;
    let manual = manual_projections(conn, workspace_id)?;
    let anomalies: Vec<AnomalyProjection> = conflicts
        .iter()
        .map(|c| AnomalyProjection {
            code: "source_conflict".into(),
            metric_id: Some(c.metric_id.to_string()),
            message: format!("来源冲突未解决（unit={}）", c.unit),
            evidence_ids: c
                .candidate_values
                .iter()
                .map(|v| v.evidence_id.clone())
                .collect(),
        })
        .collect();
    let target_period = snapshot
        .target_period
        .unwrap_or_else(|| next_month(snapshot.period_to));
    let optional_diagnosis_reference = optional_report.map(|report| OptionalDiagnosisReference {
        report_id: report
            .get("report_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        report_revision_no: report
            .get("report_revision_no")
            .and_then(|v| v.as_u64())
            .unwrap_or(1) as u32,
        analysis_revision_no: revision,
        analysis_input_fingerprint: fingerprint.clone(),
        framework: serde_json::from_value(json!(report
            .get("framework")
            .and_then(|v| v.as_str())
            .unwrap_or("5why")))
        .unwrap_or(FrameworkProjection::Why5),
        summary: report
            .get("scope")
            .and_then(|s| s.get("problem_statement"))
            .and_then(|p| p.as_str())
            .unwrap_or_default()
            .to_string(),
        facts: vec![],
        hypotheses: vec![],
        evidence_ids: vec![],
    });
    let payload = TaskInputProjection {
        request: TaskRequestProjection {
            input_schema_version: InputSchemaVersion::V2,
            run_id: run_id.to_string(),
            analysis_revision_no: revision,
            analysis_input_fingerprint: fingerprint.clone(),
            prompt_version: TaskPromptVersion::V23,
            workflow: TaskWorkflowMarker::Task,
            schema_version: TaskSchemaVersion::V2,
        },
        workspace: workspace_projection(
            conn,
            workspace_id,
            used_definition_closure(defs, &observed_metric_keys(&snapshot), &[]),
        )?,
        metrics,
        source_manifest: selection.manifest.clone(),
        source_records: selection.records.clone(),
        manual_inputs: manual,
        missing_or_anomalous_inputs: anomalies,
        source_conflicts: conflicts,
        period: TaskPeriodProjection {
            source_from: snapshot.period_from,
            source_to: snapshot.period_to,
            target_from: target_period.from,
            target_to: target_period.to,
            target_grain: CadenceProjection::Month,
            period_policy_version: snapshot.policy_version,
        },
        business_review: BusinessReviewProjection {
            summary: format!(
                "期间 {}~{} 分析快照（revision {revision}）",
                snapshot.period_from, snapshot.period_to
            ),
            prior_task_outcomes: vec![],
        },
        task_skill_context: TaskSkillContextProjection {
            rules_version: TaskPromptVersion::V23,
            required_fields: RequiredFieldsConst,
            category_options: CategoryOptionsConst,
        },
        optional_diagnosis_reference,
        user_context: Some(user_context.to_string()),
        output_contract: TaskOutputContract {
            schema: TaskOutputSchemaId::TaskOutputV2,
            schema_version: TaskSchemaVersion::V2,
        },
    };
    let payload_json = serde_json::to_string(&payload)
        .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
    let value: Value = serde_json::from_str(&payload_json).expect("serialize roundtrip");
    dsheng_core::provider::schema::validate_task_input(&value)
        .map_err(|errors| CliError::new(CliErrorCode::InputContractInvalid, errors.join("; ")))?;
    Ok(material_of(
        payload_json,
        user_context,
        Workflow::Task,
        None,
        revision,
        fingerprint,
        Uuid::parse_str(run_id).map_err(|_| CliError::invalid("run_id 非法"))?,
        selection,
    ))
}

#[allow(clippy::too_many_arguments)]
fn material_of(
    payload_json: String,
    user_text: &str,
    workflow: Workflow,
    framework: Option<Framework>,
    revision: u32,
    fingerprint: String,
    run_id: Uuid,
    selection: TrimmedSourceSelection,
) -> GenerationMaterial {
    // 指纹覆盖清单（§2.2）不含 generation_session_id（实现细节）：哈希前剔除，
    // 否则预览新建会话后发送重建取到新 id → CONFIRMATION_STALE 自锁。
    fn blank_session_ids(v: &mut Value) {
        match v {
            Value::Object(map) => {
                for (k, val) in map.iter_mut() {
                    if k == "generation_session_id" {
                        *val = Value::String(String::new());
                    } else {
                        blank_session_ids(val);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(blank_session_ids),
            _ => {}
        }
    }
    let hash_material = {
        let mut v: Value = serde_json::from_str(&payload_json)
            .unwrap_or_else(|_| Value::String(payload_json.clone()));
        blank_session_ids(&mut v);
        serde_json::to_string(&v).unwrap_or(payload_json.clone())
    };
    let input_hash = dsheng_core::analysis::fingerprint::sha256_hex(&hash_material);
    let manifest_material: Vec<Value> = selection
        .manifest
        .iter()
        .map(|m| {
            json!({
                "content_hash": m.content_hash,
                "range": m.selected_range,
                "selection_hash": m.selection_hash,
                "sheet": m.sheet,
                "source_id": m.source_id.to_string(),
            })
        })
        .collect();
    let source_manifest_hash = dsheng_core::analysis::fingerprint::sha256_hex(
        &serde_json::to_string(&manifest_material).unwrap(),
    );
    GenerationMaterial {
        payload_json,
        user_supplied_text: user_text.to_string(),
        framework,
        workflow,
        input_hash,
        source_manifest_hash,
        analysis_revision_no: revision,
        analysis_input_fingerprint: fingerprint,
        run_id,
        report_id: None,
        report_revision_no: None,
        scope_rows: selection.scope_rows,
    }
}

fn next_month(d: chrono::NaiveDate) -> dsheng_core::contracts::domain::DateRange {
    use chrono::Datelike;
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    dsheng_core::analysis::periods::natural_month_span(
        chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap_or(d),
    )
}

struct RunSnapshot {
    period_from: chrono::NaiveDate,
    period_to: chrono::NaiveDate,
    target_period: Option<dsheng_core::contracts::domain::DateRange>,
    policy_version: String,
    metrics: Value,
}

fn load_snapshot(conn: &Connection, run_id: &str, revision: u32) -> CliResult<RunSnapshot> {
    let (normalized_raw, policy_raw): (String, String) = conn
        .query_row(
            "SELECT normalized_metrics_json, period_policy_snapshot_json FROM run_revisions
             WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "run revision 不存在"))?;
    let normalized: Value = serde_json::from_str(&normalized_raw)
        .map_err(|_| CliError::new(CliErrorCode::LocalSaveFailed, "normalized 快照损坏"))?;
    let policy: Value = serde_json::from_str(&policy_raw).unwrap_or_else(|_| json!({}));
    let parse_d = |v: Option<&str>| -> Option<chrono::NaiveDate> {
        v.and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
    };
    Ok(RunSnapshot {
        period_from: parse_d(normalized["period"]["from"].as_str())
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
        period_to: parse_d(normalized["period"]["to"].as_str())
            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap()),
        target_period: None,
        policy_version: policy
            .get("version")
            .and_then(|v| v.as_u64())
            .map(|v| v.to_string())
            .unwrap_or_else(|| "1".into()),
        metrics: normalized["metrics"].clone(),
    })
}

fn load_definition_projections(
    conn: &Connection,
    workspace_id: Uuid,
) -> CliResult<Vec<MetricDefinitionProjection>> {
    let mut stmt = conn.prepare(
        // §1.4：metric_id 投影 stable_metric_id（定义身份，版本变化保留身份）；
        // version/semantic_hash 仍为当前具体版本行值（revision 冻结语义）
        "SELECT m.stable_metric_id, m.metric_key, m.version, m.semantic_hash, m.label,
                m.semantic_type, m.unit_signature_json, m.direction, m.target_constraint_scope,
                m.revenue_semantics, m.aggregation, m.formula_json, m.required_inputs_json,
                m.effective_from
         FROM metric_definitions m
         WHERE m.workspace_id = ?1 AND m.version =
               (SELECT MAX(version) FROM metric_definitions m2
                WHERE m2.workspace_id = m.workspace_id AND m2.metric_key = m.metric_key)
         ORDER BY m.metric_key",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
        Ok(serde_json::json!({
            "metric_id": r.get::<_, String>(0)?,
            "metric_key": r.get::<_, String>(1)?,
            "version": r.get::<_, i64>(2)?,
            "semantic_hash": r.get::<_, String>(3)?,
            "label": r.get::<_, String>(4)?,
            "semantic_type": r.get::<_, String>(5)?,
            "unit_signature": serde_json::from_str::<Value>(&r.get::<_, String>(6)?).unwrap_or(json!(null)),
            "direction": r.get::<_, String>(7)?,
            "target_constraint_scope": r.get::<_, String>(8)?,
            "revenue_semantics": r.get::<_, i64>(9)? == 1,
            "aggregation": r.get::<_, String>(10)?,
            "formula": r.get::<_, Option<String>>(11)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()),
            "required_inputs": serde_json::from_str::<Value>(&r.get::<_, String>(12)?).unwrap_or(json!([])),
            "denominator_policy": "positive_only",
            "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
            "effective_from": r.get::<_, String>(13)?,
            "scope_key": "全公司",
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let v = row?;
        let parsed: MetricDefinitionProjection = serde_json::from_value(v)
            .map_err(|e| CliError::new(CliErrorCode::InputContractInvalid, e.to_string()))?;
        out.push(parsed);
    }
    Ok(out)
}

/// §1.4：workspace.metric_definitions 只发送本次使用指标及其公式依赖的
/// 冻结定义。使用集＝分析快照观察到/用户聚焦的 metric_key ∪ 其 AST/
/// required_inputs 依赖闭包（闭包仅收工作区已注册键）。
fn used_definition_closure(
    defs: Vec<MetricDefinitionProjection>,
    observed_keys: &std::collections::BTreeSet<String>,
    focus_keys: &[String],
) -> Vec<MetricDefinitionProjection> {
    fn ast_refs(node: &FormulaAst, out: &mut Vec<String>) {
        match node {
            FormulaAst::Ref { metric_key } => out.push(metric_key.clone()),
            FormulaAst::Const { .. } => {}
            FormulaAst::Add { args }
            | FormulaAst::Sub { args }
            | FormulaAst::Mul { args }
            | FormulaAst::Div { args } => {
                ast_refs(&args[0], out);
                ast_refs(&args[1], out);
            }
        }
    }
    let by_key: std::collections::HashMap<&str, &MetricDefinitionProjection> =
        defs.iter().map(|d| (d.metric_key.as_str(), d)).collect();
    let mut used: std::collections::BTreeSet<String> = observed_keys.clone();
    used.extend(focus_keys.iter().cloned());
    let mut queue: Vec<String> = used.iter().cloned().collect();
    while let Some(key) = queue.pop() {
        let Some(def) = by_key.get(key.as_str()) else {
            continue;
        };
        let mut refs: Vec<String> = Vec::new();
        if let Some(formula) = &def.formula {
            ast_refs(formula, &mut refs);
        }
        refs.extend(def.required_inputs.iter().cloned());
        for r in refs {
            if by_key.contains_key(r.as_str()) && !used.contains(&r) {
                used.insert(r.clone());
                queue.push(r);
            }
        }
    }
    defs.into_iter()
        .filter(|d| used.contains(&d.metric_key))
        .collect()
}

/// 分析快照中出现观察的 metric_key 集合。
fn observed_metric_keys(snapshot: &RunSnapshot) -> std::collections::BTreeSet<String> {
    snapshot
        .metrics
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|m| m["metric_key"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn workspace_projection(
    conn: &Connection,
    workspace_id: Uuid,
    defs: Vec<MetricDefinitionProjection>,
) -> CliResult<WorkspaceProjection> {
    let (tz, cadence): (String, String) = conn
        .query_row(
            "SELECT COALESCE(p.timezone_id, 'Asia/Shanghai'), p.cadence
             FROM workspaces w LEFT JOIN period_policies p
                  ON p.period_policy_id = w.current_period_policy_id
             WHERE w.workspace_id = ?1",
            params![workspace_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(CliError::from)?;
    Ok(WorkspaceProjection {
        workspace_id: workspace_id.to_string(),
        industry: None,
        cadence: serde_json::from_value(json!(cadence)).unwrap_or(CadenceProjection::Month),
        timezone: tz,
        metric_definitions: defs,
    })
}

fn dec_number(v: &Value) -> Option<serde_json::Number> {
    v.as_str()
        .and_then(|s| serde_json::from_str::<serde_json::Number>(s).ok())
}

fn metric_projections(
    conn: &Connection,
    workspace_id: Uuid,
    snapshot: &RunSnapshot,
) -> CliResult<Vec<MetricProjection>> {
    let defs = load_definition_projections(conn, workspace_id)?;
    let mut out = Vec::new();
    for def in defs {
        let row = snapshot
            .metrics
            .as_array()
            .and_then(|arr| {
                arr.iter()
                    .find(|m| m["metric_key"].as_str() == Some(def.metric_key.as_str()))
            })
            .cloned();
        let Some(row) = row else { continue };
        let mom: Option<ComparisonProjection> = row
            .get("mom")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value(clone_with_numbers(v)).ok());
        let yoy: Option<ComparisonProjection> = row
            .get("yoy")
            .filter(|v| !v.is_null())
            .and_then(|v| serde_json::from_value(clone_with_numbers(v)).ok());
        out.push(MetricProjection {
            metric_id: def.metric_id,
            label: def.label.clone(),
            unit: def.unit_signature.to_model_unit_string(),
            direction: def.direction,
            target_constraint_scope: def.target_constraint_scope,
            explicit_target: None,
            annual_allocated_target: None,
            prior_year_actual: None,
            target: row.get("target_value").and_then(dec_number),
            actual: row.get("actual_value").and_then(dec_number),
            delta: row.get("delta").and_then(dec_number),
            mom,
            yoy,
            evidence_ids: vec![],
        });
    }
    Ok(out)
}

/// decimal 字符串 → JSON number 字面量（decimal-aware，不经 f64）。
fn clone_with_numbers(v: &Value) -> Value {
    match v {
        Value::String(s) => serde_json::from_str::<serde_json::Number>(s)
            .map(Value::Number)
            .unwrap_or_else(|_| Value::String(s.clone())),
        Value::Array(a) => Value::Array(a.iter().map(clone_with_numbers).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), clone_with_numbers(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// 生成载荷来源范围裁剪（prompt-context-validation.md §4：收窄“选定范围”本身）
// ---------------------------------------------------------------------------

/// 裁剪结果：manifest（新范围）/source_records（逐格）/预览 scope 行。
pub struct TrimmedSourceSelection {
    pub manifest: Vec<SourceManifestEntry>,
    pub records: Vec<SourceRecordProjection>,
    pub scope_rows: Vec<Value>,
}

/// 旁表单元格快照（analyze 落库；生成时只读）。
struct CellSnap {
    row_no: usize,
    col_no: usize,
    raw_text: String,
    cell_kind: String,
    value_state: String,
    typed_value: Option<String>,
    formula: Option<String>,
    merged_range: Option<String>,
}

fn source_scope_unavailable() -> CliError {
    CliError::new(
        CliErrorCode::DataValidationFailed,
        "该分析版本的来源范围快照缺失或不可用，请使用同一数据目录重新运行本地分析",
    )
}

/// 来源必须绑定到不可变分析版本；历史空 source_id 仅允许同企业 content_hash 匹配。
/// 基期只展开当前版本显式引用的 revision，不递归引入基期的其他来源。
fn revision_source_scope(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: u32,
) -> CliResult<std::collections::BTreeSet<(String, String)>> {
    #[derive(serde::Deserialize)]
    struct BaseRevision {
        run_id: String,
        revision_no: u32,
    }

    let read_snapshot = |id: &str, rev: u32| -> CliResult<(String, String)> {
        conn.query_row(
            "SELECT rr.source_selection_snapshot_json, rr.comparison_base_refs_json
             FROM run_revisions rr JOIN analysis_runs ar ON ar.run_id = rr.run_id
             WHERE ar.workspace_id = ?1 AND rr.run_id = ?2 AND rr.revision_no = ?3",
            params![workspace_id.to_string(), id, rev],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => source_scope_unavailable(),
            other => CliError::from(other),
        })
    };
    let (current_sources, base_refs) = read_snapshot(run_id, revision)?;
    let mut snapshots = vec![current_sources];
    let bases: Vec<BaseRevision> =
        serde_json::from_str(&base_refs).map_err(|_| source_scope_unavailable())?;
    for base in bases {
        snapshots.push(read_snapshot(&base.run_id, base.revision_no)?.0);
    }

    let mut allowed = std::collections::BTreeSet::new();
    let mut stmt = conn.prepare(
        "SELECT source_id FROM source_files
         WHERE workspace_id = ?1 AND content_hash = ?2
           AND (?3 = '' OR source_id = ?3) AND (?4 IS NOT NULL OR kind = 'csv')",
    )?;
    for snapshot in snapshots {
        let selections: Vec<dsheng_core::analysis::fingerprint::SourceSelection> =
            serde_json::from_str(&snapshot).map_err(|_| source_scope_unavailable())?;
        // [] 是合法的纯手工输入分析；保持空来源白名单，绝不扩大到整个工作区。
        for selection in selections {
            if selection.content_hash.is_empty()
                || selection.sheet.as_deref().is_some_and(str::is_empty)
            {
                return Err(source_scope_unavailable());
            }
            let ids: Vec<String> = stmt
                .query_map(
                    params![
                        workspace_id.to_string(),
                        selection.content_hash,
                        selection.source_id,
                        selection.sheet,
                    ],
                    |r| r.get(0),
                )?
                .collect::<Result<_, _>>()?;
            if ids.is_empty() {
                return Err(source_scope_unavailable());
            }
            let sheet = selection.sheet.unwrap_or_else(|| "csv".to_string());
            for id in ids {
                allowed.insert((id, sheet.clone()));
            }
        }
    }
    Ok(allowed)
}

/// “相关行/列”选定范围计算（确定性：同一 DB 状态 → 同一范围/记录/指纹）：
///
/// - 文件与 sheet 先限制为所选 revision＋其显式比较基期的来源快照；
/// - 标准表（核算/推移，有表头块）：选定行＝表头块行 ∪ 标签列命中已注册指标
///   的行（metric_key/label/别名精确匹配）∪ 分析快照观察（mom/yoy evidence）
///   所在行 ∪ 冲突候选 evidence 所在行 ∪ 用户 --evidence 指向的行；
///   行内发送该行 A..最后非空列的全部非空格（仍逐格）。
/// - 推移表转置 sheet（§4.3）：选定列＝period 列（A 列）∪ 指标列整列；
///   非指标列不发；证据锚点格仍保留（evidence 单元格不得丢）。
/// - 无表头 sheet（generic）：仅证据锚点行入选；无锚点则整个 sheet 不发
///   （未选内容不发）。
/// - 未贡献任何选定记录的来源不进 source_manifest（未选文件始终不发）。
pub fn trimmed_source_selection(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: u32,
    extra_evidence: &[String],
) -> CliResult<TrimmedSourceSelection> {
    crate::store::sources::ensure_sidecar_tables(conn)?;
    let source_scope = revision_source_scope(conn, workspace_id, run_id, revision)?;
    // 工作区指标名全集（行/列识别口径与扫描期一致）
    let metric_names: std::collections::HashSet<String> =
        crate::store::sources::metric_label_names(conn, workspace_id)?
            .into_iter()
            .collect();
    // 来源文件（file_label→source 解析锚点用；输出按 file_label 排序）
    struct SourceRow {
        source_id: String,
        file_label: String,
        kind: String,
        content_hash: String,
    }
    let sources: Vec<SourceRow> = {
        let mut stmt = conn.prepare(
            "SELECT source_id, file_label, kind, content_hash FROM source_files
             WHERE workspace_id = ?1 ORDER BY file_label, created_at, source_id",
        )?;
        let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
            Ok(SourceRow {
                source_id: r.get(0)?,
                file_label: r.get(1)?,
                kind: r.get(2)?,
                content_hash: r.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|source| source_scope.iter().any(|(id, _)| id == &source.source_id))
            .collect()
    };
    // 布局快照（按 source 分组；sheet 有序）
    let mut layouts: std::collections::BTreeMap<String, std::collections::BTreeMap<String, Value>> =
        Default::default();
    {
        let mut stmt = conn.prepare(
            "SELECT source_id, sheet, layout_json FROM cli_source_sheet_layouts
             WHERE workspace_id = ?1 AND source_id = ?2 AND sheet = ?3",
        )?;
        for (source_id, selected_sheet) in &source_scope {
            let rows = stmt.query_map(
                params![workspace_id.to_string(), source_id, selected_sheet],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )?;
            for row in rows {
                let (sid, sheet, raw) = row?;
                let layout: Value =
                    serde_json::from_str(&raw).map_err(|_| source_scope_unavailable())?;
                if !layout.is_object() {
                    return Err(source_scope_unavailable());
                }
                layouts.entry(sid).or_default().insert(sheet, layout);
            }
        }
    }
    // 单元格快照（按 source/sheet/行/列有序）
    type SheetCells = std::collections::BTreeMap<usize, Vec<CellSnap>>;
    let mut cells: std::collections::BTreeMap<(String, String), SheetCells> = Default::default();
    {
        let mut stmt = conn.prepare(
            "SELECT source_id, sheet, row_no, col_no, raw_text, cell_kind, value_state,
                    typed_value, formula, merged_range
             FROM cli_source_cells WHERE workspace_id = ?1 AND source_id = ?2 AND sheet = ?3
             ORDER BY source_id, sheet, row_no, col_no",
        )?;
        for (source_id, selected_sheet) in &source_scope {
            let rows = stmt.query_map(
                params![workspace_id.to_string(), source_id, selected_sheet],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        CellSnap {
                            row_no: r.get::<_, i64>(2)? as usize,
                            col_no: r.get::<_, i64>(3)? as usize,
                            raw_text: r.get(4)?,
                            cell_kind: r.get(5)?,
                            value_state: r.get(6)?,
                            typed_value: r.get(7)?,
                            formula: r.get(8)?,
                            merged_range: r.get(9)?,
                        },
                    ))
                },
            )?;
            for row in rows {
                let (sid, sheet, snap) = row?;
                cells
                    .entry((sid, sheet))
                    .or_default()
                    .entry(snap.row_no)
                    .or_default()
                    .push(snap);
            }
        }
    }
    // 来源已被版本快照选中，但其旁表快照不完整时必须让用户重算，不能静默丢弃。
    // 不要求最终裁剪有记录：有效 sheet 仍可按原有相关行/列规则不入选。
    for (source_id, sheet) in &source_scope {
        let has_layout = layouts
            .get(source_id)
            .is_some_and(|sheets| sheets.contains_key(sheet));
        let has_cells = cells
            .get(&(source_id.clone(), sheet.clone()))
            .is_some_and(|rows| !rows.is_empty());
        if !has_layout || !has_cells {
            return Err(source_scope_unavailable());
        }
    }
    // 证据锚点（file_label, sheet, row 1 基, col 0 基）：观察/补0/冲突候选/用户
    let mut anchors: Vec<(String, String, usize, usize)> = Vec::new();
    {
        let raw: Option<(String, Option<String>, String)> = conn
            .query_row(
                "SELECT normalized_metrics_json, comparisons_json, period_policy_snapshot_json
                 FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
                params![run_id, revision as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
            .map_err(CliError::from)?;
        let mut evidence_ids: Vec<String> = Vec::new();
        if let Some((normalized, comparisons, policy)) = raw {
            for text in [Some(normalized), comparisons, Some(policy)]
                .into_iter()
                .flatten()
            {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    collect_evidence_ids(&v, &mut evidence_ids);
                }
            }
        }
        {
            let mut stmt = conn.prepare(
                "SELECT candidate_values_json FROM source_conflicts
                 WHERE workspace_id = ?1 AND run_id = ?2",
            )?;
            let rows: Vec<String> = stmt
                .query_map(params![workspace_id.to_string(), run_id], |r| r.get(0))?
                .collect::<Result<_, _>>()
                .map_err(CliError::from)?;
            for text in rows {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    collect_evidence_ids(&v, &mut evidence_ids);
                }
            }
        }
        evidence_ids.extend(extra_evidence.iter().cloned());
        // file_label → source（同 label 多来源取最早注册：sources 已按
        // file_label/created_at/source_id 排序，entry 保留首个即最早）
        let mut label_to_source: std::collections::HashMap<&str, &SourceRow> = Default::default();
        for s in &sources {
            label_to_source.entry(s.file_label.as_str()).or_insert(s);
        }
        for id in evidence_ids {
            let Some((file_label, sheet, row, col)) = parse_cell_evidence(&id) else {
                continue; // manual:/content-hash 证据不经单元格域（manual_inputs 另投影）
            };
            if !label_to_source.contains_key(file_label.as_str()) {
                continue;
            }
            anchors.push((file_label, sheet, row, col));
        }
        anchors.sort();
        anchors.dedup();
    }

    // 逐来源/逐 sheet 计算选定范围
    let mut manifest = Vec::new();
    let mut records: Vec<SourceRecordProjection> = Vec::new();
    let mut scope_rows = Vec::new();
    for source in &sources {
        let Some(sheets) = layouts.get(&source.source_id) else {
            continue;
        };
        for (sheet, layout) in sheets {
            let Some(rows_of_sheet) = cells.get(&(source.source_id.clone(), sheet.clone())) else {
                continue;
            };
            let sheet_anchors: Vec<(usize, usize)> = anchors
                .iter()
                .filter(|(label, s, _, _)| *label == source.file_label && s == sheet)
                .map(|(_, _, r, c)| (*r, *c))
                .collect();
            let total_rows = rows_of_sheet.len();
            let total_cols = rows_of_sheet
                .values()
                .flat_map(|cells_row| cells_row.iter().map(|c| c.col_no))
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            // 选定单元格（行/列或证据锚点），按行列序输出
            let mut picked: std::collections::BTreeMap<usize, Vec<&CellSnap>> = Default::default();
            let mut trim_note = String::new();
            if let Some(transposed) = layout.get("transposed") {
                // 转置推移表：指标列整列（含 period 列 A＝col 0）；非指标列不发
                let mut cols: std::collections::BTreeSet<usize> =
                    std::collections::BTreeSet::from([0usize]);
                if let Some(list) = transposed.get("metric_columns").and_then(|v| v.as_array()) {
                    for pair in list {
                        if let Some(col) = pair.get(0).and_then(|v| v.as_u64()) {
                            cols.insert(col as usize);
                        }
                    }
                }
                for (row_no, row_cells) in rows_of_sheet {
                    for cell in row_cells {
                        if cols.contains(&cell.col_no) {
                            picked.entry(*row_no).or_default().push(cell);
                        }
                    }
                }
                // 证据锚点格保留（evidence 单元格不得丢——即使位于非指标列）
                for (row_no, col_no) in &sheet_anchors {
                    if let Some(cell) = rows_of_sheet
                        .get(row_no)
                        .and_then(|row_cells| row_cells.iter().find(|c| c.col_no == *col_no))
                    {
                        picked.entry(*row_no).or_default().push(cell);
                    }
                }
                let selected_cols = picked
                    .values()
                    .flat_map(|row_cells| row_cells.iter().map(|c| c.col_no))
                    .collect::<std::collections::BTreeSet<_>>();
                if !picked.is_empty() {
                    trim_note =
                        format!("已按指标列裁剪：{}/{} 列", selected_cols.len(), total_cols);
                }
            } else if let Some(header) = layout.get("header") {
                // 标准表：表头块行 ∪ 标签列命中已注册指标的行 ∪ 证据锚点行。
                // 仅剩表头（无指标行/锚点行）的 sheet 整体不选（该 sheet 对本次
                // 生成无支撑；未选内容不发）。
                let header_row = header.get("row").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let header_rows = header
                    .get("row_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as usize;
                let label_col = layout
                    .get("label_col")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize);
                let mut relevant_rows: Vec<usize> = Vec::new();
                for (row_no, row_cells) in rows_of_sheet {
                    let in_header = *row_no >= header_row && *row_no < header_row + header_rows;
                    if in_header {
                        continue;
                    }
                    let label_hit = label_col.is_some_and(|lc| {
                        row_cells
                            .iter()
                            .find(|c| c.col_no == lc)
                            .map(|c| metric_names.contains(c.raw_text.trim()))
                            .unwrap_or(false)
                    });
                    let anchor_hit = sheet_anchors.iter().any(|(r, _)| r == row_no);
                    if label_hit || anchor_hit {
                        relevant_rows.push(*row_no);
                    }
                }
                if relevant_rows.is_empty() {
                    continue;
                }
                for (row_no, row_cells) in rows_of_sheet {
                    let in_header = *row_no >= header_row && *row_no < header_row + header_rows;
                    if in_header || relevant_rows.contains(row_no) {
                        picked.insert(*row_no, row_cells.iter().collect());
                    }
                }
                trim_note = format!("已按相关行裁剪：{}/{} 行", picked.len(), total_rows);
            } else {
                // 无表头 sheet：仅证据锚点行（未选内容不发）
                for (row_no, col_no) in &sheet_anchors {
                    if let Some(row_cells) = rows_of_sheet.get(row_no) {
                        let cell = row_cells.iter().find(|c| c.col_no == *col_no);
                        if let Some(cell) = cell {
                            picked.entry(*row_no).or_default().push(cell);
                        }
                    }
                }
                if !picked.is_empty() {
                    trim_note = format!("已按证据行裁剪：{}/{} 行", picked.len(), total_rows);
                }
            }
            if picked.is_empty() {
                continue;
            }
            // 去重同行重复引用（锚点＋列选可能重叠）并按列排序
            let mut ordered: Vec<(usize, Vec<&CellSnap>)> = Vec::new();
            for (row_no, mut row_cells) in picked {
                row_cells.sort_by_key(|c| c.col_no);
                row_cells.dedup_by_key(|c| c.col_no);
                ordered.push((row_no, row_cells));
            }
            let min_row = ordered.first().map(|(r, _)| *r).unwrap_or(1);
            let max_row = ordered.last().map(|(r, _)| *r).unwrap_or(1);
            let max_col = ordered
                .iter()
                .flat_map(|(_, cells_row)| cells_row.iter().map(|c| c.col_no))
                .max()
                .unwrap_or(0);
            let range_str = format!(
                "A{}:{}{}（{trim_note}）",
                min_row,
                dsheng_core::import::headers::column_letter(max_col),
                max_row
            );
            let sheet_label: Option<String> = if source.kind == "csv" {
                None
            } else {
                Some(sheet.clone())
            };
            manifest.push(SourceManifestEntry {
                source_id: Uuid::parse_str(&source.source_id).unwrap_or(Uuid::nil()),
                file_label: source.file_label.clone(),
                sheet: sheet_label.clone(),
                selected_range: range_str.clone(),
                content_hash: source.content_hash.clone(),
                selection_hash: dsheng_core::analysis::fingerprint::sha256_hex(&format!(
                    "{}|{range_str}|{}",
                    source.source_id, source.content_hash
                )),
            });
            scope_rows.push(json!({
                "source_id": source.source_id,
                "file_label": source.file_label,
                "sheet": sheet_label,
                "selected_range": range_str,
                "row_count": ordered.len(),
                "content_hash": source.content_hash,
            }));
            let sheet_or_csv = if source.kind == "csv" {
                "csv".to_string()
            } else {
                sheet.clone()
            };
            for (row_no, row_cells) in ordered {
                for cell in row_cells {
                    let a1 = format!(
                        "{}{row_no}",
                        dsheng_core::import::headers::column_letter(cell.col_no)
                    );
                    records.push(SourceRecordProjection {
                        evidence_id: format!("cell:{}:{sheet_or_csv}:{a1}", source.source_id),
                        source_id: Uuid::parse_str(&source.source_id).unwrap_or(Uuid::nil()),
                        file_label: source.file_label.clone(),
                        sheet: sheet_label.clone(),
                        cell_or_row: a1,
                        values: SourceCellValues {
                            cell_type: match cell.cell_kind.as_str() {
                                "text" => CellTypeProjection::Text,
                                "bool" => CellTypeProjection::Boolean,
                                "formula" => CellTypeProjection::Formula,
                                "error" => CellTypeProjection::Error,
                                _ => CellTypeProjection::Number,
                            },
                            raw_text: cell.raw_text.clone(),
                            typed_value: cell.typed_value.clone().map(|v| {
                                if cell.cell_kind == "bool" {
                                    TypedCellValue::Bool(v == "true")
                                } else {
                                    TypedCellValue::Str(v)
                                }
                            }),
                            formula: cell.formula.clone(),
                            value_state: match cell.value_state.as_str() {
                                "valid" => ValueStateProjection::Valid,
                                "pending_input" => ValueStateProjection::PendingInput,
                                "invalid" => ValueStateProjection::Invalid,
                                "conflict" => ValueStateProjection::Conflict,
                                "configuration_required" => {
                                    ValueStateProjection::ConfigurationRequired
                                }
                                "unsupported_formula" => ValueStateProjection::UnsupportedFormula,
                                "precision_unverified" => ValueStateProjection::PrecisionUnverified,
                                _ => ValueStateProjection::Missing,
                            },
                            number_format: None,
                            merged_range: cell.merged_range.clone(),
                            mapping: None,
                        },
                    });
                }
            }
        }
    }
    // 合同排序：source_id/sheet/行列定位/evidence_id
    records.sort_by(|a, b| {
        a.source_id
            .cmp(&b.source_id)
            .then_with(|| a.sheet.cmp(&b.sheet))
            .then_with(|| a.cell_or_row.cmp(&b.cell_or_row))
            .then_with(|| a.evidence_id.cmp(&b.evidence_id))
    });
    Ok(TrimmedSourceSelection {
        manifest,
        records,
        scope_rows,
    })
}

/// 递归收集 JSON 内全部 evidence_ids/zero_filled_evidence_ids 数组字符串。
fn collect_evidence_ids(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if k == "evidence_ids" || k == "zero_filled_evidence_ids" {
                    if let Some(items) = val.as_array() {
                        for item in items {
                            if let Some(id) = item.as_str() {
                                out.push(id.to_string());
                            }
                        }
                    }
                } else {
                    collect_evidence_ids(val, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|i| collect_evidence_ids(i, out)),
        _ => {}
    }
}

/// 解析 `cell:{file_label}:{sheet}:{A1}` → (file_label, sheet, 行 1 基, 列 0 基)。
/// file_label 可含冒号；sheet/列行不可（末两段固定）。
fn parse_cell_evidence(id: &str) -> Option<(String, String, usize, usize)> {
    let rest = id.strip_prefix("cell:")?;
    let mut parts = rest.rsplitn(3, ':');
    let a1 = parts.next()?;
    let sheet = parts.next()?;
    let file_label = parts.next()?;
    if file_label.is_empty() || sheet.is_empty() {
        return None;
    }
    let col_letters: String = a1.chars().take_while(char::is_ascii_alphabetic).collect();
    let row_digits: String = a1.chars().skip_while(char::is_ascii_alphabetic).collect();
    if col_letters.is_empty() || row_digits.is_empty() {
        return None;
    }
    let mut col = 0usize;
    for ch in col_letters.chars() {
        col = col * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize + 1);
    }
    let row: usize = row_digits.parse().ok()?;
    Some((file_label.to_string(), sheet.to_string(), row, col - 1))
}

fn manual_projections(
    conn: &Connection,
    workspace_id: Uuid,
) -> CliResult<Vec<ManualInputProjection>> {
    let mut stmt = conn.prepare(
        // manualInput.metric_id 同样投影定义身份（manual_inputs 行存写入时的
        // 具体版本行 id，JOIN 归并为 stable_metric_id）
        "SELECT mi.input_id, md.stable_metric_id, mi.measure_role, mi.scope_key, mi.value,
                mi.unit, mi.confirmed, mi.source_note, mi.input_revision, mi.raw_text
         FROM manual_inputs mi
         JOIN metric_definitions md ON md.metric_id = mi.metric_id
         WHERE mi.workspace_id = ?1",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, i64>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, Option<String>>(9)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (input_id, metric_id, role, scope, value, unit, confirmed, note, revision, raw) = row?;
        let (Ok(input_uuid), Ok(metric_uuid)) =
            (Uuid::parse_str(&input_id), Uuid::parse_str(&metric_id))
        else {
            continue;
        };
        out.push(ManualInputProjection {
            input_id: input_uuid,
            metric_id: metric_uuid,
            value,
            unit,
            confirmed: confirmed == 1,
            source_note: note,
            evidence_ids: vec![format!("manual:{input_id}:r{revision}")],
            input_revision: revision as u32,
            measure_role: serde_json::from_value(json!(role))
                .unwrap_or(MeasureRoleProjection::Actual),
            period: None,
            scope_key: scope,
            raw_text: raw.unwrap_or_default(),
        });
    }
    Ok(out)
}

fn conflict_projections(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
) -> CliResult<Vec<SourceConflictProjection>> {
    let mut stmt = conn.prepare(
        "SELECT conflict_id, metric_id, measure_role, period_from, period_to, scope_key, unit,
                candidate_values_json, resolution_status, resolution_id, selected_evidence_id,
                resolution_note
         FROM source_conflicts WHERE workspace_id = ?1 AND run_id = ?2",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string(), run_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
            r.get::<_, String>(8)?,
            r.get::<_, Option<String>>(9)?,
            r.get::<_, Option<String>>(10)?,
            r.get::<_, Option<String>>(11)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (
            cid,
            mid,
            role,
            from,
            to,
            scope,
            unit,
            candidates_raw,
            status,
            resolution_id,
            selected,
            note,
        ) = row?;
        let (Ok(conflict_uuid), Ok(metric_uuid)) = (Uuid::parse_str(&cid), Uuid::parse_str(&mid))
        else {
            continue;
        };
        let candidates: Vec<ConflictCandidateValue> =
            serde_json::from_str::<Value>(&candidates_raw)
                .ok()
                .and_then(|v| serde_json::from_value(clone_with_numbers(&v)).ok())
                .unwrap_or_default();
        out.push(SourceConflictProjection {
            conflict_id: conflict_uuid,
            metric_id: metric_uuid,
            measure_role: serde_json::from_value(json!(role))
                .unwrap_or(MeasureRoleProjection::Actual),
            period: format!("{from}/{to}"),
            scope_key: scope,
            unit,
            candidate_values: candidates,
            resolution_status: serde_json::from_value(json!(status))
                .unwrap_or(ResolutionStatusProjection::Unresolved),
            resolution_id: resolution_id.and_then(|s| Uuid::parse_str(&s).ok()),
            selected_evidence_id: selected,
            resolution_note: note,
        });
    }
    Ok(out)
}

fn session_id_of(conn: &Connection, workspace_id: Uuid, run_id: &str) -> String {
    conn.query_row(
        "SELECT session_id FROM generation_sessions
         WHERE workspace_id = ?1 AND run_id = ?2 ORDER BY created_at DESC LIMIT 1",
        params![workspace_id.to_string(), run_id],
        |r| r.get(0),
    )
    .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

/// 会话身份按生成输入指纹解析：同指纹复用当前会话；指纹变化（§2.2 手工值/
/// Provider 快照等变化）＝新一代输入，另建新会话（旧会话保留不写）。
fn session_id_for_fingerprint(
    conn: &Connection,
    workspace_id: Uuid,
    _run_id: &str,
    generation_input_fingerprint: &str,
) -> String {
    conn.query_row(
        "SELECT session_id FROM generation_sessions
         WHERE workspace_id = ?1 AND generation_input_fingerprint = ?2
         ORDER BY created_at DESC LIMIT 1",
        params![workspace_id.to_string(), generation_input_fingerprint],
        |r| r.get(0),
    )
    .unwrap_or_else(|_| Uuid::new_v4().to_string())
}

// ---------------------------------------------------------------------------
// 预览与确认发送
// ---------------------------------------------------------------------------

pub struct ProviderConfigSnapshot {
    pub provider_id: Uuid,
    pub revision: u32,
    pub name: String,
    pub base_url: String,
    pub model: String,
    /// 流式开关（providers.stream，0003 起可为 true）。
    pub stream: bool,
    pub timeouts: Timeouts,
    pub budget: Budget,
    pub key_present: bool,
}

pub fn provider_snapshot(
    conn: &Connection,
    workspace_id: Uuid,
    provider_id: Uuid,
) -> CliResult<ProviderConfigSnapshot> {
    type ProviderRow = (
        String,
        String,
        String,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
    );
    let row: ProviderRow = conn
        .query_row(
            "SELECT name, base_url, model, revision, stream, connect_seconds, response_seconds,
                    total_seconds, context_window_tokens, output_reserve_tokens, api_key,
                    (SELECT workspace_id FROM providers WHERE provider_id = ?1)
             FROM providers WHERE provider_id = ?1",
            params![provider_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .map_err(|_| CliError::new(CliErrorCode::WorkspaceMismatch, "provider 不存在"))?;
    let owner = row.11.unwrap_or_default();
    if owner != workspace_id.to_string() {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "provider 不属于所选 workspace",
        ));
    }
    Ok(ProviderConfigSnapshot {
        provider_id,
        revision: row.3 as u32,
        name: row.0,
        base_url: row.1,
        model: row.2,
        stream: row.4 == 1,
        timeouts: Timeouts {
            connect_seconds: row.5 as u32,
            response_seconds: row.6 as u32,
            total_seconds: row.7 as u32,
        },
        budget: Budget {
            context_window_tokens: row.8 as u32,
            output_reserve_tokens: row.9 as u32,
        },
        key_present: row.10.is_some(),
    })
}

/// 组装请求（core assemble：用户文本/预算/字节上限检查）。
fn assemble(
    workflow: Workflow,
    framework: Option<Framework>,
    provider: &ProviderConfigSnapshot,
    material: &GenerationMaterial,
) -> CliResult<dsheng_core::generation::assemble::AssembledRequest> {
    dsheng_core::generation::assemble::assemble_request(
        workflow,
        framework,
        &provider.model,
        &material.payload_json,
        &material.user_supplied_text,
        &provider.budget,
    )
    .map_err(|e| CliError::new(e.code, e.detail))
}

// 同一指纹会复用会话；旧 schema 不支持追加该报告的新 revision。
// 必须在预览和发送前都检查，避免重复请求后才遇到保存冲突。
fn ensure_diagnosis_storage_available(
    conn: &Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
) -> CliResult<()> {
    if material.workflow == Workflow::Diagnosis {
        let existing: i64 = conn.query_row(
            "SELECT COUNT(*) FROM diagnosis_reports r JOIN generation_sessions s ON s.session_id = r.session_id
             WHERE s.workspace_id = ?1 AND s.run_id = ?2 AND s.generation_input_fingerprint = ?3",
            params![workspace_id.to_string(), material.run_id.to_string(), material.input_hash], |r| r.get(0),
        )?;
        if existing > 0 {
            return Err(CliError::new(
                CliErrorCode::DataValidationFailed,
                "相同问题和输入已有诊断报告，请查看本期诊断记录。此次未发送模型请求。",
            ));
        }
    }
    Ok(())
}

/// dry-run 预览：建 confirmation（TTL 600s）＋ confirmationPreview 视图。
pub fn create_preview(
    conn: &mut Connection,
    workspace_id: Uuid,
    provider: &ProviderConfigSnapshot,
    material: &GenerationMaterial,
    operation: GenerationOperation,
    prompt_version: &str,
    schema_version: &str,
) -> CliResult<Value> {
    ensure_diagnosis_storage_available(conn, workspace_id, material)?;
    if !provider.key_present {
        return Err(CliError::new(
            CliErrorCode::ProviderKeyMissing,
            "Provider 未配置 API Key（--api-key-stdin 经 provider set 配置）",
        ));
    }
    let assembled = assemble(material.workflow, material.framework, provider, material)?;
    let now = Utc::now();
    let binding = ConfirmationBinding {
        confirmation_id: Uuid::new_v4(),
        workflow: material.workflow,
        operation,
        workspace_id,
        provider_id: provider.provider_id,
        provider_revision: provider.revision,
        run_id: Some(material.run_id),
        analysis_revision_no: Some(material.analysis_revision_no),
        analysis_input_fingerprint: material.analysis_input_fingerprint.clone(),
        generation_input_fingerprint: material.input_hash.clone(),
        report_id: material.report_id,
        report_revision_no: material.report_revision_no,
        prompt_version: prompt_version.to_string(),
        schema_version: schema_version.to_string(),
        context_policy_version: Some(
            dsheng_core::contracts::prompts::CONTEXT_POLICY_VERSION.to_string(),
        ),
        source_manifest_hash: material.source_manifest_hash.clone(),
        input_hash: material.input_hash.clone(),
        request_bytes: assembled.request_bytes,
        conservative_input_tokens: assembled.conservative_input_tokens,
        budget: provider.budget,
        timeouts: provider.timeouts,
        created_at: now,
        expires_at: now + chrono::Duration::seconds(CONFIRMATION_TTL_SECONDS),
    };
    let mut store = SqliteGenerationStore { conn };
    let gate = ConfirmationGate;
    let created = gate
        .create(&mut store, binding)
        .map_err(|(code, msg)| CliError::new(code, msg))?;
    // 会话行（send 时消费确认复用）
    conn.execute(
        "INSERT OR IGNORE INTO generation_sessions (session_id, workspace_id, run_id,
                analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                workflow, framework, state, prompt_version, schema_version, input_schema_version,
                context_policy_version, source_manifest_hash, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'preview', ?9, ?10, '2', '2', ?11, ?12, ?12)",
        params![
            session_id_for_fingerprint(
                conn,
                workspace_id,
                &material.run_id.to_string(),
                &material.input_hash
            ),
            workspace_id.to_string(),
            material.run_id.to_string(),
            material.analysis_revision_no as i64,
            material.analysis_input_fingerprint,
            material.input_hash,
            serde_json::to_value(material.workflow)
                .unwrap()
                .as_str()
                .unwrap(),
            material.framework.map(framework_db_str),
            prompt_version,
            schema_version,
            material.source_manifest_hash,
            now_iso()
        ],
    )
    .map_err(CliError::from)?;
    let source_scope_summary: Vec<Value> = material.scope_rows.clone();
    Ok(json!({
        "confirmation_id": created.confirmation_id.to_string(),
        "expires_at": dsheng_core::repository::utc_iso(created.expires_at),
        "workspace_id": workspace_id.to_string(),
        "workflow": serde_json::to_value(created.workflow).unwrap(),
        "operation": serde_json::to_value(created.operation).unwrap(),
        "provider": {
            "provider_id": provider.provider_id.to_string(),
            "revision": provider.revision,
            "name": provider.name,
            "base_url": provider.base_url,
            "model": provider.model,
        },
        "run_id": material.run_id.to_string(),
        "analysis_revision_no": material.analysis_revision_no,
        "analysis_input_fingerprint": material.analysis_input_fingerprint,
        "report_id": material.report_id.map(|u| u.to_string()),
        "report_revision_no": material.report_revision_no,
        "input_manifest_hash": material.input_hash,
        "prompt_version": prompt_version,
        "schema_version": schema_version,
        "context_policy_version": dsheng_core::contracts::prompts::CONTEXT_POLICY_VERSION,
        "source_scope_summary": source_scope_summary,
        "request_bytes": assembled.request_bytes,
        "conservative_input_tokens": assembled.conservative_input_tokens,
        "budget": provider.budget,
        "timeouts": provider.timeouts,
        "request_count_maximum": REQUEST_COUNT_MAXIMUM,
        "key_present": true,
    }))
}

/// 确认发送（双门禁之后）：重验 → 消费建 attempt → 派发（注入）→ 落库 → 终态。
/// `on_progress`：发送阶段与流式进度；预览不触发。
/// 返回 (attempt_id, 提取后的输出对象)；成功终态由后续保存事务提交。
#[allow(clippy::too_many_arguments)]
pub fn send_confirmed(
    conn: &mut Connection,
    workspace_id: Uuid,
    provider: &ProviderConfigSnapshot,
    material: &GenerationMaterial,
    confirmation_id: Uuid,
    prompt_version: &str,
    schema_version: &str,
    dispatcher: &mut dyn RequestDispatch,
    on_progress: &mut dyn FnMut(StreamProgressEvent),
) -> CliResult<(Uuid, Value)> {
    ensure_diagnosis_storage_available(conn, workspace_id, material)?;
    let api_key = crate::store::settings::provider_api_key(conn, provider.provider_id)?
        .ok_or_else(|| {
            CliError::new(
                CliErrorCode::ProviderKeyMissing,
                "Provider Key 已被清除（须重新预览确认）",
            )
        })?;
    let gate = ConfirmationGate;
    let mut store = SqliteGenerationStore { conn };
    let binding = store.load_confirmation(confirmation_id).ok_or_else(|| {
        CliError::new(
            CliErrorCode::ConfirmationRequired,
            "确认不存在，须先 --dry-run 预览",
        )
    })?;
    let assembled = assemble(material.workflow, material.framework, provider, material)?;
    // 发送前重验快照：payload/manifest/输入指纹/预算逐项对比
    let snapshot = SendSnapshot {
        workspace_id,
        provider_id: provider.provider_id,
        provider_revision: provider.revision,
        run_id: Some(material.run_id),
        analysis_revision_no: Some(material.analysis_revision_no),
        analysis_input_fingerprint: material.analysis_input_fingerprint.clone(),
        generation_input_fingerprint: material.input_hash.clone(),
        report_id: material.report_id,
        report_revision_no: material.report_revision_no,
        prompt_version: prompt_version.to_string(),
        schema_version: schema_version.to_string(),
        context_policy_version: Some(
            dsheng_core::contracts::prompts::CONTEXT_POLICY_VERSION.to_string(),
        ),
        source_manifest_hash: material.source_manifest_hash.clone(),
        input_hash: material.input_hash.clone(),
        request_bytes: assembled.request_bytes,
        conservative_input_tokens: assembled.conservative_input_tokens,
        budget: provider.budget,
        timeouts: provider.timeouts,
    };
    gate.verify_before_send(&binding, &snapshot, Utc::now())
        .map_err(|(code, msg)| CliError::new(code, msg))?;
    let outcome = gate
        .consume_and_create_attempt(&mut store, confirmation_id, Utc::now())
        .map_err(|(code, msg)| CliError::new(code, msg))?;
    let attempt = outcome.attempt().clone();
    let started = std::time::Instant::now();
    ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
    on_progress(StreamProgressEvent {
        attempt_id: attempt.attempt_id,
        confirmation_id,
        workspace_id,
        chunk_seq: 0,
        bytes: 0,
        elapsed_ms: 0,
        stage: "waiting",
    });
    ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
    if let Err((code, msg)) = gate.mark_dispatched(&mut store, attempt.attempt_id, Utc::now()) {
        ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
        return Err(CliError::new(code, msg));
    }
    let runtime = ProviderRuntime {
        name: provider.name.clone(),
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        stream: provider.stream,
        timeouts: provider.timeouts,
        budget: provider.budget,
        api_key,
    };
    set_attempt_stage(
        store.conn,
        workspace_id,
        attempt.attempt_id,
        "waiting_response",
    )?;
    let mut last_chunk = (0, 0);
    let response_result = {
        let should_cancel =
            || ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id).is_err();
        let mut chunk_progress = |seq: u64, bytes: u64, ms: i64| {
            if should_cancel() {
                return;
            }
            last_chunk = (seq, bytes);
            on_progress(StreamProgressEvent {
                attempt_id: attempt.attempt_id,
                confirmation_id,
                workspace_id,
                chunk_seq: seq,
                bytes,
                elapsed_ms: ms,
                stage: "receiving",
            });
        };
        dispatcher.dispatch_cancellable(
            &runtime,
            &assembled.request,
            &mut chunk_progress,
            &should_cancel,
        )
    };
    // A terminal response racing with manual stop must never enter parsing/saving.
    ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
    let response = match response_result {
        Ok(response) => response,
        Err((code, msg)) => {
            let error = CliError::new(code, msg);
            finish_attempt_error(
                store.conn,
                workspace_id,
                attempt.attempt_id,
                &error,
                "request",
            )?;
            return Err(error);
        }
    };
    set_attempt_stage(
        store.conn,
        workspace_id,
        attempt.attempt_id,
        "schema_validation",
    )?;
    on_progress(StreamProgressEvent {
        attempt_id: attempt.attempt_id,
        confirmation_id,
        workspace_id,
        chunk_seq: last_chunk.0,
        bytes: last_chunk.1,
        elapsed_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
        stage: "validating",
    });
    ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
    // 提取正文（C 方案：严格优先＋唯一裸任务对象本地包装）
    let content = response
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .unwrap_or_default();
    if let Some(reason) = response
        .choices
        .first()
        .and_then(|c| c.finish_reason.as_deref())
    {
        if reason != "stop" {
            let error = CliError::new(
                CliErrorCode::IncompleteOutput,
                format!("finish_reason={reason} 非 stop"),
            );
            finish_attempt_error(
                store.conn,
                workspace_id,
                attempt.attempt_id,
                &error,
                "validation",
            )?;
            return Err(error);
        }
    }
    let extraction = dsheng_core::provider::extract::choose(
        &content,
        &SchemaChecksImpl,
        dsheng_core::provider::extract::ExtractPolicy::Adapt,
    )
    .to_extraction_outcome();
    let output = extraction.object.clone();
    match output {
        Some(obj) => {
            // TS-22 提交门禁：发送期间分析被重做 → 迟到响应不提交，
            // attempt 置 failed(INPUT_REVISION_CHANGED)（不覆盖上一有效结果）
            commit_gate_before_save(
                &mut *store.conn,
                workspace_id,
                material.run_id,
                attempt.attempt_id,
                material.analysis_revision_no,
                &material.analysis_input_fingerprint,
            )?;
            // Both workflows commit success only alongside their locally saved output.
            set_attempt_stage(store.conn, workspace_id, attempt.attempt_id, "saving")?;
            on_progress(StreamProgressEvent {
                attempt_id: attempt.attempt_id,
                confirmation_id,
                workspace_id,
                chunk_seq: last_chunk.0,
                bytes: last_chunk.1,
                elapsed_ms: started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                stage: "saving",
            });
            ensure_attempt_active(store.conn, workspace_id, attempt.attempt_id)?;
            Ok((attempt.attempt_id, obj))
        }
        None => {
            let error = CliError::new(
                CliErrorCode::InvalidJson,
                format!("响应无可提取 JSON 对象（stage={:?}）", extraction.stage),
            );
            finish_attempt_error(
                store.conn,
                workspace_id,
                attempt.attempt_id,
                &error,
                "validation",
            )?;
            Err(error)
        }
    }
}

/// 提取检查（任务输出合同判定；诊断文本由上层走 turn schema）。
struct SchemaChecksImpl;

impl dsheng_core::provider::extract::ExtractionChecks for SchemaChecksImpl {
    fn root_contract_passes(&self, v: &Value) -> bool {
        dsheng_core::provider::schema::validate_task_output(v).is_ok()
    }
    fn task_subschema_passes(&self, v: &Value) -> bool {
        dsheng_core::provider::schema::ContractValidators::errors_of(
            &dsheng_core::provider::schema::contract_validators().task_sub,
            v,
        )
        .is_empty()
    }
}

// ---------------------------------------------------------------------------
// 生成提交门禁（TS-22 / 用例 60；集成轮 1 复审 P2-4 接线）
// ---------------------------------------------------------------------------

/// attempt 置 failed 并落 error_code=INPUT_REVISION_CHANGED（TS-22 迟到终态）。
///
/// 经 core `ConfirmationGate::finish` 推进（终态不可回退；已是终态则
/// IDEMPOTENCY_CONFLICT）；error_code/error_details 单独落列（gate 不携带）。
/// 不写候选/报告——失败不覆盖上一有效结果。
pub fn finish_attempt_input_revision_changed(
    store: &mut SqliteGenerationStore<'_>,
    attempt_id: Uuid,
) -> CliResult<()> {
    ConfirmationGate
        .finish(store, attempt_id, TerminalState::Failed, Utc::now())
        .map_err(|(code, msg)| CliError::new(code, msg))?;
    store
        .conn
        .execute(
            "UPDATE generation_attempts SET error_code = 'INPUT_REVISION_CHANGED',
                    error_details_json = ?2
             WHERE attempt_id = ?1 AND terminal_state = 'failed'",
            params![
                attempt_id.to_string(),
                serde_json::to_string(&json!({
                    "stage": "commit_gate",
                    "reason": "analysis revision changed while request in flight"
                }))
                .expect("json")
            ],
        )
        .map_err(CliError::from)?;
    Ok(())
}

/// 生成提交门禁（TS-22）：响应返回后、提交前核对当前分析 revision/指纹。
///
/// 迟到（任一不一致）→ attempt 置 failed(INPUT_REVISION_CHANGED) 并返回
/// `Err(INPUT_REVISION_CHANGED)`——不提交输出、不覆盖上一有效结果，
/// 重试须基于新 revision 重新预览确认。
pub fn commit_gate_before_save(
    conn: &mut Connection,
    workspace_id: Uuid,
    run_id: Uuid,
    attempt_id: Uuid,
    attempt_revision_no: u32,
    attempt_fingerprint: &str,
) -> CliResult<()> {
    let (current_rev, current_fp) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        &run_id.to_string(),
    )
    .map_err(CliError::from)?;
    let gate_ok = dsheng_core::repository::tasks::check_generation_commit_gate(
        current_rev,
        &current_fp,
        attempt_revision_no,
        attempt_fingerprint,
    );
    if gate_ok.is_err() {
        let mut store = SqliteGenerationStore { conn };
        finish_attempt_input_revision_changed(&mut store, attempt_id)?;
        return Err(CliError::new(
            CliErrorCode::InputRevisionChanged,
            "迟到响应：分析 revision 已变化，不提交（须基于新 revision 重新预览确认）",
        ));
    }
    Ok(())
}

/// generation status（attempt 状态对象）。
type AttemptStatusRow = (
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
    Option<String>,
    String,
);

pub fn attempt_status(conn: &Connection, workspace_id: Uuid, attempt_id: Uuid) -> CliResult<Value> {
    let row: Option<AttemptStatusRow> = conn
        .query_row(
            "SELECT a.attempt_id, s.session_id, a.operation, a.request_stage, a.request_count,
                    a.terminal_state, a.error_code, a.workspace_id
             FROM generation_attempts a JOIN generation_sessions s ON s.session_id = a.session_id
             WHERE a.attempt_id = ?1",
            params![attempt_id.to_string()],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                ))
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(CliError::from)?;
    let Some((aid, session, operation, stage, count, terminal, error_code, owner)) = row else {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "attempt 不存在",
        ));
    };
    if owner != workspace_id.to_string() {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "attempt 不属于所选 workspace",
        ));
    }
    Ok(json!({
        "attempt_id": aid,
        "session_id": session,
        "operation": operation,
        "request_stage": stage,
        "request_count": count,
        "terminal_state": terminal,
        "error_code": error_code,
    }))
}

const REMOTE_BILLING_NOTE: &str = "本地已请求停止；不保证远端已停止计费";

fn cancel_inflight(conn: &Connection, workspace_id: Uuid, attempt_id: Uuid) -> CliResult<bool> {
    let changed = conn.execute(
        "UPDATE generation_attempts SET terminal_state = 'cancelled', request_stage = 'terminal',
            error_code = 'CANCELLED', finished_at = ?3
         WHERE attempt_id = ?1 AND workspace_id = ?2 AND terminal_state IS NULL",
        params![attempt_id.to_string(), workspace_id.to_string(), now_iso()],
    )?;
    Ok(changed == 1)
}

/// Stop by the preview identity, even if the first progress event has not arrived.
/// Pre-consumption cancellation atomically creates an unsent, cancelled attempt;
/// a concurrent sender can only observe that terminal record and cannot dispatch.
pub fn cancel_by_confirmation(
    conn: &mut Connection,
    workspace_id: Uuid,
    confirmation_id: Uuid,
) -> CliResult<Value> {
    let binding = SqliteGenerationStore { conn: &mut *conn }
        .load_confirmation(confirmation_id)
        .ok_or_else(|| CliError::new(CliErrorCode::ConfirmationRequired, "确认不存在"))?;
    if binding.workspace_id != workspace_id {
        return Err(CliError::new(
            CliErrorCode::WorkspaceMismatch,
            "确认不属于所选 workspace",
        ));
    }
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let attempt = consume_confirmation_record(&tx, &binding, Utc::now())?
        .attempt()
        .clone();
    let cancelled = cancel_inflight(&tx, workspace_id, attempt.attempt_id)?
        || attempt.terminal == Some(TerminalState::Cancelled);
    tx.commit()?;
    Ok(json!({
        "attempt_id": attempt.attempt_id.to_string(),
        "cancelled": cancelled,
        "remote_billing_note": REMOTE_BILLING_NOTE,
    }))
}

/// CLI cancellation shares the same terminal CAS as confirmation-based stopping.
pub fn cancel_attempt(
    conn: &mut Connection,
    workspace_id: Uuid,
    attempt_id: Uuid,
) -> CliResult<Value> {
    attempt_status(conn, workspace_id, attempt_id)?;
    if !cancel_inflight(conn, workspace_id, attempt_id)? {
        return Err(CliError::new(
            CliErrorCode::IdempotencyConflict,
            "attempt 已是终态，无法取消",
        ));
    }
    Ok(json!({
        "attempt_id": attempt_id.to_string(),
        "cancelled": true,
        "remote_billing_note": REMOTE_BILLING_NOTE,
    }))
}

/// Re-read the persisted state at each asynchronous or local commit boundary.
fn ensure_attempt_active(conn: &Connection, workspace_id: Uuid, attempt_id: Uuid) -> CliResult<()> {
    let terminal: Option<String> = conn.query_row(
        "SELECT terminal_state FROM generation_attempts WHERE attempt_id = ?1 AND workspace_id = ?2",
        params![attempt_id.to_string(), workspace_id.to_string()], |r| r.get(0),
    )?;
    match terminal.as_deref() {
        None => Ok(()),
        Some("cancelled") => Err(CliError::new(CliErrorCode::Cancelled, "生成已手动停止")),
        Some(_) => Err(CliError::new(
            CliErrorCode::IdempotencyConflict,
            "attempt 已终止，不提交迟到响应",
        )),
    }
}

fn set_attempt_stage(
    conn: &Connection,
    workspace_id: Uuid,
    attempt_id: Uuid,
    stage: &str,
) -> CliResult<()> {
    let changed = conn.execute(
        "UPDATE generation_attempts SET request_stage = ?3
         WHERE attempt_id = ?1 AND workspace_id = ?2 AND terminal_state IS NULL",
        params![attempt_id.to_string(), workspace_id.to_string(), stage],
    )?;
    if changed != 1 {
        ensure_attempt_active(conn, workspace_id, attempt_id)?;
    }
    Ok(())
}

fn finish_attempt_error(
    conn: &Connection,
    workspace_id: Uuid,
    attempt_id: Uuid,
    error: &CliError,
    stage: &str,
) -> CliResult<()> {
    let terminal = match error.code {
        CliErrorCode::Cancelled => "cancelled",
        CliErrorCode::Interrupted => "interrupted",
        CliErrorCode::ConnectTimeout
        | CliErrorCode::ResponseTimeout
        | CliErrorCode::TotalTimeout => "timed_out",
        _ => "failed",
    };
    conn.execute(
        "UPDATE generation_attempts SET terminal_state = ?3, request_stage = 'terminal',
            error_code = ?4, error_details_json = ?5, finished_at = ?6
         WHERE attempt_id = ?1 AND workspace_id = ?2 AND terminal_state IS NULL",
        params![
            attempt_id.to_string(),
            workspace_id.to_string(),
            terminal,
            error.code.as_str(),
            json!({"stage":stage}).to_string(),
            now_iso()
        ],
    )?;
    Ok(())
}

/// 供 tests 使用的保守 token 估算重导出（DI 断言用）。
pub fn tokens_of(system_bytes: u64, user_bytes: u64) -> u64 {
    conservative_input_tokens(system_bytes, user_bytes)
}

/// Save a validated task candidate and its success terminal in the same transaction.
pub fn save_task_candidate(
    conn: &mut Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
    attempt_id: Uuid,
    output: &Value,
    envelope_locally_reconstructed: bool,
) -> CliResult<String> {
    let result = (|| {
        dsheng_core::provider::schema::validate_task_output(output).map_err(|errors| {
            CliError::new(CliErrorCode::SchemaValidationFailed, errors.join("; "))
        })?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let matches: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM generation_attempts a
             JOIN generation_sessions s ON s.session_id = a.session_id
             WHERE a.attempt_id = ?1 AND a.workspace_id = ?2 AND s.workspace_id = ?2
               AND a.workflow = 'task' AND s.workflow = 'task' AND s.run_id = ?3
               AND s.analysis_revision_no = ?4 AND s.analysis_input_fingerprint = ?5
               AND a.input_hash = ?6)",
            params![
                attempt_id.to_string(),
                workspace_id.to_string(),
                material.run_id.to_string(),
                material.analysis_revision_no,
                material.analysis_input_fingerprint,
                material.input_hash
            ],
            |r| r.get(0),
        )?;
        if !matches {
            return Err(CliError::new(
                CliErrorCode::DataValidationFailed,
                "任务 attempt 与会话不匹配",
            ));
        }
        ensure_attempt_active(&tx, workspace_id, attempt_id)?;
        let (revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
            &tx,
            &workspace_id.to_string(),
            &material.run_id.to_string(),
        )
        .map_err(CliError::from)?;
        dsheng_core::repository::tasks::check_generation_commit_gate(
            revision,
            &fingerprint,
            material.analysis_revision_no,
            &material.analysis_input_fingerprint,
        )
        .map_err(|_| {
            CliError::new(
                CliErrorCode::InputRevisionChanged,
                "迟到响应：分析 revision 已变化，不提交任务候选",
            )
        })?;
        let candidate_id = Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id,
                analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                candidate_revision_no, attempt_id, output_json, envelope_locally_reconstructed,
                state, prompt_version, schema_version, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7, ?8, ?9, 'candidate', '2.3', '2', ?10)",
            params![
                candidate_id,
                workspace_id.to_string(),
                material.run_id.to_string(),
                material.analysis_revision_no as i64,
                material.analysis_input_fingerprint,
                material.input_hash,
                attempt_id.to_string(),
                serde_json::to_string(output).unwrap(),
                envelope_locally_reconstructed as i64,
                now_iso()
            ],
        )
        .map_err(CliError::from)?;

        let changed = tx.execute(
            "UPDATE generation_attempts SET terminal_state = 'success', request_stage = 'terminal',
                finished_at = ?2 WHERE attempt_id = ?1 AND terminal_state IS NULL",
            params![attempt_id.to_string(), now_iso()],
        )?;
        if changed != 1 {
            return Err(CliError::new(
                CliErrorCode::IdempotencyConflict,
                "attempt 已终止，不提交迟到响应",
            ));
        }
        tx.commit()?;
        Ok(candidate_id)
    })();
    if let Err(error) = &result {
        finish_attempt_error(conn, workspace_id, attempt_id, error, "task_save")?;
    }
    result
}

/// 保存已经通过 schema 校验的诊断输出；本地事务同时写轮次和完整报告。
/// 会话绑定来自本次 attempt，而不是同一 run 的“最新”会话。
pub fn save_diagnosis_output(
    conn: &mut Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
    framework: Framework,
    attempt_id: Uuid,
    output: &Value,
) -> CliResult<Value> {
    let result = save_diagnosis_output_transaction(
        conn,
        workspace_id,
        material,
        framework,
        attempt_id,
        output,
    );
    if let Err(error) = &result {
        // 失败事务已经回滚；仅推进在途 attempt，绝不覆盖取消/超时等终态。
        conn.execute(
            "UPDATE generation_attempts SET terminal_state = 'failed', request_stage = 'terminal',
                error_code = ?3, error_details_json = ?4, finished_at = ?5
             WHERE attempt_id = ?1 AND workspace_id = ?2 AND workflow = 'diagnosis'
               AND terminal_state IS NULL",
            params![
                attempt_id.to_string(),
                workspace_id.to_string(),
                error.code.as_str(),
                json!({"stage":"diagnosis_save"}).to_string(),
                now_iso()
            ],
        )?;
    }
    result
}

fn save_diagnosis_output_transaction(
    conn: &mut Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
    framework: Framework,
    attempt_id: Uuid,
    output: &Value,
) -> CliResult<Value> {
    use dsheng_core::generation::report::{
        apply_report_patch, ReportPatch, ReportPeriod, ReportScope, ReportSeed,
    };
    dsheng_core::provider::schema::validate_diagnosis_turn(framework, output)
        .map_err(|errors| CliError::new(CliErrorCode::SchemaValidationFailed, errors.join("; ")))?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (session_id, provider_id, terminal): (String, String, Option<String>) = tx
        .query_row(
            "SELECT a.session_id, a.provider_id, a.terminal_state FROM generation_attempts a
         JOIN generation_sessions s ON s.session_id = a.session_id
         WHERE a.attempt_id = ?1 AND a.workspace_id = ?2 AND s.workspace_id = ?2
           AND s.run_id = ?3 AND s.workflow = 'diagnosis' AND s.framework = ?4",
            params![
                attempt_id.to_string(),
                workspace_id.to_string(),
                material.run_id.to_string(),
                framework_db_str(framework)
            ],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(|_| {
            CliError::new(
                CliErrorCode::DataValidationFailed,
                "诊断 attempt 与会话不匹配",
            )
        })?;
    if terminal.is_some() {
        ensure_attempt_active(&tx, workspace_id, attempt_id)?;
    }
    let (revision, fingerprint) = dsheng_core::repository::analysis_runs::current_revision(
        &tx,
        &workspace_id.to_string(),
        &material.run_id.to_string(),
    )
    .map_err(CliError::from)?;
    dsheng_core::repository::tasks::check_generation_commit_gate(
        revision,
        &fingerprint,
        material.analysis_revision_no,
        &material.analysis_input_fingerprint,
    )
    .map_err(|_| {
        CliError::new(
            CliErrorCode::InputRevisionChanged,
            "迟到响应：分析 revision 已变化，不提交诊断报告",
        )
    })?;
    let existing: i64 = tx.query_row(
        "SELECT COUNT(*) FROM diagnosis_reports WHERE session_id = ?1",
        [&session_id],
        |r| r.get(0),
    )?;
    if existing > 0 {
        return Err(CliError::new(
            CliErrorCode::DataValidationFailed,
            "当前版本尚不支持保存诊断续问的新版本；已保存报告保持不变",
        ));
    }
    let turn_no: i64 = tx.query_row(
        "SELECT COALESCE(MAX(turn_no), 0) + 1 FROM diagnosis_turns WHERE session_id = ?1",
        [&session_id],
        |r| r.get(0),
    )?;
    let payload: Value = serde_json::from_str(&material.payload_json)
        .map_err(|_| CliError::new(CliErrorCode::InputContractInvalid, "诊断输入 JSON 损坏"))?;
    let parse_error =
        |e: serde_json::Error| CliError::new(CliErrorCode::InputContractInvalid, e.to_string());
    let now = now_iso();
    let patch = output.get("report_patch").filter(|v| !v.is_null());
    let report = if let Some(patch) = patch {
        let patch: ReportPatch = serde_json::from_value(patch.clone()).map_err(parse_error)?;
        let seed = ReportSeed {
            report_id: Uuid::new_v4().to_string(),
            generation_session_id: session_id.clone(),
            workspace_id: workspace_id.to_string(),
            run_id: material.run_id.to_string(),
            framework: framework.into(),
            scope: ReportScope {
                problem_statement: payload
                    .pointer("/scope/problem_statement")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CliError::new(CliErrorCode::InputContractInvalid, "诊断输入缺少问题范围")
                    })?
                    .to_string(),
                focus_metric_ids: serde_json::from_value(
                    payload["scope"]["focus_metric_ids"].clone(),
                )
                .map_err(parse_error)?,
            },
            period: ReportPeriod {
                from: payload
                    .pointer("/period/source_from")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                to: payload
                    .pointer("/period/source_to")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                period_policy_version: payload
                    .pointer("/period/period_policy_version")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        CliError::new(CliErrorCode::InputContractInvalid, "诊断输入缺少周期策略")
                    })?
                    .to_string(),
            },
            source_manifest: serde_json::from_value(payload["source_manifest"].clone())
                .map_err(parse_error)?,
            analysis_revision_no: material.analysis_revision_no,
            analysis_input_fingerprint: material.analysis_input_fingerprint.clone(),
            now: now.clone(),
            current_user_message: Some(material.user_supplied_text.clone()),
        };
        let report =
            apply_report_patch(None, &patch, &seed).map_err(|e| CliError::new(e.code, e.detail))?;
        let value = serde_json::to_value(&report).map_err(parse_error)?;
        dsheng_core::provider::schema::validate_diagnosis_report(&value).map_err(|errors| {
            CliError::new(CliErrorCode::SchemaValidationFailed, errors.join("; "))
        })?;
        tx.execute(
            "INSERT INTO diagnosis_reports (report_id, workspace_id, run_id, analysis_revision_no,
                analysis_input_fingerprint, report_revision_no, session_id, framework,
                problem_statement, focus_metric_id, report_json, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'accepted', ?12, ?12)",
            params![
                report.report_id,
                report.workspace_id,
                report.run_id,
                report.analysis_revision_no,
                report.analysis_input_fingerprint,
                report.revision_no,
                session_id,
                framework_db_str(framework),
                report.scope.problem_statement,
                report.scope.focus_metric_ids.first(),
                value.to_string(),
                now
            ],
        )?;
        Some(report)
    } else {
        None
    };
    let state = match output["turn_disposition"].as_str() {
        Some("needs_clarification") => "clarification",
        Some("off_topic_redirect") => "redirect",
        _ => "patched",
    };
    let turn_id = Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO diagnosis_turns (turn_id, session_id, workspace_id, turn_no, round_no,
            user_message, turn_kind, turn_state, accepted_facts_json, report_patch_json,
            report_revision_after, context_manifest_json, prompt_version, schema_version,
            provider_id, attempt_id, clarification_questions_json, redirect_message, stop_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, 'model_turn', ?6, ?7, ?8, ?9, '{}', '2.0', '3', ?10, ?11, ?12, ?13, ?14, ?15)",
        params![turn_id, session_id, workspace_id.to_string(), turn_no, material.user_supplied_text,
            state, output["accepted_facts"].to_string(), patch.map(Value::to_string),
            report.as_ref().map(|r| r.revision_no), provider_id, attempt_id.to_string(),
            output["clarification_questions"].to_string(), output["redirect_message"].as_str(),
            output["stop_reason"].as_str(), now],
    )?;
    // 仅保存已确认的范围元数据，不保存完整 prompt 或原始表内容。
    tx.execute("UPDATE generation_sessions SET scope_snapshot_json = ?2, updated_at = ?3 WHERE session_id = ?1",
        params![session_id, json!({"scope":payload["scope"],"period":payload["period"],"source_manifest":payload["source_manifest"]}).to_string(), now])?;
    let changed = tx.execute(
        "UPDATE generation_attempts SET terminal_state = 'success', request_stage = 'terminal',
            finished_at = ?2 WHERE attempt_id = ?1 AND terminal_state IS NULL",
        params![attempt_id.to_string(), now],
    )?;
    if changed != 1 {
        return Err(CliError::new(
            CliErrorCode::IdempotencyConflict,
            "诊断 attempt 已终止，不提交迟到响应",
        ));
    }
    tx.commit()?;
    Ok(
        json!({"attempt_id":attempt_id.to_string(),"session_id":session_id,
        "turn_id":turn_id,"turn_state":state,"report_id":report.as_ref().map(|r| &r.report_id)}),
    )
}

/// run 归属 workspace（generation_cmd 辅助）。
pub fn workspace_of_run(conn: &Connection, run_id: &str) -> CliResult<String> {
    conn.query_row(
        "SELECT workspace_id FROM analysis_runs WHERE run_id = ?1",
        params![run_id],
        |r| r.get(0),
    )
    .map_err(|_| CliError::new(CliErrorCode::WorkspaceMismatch, "run 不存在"))
}

/// 工作区全部指标 ID（stable + 各版本），供 focus metric 校验兼容两种 ID。
pub fn workspace_metric_ids(
    conn: &rusqlite::Connection,
    workspace_id: &uuid::Uuid,
) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT metric_id FROM metric_definitions WHERE workspace_id = ?1
         UNION SELECT stable_metric_id FROM metric_definitions WHERE workspace_id = ?1",
    )?;
    let ids = stmt
        .query_map(rusqlite::params![workspace_id.to_string()], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    Ok(ids)
}
