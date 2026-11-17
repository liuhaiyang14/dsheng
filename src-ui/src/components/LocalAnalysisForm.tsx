import { useEffect, useMemo, useRef, useState } from 'react';
import type { ScanSummaryView } from '../api';
import { useApp } from '../state/store';
import { currentPeriodPolicy, periodIdentity, periodLabel } from '../state/periods';

type AvailablePeriod = NonNullable<ScanSummaryView['available_periods']>[number];
const periodKey = (period: AvailablePeriod) => `${period.grain}:${period.from}:${period.to}`;

function OverwriteConfirmation({ period, monthStartDay, onCancel, onConfirm, confirmDisabled }: {
  period: AvailablePeriod; monthStartDay: number; onCancel: () => void; onConfirm: () => void; confirmDisabled: boolean;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);
  return <div className="modal-scrim" data-testid="analysis-overwrite-confirmation">
    <div className="modal" ref={dialog} role="dialog" aria-modal="true" aria-label="确认重新分析" onKeyDown={(event) => {
      if (event.key === 'Escape') { event.preventDefault(); onCancel(); }
      if (event.key !== 'Tab') return;
      const buttons = dialog.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
      if (!buttons?.length) return;
      const first = buttons[0], last = buttons[buttons.length - 1];
      if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last.focus(); }
      else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
    }}>
      <div className="modal-head"><h3>重新分析 {periodLabel(period, period.grain, monthStartDay)}</h3></div>
      <div className="modal-body">
        <p>分析范围：{period.from} 至 {period.to}</p>
        <p>将以本次数据生成新的修订版，旧版本及其结果会完整保留。</p>
      </div>
      <div className="modal-actions">
        <button ref={cancel} type="button" className="outline" onClick={onCancel}>取消</button>
        <button type="button" className="primary" disabled={confirmDisabled} onClick={onConfirm}>生成新修订版</button>
      </div>
    </div>
  </div>;
}

