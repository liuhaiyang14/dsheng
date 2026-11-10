/**
 * 发送预览确认弹层（confirmationPreview 呈现）。
 *
 * 发送前让用户确认期间、数据范围和可选历史参考；取消不会发送。
 */

import type { ReactNode } from 'react';
import type { ConfirmationPreview } from '../api';

const FRAMEWORK_LABELS: Record<string, string> = {
  '5why': '五问根因分析',
  '5w': '五W问题界定',
  '5w2h': '五W二H行动分析',
};

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
            <div className="diagnosis-report-meta">请确认本次使用的数据范围后再发送</div>
          </div>
          <span className="tag gold">发送前预览</span>
        </div>
        <div className="modal-body">
          {preview.diagnosis_replacement ? (
            <div className="notice" role="note" data-testid="diagnosis-replacement-notice" style={{ marginBottom: 16 }}>
              当前已有第 {preview.diagnosis_replacement.version_no} 版报告。确认后将重新生成，成功时以第 {preview.diagnosis_replacement.next_version_no} 版替换当前展示；旧版保留在版本选择中。失败或停止不会替换现有报告。
            </div>
          ) : null}
          <div className="kv-list">
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
                  {(FRAMEWORK_LABELS[preview.task_context_summary.diagnosis_framework] ?? '已保存诊断报告')} · {preview.task_context_summary.diagnosis_summary}
                  <br />事实及用户陈述 {preview.task_context_summary.diagnosis_fact_count} 项；假设及待确认内容 {preview.task_context_summary.diagnosis_hypothesis_count} 项，连同对应证据发送。
                </> : '未引用诊断报告。'}</span>
              </div>
            </> : null}
            <div className="kv">
              <label>发送方式</label>
              <span>本次只发送一次；如需重新生成，可由你再次发起。</span>
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
