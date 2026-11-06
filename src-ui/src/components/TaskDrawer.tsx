/**
 * 任务详情抽屉（S6）。
 *
 * 呈现层规则（一切以核心返回为准，UI 只呈现与提交合同化输入）：
 * - auto 进度：显示核心计算值（提交 progress=null，服务计算）；
 * - manual 进度：必填原因；blocked 必填理由；completed 必须 100；
 * - 重开（completed → 非 completed）需用户确认；
 * - SMART 校核只读展示 generated/stale；
 * - 步骤增删改排序经 content 操作（step_operations）提交；
 * - 保存成功后以核心返回刷新，错误展示 envelope 的 message。
 */

import { useEffect, useMemo, useState } from 'react';
import {
  CYCLE_ROLE_LABELS,
  TASK_CATEGORY_LABELS,
  TASK_STATUS_LABELS,
  groupDecimal,
  type DshengClient,
  type MetricRowView,
  type StepOperation,
  type SmartCheckView,
  type TaskEditInput,
  type TaskStatus,
  type TaskView,
  type WorkspaceView,
} from '../api';
import { Tag } from './bits';
import { normalizeTaskDecimal } from './taskForm';

const STATUS_OPTIONS: TaskStatus[] = ['not_started', 'in_progress', 'completed', 'blocked'];
const CATEGORY_OPTIONS = Object.entries(TASK_CATEGORY_LABELS) as [
  keyof typeof TASK_CATEGORY_LABELS,
  string,
][];

const PROGRESS_INPUT_PATTERN = /^(?:(?:0|[1-9][0-9]?)(?:\.[0-9]?[1-9])?|100)$/;

const SMART_DIMENSION_LABELS = { S: '具体性 S', M: '可衡量 M', A: '可实现 A', R: '相关性 R', T: '时限 T' };
const SMART_STATUS_LABELS = { met: '通过', needs_confirmation: '待确认', not_applicable: '不适用' };

/** 仅检查读视图的展示结构；不在前端重新判断或推算 SMART 结论。 */
function readSmartCheck(value: unknown): SmartCheckView | null {
  const record = (item: unknown): item is Record<string, unknown> => item != null && typeof item === 'object' && !Array.isArray(item);
  if (!record(value) || !['generated', 'stale'].includes(value.smart_status as string)
    || typeof value.assessed_task_revision !== 'number' || !Number.isInteger(value.assessed_task_revision) || value.assessed_task_revision < 1
    || !Array.isArray(value.checks) || !value.checks.every((check: unknown) => record(check)
      && typeof check.dimension === 'string' && Object.prototype.hasOwnProperty.call(SMART_DIMENSION_LABELS, check.dimension)
      && typeof check.status === 'string' && Object.prototype.hasOwnProperty.call(SMART_STATUS_LABELS, check.status)
      && (check.note === null || typeof check.note === 'string'))) return null;
  return value as unknown as SmartCheckView;
}

interface DraftStep {
  /** 待提交的操作（已有步骤的临时编辑态）。 */
  step_id?: string;
  client_step_key?: string;
  action: string;
  owner: string | null;
  start_date: string | null;
  due_date: string | null;
  done_criteria: string | null;
  status: TaskStatus;
  blocked_reason: string | null;
  /** UI 排序位置。 */
  order: number;
}