export function LocalAnalysisForm() {
  const app = useApp();
  const detail = app.data.currentDetail;
  const scan = app.data.scan;
  const policy = currentPeriodPolicy(app.data.policies, app.activeWorkspace?.current_period_policy_id);
  const cadence = policy?.policy.cadence;
  const monthStartDay = policy?.policy.month_start_day ?? 1;
  const scanMatchesPolicy = scan?.workspace_id === app.activeWorkspaceId
    && !!policy && scan.available_period_policy_id === policy.period_policy_id;
  const mappingPending = (scan?.source_profile?.mode === 'generic' || scan?.source_profile?.mode === 'profile')
    && (scan.files ?? []).some((file) => file.mapping_status === 'pending_review'
      && !['profile_mapping', 'reference_only'].includes(file.review?.confirmation?.target_role ?? ''));
  const periods = useMemo(() => {
    const found = new Map<string, AvailablePeriod>();
    if (scanMatchesPolicy && scan) {
      for (const period of scan.available_periods ?? []) {
        if (period.grain === cadence) found.set(periodKey(period), period);
      }
    }
    return [...found.values()].sort((a, b) => b.to.localeCompare(a.to) || b.from.localeCompare(a.from));
  }, [scan, scanMatchesPolicy, cadence]);
  const [selection, setSelection] = useState('');
  const [scanning, setScanning] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const [creatorOpen, setCreatorOpen] = useState(app.periodCreationRequest > 0);
  useEffect(() => { if (app.periodCreationRequest) setCreatorOpen(true); }, [app.periodCreationRequest]);
  const [confirmation, setConfirmation] = useState<{ period: AvailablePeriod; workspaceId: string; policyId: string; directory: string; scan: ScanSummaryView } | null>(null);
  const mounted = useRef(true);
  const context = useRef({ workspaceId: app.activeWorkspaceId, policyId: policy?.period_policy_id });
  context.current = { workspaceId: app.activeWorkspaceId, policyId: policy?.period_policy_id };
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const disabled = app.loading || app.contextLockReason != null || scanning;
  const selected = periods.find((period) => periodKey(period) === selection);
  useEffect(() => {
    const matching = detail?.grain === cadence ? periods.find((period) => periodIdentity(period, period.grain, monthStartDay) === periodIdentity(detail.period, detail.grain, monthStartDay)) : undefined;
    setSelection(periods.length ? periodKey(matching ?? periods[0]) : '');
  }, [periods, detail?.run_id, detail?.period.from, detail?.period.to, detail?.grain, cadence, monthStartDay]);
  useEffect(() => { setConfirmation(null); }, [app.activeWorkspaceId, policy?.period_policy_id, selection, scan, app.selectedRun?.runId]);

  const rescan = async () => {
    if (disabled || !app.client || !app.activeWorkspaceId || !app.sourceDirectory) return;
    const workspaceId = app.activeWorkspaceId;
    const policyId = policy?.period_policy_id;
    setScanning(true);
    setScanError(null);
    app.setContextLock('正在重新扫描数据，请完成后切换企业或页面。');
    try {
      const selectedProfile = scan?.source_profile;
      const profile = selectedProfile?.mode === 'generic' ? 'generic'
        : selectedProfile?.profile_ref ?? selectedProfile?.profile_id ?? app.sourceInputProfile;
      if (!profile.trim()) throw new Error('请先到数据源选择输入模板并填写自定义模板路径。');
      const summary = await app.client.scan(
        workspaceId,
        app.sourceDirectory,
        profile === 'generic' ? undefined : profile,
        profile === 'generic',
      );
      if (!mounted.current || context.current.workspaceId !== workspaceId || context.current.policyId !== policyId) return;
      app.rememberSourceProfile(profile);
      await app.reloadWorkspaceData({ keepScan: summary });
    } catch (error) {
      if (mounted.current && context.current.workspaceId === workspaceId && context.current.policyId === policyId) {
        setScanError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (mounted.current) setScanning(false);
      app.setContextLock(null);
    }
  };
  const submit = () => {
    if (disabled || mappingPending || !selected || !app.activeWorkspaceId || !policy || !app.sourceDirectory || !scan) return;
    const exists = app.data.history.some((entry) => entry.cadence === selected.grain
      && periodIdentity(entry.period, entry.cadence, monthStartDay) === periodIdentity(selected, selected.grain, monthStartDay));
    if (exists) {
      setConfirmation({ period: selected, workspaceId: app.activeWorkspaceId, policyId: policy.period_policy_id, directory: app.sourceDirectory, scan });
    } else void app.runLocalAnalysis(selected);
  };
  const confirm = () => {
    if (!confirmation || disabled || confirmation.workspaceId !== app.activeWorkspaceId
      || confirmation.policyId !== policy?.period_policy_id || confirmation.directory !== app.sourceDirectory || confirmation.scan !== scan
      || !selected || periodKey(confirmation.period) !== periodKey(selected)) return;
    setConfirmation(null);
    void app.runLocalAnalysis(confirmation.period);
  };
  const scanNeeded = !scanMatchesPolicy || scan?.available_periods === undefined
    || (scan.available_periods.length > 0 && !periods.length);
  const genericProfile = scan?.source_profile?.mode === 'generic';
  return <>
    <details className="local-analysis panel period-generation-panel" open={!detail || creatorOpen ? true : undefined}>
      <summary>{detail && !creatorOpen ? '重新分析数据' : '生成并保存分析期间'}</summary>
      <form className="period-controls" onSubmit={(event) => { event.preventDefault(); submit(); }}>
        <label className="period-field">分析期间
          <select className="select" aria-label="分析期间" data-testid="local-analysis-period-select" required value={selection}
            disabled={disabled || !periods.length} onChange={(event) => setSelection(event.target.value)}>
            {!selected ? <option value="">{scanning ? '正在扫描…' : '暂无可分析期间'}</option> : null}
            {periods.map((period) => <option key={periodKey(period)} value={periodKey(period)}>{periodLabel(period, period.grain, monthStartDay)}</option>)}
          </select>
        </label>
        <button type="submit" className="primary" disabled={disabled || mappingPending || !selected || !app.activeWorkspaceId || !app.sourceDirectory || !cadence}>{detail && !creatorOpen ? '运行本地分析' : '生成并保存期间'}</button>
        {app.sourceDirectory ? <button type="button" className="outline" disabled={disabled} onClick={() => void rescan()}>{scanning ? '正在扫描…' : '重新扫描数据'}</button> : null}
      </form>
      <p className="period-help">选择月份后，点击“生成并保存期间”将在本机计算并保存两表；保存成功后该月份会出现在页眉。已有期间可运行本地分析生成修订版。</p>
      {!app.sourceDirectory ? <p className="period-help">请先导入数据。<button type="button" className="outline" onClick={() => app.setScreen('source')}>前往数据源</button></p>
        : <>
          {scanNeeded ? <p className="period-help">当前扫描结果还没有按本企业的核算周期生成可分析期间，请点击“重新扫描数据”。</p> : null}
          {!periods.length ? <div className="period-empty-guidance" data-testid="analysis-period-empty-guidance">
            <p className="period-help">{genericProfile
              ? '当前选择的是“通用数据模板”。它只能识别已经整理成“期间、指标、实际值”的表格；销售出库、采购入库、工资条等明细单据需要在“数据源”选择对应的输入模板，并确认关键日期列。'
              : '当前还没有可运行的期间。请先在“数据源”确认文件用途和关键日期列，再重新扫描数据。'}</p>
            <p className="period-help">期间不会在导入时自动保存。完成“选择输入模板 → 确认文件用途和关键日期 → 重新扫描 → 选择月份 → 运行本地分析”后，结果才会保存到页眉的期间列表。</p>
            <button type="button" className="outline period-source-action" onClick={() => app.setScreen('source')}>去数据源选择输入模板</button>
          </div> : null}
          {mappingPending ? <p className="period-help">部分原始文件的业务用途尚未确认，请先到「数据源」逐项确认文件用途；不确定的文件可保留“待补”。<button type="button" className="outline period-source-action" disabled={disabled} onClick={() => app.setScreen('source')}>去确认文件用途和关键列</button></p> : null}
          <p className="period-help">数据目录：{app.sourceDirectory}</p>
        </>}
      {scanError ? <p className="notice error" role="alert">{scanError}</p> : null}
    </details>
    {confirmation ? <OverwriteConfirmation period={confirmation.period} monthStartDay={monthStartDay} confirmDisabled={disabled} onCancel={() => setConfirmation(null)} onConfirm={confirm} /> : null}
  </>;
}
