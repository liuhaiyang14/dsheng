import { useEffect, useRef } from 'react';
import type { ConfirmationPreview } from '../api';
import { groupDecimal } from '../api';
import { PreviewModal } from './PreviewModal';
import type { GenerationController } from './useGeneration';

const stageLabels: Record<string, string> = {
  starting: '正在准备请求', preparing: '正在准备请求', started: '请求已创建',
  connecting: '正在连接模型服务', waiting: '等待模型响应', dispatched: '等待模型响应',
  receiving: '正在接收模型响应', reasoning: '模型正在思考', content: '正在接收正文',
  validating: '正在校验生成结果', saving: '正在保存结果', completed: '生成已完成',
  succeeded: '生成已完成', failed: '生成失败', cancelled: '已停止', stopping: '正在停止',
};

export function GenerationDialog({
  preview, title, confirmLabel, generation, onConfirm, onClose, contextLoading = false,
}: {
  preview: ConfirmationPreview;
  title: string;
  confirmLabel: string;
  generation: GenerationController;
  onConfirm: () => void;
  onClose: () => void;
  contextLoading?: boolean;
}) {
  const { state, active } = generation;
  const dialog = useRef<HTMLDivElement>(null);
  const phase = state.phase;
  useEffect(() => {
    if (phase !== 'idle') dialog.current?.focus();
  }, [phase]);
  if (phase === 'idle') {
    return <PreviewModal preview={preview} title={`${title} · 发送预览`} confirmLabel={confirmLabel}
      onConfirm={onConfirm} onCancel={onClose} confirmDisabled={contextLoading}>
      {contextLoading ? <p className="panel-note">正在刷新所选期间，请稍候再确认发送。</p> : null}
    </PreviewModal>;
  }
  const label = phase === 'succeeded' ? '生成完成' : phase === 'failed' ? '生成失败'
    : phase === 'cancelled' ? '已停止' : phase === 'stopping' ? '正在停止'
    : phase === 'starting' ? '正在准备请求' : '正在生成';
  const event = state.latest;
  const elapsed = Math.max(state.elapsedMs, event?.elapsed_ms ?? 0);
  const secondsSinceOutput = state.lastActivityAt == null ? null
    : Math.floor(Math.max(0, Date.now() - state.lastActivityAt) / 1000);
  const receiving = secondsSinceOutput != null && secondsSinceOutput < 2;
  const stage = event?.stage;
  const outputStage = stage === 'receiving'
    ? receiving ? '正在接收模型响应' : secondsSinceOutput == null ? '等待模型输出' : '等待后续输出'
    : stage ? stageLabels[stage] ?? '正在处理模型响应' : label;
  const activityHint = stage === 'validating' ? '模型输出已结束，正在校验生成结果。'
    : stage === 'saving' ? '校验已通过，正在保存结果。'
    : stage === 'completed' || stage === 'succeeded' ? '模型输出已结束，正在确认保存结果。'
    : phase === 'starting' ? '正在连接实时状态通道，请稍候。'
    : secondsSinceOutput == null ? '正在等待模型输出。等待期间可以手动停止。'
    : receiving ? '正在接收模型输出，完整结果将在校验和保存后显示。'
    : '等待后续输出，当前请求仍在进行，可以手动停止。';
  return (
    <div className="modal-scrim" data-testid="preview-modal">
      <div className="modal generation-dialog" role="dialog" aria-modal="true" aria-label={`${title} · ${label}`}
        ref={dialog} tabIndex={-1} onKeyDown={(key) => {
          if (key.key === 'Escape') { key.preventDefault(); if (!active) onClose(); }
          if (key.key === 'Tab') {
            const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
            if (!buttons?.length) { key.preventDefault(); return; }
            const first = buttons[0], last = buttons[buttons.length - 1];
            if (key.shiftKey && (document.activeElement === first || document.activeElement === dialog.current)) {
              key.preventDefault(); last.focus();
            } else if (!key.shiftKey && document.activeElement === last) {
              key.preventDefault(); first.focus();
            }
          }
        }}>
        <div className="modal-head">
          <div><h3>{title}</h3><div className="diagnosis-report-meta">{preview.provider.name} · {preview.provider.model}</div></div>
          <span className={`tag ${phase === 'failed' ? 'red' : phase === 'succeeded' ? 'green' : 'gold'}`}>{label}</span>
        </div>
        <div className="modal-body" data-testid="generation-progress-panel">
          <div className="generation-stage" role="status" aria-live="polite">
            {active ? <span className="generation-spinner" aria-hidden="true" /> : <span className="generation-terminal-mark" aria-hidden="true">{phase === 'succeeded' ? '✓' : '—'}</span>}
            <div>
              <strong data-testid="generation-progress-stage">{active && phase !== 'stopping' && event
                ? outputStage : label}</strong>
              <p className="muted" data-testid="generation-progress-hint">{state.message ?? activityHint}</p>
              {active ? <p className="muted" data-testid="generation-last-output">{secondsSinceOutput == null
                ? '尚未收到模型输出'
                : `距上次输出 ${groupDecimal(String(secondsSinceOutput))} 秒`}</p> : null}
            </div>
          </div>
          <dl className="generation-stats" data-testid="generation-progress-stats">
            <div><dt>已用时间</dt><dd>{Math.floor(elapsed / 1000)}<small> 秒</small></dd></div>
            <div><dt>已接收</dt><dd>{event ? groupDecimal(String(event.bytes)) : '—'}<small> 字节</small></dd></div>
            <div><dt>输出活动</dt><dd>{event ? groupDecimal(String(event.chunk_seq)) : '—'}<small> 块</small></dd></div>
          </dl>
          {(event?.content_chars != null || event?.reasoning_chars != null) ? (
            <p className="generation-text-counts">正文 {groupDecimal(String(event.content_chars ?? 0))} 字符
              {' · '}思考活动 {groupDecimal(String(event.reasoning_chars ?? 0))} 字符（仅计数）</p>
          ) : null}
          {state.error ? <div className="notice error" role="alert">{state.error}</div> : null}
          {active ? <p className="panel-note">停止会请求结束本次生成；远端已产生的用量仍以服务商记录为准。</p> : null}
        </div>
        <div className="modal-actions">
          {state.canRetryConnection && !active ? <button type="button" className="outline"
            disabled={contextLoading} onClick={onConfirm}>重试连接并发送</button> : null}
          {active ? <button type="button" className="outline" onClick={generation.stop}
            disabled={phase === 'stopping'} data-testid="generation-progress-cancel">
            {phase === 'stopping' ? '正在停止…' : '停止生成'}
          </button> : <button type="button" className="primary" data-testid="generation-close" onClick={onClose}>
            {phase === 'succeeded' ? '查看结果' : '关闭'}
          </button>}
        </div>
      </div>
    </div>
  );
}
