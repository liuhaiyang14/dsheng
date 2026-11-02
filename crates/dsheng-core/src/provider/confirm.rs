//! 确认门禁与 attempt 生命周期（provider-config-delivery.md §2、主计划 §8.1）。
//!
//! - `--dry-run` 预览创建本地 confirmation，TTL 600 秒；绑定清单为
//!   [`ConfirmationBinding`] 全字段（Provider revision、分析/报告 revision、
//!   指纹、prompt/schema/context 版本、scope/source manifest hash、本轮输入
//!   hash、payload 大小与预算快照；Key 只通过 provider_revision 间接绑定）。
//! - 发送前按当前快照**逐项重验**；任一变化/过期/跨企业 → `CONFIRMATION_STALE`，
//!   不静默取消引用或切 Provider 后发送。
//! - 消费 confirmation 与创建 attempt **原子完成**；重复消费返回已建立的
//!   attempt（不派发第二条请求）。
//! - 每动作最多 1 条 HTTP 请求、无自动重试；仅 terminal 且非成功的 attempt
//!   可重试，且需全新 confirmation（新 confirmation_id）。
//!
//! 持久化通过 [`GenerationStore`] trait 抽象（生产实现由数据工作包提供
//! SQLite 事务；测试用内存实现）。本模块为纯逻辑/依赖注入层，
//! 不做 Provider 调用、无网络、不保存 Key/prompt 正文。

use crate::contracts::cli::CliErrorCode;
use crate::contracts::provider::{
    ConfirmationBinding, GenerationOperation, RequestStage, TerminalState, Workflow,
    CONFIRMATION_TTL_SECONDS, REQUEST_COUNT_MAXIMUM,
};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

/// attempt 记录（不保留 Key/完整 prompt/payload）。
#[derive(Debug, Clone, PartialEq)]
pub struct AttemptRecord {
    pub attempt_id: Uuid,
    pub confirmation_id: Uuid,
    pub workflow: Workflow,
    pub operation: GenerationOperation,
    pub workspace_id: Uuid,
    pub provider_id: Uuid,
    /// Provider 配置 revision 快照（发送后不跟随配置变化重路由）。
    pub provider_revision: u32,
    pub stage: RequestStage,
    pub terminal: Option<TerminalState>,
    /// 已派发 HTTP 请求数（上限 1）。
    pub request_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 发送前重验快照：与绑定清单逐字段对比的当前状态。
#[derive(Debug, Clone, PartialEq)]
pub struct SendSnapshot {
    pub workspace_id: Uuid,
    pub provider_id: Uuid,
    pub provider_revision: u32,
    pub run_id: Option<Uuid>,
    pub analysis_revision_no: Option<u32>,
    pub analysis_input_fingerprint: String,
    pub generation_input_fingerprint: String,
    pub report_id: Option<Uuid>,
    pub report_revision_no: Option<u32>,
    pub prompt_version: String,
    pub schema_version: String,
    pub context_policy_version: Option<String>,
    pub source_manifest_hash: String,
    pub input_hash: String,
    pub request_bytes: u64,
    pub conservative_input_tokens: u64,
    pub budget: crate::contracts::settings::Budget,
    pub timeouts: crate::contracts::settings::Timeouts,
}

impl SendSnapshot {
    /// 由确认绑定直接构造快照（发送端重建 payload 后逐项重算的对照目标）。
    pub fn from_binding(b: &ConfirmationBinding) -> Self {
        Self {
            workspace_id: b.workspace_id,
            provider_id: b.provider_id,
            provider_revision: b.provider_revision,
            run_id: b.run_id,
            analysis_revision_no: b.analysis_revision_no,
            analysis_input_fingerprint: b.analysis_input_fingerprint.clone(),
            generation_input_fingerprint: b.generation_input_fingerprint.clone(),
            report_id: b.report_id,
            report_revision_no: b.report_revision_no,
            prompt_version: b.prompt_version.clone(),
            schema_version: b.schema_version.clone(),
            context_policy_version: b.context_policy_version.clone(),
            source_manifest_hash: b.source_manifest_hash.clone(),
            input_hash: b.input_hash.clone(),
            request_bytes: b.request_bytes,
            conservative_input_tokens: b.conservative_input_tokens,
            budget: b.budget,
            timeouts: b.timeouts,
        }
    }
}

/// 持久化抽象：confirmation/attempt 的最小事务面。
/// 生产实现（SQLite，数据工作包所有权）必须保证 `consume_confirmation`
/// 的原子性；本 trait 的内存实现只用于纯逻辑测试。
pub trait GenerationStore {
    type Error: std::fmt::Debug;

