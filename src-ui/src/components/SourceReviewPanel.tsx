import { useState } from 'react';
import type { ScanFileInfo } from '../api';

export const SOURCE_METRIC_LABELS: Record<string, string> = {
  sales_net: '不含税销售额', order_cost: '订单成本', purchase_signed_amount: '采购订单金额',
  inbound_amount: '采购入库金额', receipts_amount: '收款金额', payments_amount: '付款金额',
  invoice_amount: '开票金额', operating_expense: '经营费用', labor_cost: '人工费',
  effective_hours: '有效工时',
};
// 通用表需要的是列角色，不能把明细模板的指标 key 当作通用列映射。
const GENERIC_FIELD_LABELS = { period: '分析期间', metric_label: '指标名称', actual: '实际值', planned: '计划 / 目标值', annual_month_plan: '年度月度计划值' };
type TaxDraft = { basis: '' | 'inclusive' | 'exclusive'; rate: string };
type Draft = { confirmed: boolean; field: string; column: string; sheet: string; taxes: Record<string, TaxDraft> };
function genericRole(role: string) {
  return ({ date: 'period', metric_id: 'metric_label', label: 'metric_label', plan: 'planned' } as Record<string, string>)[role] ?? role;
}
function genericOverrides(file: ScanFileInfo) {
  if (file.review?.column_overrides) return file.review.column_overrides.map((entry) => ({ ...entry, role: genericRole(entry.role) }));
  const saved = file.review?.confirmation;
  const role = genericRole(saved?.target_role ?? saved?.tool_field ?? '');
  const column = saved?.column ?? saved?.recognized ?? '';
  return role in GENERIC_FIELD_LABELS && /^[A-Za-z]{1,3}$/.test(column.trim()) ? [{ role, column, sheet: saved?.sheet }] : [];
}
function savedGenericFields(file: ScanFileInfo) {
  const saved = file.review?.confirmation;
  const role = genericRole(saved?.target_role ?? saved?.tool_field ?? '');
  const overrides = genericOverrides(file);
  const entry = overrides.find((item) => item.role === role && (item.sheet ?? '') === (saved?.sheet ?? '')) ?? overrides[0];
  return entry && entry.role in GENERIC_FIELD_LABELS && /^[A-Za-z]{1,3}$/.test(entry.column.trim())
    ? { field: entry.role, column: entry.column.trim().toUpperCase(), sheet: entry.sheet ?? '' } : { field: '', column: '', sheet: '' };
}
export type SourceConfirmation = {
  tool_field: string; source_label: string; recognized: string; usage: string;
  source_id: string; source: string; target_role: string; column?: string; sheet?: string;
  tax_treatments?: Record<string, { basis: 'inclusive' | 'exclusive'; rate_percent?: string }>;
};

