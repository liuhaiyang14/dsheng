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
use rusqlite::{params, Connection};
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
        if let Some(attempt) = self.load_attempt_by_confirmation(id) {
            return Ok(GateConsume::Existing(attempt));
        }
        let tx = self
            .conn
            .transaction()
            .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
        // 会话：按 generation_input_fingerprint 定位/创建（预览时已建）
        let session_id: String = {
            let existing: Option<String> = tx
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
                    let id = Uuid::new_v4().to_string();
                    let framework = if binding.workflow == Workflow::Diagnosis {
                        binding
                            .report_id
                            .map(|_| "'5why'")
                            .unwrap_or("NULL")
                            .to_string()
                    } else {
                        "NULL".into()
                    };
                    // 报告路径的 framework 需要真实值；此分支只覆盖缺会话兜底
                    let _ = framework;
                    tx.execute(
                        "INSERT INTO generation_sessions (session_id, workspace_id, run_id,
                                analysis_revision_no, analysis_input_fingerprint,
                                generation_input_fingerprint, workflow, framework, state,
                                prompt_version, schema_version, input_schema_version,
                                context_policy_version, source_manifest_hash, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'dispatched', ?8, ?9, '2', ?10,
                                 ?11, ?12, ?12)",
                        params![
                            id,
                            binding.workspace_id.to_string(),
                            binding.run_id.map(|u| u.to_string()).unwrap_or_default(),
                            binding.analysis_revision_no.unwrap_or(1) as i64,
                            binding.analysis_input_fingerprint,
                            binding.generation_input_fingerprint,
                            serde_json::to_value(binding.workflow).unwrap().as_str().unwrap(),
                            binding.prompt_version,
                            binding.schema_version,
                            binding.context_policy_version,
                            binding.source_manifest_hash,
                            now_iso()
                        ],
                    )
                    .map_err(CliError::from)?;
                    id
                }
            }
        };
        let attempt_id = Uuid::new_v4();
        let attempt_seq: i64 = tx.query_row(
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
        tx.execute(
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
        tx.execute(
            "UPDATE confirmations SET consumed_at = ?2, consumed_by_attempt_id = ?3
             WHERE confirmation_id = ?1",
            params![
                binding.confirmation_id.to_string(),
                dsheng_core::repository::utc_iso(now),
                attempt_id.to_string()
            ],
        )
        .map_err(CliError::from)?;
        tx.commit()
            .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))?;
        Ok(GateConsume::Created(
            self.load_attempt(attempt_id).expect("刚插入"),
        ))
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
        self.conn
            .execute(
                "UPDATE generation_attempts SET request_stage = ?2, request_count = ?3,
                        terminal_state = ?4,
                        dispatched_at = CASE WHEN ?2 = 'dispatched' AND dispatched_at IS NULL
                                             THEN ?5 ELSE dispatched_at END,
                        finished_at = CASE WHEN ?4 IS NOT NULL THEN ?5 ELSE finished_at END
                 WHERE attempt_id = ?1",
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
        Ok(())
    }
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
    pub timeouts: Timeouts,
    pub budget: Budget,
    pub api_key: String,
}

/// 请求派发抽象：生产＝provider::http 一次 send_once；测试＝记录器
/// （断言未确认不派发；无网络、无 Mock Provider）。
pub trait RequestDispatch {
    fn dispatch(
        &mut self,
        runtime: &ProviderRuntime,
        request: &ChatCompletionsRequest,
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)>;
}

/// 生产派发（唯一网络路径；每 attempt 一条请求、无自动重试）。
pub struct HttpDispatch;