    /// 插入新 confirmation；confirmation_id 重复返回错误。
    fn insert_confirmation(&mut self, binding: &ConfirmationBinding) -> Result<(), Self::Error>;

    fn load_confirmation(&self, id: Uuid) -> Option<ConfirmationBinding>;

    /// 原子消费：未消费则标记消费并创建 attempt；已消费则返回已有 attempt。
    fn consume_confirmation(
        &mut self,
        id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<ConsumeOutcome, Self::Error>;

    fn load_attempt(&self, attempt_id: Uuid) -> Option<AttemptRecord>;

    /// 按 confirmation 查 attempt（重复消费路径）。
    fn load_attempt_by_confirmation(&self, confirmation_id: Uuid) -> Option<AttemptRecord>;

    /// 记录 attempt 状态推进（阶段/终态/请求计数）。
    fn update_attempt(&mut self, attempt: &AttemptRecord) -> Result<(), Self::Error>;
}

/// 消费结果：新建或复用已有 attempt（不派发第二条请求）。
#[derive(Debug, Clone, PartialEq)]
pub enum ConsumeOutcome {
    Created(AttemptRecord),
    Existing(AttemptRecord),
}

impl ConsumeOutcome {
    pub fn attempt(&self) -> &AttemptRecord {
        match self {
            ConsumeOutcome::Created(a) | ConsumeOutcome::Existing(a) => a,
        }
    }
}

/// 取消结果：发送前取消（exit 2）与派发后取消（exit 4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelOutcome {
    /// confirmation 未消费即取消 → CONFIRMATION_CANCELLED（2）。
    CancelledBeforeDispatch,
    /// attempt 已建立（可能已派发）→ CANCELLED（4），不保证远端停算。
    CancelledAfterDispatch,
}

/// 确认门禁服务（纯逻辑，依赖注入 store 与时钟参数）。
#[derive(Debug, Clone, Copy, Default)]
pub struct ConfirmationGate;

pub type GateError = (CliErrorCode, String);

impl ConfirmationGate {
    /// 创建 confirmation（TTL 600 秒；不持久化完整 prompt/payload/Key）。
    pub fn create<S: GenerationStore>(
        &self,
        store: &mut S,
        binding: ConfirmationBinding,
    ) -> Result<ConfirmationBinding, GateError> {
        if binding.expires_at - binding.created_at != Duration::seconds(CONFIRMATION_TTL_SECONDS) {
            return Err((
                CliErrorCode::InvalidArgument,
                format!("确认 TTL 必须为 {CONFIRMATION_TTL_SECONDS} 秒"),
            ));
        }
        store.insert_confirmation(&binding).map_err(|e| {
            (
                CliErrorCode::LocalSaveFailed,
                format!("确认保存失败: {e:?}"),
            )
        })?;
        Ok(binding)
    }

