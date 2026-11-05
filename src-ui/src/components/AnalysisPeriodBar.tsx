import { useEffect, useState } from 'react';
import { useApp } from '../state/store';
import type { Cadence } from '../api';

export const PERIOD_GRAIN_LABELS: Record<Cadence, string> = {
  day: '日度', week: '周度', month: '月度', quarter: '季度',
};

/** 全页面共用的已保存期间选择；新日期范围只在显式本地分析后生效。 */
export function AnalysisPeriodBar() {
  const app = useApp();
  const detail = app.data.currentDetail;
  const disabled = app.loading || app.contextLockReason != null;
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [grain, setGrain] = useState<Cadence>('month');
  useEffect(() => {
    setFrom(detail?.period.from ?? app.data.scan?.coverage.from ?? '');
    setTo(detail?.period.to ?? app.data.scan?.coverage.to ?? '');
    setGrain(detail?.grain ?? app.data.scan?.coverage.granularity ?? 'month');
  }, [detail?.run_id, detail?.analysis_revision_no, detail?.period.from, detail?.period.to,
    detail?.grain, app.data.scan?.coverage.from, app.data.scan?.coverage.to, app.data.scan?.coverage.granularity]);

  const entry = app.data.history.find((h) => h.run_id === detail?.run_id);
  const historical = detail != null && entry != null && detail.analysis_revision_no !== entry.current_revision_no;
  const scopeHelp = app.screen === 'source'
    ? '数据源展示当前企业的全部来源；下方日期只决定运行分析的范围。'
    : app.screen === 'provider' || app.screen === 'system-settings'
      ? '此页配置属于当前企业，不随查看期间过滤。'
      : app.screen === 'task'
        ? '任务单展示与所选期间关联的任务及其最新进度；历史查询保留当时的任务快照。'
        : '经营分析、诊断报告和任务单共用所选期间；切换已保存期间只读取本地数据。';
  return (
    <section className="analysis-period-bar" aria-label="全局分析区间" data-testid="analysis-period-bar">
      <div className="period-current" data-testid="current-period-label">
        {detail
          ? <>当前查看：{detail.period.from} 至 {detail.period.to} · {PERIOD_GRAIN_LABELS[detail.grain]} · 分析版本 {detail.analysis_revision_no}{historical ? '（历史快照）' : '（该期间当前版本）'}</>
          : app.loading ? '正在读取所选期间…' : '尚未选择已保存的分析期间'}
      </div>
      <div className="period-controls">
        <label className="period-field">已保存期间
          <select className="select" aria-label="已保存分析期间" data-testid="saved-period-select"
            value={app.selectedRun?.runId ?? ''} disabled={disabled || app.data.history.length === 0}
            onChange={(e) => void app.selectRun(e.target.value)}>
            {app.data.history.length === 0 ? <option value="">暂无已保存期间</option> : null}
            {[...app.data.history].sort((a, b) => b.period.to.localeCompare(a.period.to) || b.period.from.localeCompare(a.period.from)).map((h) =>
              <option key={h.run_id} value={h.run_id}>{h.period.from} 至 {h.period.to} · {PERIOD_GRAIN_LABELS[h.cadence]} · 当前版本 {h.current_revision_no}</option>)}
          </select>
        </label>
        <button type="button" className="outline" disabled={disabled || !app.activeWorkspaceId} onClick={() => void app.refresh()}>刷新所选期间</button>
      </div>
      {historical ? <p className="period-help">当前为历史快照；生成新诊断或任务前请切回该期间当前版本。
        <button type="button" className="outline" disabled={disabled} onClick={() => detail && void app.selectRun(detail.run_id)}>查看该期间当前版本</button>
      </p> : null}
      <form className="period-controls" onSubmit={(e) => { e.preventDefault(); void app.runLocalAnalysis({ from, to, grain }); }}>
        <label className="period-field">开始日期
          <input className="input" type="date" aria-label="分析开始日期" required value={from} disabled={disabled} onChange={(e) => setFrom(e.target.value)} />
        </label>
        <label className="period-field">结束日期
          <input className="input" type="date" aria-label="分析结束日期" required value={to} min={from || undefined} disabled={disabled} onChange={(e) => setTo(e.target.value)} />
        </label>
        <label className="period-field">聚合粒度
          <select className="select" aria-label="分析聚合粒度" data-testid="grain-select" value={grain} disabled={disabled} onChange={(e) => setGrain(e.target.value as Cadence)}>
            {(Object.keys(PERIOD_GRAIN_LABELS) as Cadence[]).map((g) => <option key={g} value={g}>{PERIOD_GRAIN_LABELS[g]}</option>)}
          </select>
        </label>
        <button type="submit" className="primary" disabled={disabled || !app.activeWorkspaceId || !app.sourceDirectory}>运行本地分析</button>
      </form>
      {app.contextLockReason ? <p className="period-help" role="status">{app.contextLockReason}</p> : null}
      <p className="period-help">{scopeHelp}</p>
      <p className="period-help">
        {app.sourceDirectory ? <>分析数据目录：<span title={app.sourceDirectory}>{app.sourceDirectory}</span>。修改日期或粒度后，点击“运行本地分析”才会更新结果。</>
          : <>尚未选择分析数据目录。<button type="button" className="outline" onClick={() => app.setScreen('source')}>前往数据源选择目录</button></>}
      </p>
    </section>
  );
}
