/**
 * 任务单屏（S6）。
 *
 * 门禁：「生成任务单」只能由用户显式点击触发；进入页面、查看/生成/停止
 * 诊断报告等任何事件都不自动触发任务生成。任务页不提供根因追问。
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { GenerationProgressPanel } from '../components/GenerationProgressPanel';
import { PreviewModal } from '../components/PreviewModal';
import { TaskDrawer } from '../components/TaskDrawer';
import { TimelineBoard } from '../components/TimelineBoard';
import {
  CYCLE_ROLE_LABELS,
  TASK_CATEGORY_LABELS,
  TASK_STATUS_LABELS,
  groupDecimal,
  type Cadence,
  type ConfirmationPreview,
  type DiagnosisReportSummaryView,
  type TaskCandidateView,
  type TaskReconcileInput,
  type TaskView,
  type TimelineEntryView,
} from '../api';

function cycleRoleOf(task: TaskView): 'review' | 'plan' {
  if (!task.target_period) return 'review';
  return task.target_period.from >= '2026-11-01' ? 'plan' : 'review';
}

function friendlyError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  if (/PROVIDER_REQUIRED|No explicit\/default provider|未.*默认 Provider/i.test(raw)) {
    return `本档案尚未配置默认 Provider——请到「Provider 设置」填写 Base URL／模型／API Key（Key 仅存本机），并点「设为默认」后回来重试。（${raw}）`;
  }
  if (/PROVIDER_KEY_MISSING|no Key|未配置.*Key/i.test(raw)) {
    return `Provider 缺少 API Key——请到「Provider 设置」重新保存并填入 Key。（${raw}）`;
  }
  return raw;
}

export function TaskScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const tasks = app.data.tasks;
  const detail = app.data.currentDetail;
  const runId = detail?.run_id ?? null;

  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [reports, setReports] = useState<DiagnosisReportSummaryView[]>([]);
  const [useReport, setUseReport] = useState(false);
  const [reportId, setReportId] = useState('');
  const [generateModalOpen, setGenerateModalOpen] = useState(false);
  const [preview, setPreview] = useState<ConfirmationPreview | null>(null);
  /** 确认后保留指纹供 reconcile 使用。 */
  const [lastFingerprint, setLastFingerprint] = useState<string>('');
  const [candidate, setCandidate] = useState<TaskCandidateView | null>(null);
  const [applySelection, setApplySelection] = useState<Record<string, boolean>>({});
  const [busy, setBusy] = useState(false);
  /** confirm 发送进行中（进度面板渲染窗口；终态或异常即关闭）。 */
  const [sending, setSending] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [timelineGrain, setTimelineGrain] = useState<Cadence>('day');
  const [timelineFrom, setTimelineFrom] = useState('');
  const [timelineTo, setTimelineTo] = useState('');
  const [statusFilter, setStatusFilter] = useState<string>('all');
  const [timelineEntries, setTimelineEntries] = useState<TimelineEntryView[]>([]);

  // 进入页面：只读加载诊断报告列表（供生成预览选择），绝不触发生成。
  useEffect(() => {
    if (!app.client || !app.activeWorkspaceId || !runId) {
      setReports([]);
      return;
    }
    let cancelled = false;
    app.client
      .diagnosisList(app.activeWorkspaceId, runId)
      .then((list) => {
        if (!cancelled) {
          setReports(list);
          setReportId((prev) => prev || list[0]?.report_id || '');
        }
      })
      .catch(() => {
        if (!cancelled) setReports([]);
      });
    return () => {
      cancelled = true;
    };
  }, [app.client, app.activeWorkspaceId, runId]);

  const loadTimeline = useCallback(async () => {
    if (!app.client || !app.activeWorkspaceId) {
      setTimelineEntries([]);
      return;
    }
    try {
      const from = timelineFrom || '2000-01-01';
      const to = timelineTo || '2999-12-31';
      const entries = await app.client.taskTimeline({
        workspace_id: app.activeWorkspaceId,
        from,
        to,
        grain: timelineGrain,
      });
      setTimelineEntries(
        statusFilter === 'all'
          ? entries
          : entries.filter((e) => {
              const task = tasks.find((t) => t.task_id === e.task_id);
              return task != null && task.status === statusFilter;
            }),
      );
    } catch {
      setTimelineEntries([]);
    }
  }, [app.client, app.activeWorkspaceId, timelineFrom, timelineTo, timelineGrain, statusFilter, tasks]);

  useEffect(() => {
    void loadTimeline();
  }, [loadTimeline]);

  const selectedTask = useMemo(
    () => tasks.find((t) => t.task_id === selectedTaskId) ?? null,
    [tasks, selectedTaskId],
  );

  /** 显式按钮第一步：打开生成参数（可选引用诊断报告）。 */
  const openGenerate = () => {
    if (!hasData) {
      setNotice('请先为当前企业档案导入数据并完成字段映射。');
      return;
    }
    setGenerateModalOpen(true);
  };

  /** 显式按钮第二步：请求发送预览。 */
  const requestPreview = async () => {
    if (!app.client || !app.activeWorkspaceId || !runId) return;
    setBusy(true);
    setNotice(null);
    try {
      const p = await app.client.taskGeneratePreview({
        workspace_id: app.activeWorkspaceId,
        run_id: runId,
        diagnosis_report_id: useReport && reportId ? reportId : undefined,
      });
      setPreview(p);
      setLastFingerprint(p.analysis_input_fingerprint);
      setGenerateModalOpen(false);
    } catch (err) {
      setNotice(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  /** 显式按钮第三步：确认发送 → 候选（演示层）。 */
  const cancelGeneration = async (attemptId: string) => {
    if (!app.client || !app.activeWorkspaceId) return;
    try {
      await app.client.generationCancel(app.activeWorkspaceId, attemptId);
      setNotice('已请求取消本次生成（仅本地取消；不保证远端停止计费，迟到数据不提交）。');
    } catch (err) {
      setNotice(friendlyError(err));
    }
  };

  const confirmSend = async () => {
    if (!app.client || !preview) return;
    setBusy(true);
    setSending(true);
    try {
      const c = await app.client.taskGenerateConfirm(preview.confirmation_id);
      setCandidate(c);
      setApplySelection(Object.fromEntries(c.tasks.map((t) => [t.task_key, true])));
      setNotice('候选已保存。请逐项确认后应用；应用只创建/更新任务，不重复发送请求。');
      setPreview(null);
    } catch (err) {
      setNotice(friendlyError(err));
    } finally {
      setSending(false);
      setBusy(false);
    }
  };

  /** 本地差异确认（reconcile）：用户逐项选择 new/ignore 后应用。 */
  const applyCandidate = async () => {
    if (!app.client || !app.activeWorkspaceId || !candidate || !runId || !detail) return;
    setBusy(true);
    setNotice(null);
    try {
      const input: TaskReconcileInput = {
        schema_version: '1',
        workspace_id: app.activeWorkspaceId,
        run_id: runId,
        analysis_revision_no: detail.analysis_revision_no,
        input_fingerprint: lastFingerprint || detail.captured_at,
        candidate_generation_id: candidate.candidate_generation_id,
        candidate_revision_no: candidate.candidate_revision_no,
        expected_tasks: tasks.map((t) => ({
          task_id: t.task_id,
          task_revision_no: t.task_revision_no,
        })),
        mutation_id: crypto.randomUUID(),
        mappings: candidate.tasks.map((t) =>
          applySelection[t.task_key]
            ? { action: 'new' as const, candidate_task_key: t.task_key }
            : { action: 'ignore' as const, candidate_task_key: t.task_key },
        ),
      };
      const dry = await app.client.taskReconcileDryRun(input);
      await app.client.taskReconcileApply(input, dry.apply_token);
      setCandidate(null);
      setNotice('候选已应用：新任务已创建（初始待开始）。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  if (!hasData) {
    return (
      <div>
        <div className="screen-head">
          <div>
            <div className="eyebrow">04 · action sheet</div>
            <h2 className="screen-title">任务单</h2>
            <div className="sub">
              {app.activeWorkspace?.name} · 依据经营差异明确改善目标、拆解行动步骤
            </div>
          </div>
          <div className="head-controls">
            <button type="button" className="primary" onClick={openGenerate}>
              生成任务单
            </button>
          </div>
        </div>
        <EmptyState
          title="此档案尚无任务"
          hint="请先完成数据导入与分析；任务单基于经营差异生成，不会自动触发。"
        />
      </div>
    );
  }

  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">04 · action sheet</div>
          <h2 className="screen-title">任务单</h2>
          <div className="sub">
            依据经营差异明确改善目标、拆解行动步骤，并用 SMART 原则校验目标及子目标。
          </div>
        </div>
        <div className="head-controls">
          <button
            type="button"
            className="primary"
            onClick={openGenerate}
            data-testid="generate-task-button"
          >
            生成任务单
          </button>
        </div>
      </div>

      {notice ? (
        <div className="notice workspace-empty" data-testid="task-notice">
          <span>{notice}</span>
        </div>
      ) : null}

      <div className="task-layout">
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">任务单检视 / 计划</span>
            <span className="panel-note">
              {detail?.period.from} 至 {detail?.period.to} · {tasks.length} 项任务 · 点击查看详情
            </span>
          </div>
          <table className="task-table" data-testid="task-list-table">
            <thead>
              <tr>
                <th>任务 / 指标</th>
                <th>类别 / 周期角色</th>
                <th>目标 / 节点</th>
                <th>责任岗位</th>
                <th>进度</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => {
                const role = cycleRoleOf(task);
                return (
                  <tr
                    key={task.task_id}
                    className={task.task_id === selectedTaskId ? 'selected' : ''}
                    onClick={() => setSelectedTaskId(task.task_id)}
                    data-testid={`task-row-${task.task_id.slice(0, 8)}`}
                  >
                    <td>
                      <div className="task-name">{task.title}</div>
                      <div className="file-meta">{task.summary}</div>
                      {task.evidence[0] ? (
                        <div className="task-source">
                          来源：<span className="evidence-ref">{task.evidence[0].locator}</span>
                        </div>
                      ) : null}
                    </td>
                    <td>
                      <Tag tone={role === 'review' ? 'blue' : 'red'}>
                        {CYCLE_ROLE_LABELS[role]}
                      </Tag>
                      <div className="task-source" data-testid={`task-category-${task.task_id.slice(0, 8)}`}>
                        {TASK_CATEGORY_LABELS[task.category]}
                      </div>
                    </td>
                    <td>
                      {task.target_value != null
                        ? `${groupDecimal(task.target_value)}${task.unit ? ` ${task.unit}` : ''}`
                        : '未设目标'}
                      <div className="file-meta">
                        {task.due_date ? `截止 ${task.due_date}` : '未设置截止日期'}
                      </div>
                    </td>
                    <td>{task.owner ?? '待确认'}</td>
                    <td>
                      <Tag
                        tone={
                          task.status === 'blocked'
                            ? 'red'
                            : task.status === 'completed'
                              ? 'green'
                              : 'gold'
                        }
                      >
                        {groupDecimal(task.progress)}% · {TASK_STATUS_LABELS[task.status]}
                      </Tag>
                      {task.overdue && task.status !== 'completed' ? (
                        <div className="task-source">已延期（按截止日派生）</div>
                      ) : null}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
          <div className="task-generation-note">
            <strong>任务单制定原则</strong>
            <p>
              根据经营差异和 Skill 明确改善目标，再按业务依据拆解行动步骤；SMART
              校验由核心完成，人工修改后标记待复核。责任人、节点和验收依据作为执行字段单独确认。
            </p>
            {reports.length > 0 ? (
              <label>
                <input
                  type="checkbox"
                  checked={useReport}
                  onChange={(e) => setUseReport(e.target.checked)}
                  data-testid="use-diagnosis-context"
                />
                <span>
                  可选参考本期已保存的诊断报告（{reports.length} 份）；引用只作辅助背景，
                  取消勾选或没有报告都不影响按经营数据生成任务。
                </span>
              </label>
            ) : (
              <p>本期没有已保存的诊断报告；无报告可照常生成任务。</p>
            )}
          </div>
          <div className="root-card">
            <h4>任务推进方式</h4>
            <p>
              勾选行动步骤由核心自动更新进度；手工调整必须填写原因；阻塞必须填写理由；
              完成必须达到 100%。所有变化进入任务时间线。
            </p>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">本期任务制定依据</span>
            <span className="panel-note">经营差异 · Skill · SMART 检查</span>
          </div>
          <div className="mapping">
            {(detail?.metrics ?? [])
              .filter((m) => m.delta != null)
              .slice(0, 3)
              .map((m) => (
                <div className="mapping-row" key={m.metric_id}>
                  <label>关联指标</label>
                  <span>
                    {m.metric_label} · 实绩 {groupDecimal(m.actual_value ?? '')} · 目标{' '}
                    {m.target_value != null ? groupDecimal(m.target_value) : '未设'}
                  </span>
                </div>
              ))}
            <div className="mapping-row">
              <label>目标与行动检查</label>
              <span>
                SMART 五项由核心在生成时评价；人工修改内容后标 stale，不自动重新调用模型。
              </span>
            </div>
          </div>
        </div>
      </div>

      {candidate ? (
        <div className="panel" style={{ marginTop: 14 }} data-testid="candidate-panel">
          <div className="panel-head">
            <span className="panel-title">生成建议（候选 · 未应用）</span>
            <span className="panel-note">
              不按标题近似自动匹配；逐项选择 new/ignore 后应用（本地确认，无模型请求）
            </span>
          </div>
          <div className="panel-body">
            {candidate.tasks.map((t) => (
              <label key={t.task_key} style={{ display: 'flex', gap: 8, marginBottom: 8 }}>
                <input
                  type="checkbox"
                  className="check"
                  checked={applySelection[t.task_key] ?? false}
                  onChange={(e) =>
                    setApplySelection((prev) => ({ ...prev, [t.task_key]: e.target.checked }))
                  }
                />
                <span>
                  <strong>{t.title}</strong>（{TASK_CATEGORY_LABELS[t.category]} ·{' '}
                  {t.steps.length} 步 · {t.owner ?? '责任待确认'}）
                  <br />
                  <span className="muted">{t.summary}</span>
                </span>
              </label>
            ))}
            <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
              <button type="button" className="primary" onClick={() => void applyCandidate()} disabled={busy}>
                应用所选（本地确认）
              </button>
              <button
                type="button"
                className="outline"
                onClick={() => {
                  setCandidate(null);
                  setNotice('已忽略候选；候选快照仍可追溯。');
                }}
              >
                全部忽略
              </button>
            </div>
          </div>
        </div>
      ) : null}

      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <span className="panel-title">任务时间线</span>
          <span className="panel-note">按计划日期和进度事件查看；缺日期显示“未设置”不排序</span>
        </div>
        <div className="chart-toolbar">
          <select
            className="select"
            aria-label="时间线粒度"
            value={timelineGrain}
            onChange={(e) => setTimelineGrain(e.target.value as Cadence)}
          >
            <option value="day">日</option>
            <option value="week">周</option>
            <option value="month">月</option>
            <option value="quarter">季度</option>
          </select>
          <input
            className="select"
            type="date"
            aria-label="时间线开始日期"
            value={timelineFrom}
            onChange={(e) => setTimelineFrom(e.target.value)}
          />
          <input
            className="select"
            type="date"
            aria-label="时间线结束日期"
            value={timelineTo}
            onChange={(e) => setTimelineTo(e.target.value)}
          />
          <select
            className="select"
            aria-label="状态筛选"
            value={statusFilter}
            onChange={(e) => setStatusFilter(e.target.value)}
          >
            <option value="all">全部状态</option>
            <option value="not_started">待开始</option>
            <option value="in_progress">进行中</option>
            <option value="blocked">有阻塞</option>
            <option value="completed">已完成</option>
          </select>
        </div>
        <div className="timeline-board">
          {timelineEntries.length === 0 ? (
            <div className="panel-note" style={{ padding: '10px 0' }}>
              当前筛选范围内没有任务节点或进度事件。
            </div>
          ) : (
            <TimelineBoard entries={timelineEntries} />
          )}
        </div>
      </div>

      <div className="footer-note">
        任务生成/诊断报告分别保存、分别导出；任务页不提供根因追问入口。
      </div>

      {generateModalOpen ? (
        <div className="modal-scrim" data-testid="generate-param-modal">
          <div className="modal" role="dialog" aria-label="生成任务单参数">
            <div className="modal-head">
              <div>
                <h3>生成任务单</h3>
                <div className="diagnosis-report-meta">
                  依据本期经营差异和 Skill 原则；SMART 校核由核心在生成时完成。
                </div>
              </div>
            </div>
            <div className="modal-body">
              {reports.length > 0 ? (
                <>
                  <label style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 8 }}>
                    <input
                      type="checkbox"
                      className="check"
                      checked={useReport}
                      onChange={(e) => setUseReport(e.target.checked)}
                    />
                    引用本期已保存的诊断报告（可选）
                  </label>
                  <select
                    className="select"
                    aria-label="引用报告"
                    value={reportId}
                    onChange={(e) => setReportId(e.target.value)}
                    disabled={!useReport}
                  >
                    {reports.map((r) => (
                      <option key={r.report_id} value={r.report_id}>
                        {r.problem_statement.slice(0, 24)}… · {r.framework.toUpperCase()} · 第{' '}
                        {r.report_revision_no} 版
                      </option>
                    ))}
                  </select>
                  <p className="panel-note" style={{ marginTop: 8 }}>
                    引用只作辅助背景并记录报告 ID；报告中的假设仍需核实，不会覆盖经营数据。
                  </p>
                </>
              ) : (
                <p className="panel-note">本期没有诊断报告；无报告可照常生成任务。</p>
              )}
            </div>
            <div className="modal-actions">
              <button type="button" className="outline" onClick={() => setGenerateModalOpen(false)}>
                取消
              </button>
              <button
                type="button"
                className="primary"
                onClick={() => void requestPreview()}
                disabled={busy}
                data-testid="request-task-preview"
              >
                下一步：发送预览
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {preview ? (
        <>
          <GenerationProgressPanel
            active={sending}
            title="任务生成发送中"
            onCancel={(attemptId) => void cancelGeneration(attemptId)}
          />
          <PreviewModal
          preview={preview}
          title="生成任务单 · 发送预览"
          confirmLabel="确认生成任务单（发送 1 次请求）"
          onConfirm={() => void confirmSend()}
          onCancel={() => setPreview(null)}
          busy={busy}
        />
        </>
      ) : null}

      {selectedTask && app.client && app.activeWorkspace ? (
        <TaskDrawer
          task={selectedTask}
          workspace={app.activeWorkspace}
          client={app.client}
          onClose={() => setSelectedTaskId(null)}
          onSaved={() => void app.reloadWorkspaceData()}
        />
      ) : null}
    </div>
  );
}