    /// 发送前逐项重验：任一字段变化/过期 → CONFIRMATION_STALE。
    pub fn verify_before_send(
        &self,
        binding: &ConfirmationBinding,
        snapshot: &SendSnapshot,
        now: DateTime<Utc>,
    ) -> Result<(), GateError> {
        if binding.expired(now) {
            return Err((
                CliErrorCode::ConfirmationStale,
                "确认已过期（TTL 600s），须重新预览".into(),
            ));
        }
        let b = binding;
        let s = snapshot;
        let checks: [(&str, bool); 17] = [
            ("workspace_id", b.workspace_id == s.workspace_id),
            ("provider_id", b.provider_id == s.provider_id),
            (
                "provider_revision",
                b.provider_revision == s.provider_revision,
            ),
            ("run_id", b.run_id == s.run_id),
            (
                "analysis_revision_no",
                b.analysis_revision_no == s.analysis_revision_no,
            ),
            (
                "analysis_input_fingerprint",
                b.analysis_input_fingerprint == s.analysis_input_fingerprint,
            ),
            (
                "generation_input_fingerprint",
                b.generation_input_fingerprint == s.generation_input_fingerprint,
            ),
            ("report_id", b.report_id == s.report_id),
            (
                "report_revision_no",
                b.report_revision_no == s.report_revision_no,
            ),
            ("prompt_version", b.prompt_version == s.prompt_version),
            ("schema_version", b.schema_version == s.schema_version),
            (
                "context_policy_version",
                b.context_policy_version == s.context_policy_version,
            ),
            (
                "source_manifest_hash",
                b.source_manifest_hash == s.source_manifest_hash,
            ),
            ("input_hash", b.input_hash == s.input_hash),
            ("request_bytes", b.request_bytes == s.request_bytes),
            (
                "conservative_input_tokens",
                b.conservative_input_tokens == s.conservative_input_tokens,
            ),
            (
                "budget_and_timeouts",
                b.budget == s.budget && b.timeouts == s.timeouts,
            ),
        ];
        // 跨企业/工作区变化同样 STALE（不静默切换后发送）
        if b.workspace_id != s.workspace_id {
            return Err((
                CliErrorCode::ConfirmationStale,
                "workspace 变化，确认失效（跨企业拒绝）".into(),
            ));
        }
        for (field, ok) in checks {
            if !ok {
                return Err((
                    CliErrorCode::ConfirmationStale,
                    format!("绑定项 {field} 已变化，须重新预览确认"),
                ));
            }
        }
        Ok(())
    }