export function TaskDrawer({
  task,
  workspace,
  client,
  onClose,
  onSaved,
  metrics = [],
  periodEnd,
  readOnly = false,
}: {
  task: TaskView;
  workspace: WorkspaceView;
  client: DshengClient;
  onClose: () => void;
  onSaved: () => void;
  metrics?: MetricRowView[];
  periodEnd?: string;
  readOnly?: boolean;
}) {
  const [current, setCurrent] = useState<TaskView>(task);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [dirty, setDirty] = useState(false);
  const locked = readOnly || current.lifecycle !== 'active';
  const extraKeys = ['metric_id', 'planned_value', 'actual_value', 'current_value', 'unit', 'mp_label', 'mp_unit', 'resource_need', 'success_criteria', 'note', 'review_note'] as const;
  type ExtraKey = typeof extraKeys[number];
  const valuesOf = (value: TaskView) => Object.fromEntries(extraKeys.map((key) => [key, value[key] ?? ''])) as Record<ExtraKey, string>;
  const [extra, setExtra] = useState(() => valuesOf(task));
  const updateExtra = (key: ExtraKey, value: string) => { setExtra((previous) => ({ ...previous, [key]: value })); setDirty(true); };
  const close = () => {
    if (busy) return;
    if (dirty && !window.confirm('还有未保存的修改，确认放弃并关闭？')) return;
    onClose();
  };

  /** 以核心返回的任务视图重置全部编辑态。 */
  const syncFromTask = (t: TaskView) => {
    setExtra(valuesOf(t));
    setTitle(t.title);
    setSummary(t.summary);
    setCategory(t.category);
    setOwner(t.owner ?? '');
    setPlannedStart(t.planned_start ?? '');
    setDueDate(t.due_date ?? '');
    setTargetValue(t.target_value ?? '');
    setSteps(
      t.steps.map((s, i) => ({
        step_id: s.step_id,
        action: s.action,
        owner: s.owner,
        start_date: s.start_date,
        due_date: s.due_date,
        done_criteria: s.done_criteria,
        status: s.status,
        blocked_reason: s.blocked_reason,
        order: i,
      })),
    );
    setProgressMode(t.progress_mode);
    setNextStatus(t.status);
    setManualProgress(t.progress);
  };

  // 外部（store 刷新）带来的任务版本变化时重新同步；本地保存已自行同步。
  useEffect(() => {
    if (task.task_id !== current.task_id || task.task_revision_no !== current.task_revision_no) {
      setCurrent(task);
      syncFromTask(task);
      setDirty(false);
      setNotice(null);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [task]);

  // ---- 内容编辑态 ----
  const [title, setTitle] = useState(task.title);
  const [summary, setSummary] = useState(task.summary);
  const [category, setCategory] = useState(task.category);
  const [owner, setOwner] = useState(task.owner ?? '');
  const [plannedStart, setPlannedStart] = useState(task.planned_start ?? '');
  const [dueDate, setDueDate] = useState(task.due_date ?? '');
  const [targetValue, setTargetValue] = useState(task.target_value ?? '');
  const [sourceNote, setSourceNote] = useState('');
  const [steps, setSteps] = useState<DraftStep[]>(() =>
    task.steps.map((s, i) => ({
      step_id: s.step_id,
      action: s.action,
      owner: s.owner,
      start_date: s.start_date,
      due_date: s.due_date,
      done_criteria: s.done_criteria,
      status: s.status,
      blocked_reason: s.blocked_reason,
      order: i,
    })),
  );

  // ---- 进度编辑态 ----
  const [progressMode, setProgressMode] = useState<'auto' | 'manual'>(task.progress_mode);
  const [manualProgress, setManualProgress] = useState(task.progress);
  const [nextStatus, setNextStatus] = useState<TaskStatus>(task.status);
  const [adjustmentReason, setAdjustmentReason] = useState('');
  const [blockedReason, setBlockedReason] = useState('');

  const completedCount = current.steps.filter((s) => s.status === 'completed').length;

  const markDirty = () => setDirty(true);

  const patchStep = (index: number, patch: Partial<DraftStep>) => {
    setSteps((prev) => prev.map((s, i) => (i === index ? { ...s, ...patch } : s)));
    markDirty();
  };

  const addStep = () => {
    setSteps((prev) => [
      ...prev,
      {
        client_step_key: `step-${Date.now()}-${prev.length}`,
        action: '',
        owner: null,
        start_date: null,
        due_date: null,
        done_criteria: null,
        status: 'not_started',
        blocked_reason: null,
        order: prev.length,
      },
    ]);
    markDirty();
  };

  const removeStep = (index: number) => {
    if (steps.length <= 1) {
      setNotice('任务必须保留至少一条行动步骤。');
      return;
    }
    setSteps((prev) => prev.filter((_, i) => i !== index));
    markDirty();
  };

  const moveStep = (index: number, delta: -1 | 1) => {
    const target = index + delta;
    if (target < 0 || target >= steps.length) return;
    setSteps((prev) => {
      const next = [...prev];
      [next[index], next[target]] = [next[target], next[index]];
      return next;
    });
    markDirty();
  };

  const toggleStepDone = (index: number, done: boolean) => {
    if (!steps[index].step_id || locked || busy) return;
    if (!done && current.status === 'completed'
      && !window.confirm('取消勾选步骤将把已完成任务重开（保留旧完成事件）。继续？')) return;
    patchStep(index, { status: done ? 'completed' : 'not_started', blocked_reason: null });
  };

  /** 由编辑态生成 step_operations：删除 → 更新 → 排序(move) → 新增(add)。 */
  const buildStepOperations = (): StepOperation[] => {
    const ops: StepOperation[] = [];
    const keptIds = steps.filter((s) => s.step_id).map((s) => s.step_id!);

    // 删除
    for (const original of current.steps) {
      if (!keptIds.includes(original.step_id)) {
        ops.push({ op: 'delete', step_id: original.step_id });
      }
    }

    // 更新（内容/状态 diff）
    for (const step of steps) {
      if (!step.step_id) continue;
      const original = current.steps.find((s) => s.step_id === step.step_id)!;
      const patch: Record<string, unknown> = {};
      if (original.action !== step.action) patch.action = step.action;
      if (original.owner !== step.owner) patch.owner = step.owner;
      if (original.start_date !== step.start_date) patch.start_date = step.start_date;
      if (original.due_date !== step.due_date) patch.due_date = step.due_date;
      if (original.done_criteria !== step.done_criteria) {
        patch.done_criteria = step.done_criteria;
      }
      if (original.status !== step.status) {
        patch.status = step.status;
        patch.blocked_reason = step.blocked_reason;
      }
      if (Object.keys(patch).length > 0) {
        ops.push({ op: 'update', step_id: step.step_id, patch });
      }
    }

    // 排序：保留步骤顺序变化时，从末尾向前逐个 move 到锚点前
    const originalKeptOrder = current.steps
      .map((s) => s.step_id)
      .filter((id) => keptIds.includes(id));
    if (originalKeptOrder.join(',') !== keptIds.join(',')) {
      for (let i = keptIds.length - 1; i >= 0; i -= 1) {
        ops.push({
          op: 'move',
          step_id: keptIds[i],
          before_step_id: keptIds[i + 1] ?? null,
        });
      }
    }

    // 新增（锚定到目标顺序中的下一个已存在步骤）
    const nextExistingOf = (index: number): string | null => {
      for (let j = index + 1; j < steps.length; j += 1) {
        if (steps[j].step_id) return steps[j].step_id!;
      }
      return null;
    };
    steps.forEach((step, i) => {
      if (step.step_id) return;
      ops.push({
        op: 'add',
        client_step_key: step.client_step_key ?? `step-${crypto.randomUUID()}`,
        content: {
          action: step.action,
          owner: step.owner,
          start_date: step.start_date,
          due_date: step.due_date,
          done_criteria: step.done_criteria,
        },
        before_step_id: nextExistingOf(i),
      });
    });

    return ops;
  };

  const saveTask = async () => {
    if (locked || busy) return;
    setNotice(null);
    // UI 层预校验（核心仍为最终权威）
    if (!title.trim()) {
      setNotice('任务名称不能为空。');
      return;
    }
    if (plannedStart && dueDate && plannedStart > dueDate) {
      setNotice('计划开始日期不能晚于截止日期。');
      return;
    }
    const numericValues = { target_value: normalizeTaskDecimal(targetValue), planned_value: normalizeTaskDecimal(extra.planned_value), actual_value: normalizeTaskDecimal(extra.actual_value), current_value: normalizeTaskDecimal(extra.current_value) };
    if (Object.values(numericValues).some((value) => value === null)) {
      setNotice('数值请填写普通数字，最多六位小数，不加千分位或单位。');
      return;
    }
    const numericChanged = Object.entries(numericValues).some(([key, value]) => value !== (current[key as keyof typeof numericValues] ?? ''));
    if (numericChanged && !sourceNote.trim()) {
      setNotice('修改目标/实绩等数值必须注明人工调整依据（source_note）。');
      return;
    }
    if ((category !== current.category || extra.metric_id !== (current.metric_id ?? '')) && category !== 'other' && !extra.metric_id) {
      setNotice('请选择关联指标；无关联指标的任务请选择“其他事项”。');
      return;
    }
    if (steps.some((step) => !step.action.trim())) {
      setNotice('每一项行动都需要填写内容。');
      return;
    }
    if (steps.some((step) => step.start_date && step.due_date && step.start_date > step.due_date)) {
      setNotice('行动开始日期不能晚于截止日期。');
      return;
    }
    const stepOperations = buildStepOperations();
    const patch: Record<string, unknown> = {};
    for (const key of extraKeys) {
      const value = key in numericValues ? numericValues[key as keyof typeof numericValues]! : extra[key];
      if (value !== (current[key] ?? '')) patch[key] = key === 'note' || key === 'review_note' ? value : value || null;
    }
    if (title !== current.title) patch.title = title;
    if (summary !== current.summary) patch.summary = summary;
    if (category !== current.category) patch.category = category;
    if ((owner || null) !== current.owner) patch.owner = owner || null;
    if ((plannedStart || null) !== current.planned_start) patch.planned_start = plannedStart || null;
    if ((dueDate || null) !== current.due_date) patch.due_date = dueDate || null;
    if (numericValues.target_value !== (current.target_value ?? '')) {
      patch.target_value = numericValues.target_value || null;
    }
    const hasPatch = Object.keys(patch).length > 0;
    if (!hasPatch && stepOperations.length === 0) {
      setNotice('当前没有待保存的修改。');
      return;
    }
    if (current.status === 'completed' && steps.some((step) => !step.step_id)
      && !window.confirm('新增未完成的行动项会重开已完成任务，并保留原完成记录。继续保存？')) return;
    const input: TaskEditInput = {
      schema_version: '1',
      workspace_id: workspace.workspace_id,
      task_id: current.task_id,
      expected_task_revision: current.task_revision_no,
      mutation_id: crypto.randomUUID(),
      operation: {
        kind: 'content',
        ...(hasPatch ? { patch } : {}),
        ...(stepOperations.length > 0 ? { step_operations: stepOperations } : {}),
        ...(numericChanged ? { source_note: sourceNote.trim() } : {}),
      },
    };
    setBusy(true);
    try {
      await client.taskUpdate(input);
      const updated = await client.taskShow(input.workspace_id, input.task_id);
      setCurrent(updated);
      setDirty(false);
      setSourceNote('');
      syncFromTask(updated);
      setNotice('已保存。');
      onSaved();
    } catch (err) {
      setNotice(err instanceof Error ? `保存失败：${err.message}` : String(err));
    } finally {
      setBusy(false);
    }
  };

  const saveProgress = async () => {
    if (locked || busy) return;
    setNotice(null);
    let progress: string | null = null;
    let adjustmentReasonSubmit: string | null = null;
    if (progressMode === 'manual') {
      if (!PROGRESS_INPUT_PATTERN.test(manualProgress)) {
        setNotice('手工进度必须是 0–100 的数值（最多两位小数）。');
        return;
      }
      if (!adjustmentReason.trim()) {
        setNotice('手工调整进度必须填写调整原因。');
        return;
      }
      if (nextStatus === 'completed' && manualProgress !== '100') {
        setNotice('任务进度达到 100% 后才能标记为已完成。');
        return;
      }
      if (nextStatus === 'not_started' && manualProgress !== '0') {
        setNotice('待开始任务的完成比例应为 0%。');
        return;
      }
      progress = manualProgress;
      adjustmentReasonSubmit = adjustmentReason.trim();
    }
    let blockedReasonSubmit: string | null = null;
    if (nextStatus === 'blocked') {
      if (!blockedReason.trim()) {
        setNotice('有阻塞时必须填写非空阻塞理由。');
        return;
      }
      blockedReasonSubmit = blockedReason.trim();
    }
    if (current.status === 'completed' && nextStatus !== 'completed') {
      if (
        !window.confirm('确认重开该任务？完成时间将清空，并保留旧完成事件（reopened）。')
      ) {
        return;
      }
    }
    const input: TaskEditInput = {
      schema_version: '1',
      workspace_id: workspace.workspace_id,
      task_id: current.task_id,
      expected_task_revision: current.task_revision_no,
      mutation_id: crypto.randomUUID(),
      operation: {
        kind: 'progress',
        progress_mode: progressMode,
        progress,
        status: nextStatus,
        adjustment_reason: adjustmentReasonSubmit,
        blocked_reason: blockedReasonSubmit,
      },
    };
    setBusy(true);
    try {
      await client.taskUpdate(input);
      const updated = await client.taskShow(input.workspace_id, input.task_id);
      setCurrent(updated);
      setAdjustmentReason('');
      setBlockedReason('');
      setProgressMode(updated.progress_mode);
      setNextStatus(updated.status);
      setManualProgress(updated.progress);
      setNotice('进度已保存。');
      onSaved();
    } catch (err) {
      setNotice(err instanceof Error ? `保存失败：${err.message}` : String(err));
    } finally {
      setBusy(false);
    }
  };

  const smartCheck = readSmartCheck(current.smart_check);
  const smartAvailable = smartCheck != null && smartCheck.checks.length > 0;
  const smartLabel = !smartAvailable
    ? { tone: 'gold' as const, text: 'SMART：暂无可展示评价' }
    : smartCheck.smart_status === 'generated'
      ? { tone: 'green' as const, text: 'SMART：生成时评价' }
      : { tone: 'gold' as const, text: 'SMART：内容已改待复核' };

  const cycleRole: keyof typeof CYCLE_ROLE_LABELS = current.target_period &&
    periodEnd != null && current.target_period.from > periodEnd
    ? 'plan'
    : 'review';

  const timelineEntries = useMemo(
    () =>
      [...current.events]
        .map((e) => ({
          task_id: current.task_id,
          task_title: current.title,
          kind: 'progress_event' as const,
          date: e.occurred_at.slice(0, 10),
          occurred_at: e.occurred_at,
          detail: `${e.kind}${e.reason ? `：${e.reason}` : ''}`,
        }))
        .sort((a, b) => (a.date! < b.date! ? 1 : -1)),
    [current],
  );

  return (
    <>
      <div className="drawer-scrim" onClick={close} />
      <aside className="drawer" aria-label="任务详情" data-testid="task-drawer">
        <div className="drawer-head">
          <div>
            <div className="eyebrow">TASK · {current.title}</div>
            <h3>{current.title}</h3>
            <div className="meta">
              版本 r{current.task_revision_no} · {CYCLE_ROLE_LABELS[cycleRole]} ·{' '}
              {TASK_CATEGORY_LABELS[current.category]}
              {current.metric_label ? ` · 关联指标：${current.metric_label}` : ' · 未关联指标'}
              {current.overdue ? ' · 已延期（按截止日派生）' : ''}
            </div>
          </div>
          <button type="button" className="icon-button" onClick={close} aria-label="关闭详情">
            ×
          </button>
        </div>
        <div className="drawer-body">
          {locked ? <p className="notice">当前记录只读，不能修改。</p> : null}
          <fieldset className="task-edit-fields" disabled={locked || busy}>
          <div className="progress-head">
            <span>整体进度</span>
            <span className="progress-value" data-testid="drawer-progress">
              {groupDecimal(current.progress)}%
            </span>
          </div>
          <div className="progress-track">
            <div className="progress-current" style={{ width: `${current.progress}%` }} />
          </div>
          <div className="progress-meta">
            <span data-testid="drawer-step-count">
              已完成 {completedCount} / {current.steps.length} 个步骤 ·{' '}
              {current.progress_mode === 'auto' ? '自动（按步骤计数）' : '人工（含调整原因）'}
            </span>
            <span>{current.due_date ? `截止 ${current.due_date}` : '未设截止'}</span>
          </div>
          {current.status === 'blocked' ? (
            <div className="notice" style={{ marginTop: 12, marginBottom: 0 }}>
              <b>有阻塞</b>
              <span>{current.blocked_reason}（block 优先于其他状态展示）</span>
            </div>
          ) : null}

          <div className="detail-section">
            <h4>
              任务信息 <span className="panel-note">修改后点击「保存任务」</span>
            </h4>
            <div className="detail-grid">
              <div className="detail-field">
                <label htmlFor="task-title">任务名称</label>
                <input id="task-title" className="input" value={title} onChange={(e) => { setTitle(e.target.value); markDirty(); }} />
              </div>
              <div className="detail-field">
                <label htmlFor="task-summary">任务说明</label>
                <input id="task-summary" className="input" value={summary} onChange={(e) => { setSummary(e.target.value); markDirty(); }} />
              </div>
              <div className="detail-field">
                <label htmlFor="task-category">类别</label>
                <select id="task-category" className="input" value={category} onChange={(e) => { setCategory(e.target.value as TaskView['category']); markDirty(); }}>
                  {CATEGORY_OPTIONS.map(([code, label]) => (
                    <option key={code} value={code}>{label}</option>
                  ))}
                </select>
              </div>
              <div className="detail-field">
                <label htmlFor="task-metric">关联指标</label>
                <select id="task-metric" className="input" value={extra.metric_id} onChange={(e) => updateExtra('metric_id', e.target.value)}>
                  <option value="">未关联（其他事项）</option>
                  {extra.metric_id && !metrics.some((metric) => metric.metric_id === extra.metric_id) ? <option value={extra.metric_id}>{current.metric_label ?? '当前关联指标'}</option> : null}
                  {metrics.map((metric) => <option key={metric.metric_id} value={metric.metric_id}>{metric.metric_label} · {metric.unit}</option>)}
                </select>
              </div>
              <div className="detail-field">
                <label htmlFor="task-target">目标数值</label>
                <input
                  id="task-target"
                  className="input"
                  value={targetValue}
                  onChange={(e) => { setTargetValue(e.target.value); markDirty(); }}
                  placeholder="例如 210000"
                  data-testid="target-value-input"
                />
              </div>
              <div className="detail-field">
                <label htmlFor="task-source-note">人工调整依据（改数值必填）</label>
                <input id="task-source-note" className="input" value={sourceNote} onChange={(e) => setSourceNote(e.target.value)} placeholder="注明来源，不充当模型事实" />
              </div>
              {([['planned_value', '预定数值'], ['actual_value', '实绩数值'], ['current_value', '当前数值'], ['unit', '数值单位']] as const).map(([key, label]) => (
                <div className="detail-field" key={key}><label htmlFor={`task-${key}`}>{label}</label>
                  <input id={`task-${key}`} className="input" value={extra[key]} onChange={(e) => updateExtra(key, e.target.value)} />
                </div>
              ))}
              <div className="detail-field">
                <label htmlFor="task-owner">责任岗位</label>
                <input id="task-owner" className="input" value={owner} onChange={(e) => { setOwner(e.target.value); markDirty(); }} />
              </div>
              <div className="detail-field">
                <label htmlFor="task-planned-start">计划开始</label>
                <input id="task-planned-start" type="date" className="input" value={plannedStart} onChange={(e) => { setPlannedStart(e.target.value); markDirty(); }} />
              </div>
              <div className="detail-field">
                <label htmlFor="task-due">截止日期</label>
                <input id="task-due" type="date" className="input" value={dueDate} onChange={(e) => { setDueDate(e.target.value); markDirty(); }} />
              </div>
              <div className="detail-field">
                <label>完成日期</label>
                <strong>{current.completed_at ? current.completed_at.slice(0, 10) : '未完成'}</strong>
              </div>
              {([['mp_label', '改善指标名称'], ['mp_unit', '改善指标单位'], ['resource_need', '所需资源'], ['success_criteria', '完成标准'], ['note', '备注'], ['review_note', '复盘记录']] as const).map(([key, label]) => (
                <div className="detail-field" key={key}><label htmlFor={`task-${key}`}>{label}</label>
                  <textarea id={`task-${key}`} className="input" value={extra[key]} onChange={(e) => updateExtra(key, e.target.value)} />
                </div>
              ))}
            </div>
          </div>

          <div className="detail-section">
            <div className="step-heading">
              <h4>
                行动步骤 <span className="panel-note">保存后更新进度</span>
              </h4>
              <button type="button" className="outline" onClick={addStep}>
                添加步骤
              </button>
            </div>
            {steps.map((step, index) => (
              <div className="action-step" key={step.step_id ?? step.client_step_key}>
                <input
                  type="checkbox"
                  className="check"
                  checked={step.status === 'completed'}
                  disabled={!step.step_id}
                  onChange={(e) => toggleStepDone(index, e.target.checked)}
                  aria-label={`完成步骤 ${index + 1}`}
                />
                <div className="action-copy">
                  <input
                    className="step-title-input"
                    value={step.action}
                    onChange={(e) => patchStep(index, { action: e.target.value })}
                    aria-label={`行动步骤 ${index + 1}`}
                  />
                  <small>
                    {step.status === 'completed' ? '已完成' : TASK_STATUS_LABELS[step.status]}
                    {step.blocked_reason ? ` · 阻塞：${step.blocked_reason}` : ''}
                    {step.start_date || step.due_date
                      ? ` · ${step.start_date ?? '未设开始'} 至 ${step.due_date ?? '未设截止'}`
                      : ' · 日期未设置'}
                  </small>
                  {!step.step_id ? <small>新行动项保存后可更新完成状态。</small> : null}
                  <div className="task-step-fields">
                    {([['owner', '负责人', 'text'], ['start_date', '开始日期', 'date'], ['due_date', '截止日期', 'date'], ['done_criteria', '验收标准', 'text']] as const).map(([key, label, type]) => (
                      <label key={key}>{label}<input type={type} className="input" value={step[key] ?? ''} aria-label={`行动 ${index + 1} ${label}`} onChange={(e) => patchStep(index, { [key]: e.target.value || null })} /></label>
                    ))}
                  </div>
                </div>
                <div className="step-actions">
                  <button type="button" className="step-order" title="上移" aria-label="上移行动步骤" disabled={index === 0} onClick={() => moveStep(index, -1)}>↑</button>
                  <button type="button" className="step-order" title="下移" aria-label="下移行动步骤" disabled={index === steps.length - 1} onClick={() => moveStep(index, 1)}>↓</button>
                  <button type="button" className="step-remove" title="删除步骤" aria-label="删除步骤" onClick={() => removeStep(index)}>×</button>
                </div>
              </div>
            ))}
          </div>

          <div className="detail-section" data-testid="task-smart-check">
            <h4>SMART 评价 <Tag tone={smartLabel.tone}>{smartLabel.text}</Tag></h4>
            {smartAvailable ? (
              <>
                {smartCheck.smart_status === 'stale' ? (
                  <p className="panel-note">以下为任务版本 r{smartCheck.assessed_task_revision} 的历史评价，尚未复核当前内容。</p>
                ) : null}
                <div className="kv-list">
                  {smartCheck.checks.map((c, index) => (
                    <div className="kv" key={`${c.dimension}-${index}`}>
                      <label>{SMART_DIMENSION_LABELS[c.dimension]}</label>
                      <span className={c.status === 'met' ? 'good' : c.status === 'needs_confirmation' ? 'warn' : 'panel-note'}>
                        {SMART_STATUS_LABELS[c.status]}{c.note ? ` · ${c.note}` : ''}
                      </span>
                    </div>
                  ))}
                </div>
              </>
            ) : (
              <p className="panel-note">暂无 SMART 评价，任务信息和行动项仍可查看与修改。</p>
            )}
          </div>

          <div className="detail-section">
            <h4>进度记录（按事件时间倒序）</h4>
            <div data-testid="drawer-timeline">
              {timelineEntries.length === 0 ? (
                <div className="panel-note">暂无进度记录</div>
              ) : (
                timelineEntries.map((entry, i) => (
                  <div className="timeline-item" key={i}>
                    {entry.detail}
                    <small>{entry.occurred_at.replace('T', ' ').slice(0, 16)} UTC · 当前用户</small>
                  </div>
                ))
              )}
            </div>
          </div>

          <div className="detail-section">
            <h4>更新进度</h4>
            <div className="update-form">
              <label>进度模式</label>
              <select
                className="input"
                value={progressMode}
                onChange={(e) => setProgressMode(e.target.value as 'auto' | 'manual')}
                data-testid="progress-mode-select"
              >
                <option value="auto">auto · 核心按步骤计数计算（提交 progress=null）</option>
                <option value="manual">manual · 人工填写（必填原因）</option>
              </select>
              {progressMode === 'auto' ? (
                <p className="panel-note" data-testid="auto-progress-note">
                  当前计算值：{groupDecimal(current.progress)}%（{completedCount}/{current.steps.length} 步完成）。
                  auto 模式下核心计算进度，状态须与计算一致或显式阻塞。
                </p>
              ) : (
                <>
                  <label htmlFor="manual-progress">手工完成比例（0–100，最多两位小数）</label>
                  <input
                    id="manual-progress"
                    className="input"
                    value={manualProgress}
                    onChange={(e) => setManualProgress(e.target.value)}
                    data-testid="manual-progress-input"
                  />
                  <label htmlFor="adjust-reason">调整原因（必填）</label>
                  <textarea
                    id="adjust-reason"
                    className="input"
                    value={adjustmentReason}
                    onChange={(e) => setAdjustmentReason(e.target.value)}
                    placeholder="说明为何步骤完成比率与总体评估不同"
                  />
                </>
              )}
              <label htmlFor="next-status">状态</label>
              <select
                id="next-status"
                className="input"
                value={nextStatus}
                onChange={(e) => setNextStatus(e.target.value as TaskStatus)}
                data-testid="progress-status-select"
              >
                {STATUS_OPTIONS.map((s) => (
                  <option key={s} value={s}>{TASK_STATUS_LABELS[s]}</option>
                ))}
              </select>
              {nextStatus === 'blocked' ? (
                <>
                  <label htmlFor="blocked-reason">阻塞理由（必填）</label>
                  <textarea
                    id="blocked-reason"
                    className="input"
                    value={blockedReason}
                    onChange={(e) => setBlockedReason(e.target.value)}
                    data-testid="blocked-reason-input"
                  />
                </>
              ) : null}
              {current.status === 'completed' && nextStatus !== 'completed' ? (
                <p className="panel-note">重开将清空完成时间并保留旧完成事件，保存前需确认。</p>
              ) : null}
            </div>
          </div>
          </fieldset>
        </div>
        <div className="drawer-actions">
          <div>
            <span className="save-note">
              保存到本地，不调用模型；修改内容后，原 SMART 评价标记为待复核。
            </span>
            <div className="task-save-status" data-testid="task-save-status">
              {notice ?? (dirty ? '有未保存更改' : '当前任务无待保存更改')}
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button type="button" className="outline" onClick={() => void saveTask()} disabled={busy || locked}>
              保存任务
            </button>
            <button type="button" className="primary" onClick={() => void saveProgress()} disabled={busy || locked}>
              保存进度
            </button>
          </div>
        </div>
      </aside>
    </>
  );
}
