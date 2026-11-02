//! S4 确认门禁集成测试（纯逻辑/依赖注入层，非 Provider 调用、无网络）。
//!
//! 场景对齐 provider-config-delivery.md §2 与验收 PC-02/PC-08：
//! 预览后 Provider/预算/分析/报告变化使确认失效；重复确认最多一个 attempt；
//! 最多一条请求；手动重试需全新 confirmation。

use chrono::{Duration, Utc};
use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::provider::{
    ConfirmationBinding, GenerationOperation, RequestStage, TerminalState, Workflow,
    CONFIRMATION_TTL_SECONDS,
};
use dsheng_core::provider::confirm::{
    AttemptRecord, CancelOutcome, ConfirmationGate, ConsumeOutcome, GenerationStore, SendSnapshot,
};
use uuid::Uuid;

/// 内存 GenerationStore（测试层次=纯逻辑依赖注入，非 SQLite、非 Provider 调用）。
#[derive(Default)]
struct InMemoryStore {
    confirmations: std::collections::HashMap<Uuid, ConfirmationBinding>,
    attempts: std::collections::HashMap<Uuid, AttemptRecord>,
    by_confirmation: std::collections::HashMap<Uuid, Uuid>,
}

impl GenerationStore for InMemoryStore {
    type Error = String;

    fn insert_confirmation(&mut self, binding: &ConfirmationBinding) -> Result<(), String> {
        if self.confirmations.contains_key(&binding.confirmation_id) {
            return Err("duplicate".into());
        }
        self.confirmations
            .insert(binding.confirmation_id, binding.clone());
        Ok(())
    }

    fn load_confirmation(&self, id: Uuid) -> Option<ConfirmationBinding> {
        self.confirmations.get(&id).cloned()
    }

    fn consume_confirmation(
        &mut self,
        id: Uuid,
        now: chrono::DateTime<Utc>,
    ) -> Result<ConsumeOutcome, String> {
        if let Some(attempt_id) = self.by_confirmation.get(&id) {
            return Ok(ConsumeOutcome::Existing(
                self.attempts
                    .get(attempt_id)
                    .cloned()
                    .expect("attempt 存在"),
            ));
        }
        let binding = self.confirmations.get(&id).cloned().ok_or("missing")?;
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
        self.by_confirmation.insert(id, attempt.attempt_id);
        self.attempts.insert(attempt.attempt_id, attempt.clone());
        Ok(ConsumeOutcome::Created(attempt))
    }

    fn load_attempt(&self, attempt_id: Uuid) -> Option<AttemptRecord> {
        self.attempts.get(&attempt_id).cloned()
    }

    fn load_attempt_by_confirmation(&self, confirmation_id: Uuid) -> Option<AttemptRecord> {
        self.by_confirmation
            .get(&confirmation_id)
            .and_then(|id| self.attempts.get(id))
            .cloned()
    }

    fn update_attempt(&mut self, attempt: &AttemptRecord) -> Result<(), String> {
        self.attempts.insert(attempt.attempt_id, attempt.clone());
        Ok(())
    }
}

fn binding(id: Uuid, created_at: chrono::DateTime<Utc>) -> ConfirmationBinding {
    ConfirmationBinding {
        confirmation_id: id,
        workflow: Workflow::Diagnosis,
        operation: GenerationOperation::DiagnosisGenerate,
        workspace_id: Uuid::new_v4(),
        provider_id: Uuid::new_v4(),
        provider_revision: 1,
        run_id: Some(Uuid::new_v4()),
        analysis_revision_no: Some(1),
        analysis_input_fingerprint: "a".repeat(64),
        generation_input_fingerprint: "b".repeat(64),
        report_id: None,
        report_revision_no: None,
        prompt_version: "2.0".into(),
        schema_version: "3".into(),
        context_policy_version: Some("2".into()),
        source_manifest_hash: "c".repeat(64),
        input_hash: "d".repeat(64),
        request_bytes: 20_000,
        conservative_input_tokens: 20_256,
        budget: dsheng_core::contracts::settings::DEFAULT_BUDGET,
        timeouts: dsheng_core::contracts::settings::DEFAULT_TIMEOUTS,
        created_at,
        expires_at: created_at + Duration::seconds(CONFIRMATION_TTL_SECONDS),
    }
}