    /// 消费 confirmation 并原子创建 attempt；重复消费返回已有 attempt。
    pub fn consume_and_create_attempt<S: GenerationStore>(
        &self,
        store: &mut S,
        confirmation_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<ConsumeOutcome, GateError> {
        let binding = store.load_confirmation(confirmation_id).ok_or_else(|| {
            (
                CliErrorCode::ConfirmationRequired,
                "确认不存在，须先 --dry-run 预览".to_string(),
            )
        })?;
        if binding.expired(now) {
            return Err((
                CliErrorCode::ConfirmationStale,
                "确认已过期，消费被拒绝（已发送请求的预算不受影响）".into(),
            ));
        }
        store
            .consume_confirmation(confirmation_id, now)
            .map_err(|e| {
                (
                    CliErrorCode::LocalSaveFailed,
                    format!("消费事务失败: {e:?}"),
                )
            })
    }

    /// 标记 attempt 已派发一条 HTTP 请求；重复派发拒绝（最多一条）。
    pub fn mark_dispatched<S: GenerationStore>(
        &self,
        store: &mut S,
        attempt_id: Uuid,
        now: DateTime<Utc>,
    ) -> Result<(), GateError> {
        let mut attempt = store.load_attempt(attempt_id).ok_or_else(|| {
            (
                CliErrorCode::IdempotencyConflict,
                "attempt 不存在".to_string(),
            )
        })?;
        if attempt.terminal.is_some() {
            return Err((
                CliErrorCode::IdempotencyConflict,
                "attempt 已终态，不得再次派发；重试需新确认新 attempt".into(),
            ));
        }
        if attempt.request_count >= REQUEST_COUNT_MAXIMUM {
            return Err((
                CliErrorCode::IdempotencyConflict,
                format!("每个动作最多 {REQUEST_COUNT_MAXIMUM} 条 HTTP 请求"),
            ));
        }
        attempt.request_count += 1;
        attempt.stage = RequestStage::Dispatched;
        attempt.updated_at = now;
        store.update_attempt(&attempt).map_err(|e| {
            (
                CliErrorCode::LocalSaveFailed,
                format!("attempt 更新失败: {e:?}"),
            )
        })?;
        Ok(())
    }

    /// 推进 attempt 到终态（不可回退）。
    pub fn finish<S: GenerationStore>(
        &self,
        store: &mut S,
        attempt_id: Uuid,
        terminal: TerminalState,
        now: DateTime<Utc>,
    ) -> Result<(), GateError> {
        let mut attempt = store.load_attempt(attempt_id).ok_or_else(|| {
            (
                CliErrorCode::IdempotencyConflict,
                "attempt 不存在".to_string(),
            )
        })?;
        if attempt.terminal.is_some() {
            return Err((
                CliErrorCode::IdempotencyConflict,
                "attempt 已是终态，终态不可回退".into(),
            ));
        }
        attempt.terminal = Some(terminal);
        attempt.stage = RequestStage::Terminal;
        attempt.updated_at = now;
        store.update_attempt(&attempt).map_err(|e| {
            (
                CliErrorCode::LocalSaveFailed,
                format!("attempt 更新失败: {e:?}"),
            )
        })?;
        Ok(())
    }

    /// 取消：未消费的 confirmation 取消 → exit 2；已建 attempt → exit 4。
    pub fn cancel<S: GenerationStore>(
        &self,
        store: &mut S,
        confirmation_id: Uuid,
    ) -> Result<CancelOutcome, GateError> {
        if store.load_confirmation(confirmation_id).is_none() {
            return Err((CliErrorCode::ConfirmationRequired, "确认不存在".to_string()));
        }
        if let Some(attempt) = store.load_attempt_by_confirmation(confirmation_id) {
            let _ = attempt;
            Ok(CancelOutcome::CancelledAfterDispatch)
        } else {
            Ok(CancelOutcome::CancelledBeforeDispatch)
        }
    }

    /// 重试规则：仅 terminal 且非成功的 attempt 可重试；成功 attempt 不可。
    pub fn can_retry(attempt: &AttemptRecord) -> bool {
        match attempt.terminal {
            None | Some(TerminalState::Success) => false,
            Some(
                TerminalState::Failed
                | TerminalState::Cancelled
                | TerminalState::Interrupted
                | TerminalState::TimedOut,
            ) => true,
        }
    }

    /// 重试必须使用全新 confirmation：旧 confirmation 已消费，不得复用。
    pub fn verify_retry_confirmation(
        old_attempt: &AttemptRecord,
        new_confirmation_id: Uuid,
    ) -> Result<(), GateError> {
        if !Self::can_retry(old_attempt) {
            return Err((
                CliErrorCode::IdempotencyConflict,
                "仅未成功的终态 attempt 可重试".into(),
            ));
        }
        if new_confirmation_id == old_attempt.confirmation_id {
            return Err((
                CliErrorCode::ConfirmationStale,
                "重试必须使用全新确认（新 confirmation_id）".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑/依赖注入层测试：内存 store + 显式时钟参数（非 Provider 调用）。
    use super::*;
    use crate::contracts::settings::{Budget, Timeouts};

    fn binding(confirmation_id: Uuid, created_at: DateTime<Utc>) -> ConfirmationBinding {
        ConfirmationBinding {
            confirmation_id,
            workflow: Workflow::Task,
            operation: GenerationOperation::TaskGenerate,
            workspace_id: Uuid::new_v4(),
            provider_id: Uuid::new_v4(),
            provider_revision: 3,
            run_id: Some(Uuid::new_v4()),
            analysis_revision_no: Some(2),
            analysis_input_fingerprint: "f".repeat(64),
            generation_input_fingerprint: "g".repeat(64),
            report_id: None,
            report_revision_no: None,
            prompt_version: "2.3".into(),
            schema_version: "2".into(),
            context_policy_version: None,
            source_manifest_hash: "m".repeat(64),
            input_hash: "i".repeat(64),
            request_bytes: 12_345,
            conservative_input_tokens: 12_601,
            budget: Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
            timeouts: Timeouts {
                connect_seconds: 15,
                response_seconds: 180,
                total_seconds: 240,
            },
            created_at,
            expires_at: created_at + Duration::seconds(CONFIRMATION_TTL_SECONDS),
        }
    }

    #[test]
    fn ttl_is_600s_and_expiry_rejected() {
        let gate = ConfirmationGate;
        let mut store = MemStore::default();
        let now = Utc::now();
        let b = binding(Uuid::new_v4(), now);
        gate.create(&mut store, b.clone()).unwrap();
        assert!(gate
            .verify_before_send(&b, &SendSnapshot::from_binding(&b), now)
            .is_ok());
        // 601 秒后过期
        let later = now + Duration::seconds(CONFIRMATION_TTL_SECONDS + 1);
        let err = gate
            .verify_before_send(&b, &SendSnapshot::from_binding(&b), later)
            .unwrap_err();
        assert_eq!(err.0, CliErrorCode::ConfirmationStale);
        // 过期确认消费也被拒绝
        let err2 = gate
            .consume_and_create_attempt(&mut store, b.confirmation_id, later)
            .unwrap_err();
        assert_eq!(err2.0, CliErrorCode::ConfirmationStale);
    }

    #[test]
    fn any_snapshot_change_makes_confirmation_stale() {
        let gate = ConfirmationGate;
        let now = Utc::now();
        let b = binding(Uuid::new_v4(), now);
        // 逐项篡改快照
        type Mutation = Box<dyn Fn(&mut SendSnapshot)>;
        let mut cases: Vec<(Mutation, &str)> = Vec::new();
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.provider_revision += 1),
            "provider_revision",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.analysis_revision_no = Some(9)),
            "analysis_revision_no",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.input_hash = "x".repeat(64)),
            "input_hash",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.source_manifest_hash = "y".repeat(64)),
            "source_manifest_hash",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.report_id = Some(Uuid::new_v4())),
            "report_id",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.report_revision_no = Some(4)),
            "report_revision_no",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.request_bytes += 1),
            "request_bytes",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.prompt_version = "2.2".into()),
            "prompt_version",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.schema_version = "3".into()),
            "schema_version",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.conservative_input_tokens += 1),
            "tokens",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.workspace_id = Uuid::new_v4()),
            "workspace",
        ));
        cases.push((
            Box::new(|s: &mut SendSnapshot| s.timeouts.total_seconds = 300),
            "timeouts",
        ));
        for (mutate, field) in cases {
            let mut snapshot = SendSnapshot::from_binding(&b);
            mutate(&mut snapshot);
            let err = gate.verify_before_send(&b, &snapshot, now).unwrap_err();
            assert_eq!(
                err.0,
                CliErrorCode::ConfirmationStale,
                "字段 {field} 变化必须 STALE"
            );
        }
    }

    #[test]
    fn consume_is_atomic_and_idempotent() {
        let gate = ConfirmationGate;
        let mut store = MemStore::default();
        let now = Utc::now();
        let b = binding(Uuid::new_v4(), now);
        gate.create(&mut store, b.clone()).unwrap();
        let out1 = gate
            .consume_and_create_attempt(&mut store, b.confirmation_id, now)
            .unwrap();
        assert!(matches!(out1, ConsumeOutcome::Created(_)));
        let out2 = gate
            .consume_and_create_attempt(&mut store, b.confirmation_id, now)
            .unwrap();
        assert!(matches!(out2, ConsumeOutcome::Existing(_)));
        assert_eq!(out1.attempt().attempt_id, out2.attempt().attempt_id);
        // 最多一条请求
        gate.mark_dispatched(&mut store, out1.attempt().attempt_id, now)
            .unwrap();
        let err = gate
            .mark_dispatched(&mut store, out1.attempt().attempt_id, now)
            .unwrap_err();
        assert_eq!(err.0, CliErrorCode::IdempotencyConflict);
    }

    #[test]
    fn retry_requires_new_confirmation_and_unsuccessful_terminal() {
        let gate = ConfirmationGate;
        let mut store = MemStore::default();
        let now = Utc::now();
        let b = binding(Uuid::new_v4(), now);
        gate.create(&mut store, b.clone()).unwrap();
        let attempt = gate
            .consume_and_create_attempt(&mut store, b.confirmation_id, now)
            .unwrap()
            .attempt()
            .clone();
        gate.mark_dispatched(&mut store, attempt.attempt_id, now)
            .unwrap();
        // 非终态不可重试
        assert!(!ConfirmationGate::can_retry(&attempt));
        gate.finish(&mut store, attempt.attempt_id, TerminalState::TimedOut, now)
            .unwrap();
        let timed_out = store.load_attempt(attempt.attempt_id).unwrap();
        assert!(ConfirmationGate::can_retry(&timed_out));
        // 复用旧确认拒绝
        let err =
            ConfirmationGate::verify_retry_confirmation(&timed_out, b.confirmation_id).unwrap_err();
        assert_eq!(err.0, CliErrorCode::ConfirmationStale);
        assert!(ConfirmationGate::verify_retry_confirmation(&timed_out, Uuid::new_v4()).is_ok());
        // 成功 attempt 不可重试
        let b2 = binding(Uuid::new_v4(), now);
        gate.create(&mut store, b2.clone()).unwrap();
        let ok_attempt = gate
            .consume_and_create_attempt(&mut store, b2.confirmation_id, now)
            .unwrap()
            .attempt()
            .clone();
        gate.finish(
            &mut store,
            ok_attempt.attempt_id,
            TerminalState::Success,
            now,
        )
        .unwrap();
        let success = store.load_attempt(ok_attempt.attempt_id).unwrap();
        assert!(!ConfirmationGate::can_retry(&success));
        // 终态不可回退
        let err2 = gate
            .finish(
                &mut store,
                ok_attempt.attempt_id,
                TerminalState::Failed,
                now,
            )
            .unwrap_err();
        assert_eq!(err2.0, CliErrorCode::IdempotencyConflict);
    }

    #[test]
    fn cancel_before_and_after_dispatch() {
        let gate = ConfirmationGate;
        let mut store = MemStore::default();
        let now = Utc::now();
        let b = binding(Uuid::new_v4(), now);
        gate.create(&mut store, b.clone()).unwrap();
        assert_eq!(
            gate.cancel(&mut store, b.confirmation_id).unwrap(),
            CancelOutcome::CancelledBeforeDispatch
        );
        let attempt = gate
            .consume_and_create_attempt(&mut store, b.confirmation_id, now)
            .unwrap()
            .attempt()
            .clone();
        assert_eq!(
            gate.cancel(&mut store, b.confirmation_id).unwrap(),
            CancelOutcome::CancelledAfterDispatch
        );
        let _ = attempt;
    }

    /// 内存 GenerationStore（测试层次=纯逻辑依赖注入，非 SQLite、非 Provider 调用）。
    #[derive(Default)]
    struct MemStore {
        confirmations: Vec<ConfirmationBinding>,
        consumed: Vec<Uuid>,
        attempts: Vec<AttemptRecord>,
    }

    impl GenerationStore for MemStore {
        type Error = String;

        fn insert_confirmation(&mut self, binding: &ConfirmationBinding) -> Result<(), String> {
            if self
                .confirmations
                .iter()
                .any(|c| c.confirmation_id == binding.confirmation_id)
            {
                return Err("duplicate confirmation".into());
            }
            self.confirmations.push(binding.clone());
            Ok(())
        }

        fn load_confirmation(&self, id: Uuid) -> Option<ConfirmationBinding> {
            self.confirmations
                .iter()
                .find(|c| c.confirmation_id == id)
                .cloned()
        }

        fn consume_confirmation(
            &mut self,
            id: Uuid,
            now: DateTime<Utc>,
        ) -> Result<ConsumeOutcome, String> {
            if let Some(existing) = self
                .attempts
                .iter()
                .find(|a| a.confirmation_id == id)
                .cloned()
            {
                return Ok(ConsumeOutcome::Existing(existing));
            }
            let binding = self
                .confirmations
                .iter()
                .find(|c| c.confirmation_id == id)
                .cloned()
                .ok_or("missing")?;
            if self.consumed.contains(&id) {
                return Err("race: consumed without attempt".into());
            }
            self.consumed.push(id);
            let attempt = AttemptRecord {
                attempt_id: Uuid::new_v4(),
                confirmation_id: id,
                workflow: binding.workflow,
                operation: binding.operation,
                workspace_id: binding.workspace_id,
                provider_id: binding.provider_id,
                provider_revision: binding.provider_revision,
                stage: RequestStage::Preparing,
                terminal: None,
                request_count: 0,
                created_at: now,
                updated_at: now,
            };
            self.attempts.push(attempt.clone());
            Ok(ConsumeOutcome::Created(attempt))
        }

        fn load_attempt(&self, attempt_id: Uuid) -> Option<AttemptRecord> {
            self.attempts
                .iter()
                .find(|a| a.attempt_id == attempt_id)
                .cloned()
        }

        fn load_attempt_by_confirmation(&self, confirmation_id: Uuid) -> Option<AttemptRecord> {
            self.attempts
                .iter()
                .find(|a| a.confirmation_id == confirmation_id)
                .cloned()
        }

        fn update_attempt(&mut self, attempt: &AttemptRecord) -> Result<(), String> {
            let slot = self
                .attempts
                .iter_mut()
                .find(|a| a.attempt_id == attempt.attempt_id)
                .ok_or("missing")?;
            *slot = attempt.clone();
            Ok(())
        }
    }
}