export function SourceReviewPanel({ files, generic, busy, onSave, onClose }: {
  files: ScanFileInfo[]; generic: boolean; busy: boolean;
  onSave: (mappings: SourceConfirmation[]) => Promise<void>; onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, Draft>>(() => Object.fromEntries(files.map((file) => [file.file_id, {
    confirmed: file.mapping_status === 'mapped', ...savedGenericFields(file),
    taxes: Object.fromEntries((file.review?.mappings ?? []).map((mapping) => {
      const saved = file.review?.confirmation?.tax_treatments?.[mapping.metric_key];
      return [mapping.metric_key, { basis: saved?.basis ?? '', rate: saved?.rate_percent ?? '' }];
    })),
  }])));
  const update = (id: string, patch: Partial<Draft>) => setDrafts((prev) => ({ ...prev, [id]: { ...prev[id], ...patch } }));
  const save = async () => {
    setError(null);
    const entries: SourceConfirmation[] = [];
    for (const file of files) {
      const draft = drafts[file.file_id];
      if (!draft.confirmed) continue;
      const kind = file.review?.kind ?? (generic ? 'generic' : undefined);
      const manualGeneric = kind === 'generic' && generic;
      if (!kind) { setError('当前文件缺少用途和关键列识别结果，请关闭确认区并重新扫描。'); return; }
      // 已确认的自动识别不需要补一个虚假的列覆盖；省略本项使后端保留原确认。
      if (manualGeneric && file.mapping_status === 'mapped' && !draft.field && !draft.column.trim()) continue;
      const taxes: NonNullable<SourceConfirmation['tax_treatments']> = {};
      for (const mapping of file.review?.mappings ?? []) {
        const tax = draft.taxes[mapping.metric_key];
        if (!tax?.basis) continue;
        if (tax.basis === 'inclusive') {
          if (!tax.rate.trim()) continue; // 明确保留税口径待确认，不以零代替。
          if (!/^(?:[0-9]{1,2}(?:\.[0-9]+)?|100(?:\.0+)?)$/.test(tax.rate.trim())) {
            setError(`${file.file_label}的${SOURCE_METRIC_LABELS[mapping.metric_key] ?? '金额'}税率请输入 0 至 100 之间的数字；不确定时留空。`); return;
          }
          taxes[mapping.metric_key] = { basis: 'inclusive', rate_percent: tax.rate.trim() };
        } else taxes[mapping.metric_key] = { basis: 'exclusive' };
      }
      if (manualGeneric && (!draft.field || !draft.column.trim())) {
        setError('通用表请同时选择业务字段并填写来源列字母（例如 A、B、C）；不确定的文件可保留待确认。'); return;
      }
      if (manualGeneric && !/^[A-Za-z]{1,3}$/.test(draft.column.trim())) {
        setError('来源列请填写 Excel 列字母（例如 A、B、C），不能填写中文表头。'); return;
      }
      const role = kind === 'auxiliary' ? 'reference_only' : manualGeneric ? draft.field : 'profile_mapping';
      entries.push({ tool_field: role, source_label: file.file_label, source_id: file.file_label, source: file.file_label,
        target_role: role, recognized: manualGeneric ? draft.column.trim().toUpperCase() : '已确认',
        usage: kind === 'auxiliary' ? '辅助资料，仅保留供查阅，不参与核算' : '已核对模板定义的文件用途和关键列',
        ...(manualGeneric ? { column: draft.column.trim().toUpperCase(), ...(draft.sheet ? { sheet: draft.sheet } : {}) } : {}),
        ...(kind === 'records' ? { tax_treatments: taxes } : {}),
      });
    }
    if (!entries.length) {
      if (files.some((file) => file.mapping_status === 'mapped')) { onClose(); return; }
      setError('请至少确认一个文件；其余文件可以保留待补。'); return;
    }
    await onSave(entries);
  };
  return <section className="panel source-review-editor" data-testid="field-review-panel" aria-label="确认文件用途和关键列">
    <div className="panel-head"><div><strong>确认文件用途和关键列</strong><p className="panel-note">核对这些文件将用于什么计算。金额与日期列由所选输入模板定义；如与实际业务不符，请换用合适的输入模板。</p></div>
      <span className="panel-note">已确认 {files.filter((file) => drafts[file.file_id].confirmed).length} / {files.length}</span></div>
    <div className="source-review-files">{files.map((file) => {
      const draft = drafts[file.file_id];
      const kind = file.review?.kind ?? (generic ? 'generic' : undefined);
      const overrides = genericOverrides(file);
      const sheets = [...new Set(overrides.map((entry) => entry.sheet).filter((sheet): sheet is string => !!sheet))];
      return <article className="source-review-file" key={file.file_id}>
        <div className="source-review-title"><div><h3>{file.file_label}</h3><span className="panel-note">{file.row_count} 行 · {file.sheet_count} 个工作表</span></div>
          <label className="period-field">确认状态<select className="select" aria-label={`确认状态：${file.file_label}`} value={draft.confirmed ? 'confirmed' : 'pending'} disabled={busy || !kind}
            onChange={(event) => update(file.file_id, { confirmed: event.target.value === 'confirmed' })}>
            <option value="pending" disabled={file.mapping_status === 'mapped'}>待补/待确认</option><option value="confirmed">已确认文件用途和关键列</option>
          </select></label></div>
        {file.mapping_status === 'mapped' ? <p className="period-help">文件用途已保存，可在下面重新核对税口径；如果用途或列结构不同，请更换输入模板后重新扫描。</p> : null}
        {kind === 'auxiliary' ? <p className="source-review-reference"><strong>辅助资料，仅保留供查阅，不参与核算</strong><br />此文件用于核对订单、发票或账户信息，不会重复计入销售、成本或收付款，也不会生成分析月份。</p> : null}
        {kind === 'records' ? <div className="source-review-mappings">{file.review!.mappings.map((mapping) => {
          const metric = SOURCE_METRIC_LABELS[mapping.metric_key] ?? '经营指标';
          const tax = draft.taxes[mapping.metric_key] ?? { basis: '', rate: '' };
          const taxRequired = mapping.source_tax_basis === 'inclusive' || mapping.source_tax_basis === 'confirm_at_runtime';
          return <section className="source-review-mapping" key={mapping.metric_key}>
            <h4>用于{metric}</h4>
            <dl className="source-review-columns"><div><dt>金额 / 数值列</dt><dd>{mapping.value_fields.join('、') || '待补：未识别数值列'}</dd></div><div><dt>归属日期列</dt><dd>{mapping.date_fields.join('、') || '待补：未识别日期列'}</dd></div>
              {mapping.status_field ? <div><dt>参与计算的单据</dt><dd>{mapping.status_field}：{mapping.accepted_statuses.join('、') || '全部状态'}</dd></div> : null}</dl>
            {mapping.missing_fields.length ? <p className="source-review-warning">缺少关键列：{mapping.missing_fields.join('、')}。请补齐原始文件后重新扫描，此项保持待补。</p> : null}
            {mapping.requires_confirmation ? <p className="period-help">费用分类或业务归属仍需按实际情况补充确认；确认文件用途不会自动完成分类。</p> : null}
            {taxRequired ? <div className="source-tax-confirmation">
              <p>{mapping.source_tax_basis === 'inclusive' ? '来源金额含税，需要明确税率后才能计算不含税金额。' : '来源字段未说明是否含税，请确认这一项金额的税口径。'}</p>
              <div className="period-controls"><label className="period-field">金额税口径<select className="select" aria-label={`税口径：${file.file_label}：${metric}`} value={tax.basis} disabled={busy || !draft.confirmed}
                onChange={(event) => update(file.file_id, { taxes: { ...draft.taxes, [mapping.metric_key]: { ...tax, basis: event.target.value as TaxDraft['basis'] } } })}>
                <option value="">待确认</option>{mapping.source_tax_basis === 'confirm_at_runtime' ? <option value="exclusive">金额已不含税</option> : null}<option value="inclusive">金额含税，按明确税率换算</option>
              </select></label>
              {tax.basis === 'inclusive' ? <label className="period-field">统一税率（%）<input className="select" inputMode="decimal" aria-label={`税率：${file.file_label}：${metric}`} value={tax.rate} placeholder="填写已核实的税率" disabled={busy || !draft.confirmed}
                onChange={(event) => update(file.file_id, { taxes: { ...draft.taxes, [mapping.metric_key]: { ...tax, rate: event.target.value } } })} /></label> : null}</div>
              <p className="period-help">同一用途含多种税率或无法核实，请保留“待确认”。税率留空不会按 0% 或 13% 计算；可以先保存文件确认，相关金额保持待补。</p>
            </div> : <p className="period-help">{mapping.source_tax_basis === 'exclusive' ? '模板已明确此列为不含税金额。' : '此项按原始数值统计，不执行税额换算。'}</p>}
          </section>;
        })}</div> : null}
        {kind === 'generic' && !generic ? <p className="source-review-reference"><strong>已按模板识别标准经营数据表</strong><br />期间、指标名称、实际值等列由模板自动识别；请确认文件用途，无需手工填写列。保存后会保留已识别的全部列用于计算。</p> : null}
        {kind === 'generic' && generic && file.mapping_status === 'mapped' ? <p className="period-help">当前字段识别已保存，无需重复填写。若要修改某一列，请同时选择业务字段和来源列字母；留空会保留原有识别结果。</p> : null}
        {kind === 'generic' && generic ? <>
          {overrides.length ? <div className="source-review-mapping" aria-label={`已保存的列修正：${file.file_label}`}><h4>已保存的列修正</h4><dl className="source-review-columns">{overrides.map((entry, index) => <div key={`${entry.role}:${entry.sheet ?? ''}:${index}`}><dt>{GENERIC_FIELD_LABELS[entry.role as keyof typeof GENERIC_FIELD_LABELS] ?? '业务字段'}</dt><dd>{entry.column} 列 · {entry.sheet ? `工作表：${entry.sheet}` : '全部工作表'}</dd></div>)}</dl></div> : null}
          <p className="period-help">本次仅修改所选字段的列修正；其他字段及工作表的修正会保留。</p>
          <div className="period-controls"><label className="period-field">业务字段<select className="select" aria-label={`确认字段：${file.file_label}`} value={draft.field} disabled={busy || !draft.confirmed} onChange={(e) => {
            const field = e.target.value;
            const previous = overrides.find((entry) => entry.role === field && (entry.sheet ?? '') === draft.sheet) ?? overrides.find((entry) => entry.role === field);
            update(file.file_id, { field, column: previous?.column ?? '', sheet: previous ? previous.sheet ?? '' : draft.sheet });
          }}><option value="">请选择字段</option>{Object.entries(GENERIC_FIELD_LABELS).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</select></label>
          {sheets.length ? <label className="period-field">应用工作表<select className="select" aria-label={`修正工作表：${file.file_label}`} value={draft.sheet} disabled={busy || !draft.confirmed} onChange={(e) => {
            const sheet = e.target.value;
            update(file.file_id, { sheet, column: overrides.find((entry) => entry.role === draft.field && (entry.sheet ?? '') === sheet)?.column ?? '' });
          }}><option value="">全部工作表</option>{sheets.map((sheet) => <option key={sheet} value={sheet}>{sheet}</option>)}</select></label> : null}
          <label className="period-field">来源列字母（例如 A、B、C）<input className="select" aria-label={`确认列：${file.file_label}`} value={draft.column} disabled={busy || !draft.confirmed} onChange={(e) => update(file.file_id, { column: e.target.value })} /></label></div></> : null}
        {!kind ? <p className="source-review-warning">缺少关键列识别结果，请重新扫描后再确认。</p> : null}
      </article>;
    })}</div>
    {error ? <p className="notice error" role="alert">{error}</p> : null}
    <div className="field-review-actions"><button type="button" className="outline" disabled={busy} onClick={onClose}>稍后处理</button><button type="button" className="primary" disabled={busy} data-testid="save-field-review-button" onClick={() => void save()}>{busy ? '保存并重扫中…' : '保存确认并重新扫描'}</button></div>
  </section>;
}
