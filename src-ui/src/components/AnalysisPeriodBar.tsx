import { useApp } from '../state/store';
import { currentPeriodPolicy, periodLabel, savedPeriods } from '../state/periods';
export { PERIOD_GRAIN_LABELS } from '../state/periods';

/** 页眉仅切换已有分析结果；新分析在经营分析页显式运行。 */
export function AnalysisPeriodBar() {
  const app = useApp();
  const detail = app.data.currentDetail;
  const disabled = app.loading || app.contextLockReason != null;
  const policy = currentPeriodPolicy(app.data.policies, app.activeWorkspace?.current_period_policy_id)?.policy;
  const periods = savedPeriods(app.data.history, policy?.cadence, policy?.month_start_day ?? 1);
  const entry = periods.find((h) => h.run_id === app.selectedRun?.runId);
  const exactRange = detail ? `${detail.period.from} 至 ${detail.period.to}` : '';
  return (
    <div className="header-period" aria-label="查看期间" data-testid="analysis-period-bar"
      title={app.contextLockReason ?? exactRange}>
      <label htmlFor="saved-analysis-period">期间</label>
      <select id="saved-analysis-period" className="select" aria-label="已保存分析期间"
        data-testid="saved-period-select" value={entry?.run_id ?? ''}
        disabled={disabled || periods.length === 0}
        onChange={(e) => void app.selectRun(e.target.value)}>
        {!entry ? <option value="">{app.loading ? '正在读取…' : '暂无已保存数据'}</option> : null}
        {periods.map((h) => <option key={h.run_id} value={h.run_id} title={`${h.period.from} 至 ${h.period.to}`}>
          {periodLabel(h.period, h.cadence, policy?.month_start_day ?? 1)}
        </option>)}
      </select>
      <span className="sr-only" data-testid="current-period-label">{exactRange}</span>
      <button type="button" className="period-refresh" aria-label="刷新所选期间" title="刷新所选期间"
        disabled={disabled || !app.activeWorkspaceId} onClick={() => void app.refresh()}>↻</button>
    </div>
  );
}
