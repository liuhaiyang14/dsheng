/**
 * 发送预览确认弹层（confirmationPreview 呈现）。
 *
 * 门禁：外发前必须显示 Provider、文件/工作表、payload 大小与发送范围，
 * 并由用户显式点击确认；取消不发送（P0）。
 */

import type { ReactNode } from 'react';
import type { ConfirmationPreview } from '../api';
import { groupDecimal } from '../api';

export function PreviewModal({
  preview,
  title,
  confirmLabel,
  onConfirm,
  onCancel,
  busy,
  confirmDisabled,
  children,
}: {
  preview: ConfirmationPreview;
  title: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
  busy?: boolean;
  confirmDisabled?: boolean;
  /** 附加选择（如任务生成时可选引用诊断报告）。 */
  children?: ReactNode;
}) {
  return (
    <div className="modal-scrim" data-testid="preview-modal">
      <div className="modal" role="dialog" aria-label={title}>
        <div className="modal-head">
          <div>
            <h3>{title}</h3>
            <div className="diagnosis-report-meta">
              确认编号 {preview.confirmation_id.slice(0, 8)}… · 有效期至{' '}
              {preview.expires_at.replace('T', ' ').slice(0, 16)} UTC
            </div>
          </div>
          <span className="tag gold">发送前预览</span>
        </div>
        <div className="modal-body">
          <div className="kv-list">
            <div className="kv">
              <label>Provider</label>
              <span>
                {preview.provider.name} · {preview.provider.model}（revision{' '}
                {preview.provider.revision}）
              </span>
            </div>
            <div className="kv">
              <label>Base URL</label>
              <span>{preview.provider.base_url}</span>
            </div>
            <div className="kv">
              <label>发送范围（文件 / 工作表 / 行数）</label>
              <span>
                {preview.source_scope_summary
                  .map(
                    (s) =>
                      `${s.file_label}${s.sheet ? ` / ${s.sheet}` : ''} · ${s.selected_range} · ${s.row_count} 行`,
                  )
                  .join('；')}
              </span>
            </div>
            <div className="kv">
              <label>请求大小与预算</label>
              <span>
                {groupDecimal(String(preview.request_bytes))} 字节 · 保守输入{' '}
                {groupDecimal(String(preview.conservative_input_tokens))} tokens · 窗口{' '}
                {groupDecimal(String(preview.budget.context_window_tokens))} · 输出预留{' '}
                {groupDecimal(String(preview.budget.output_reserve_tokens))}
              </span>
            </div>
            {preview.task_context_summary ? <>
              <div className="kv" data-testid="preview-task-history">
                <label>历史任务</label>
                <span>{preview.task_context_summary.prior_task_titles.length > 0 ? <>
                  {preview.task_context_summary.prior_task_titles.length} 项正式任务；发送其当前进度、执行记录及已有目标。
                  <details><summary>查看任务范围</summary>
                    <ul>{preview.task_context_summary.prior_task_titles.map((taskTitle, index) => <li key={index}>{taskTitle}</li>)}</ul>
                  </details>
                </> : '没有相关历史任务，本次依据当前经营数据制定计划。'}</span>
              </div>
              <div className="kv" data-testid="preview-task-diagnosis">
                <label>诊断参考</label>
                <span>{preview.task_context_summary.diagnosis_framework ? <>
                  {preview.task_context_summary.diagnosis_framework.toUpperCase()} · {preview.task_context_summary.diagnosis_summary}
                  <br />事实及用户陈述 {preview.task_context_summary.diagnosis_fact_count} 项；假设及待确认内容 {preview.task_context_summary.diagnosis_hypothesis_count} 项，连同对应证据发送。
                </> : '未引用诊断报告。'}</span>
              </div>
            </> : null}
            <div className="kv">
              <label>版本绑定</label>
              <span>
                分析 revision {preview.analysis_revision_no} · 指纹{' '}
                {preview.analysis_input_fingerprint.slice(0, 10)}… · prompt{' '}
                {preview.prompt_version} · schema {preview.schema_version}
              </span>
            </div>
            <div className="kv">
              <label>请求次数</label>
              <span>最多 {preview.request_count_maximum} 次；无自动重试</span>
            </div>
          </div>
          {children}
        </div>
        <div className="modal-actions">
          <button type="button" className="outline" onClick={onCancel} disabled={busy}>
            取消（不发送）
          </button>
          <button
            type="button"
            className="primary"
            onClick={onConfirm}
            disabled={busy || confirmDisabled}
            data-testid="preview-confirm"
          >
            {busy ? '处理中…' : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