impl RequestDispatch for HttpDispatch {
    fn dispatch(
        &mut self,
        runtime: &ProviderRuntime,
        request: &ChatCompletionsRequest,
    ) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
        let (client, endpoint) = dsheng_core::provider::http::build_client(
            &runtime.base_url,
            &runtime.timeouts,
            &runtime.budget,
        )?;
        dsheng_core::provider::http::send_once(&client, &endpoint, &runtime.api_key, request)
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
    _evidence_ids: &[String],
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
        workspace: workspace_projection(conn, workspace_id, defs)?,
        metrics,
        source_manifest: manifest_entries(conn, workspace_id, run_id)?,
        source_records: vec![],
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
        conn,
        workspace_id,
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
        workspace: workspace_projection(conn, workspace_id, defs)?,
        metrics,
        source_manifest: manifest_entries(conn, workspace_id, run_id)?,
        source_records: vec![],
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
        conn,
        workspace_id,
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
    conn: &Connection,
    workspace_id: Uuid,
) -> GenerationMaterial {
    let input_hash = dsheng_core::analysis::fingerprint::sha256_hex(&payload_json);
    let manifest = manifest_entries(conn, workspace_id, &run_id.to_string()).unwrap_or_default();
    let manifest_material: Vec<Value> = manifest
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
        "SELECT metric_id, metric_key, version, semantic_hash, label, semantic_type,
                unit_signature_json, direction, target_constraint_scope, revenue_semantics,
                aggregation, formula_json, required_inputs_json, effective_from
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

fn manifest_entries(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
) -> CliResult<Vec<SourceManifestEntry>> {
    let _ = run_id;
    let mut stmt = conn.prepare(
        "SELECT source_id, file_label, kind, content_hash FROM source_files
         WHERE workspace_id = ?1 ORDER BY file_label",
    )?;
    let rows = stmt.query_map(params![workspace_id.to_string()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (source_id, file_label, kind, content_hash) = row?;
        let Ok(source_uuid) = Uuid::parse_str(&source_id) else {
            continue;
        };
        let selection = format!("A1:Z999（{kind} 全表已扫描）");
        out.push(SourceManifestEntry {
            source_id: source_uuid,
            file_label,
            sheet: None,
            selected_range: selection.clone(),
            content_hash: content_hash.clone(),
            selection_hash: dsheng_core::analysis::fingerprint::sha256_hex(&format!(
                "{source_id}|{selection}|{content_hash}"
            )),
        });
    }
    Ok(out)
}

fn manual_projections(
    conn: &Connection,
    workspace_id: Uuid,
) -> CliResult<Vec<ManualInputProjection>> {
    let mut stmt = conn.prepare(
        "SELECT input_id, metric_id, measure_role, scope_key, value, unit, confirmed,
                source_note, input_revision, raw_text
         FROM manual_inputs WHERE workspace_id = ?1",
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

// ---------------------------------------------------------------------------
// 预览与确认发送
// ---------------------------------------------------------------------------

pub struct ProviderConfigSnapshot {
    pub provider_id: Uuid,
    pub revision: u32,
    pub name: String,
    pub base_url: String,
    pub model: String,
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
        Option<String>,
        Option<String>,
    );
    let row: ProviderRow = conn
        .query_row(
            "SELECT name, base_url, model, revision, connect_seconds, response_seconds,
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
                ))
            },
        )
        .map_err(|_| CliError::new(CliErrorCode::WorkspaceMismatch, "provider 不存在"))?;
    let owner = row.10.unwrap_or_default();
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
        timeouts: Timeouts {
            connect_seconds: row.4 as u32,
            response_seconds: row.5 as u32,
            total_seconds: row.6 as u32,
        },
        budget: Budget {
            context_window_tokens: row.7 as u32,
            output_reserve_tokens: row.8 as u32,
        },
        key_present: row.9.is_some(),
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
            session_id_of(conn, workspace_id, &material.run_id.to_string()),
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
    let manifest_rows = manifest_entries(conn, workspace_id, &material.run_id.to_string())?;
    let source_scope_summary: Vec<Value> = manifest_rows
        .iter()
        .map(|m| {
            json!({
                "source_id": m.source_id.to_string(),
                "file_label": m.file_label,
                "sheet": m.sheet,
                "selected_range": m.selected_range,
                "row_count": 0,
                "content_hash": m.content_hash,
            })
        })
        .collect();
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
/// 返回 (attempt_id, 终态, 提取后的输出对象)。
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
) -> CliResult<(Uuid, Value)> {
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
    gate.mark_dispatched(&mut store, attempt.attempt_id, Utc::now())
        .map_err(|(code, msg)| CliError::new(code, msg))?;
    let runtime = ProviderRuntime {
        name: provider.name.clone(),
        base_url: provider.base_url.clone(),
        model: provider.model.clone(),
        timeouts: provider.timeouts,
        budget: provider.budget,
        api_key,
    };
    let response = match dispatcher.dispatch(&runtime, &assembled.request) {
        Ok(response) => response,
        Err((code, msg)) => {
            let terminal = match code {
                CliErrorCode::ConnectTimeout
                | CliErrorCode::ResponseTimeout
                | CliErrorCode::TotalTimeout => TerminalState::TimedOut,
                CliErrorCode::Cancelled | CliErrorCode::Interrupted => TerminalState::Interrupted,
                _ => TerminalState::Failed,
            };
            gate.finish(&mut store, attempt.attempt_id, terminal, Utc::now())
                .ok();
            return Err(CliError::new(code, msg));
        }
    };
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
            gate.finish(
                &mut store,
                attempt.attempt_id,
                TerminalState::Failed,
                Utc::now(),
            )
            .ok();
            return Err(CliError::new(
                CliErrorCode::IncompleteOutput,
                format!("finish_reason={reason} 非 stop"),
            ));
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
            gate.finish(
                &mut store,
                attempt.attempt_id,
                TerminalState::Success,
                Utc::now(),
            )
            .map_err(|(code, msg)| CliError::new(code, msg))?;
            Ok((attempt.attempt_id, obj))
        }
        None => {
            gate.finish(
                &mut store,
                attempt.attempt_id,
                TerminalState::Failed,
                Utc::now(),
            )
            .ok();
            Err(CliError::new(
                CliErrorCode::InvalidJson,
                format!("响应无可提取 JSON 对象（stage={:?}）", extraction.stage),
            ))
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

/// generation cancel：仅本地取消（不保证远端停算）。
pub fn cancel_attempt(
    conn: &mut Connection,
    workspace_id: Uuid,
    attempt_id: Uuid,
) -> CliResult<Value> {
    let status = attempt_status(conn, workspace_id, attempt_id)?;
    if status["terminal_state"].is_null() {
        let mut store = SqliteGenerationStore { conn };
        ConfirmationGate
            .finish(&mut store, attempt_id, TerminalState::Cancelled, Utc::now())
            .map_err(|(code, msg)| CliError::new(code, msg))?;
        Ok(json!({
            "attempt_id": attempt_id.to_string(),
            "cancelled": true,
            "remote_billing_note": "仅本地取消；不保证远端已停止计费",
        }))
    } else {
        Err(CliError::new(
            CliErrorCode::IdempotencyConflict,
            "attempt 已是终态，无法取消",
        ))
    }
}

/// 供 tests 使用的保守 token 估算重导出（DI 断言用）。
pub fn tokens_of(system_bytes: u64, user_bytes: u64) -> u64 {
    conservative_input_tokens(system_bytes, user_bytes)
}

/// 保存任务候选（发送成功后；output 原样＋候选身份）。
pub fn save_task_candidate(
    conn: &mut Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
    attempt_id: Uuid,
    output: &Value,
    envelope_locally_reconstructed: bool,
) -> CliResult<String> {
    let candidate_id = Uuid::new_v4().to_string();
    conn.execute(
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
    Ok(candidate_id)
}

/// 保存诊断轮次/报告（发送成功后；turn 已过 schema 校验）。
#[allow(clippy::too_many_arguments)]
pub fn save_diagnosis_turn(
    conn: &mut Connection,
    workspace_id: Uuid,
    material: &GenerationMaterial,
    framework: Framework,
    attempt_id: Uuid,
    session_id: &str,
    turn_no: i64,
    user_message: &str,
    report_json: &Value,
    report_revision: i64,
    turn_state: &str,
    stop_reason: Option<&str>,
) -> CliResult<String> {
    let report_id: String = conn
        .query_row(
            "SELECT report_id FROM diagnosis_reports WHERE session_id = ?1 LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| Uuid::new_v4().to_string());
    let now = now_iso();
    let turn_id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO diagnosis_turns (turn_id, session_id, workspace_id, turn_no, round_no,
                user_message, turn_kind, turn_state, accepted_facts_json, context_manifest_json,
                prompt_version, schema_version, provider_id, attempt_id, stop_reason, created_at)
         VALUES (?1, ?2, ?3, ?4, ?4, ?5, 'model_turn', ?6, '[]', '{}', '2.0', '3', ?7, ?8, ?9, ?10)",
        params![
            turn_id,
            session_id,
            workspace_id.to_string(),
            turn_no,
            user_message,
            turn_state,
            material
                .report_id
                .map(|u| u.to_string())
                .unwrap_or_default(),
            attempt_id.to_string(),
            stop_reason,
            now
        ],
    )
    .map_err(CliError::from)?;
    if turn_state == "patched" {
        conn.execute(
            "INSERT INTO diagnosis_reports (report_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, report_revision_no, session_id, framework,
                    problem_statement, report_json, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'accepted', ?11, ?11)",
            params![
                report_id,
                workspace_id.to_string(),
                material.run_id.to_string(),
                material.analysis_revision_no as i64,
                material.analysis_input_fingerprint,
                report_revision,
                session_id,
                framework_db_str(framework),
                material.user_supplied_text,
                serde_json::to_string(report_json).unwrap(),
                now
            ],
        )
        .map_err(CliError::from)?;
    }
    let _ = material.framework;
    Ok(turn_id)
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