#[test]
fn full_flow_preview_confirm_dispatch_finish() {
    let gate = ConfirmationGate;
    let mut store = InMemoryStore::default();
    let now = Utc::now();

    // dry-run 预览 → 创建确认
    let b = binding(Uuid::new_v4(), now);
    gate.create(&mut store, b.clone()).unwrap();

    // 发送前重验通过 → 消费并创建 attempt（原子）
    gate.verify_before_send(&b, &SendSnapshot::from_binding(&b), now)
        .unwrap();
    let attempt = gate
        .consume_and_create_attempt(&mut store, b.confirmation_id, now)
        .unwrap();
    let attempt_id = attempt.attempt().attempt_id;

    // 最多一条 HTTP 请求
    gate.mark_dispatched(&mut store, attempt_id, now).unwrap();
    let second = gate
        .mark_dispatched(&mut store, attempt_id, now)
        .unwrap_err();
    assert_eq!(second.0, CliErrorCode::IdempotencyConflict);

    // 终态
    gate.finish(&mut store, attempt_id, TerminalState::Success, now)
        .unwrap();
    let done = store.load_attempt(attempt_id).unwrap();
    assert_eq!(done.terminal, Some(TerminalState::Success));
    assert_eq!(done.request_count, 1);
    assert_eq!(done.stage, RequestStage::Terminal);

    // 成功 attempt 不可重试；新确认也不行
    let new_confirmation = binding(Uuid::new_v4(), now);
    gate.create(&mut store, new_confirmation.clone()).unwrap();
    assert!(
        ConfirmationGate::verify_retry_confirmation(&done, new_confirmation.confirmation_id)
            .is_err()
    );
}

#[test]
fn duplicate_confirm_returns_existing_attempt_without_second_dispatch() {
    let gate = ConfirmationGate;
    let mut store = InMemoryStore::default();
    let now = Utc::now();
    let b = binding(Uuid::new_v4(), now);
    gate.create(&mut store, b.clone()).unwrap();
    let first = gate
        .consume_and_create_attempt(&mut store, b.confirmation_id, now)
        .unwrap();
    gate.mark_dispatched(&mut store, first.attempt().attempt_id, now)
        .unwrap();
    // 第二次携带同一 --confirmation-id --confirm-send：返回已有 attempt，
    // 不派发第二条
    let second = gate
        .consume_and_create_attempt(&mut store, b.confirmation_id, now)
        .unwrap();
    assert_eq!(first.attempt().attempt_id, second.attempt().attempt_id);
    let attempt = store.load_attempt(first.attempt().attempt_id).unwrap();
    assert_eq!(attempt.request_count, 1);
}

#[test]
fn provider_or_input_change_stales_confirmation() {
    let gate = ConfirmationGate;
    let now = Utc::now();
    let b = binding(Uuid::new_v4(), now);
    // Provider revision 变化（用户在预览后改了配置）
    let mut s1 = SendSnapshot::from_binding(&b);
    s1.provider_revision += 1;
    assert_eq!(
        gate.verify_before_send(&b, &s1, now).unwrap_err().0,
        CliErrorCode::ConfirmationStale
    );
    // 分析 revision 变化
    let mut s2 = SendSnapshot::from_binding(&b);
    s2.analysis_revision_no = Some(7);
    assert_eq!(
        gate.verify_before_send(&b, &s2, now).unwrap_err().0,
        CliErrorCode::ConfirmationStale
    );
    // 报告 revision 变化（另一窗口续问了同一报告）
    let mut s3 = SendSnapshot::from_binding(&b);
    s3.report_revision_no = Some(2);
    assert_eq!(
        gate.verify_before_send(&b, &s3, now).unwrap_err().0,
        CliErrorCode::ConfirmationStale
    );
    // 输入 hash 变化
    let mut s4 = SendSnapshot::from_binding(&b);
    s4.input_hash = "e".repeat(64);
    assert_eq!(
        gate.verify_before_send(&b, &s4, now).unwrap_err().0,
        CliErrorCode::ConfirmationStale
    );
}

#[test]
fn cancel_semantics_before_and_after_dispatch() {
    let gate = ConfirmationGate;
    let mut store = InMemoryStore::default();
    let now = Utc::now();
    let b = binding(Uuid::new_v4(), now);
    gate.create(&mut store, b.clone()).unwrap();
    // 发送前取消：本地动作，exit 2
    assert_eq!(
        gate.cancel(&mut store, b.confirmation_id).unwrap(),
        CancelOutcome::CancelledBeforeDispatch
    );
    // 消费后取消：exit 4，不保证远端停算
    gate.consume_and_create_attempt(&mut store, b.confirmation_id, now)
        .unwrap();
    assert_eq!(
        gate.cancel(&mut store, b.confirmation_id).unwrap(),
        CancelOutcome::CancelledAfterDispatch
    );
}

#[test]
fn missing_confirmation_requires_preview() {
    let gate = ConfirmationGate;
    let mut store = InMemoryStore::default();
    let err = gate
        .consume_and_create_attempt(&mut store, Uuid::new_v4(), Utc::now())
        .unwrap_err();
    assert_eq!(err.0, CliErrorCode::ConfirmationRequired);
}
