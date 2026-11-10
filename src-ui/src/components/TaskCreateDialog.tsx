import { useRef, useState } from 'react';
import { normalizeTaskDecimal } from './taskForm';
import { TASK_CATEGORY_LABELS, type CycleRole, type DshengClient, type HistoryDetailView, type StepContent, type TaskCategory, type TaskCreateInput, type TaskView } from '../api';

const blankStep = (): StepContent => ({ action: '', owner: null, start_date: null, due_date: null, done_criteria: null });
const optionalFields = [
  ['owner', '负责人'], ['target_value', '目标数值'], ['planned_value', '预定数值'],
  ['actual_value', '实绩数值'], ['current_value', '当前数值'], ['unit', '数值单位'],
  ['resource_need', '所需资源'], ['success_criteria', '完成标准'],
] as const;
type OptionalField = typeof optionalFields[number][0];

function humanUnit(value: string | null | undefined): string {
  if (!value) return '';
  try {
    const parsed = JSON.parse(value) as unknown;
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      const baseUnit = (parsed as Record<string, unknown>).base_unit;
      if (typeof baseUnit === 'string' && baseUnit.trim()) return baseUnit;
    }
  } catch {
    // 已经是普通单位文本。
  }
  return value;
}

export function TaskCreateDialog({ client, workspaceId, detail, onClose, onCreated }: {
  client: DshengClient; workspaceId: string; detail: HistoryDetailView;
  onClose: () => void; onCreated: (task: TaskView) => void;
}) {
  // 表单绑定打开时的企业和分析版本；外层锁定企业/期间切换。
  const [context] = useState(() => ({ workspaceId, detail }));
  const [title, setTitle] = useState('');
  const [summary, setSummary] = useState('');
  const [category, setCategory] = useState<TaskCategory>('other');
  const [metricId, setMetricId] = useState('');
  const [cycleRole, setCycleRole] = useState<CycleRole>('plan');
  const [values, setValues] = useState(() => Object.fromEntries(optionalFields.map(([key]) => [key, ''])) as Record<OptionalField, string>);
  const [sourceNote, setSourceNote] = useState('');
  const [plannedStart, setPlannedStart] = useState('');
  const [dueDate, setDueDate] = useState('');
  const [steps, setSteps] = useState<StepContent[]>([blankStep()]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const intent = useRef<{ signature: string; mutationId: string }>();
  const targetPeriod = cycleRole === 'review' ? context.detail.period : context.detail.task_plan_period;
  const editStep = (index: number, field: keyof StepContent, value: string) => {
    setDirty(true);
    setSteps((previous) => previous.map((step, i) => i === index ? { ...step, [field]: field === 'action' ? value : value || null } : step));
  };
  const close = () => { if (!busy && (!dirty || window.confirm('还没有保存这项任务，确认放弃并关闭？'))) onClose(); };
  const save = async () => {
    if (busy) return;
    setError(null);
    const fingerprint = context.detail.analysis_input_fingerprint;
    if (!fingerprint) { setError('请刷新当前期间后再新建任务。'); return; }
    if (!title.trim()) { setError('请填写任务名称。'); return; }
    if (category !== 'other' && !metricId) { setError('请选择关联指标；无关联指标时请选择“其他事项”。'); return; }
    if (steps.some((step) => !step.action.trim())) { setError('请填写每一项行动内容，至少保留一项。'); return; }
    if ((plannedStart && dueDate && plannedStart > dueDate) || steps.some((step) => step.start_date && step.due_date && step.start_date > step.due_date)) {
      setError('开始日期不能晚于截止日期。'); return;
    }
    const numericValues = { target_value: normalizeTaskDecimal(values.target_value), planned_value: normalizeTaskDecimal(values.planned_value), actual_value: normalizeTaskDecimal(values.actual_value), current_value: normalizeTaskDecimal(values.current_value) };
    const numeric = Object.values(numericValues);
    if (numeric.some((value) => value === null)) { setError('数值请填写普通数字，最多六位小数，不加千分位或单位。'); return; }
    if (numeric.some(Boolean) && !sourceNote.trim()) { setError('填写数值时，请注明数值依据。'); return; }
    const draft = {
      schema_version: '1' as const, workspace_id: context.workspaceId, run_id: context.detail.run_id,
      analysis_revision_no: context.detail.analysis_revision_no, input_fingerprint: fingerprint,
      cycle_role: cycleRole, source_note: sourceNote.trim() || null,
      content: {
        title: title.trim(), summary: summary.trim(), category, metric_id: metricId || null,
        ...Object.fromEntries(optionalFields.filter(([key]) => !(key in numericValues)).map(([key]) => [key, values[key].trim() || null])),
        ...Object.fromEntries(Object.entries(numericValues).filter(([,value]) => value !== '')),
        planned_start: plannedStart || null, due_date: dueDate || null,
      }, steps: steps.map((step) => ({ ...step, action: step.action.trim() })),
    };
    const signature = JSON.stringify(draft);
    if (intent.current?.signature !== signature) intent.current = { signature, mutationId: crypto.randomUUID() };
    const input: TaskCreateInput = { ...draft, mutation_id: intent.current.mutationId };
    setBusy(true);
    try { onCreated(await client.taskCreate(input)); }
    catch (err) { setError(err instanceof Error ? err.message : String(err)); }
    finally { setBusy(false); }
  };
  return <div className="modal-scrim" data-testid="task-create-modal">
    <section className="modal task-create-dialog" role="dialog" aria-modal="true" aria-label="新增任务">
      <div className="modal-head"><div><h3>新增任务</h3><p className="panel-note">手动填写，直接保存到本地，不调用 AI。</p></div></div>
      <div className="modal-body">
        <fieldset disabled={busy} className="task-edit-fields" onChange={() => setDirty(true)}>
          <div className="detail-grid">
            <label className="detail-field">任务名称<input autoFocus className="input" value={title} onChange={(e) => setTitle(e.target.value)} /></label>
            <label className="detail-field">任务说明<textarea className="input" value={summary} onChange={(e) => setSummary(e.target.value)} /></label>
            <label className="detail-field">归属期间<select className="input" value={cycleRole} onChange={(e) => setCycleRole(e.target.value as CycleRole)}><option value="plan">下期计划</option><option value="review">本期任务</option></select></label>
            <label className="detail-field">类别<select className="input" value={category} onChange={(e) => setCategory(e.target.value as TaskCategory)}>{Object.entries(TASK_CATEGORY_LABELS).map(([key, label]) => <option value={key} key={key}>{label}</option>)}</select></label>
            <label className="detail-field">关联指标<select className="input" value={metricId} onChange={(e) => { setMetricId(e.target.value); const metric = context.detail.metrics.find((item) => item.metric_id === e.target.value); if (metric) setValues((previous) => ({ ...previous, unit: metric.unit })); }}><option value="">未关联（其他事项）</option>{context.detail.metrics.map((metric) => <option key={metric.metric_id} value={metric.metric_id}>{metric.metric_label} · {humanUnit(metric.unit)}</option>)}</select></label>
            {optionalFields.filter(([key]) => ['owner', 'target_value', 'unit'].includes(key)).map(([key, label]) => <label className="detail-field" key={key}>{label}<input className="input" value={key === 'unit' ? humanUnit(values[key]) : values[key]} onChange={(e) => setValues((previous) => ({ ...previous, [key]: e.target.value }))} /></label>)}
            <label className="detail-field">数值依据<input className="input" value={sourceNote} onChange={(e) => setSourceNote(e.target.value)} placeholder="填写目标或实绩等数值时必填" /></label>
            <label className="detail-field">计划开始<input type="date" className="input" min={targetPeriod?.from} max={targetPeriod?.to} value={plannedStart} onChange={(e) => setPlannedStart(e.target.value)} /></label>
            <label className="detail-field">截止日期<input type="date" className="input" min={targetPeriod?.from} max={targetPeriod?.to} value={dueDate} onChange={(e) => setDueDate(e.target.value)} /></label>
          </div>
          <details><summary>更多信息：预定、实绩与资源</summary><div className="detail-grid" style={{ marginTop: 12 }}>
            {optionalFields.filter(([key]) => !['owner', 'target_value', 'unit'].includes(key)).map(([key, label]) => <label className="detail-field" key={key}>{label}<input className="input" value={values[key]} onChange={(e) => setValues((previous) => ({ ...previous, [key]: e.target.value }))} /></label>)}
          </div></details>
          <p className="panel-note">{targetPeriod ? `任务期间：${targetPeriod.from} 至 ${targetPeriod.to}。` : '下期计划按当前分析保存的企业周期顺延。'} 行动日期请填写在任务期间内。</p>
          <div className="step-heading"><h4>行动项</h4><button type="button" className="outline" onClick={() => { setSteps((previous) => [...previous, blankStep()]); setDirty(true); }}>新增行动项</button></div>
          {steps.map((step, index) => <div className="task-create-step" key={index}>
            <label className="detail-field">行动内容 {index + 1}<textarea className="input task-action-textarea" rows={3} value={step.action} onChange={(e) => editStep(index, 'action', e.target.value)} /></label>
            <div className="task-step-fields">{([['owner', '负责人', 'text'], ['start_date', '开始日期', 'date'], ['due_date', '截止日期', 'date'], ['done_criteria', '验收标准', 'text']] as const).map(([key, label, type]) => <label key={key}>{label}<input className="input" type={type} min={type === 'date' ? targetPeriod?.from : undefined} max={type === 'date' ? targetPeriod?.to : undefined} value={step[key] ?? ''} aria-label={`新行动 ${index + 1} ${label}`} onChange={(e) => editStep(index, key, e.target.value)} /></label>)}</div>
            <button type="button" className="outline" disabled={steps.length <= 1} onClick={() => { setSteps((previous) => previous.filter((_, i) => i !== index)); setDirty(true); }}>删除行动项 {index + 1}</button>
          </div>)}
        </fieldset>
        {error ? <p className="notice" role="alert">{error}</p> : null}
      </div>
      <div className="modal-actions"><button type="button" className="outline" disabled={busy} onClick={close}>取消</button><button type="button" className="primary" disabled={busy || !context.detail.analysis_input_fingerprint} onClick={() => void save()}>{busy ? '正在保存…' : '创建任务'}</button></div>
    </section>
  </div>;
}
