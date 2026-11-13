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
        <p>将覆盖该期间已有分析记录，以本次数据为准。</p>
      </div>
      <div className="modal-actions">
        <button ref={cancel} type="button" className="outline" onClick={onCancel}>取消</button>
        <button type="button" className="primary" disabled={confirmDisabled} onClick={onConfirm}>覆盖并重新分析</button>
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
  const mappingPending = scan?.source_profile?.mode === 'generic'
    && (scan.files ?? []).some((file) => file.mapping_status === 'pending_review');
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
    try {
      const selectedProfile = scan?.source_profile;
      const summary = await app.client.scan(
        workspaceId,
        app.sourceDirectory,
        selectedProfile && selectedProfile.mode !== 'generic'
          ? (selectedProfile.profile_ref ?? selectedProfile.profile_id)
          : undefined,
        !selectedProfile || selectedProfile.mode === 'generic',
      );
      if (!mounted.current || context.current.workspaceId !== workspaceId || context.current.policyId !== policyId) return;
      await app.reloadWorkspaceData({ keepScan: summary });
    } catch (error) {
      if (mounted.current && context.current.workspaceId === workspaceId && context.current.policyId === policyId) {
        setScanError(error instanceof Error ? error.message : String(error));
      }
    } finally {
      if (mounted.current) setScanning(false);
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
  return <>
    <details className="local-analysis" open={detail ? undefined : true}>
      <summary>{detail ? '重新分析数据' : '分析导入的数据'}</summary>
      <form className="period-controls" onSubmit={(event) => { event.preventDefault(); submit(); }}>
        <label className="period-field">分析期间
          <select className="select" aria-label="分析期间" data-testid="local-analysis-period-select" required value={selection}
            disabled={disabled || !periods.length} onChange={(event) => setSelection(event.target.value)}>
            {!selected ? <option value="">{scanning ? '正在扫描…' : '暂无可分析期间'}</option> : null}
            {periods.map((period) => <option key={periodKey(period)} value={periodKey(period)}>{periodLabel(period, period.grain, monthStartDay)}</option>)}
          </select>
        </label>
        <button type="submit" className="primary" disabled={disabled || mappingPending || !selected || !app.activeWorkspaceId || !app.sourceDirectory || !cadence}>运行本地分析</button>
        {app.sourceDirectory ? <button type="button" className="outline" disabled={disabled} onClick={() => void rescan()}>{scanning ? '正在扫描…' : '重新扫描数据'}</button> : null}
      </form>
      <p className="period-help">仅列出导入数据中可分析的期间，按系统设置的核算周期计算。</p>
      {!app.sourceDirectory ? <p className="period-help">请先导入数据。<button type="button" className="outline" onClick={() => app.setScreen('source')}>前往数据源</button></p>
        : <>
          {scanNeeded ? <p className="period-help">请重新扫描数据，读取当前核算周期的可分析期间。</p>
            : !periods.length ? <p className="period-help">未识别到可分析的数据，请检查数据源后重新扫描。</p> : null}
          {mappingPending ? <p className="period-help">通用 source 的字段映射尚未确认，请先到「数据源」点击“确认通用字段映射”。</p> : null}
          <p className="period-help">数据目录：{app.sourceDirectory}</p>
        </>}
      {scanError ? <p className="notice error" role="alert">{scanError}</p> : null}
    </details>
    {confirmation ? <OverwriteConfirmation period={confirmation.period} monthStartDay={monthStartDay} confirmDisabled={disabled} onCancel={() => setConfirmation(null)} onConfirm={confirm} /> : null}
  </>;
}
