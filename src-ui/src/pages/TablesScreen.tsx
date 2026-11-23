/**
 * 生成两表屏：两表只来自当前 analysis revision 的本地计算结果。
 * 页签切换核算表/推移表，下载调用核心 export_tables；这里不读取 source 文件，
 * 也不发起 Provider 请求。
 */
import { useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { diffTone, favorableOf, formatMetricDifference, formatMetricValue, formatRatioPercent } from '../api';
import type { MetricSeriesView } from '../api';
import { TargetValue } from '../components/TargetValue';

type TableTab = 'accounting' | 'trend';

function valueCount(values: Array<string | null>): number {
  return values.filter((value) => value != null && value !== '').length;
}

function seriesText(
  series: MetricSeriesView,
  values: Array<string | null>,
  label: string,
): string {
  const entries = values
    .map((value, index) => value == null || value === '' ? null : `${series.buckets[index] ?? '—'}：${formatMetricValue(value, series.unit, series.unit_signature)}`)
    .filter((entry): entry is string => entry != null);
  if (!entries.length) return `待补（未识别${label}值）`;
  const missing = Math.max(0, series.buckets.length - entries.length);
  return `${entries.join('；')}${missing ? `；${missing} 个期间待补` : ''}`;
}

export function TablesScreen() {
  const app = useApp();
  const detail = app.data.currentDetail;
  const hasData = workspaceHasData(app.data);
  const [tab, setTab] = useState<TableTab>('accounting');
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [regenerating, setRegenerating] = useState(false);
  const [regenerateOpen, setRegenerateOpen] = useState(false);

  const choosePath = async (title: string, defaultPath: string): Promise<string | null> => {
    const w = window as unknown as {
      __TAURI__?: { dialog?: { save?: (o: { title?: string; defaultPath?: string }) => Promise<string | null> } };
    };
    const save = w.__TAURI__?.dialog?.save;
    if (typeof save === 'function') {
      return save({ title, defaultPath });
    }
    return window.prompt(`保存${title}的路径（演示/测试通道）`, `/tmp/dsheng-${defaultPath}`);
  };

  const download = async (kind: 'accounting' | 'trend' | 'bundle') => {
    if (!app.client || !app.activeWorkspaceId || !detail) return;
    if (detail.two_tables_ready !== true) {
      setNotice('当前分析版本尚未生成完整两表，暂不可下载。');
      return;
    }
    const suffix = kind === 'bundle' ? 'zip' : 'xlsx';
    const name = kind === 'bundle' ? '两表包' : kind === 'accounting' ? '核算表' : '推移表';
    const outPath = await choosePath(`保存${name}`, `${name}_revision${detail.analysis_revision_no}.${suffix}`);
    if (!outPath) return;
    setBusy(true);
    setNotice(null);
    try {
      const result = await app.client.exportTables({
        workspaceId: app.activeWorkspaceId,
        runId: detail.run_id,
        revision: detail.analysis_revision_no,
        kind,
        outPath,
      });
      setNotice(`已生成${name}：${result.out}`);
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const downloadOutputTemplate = async () => {
    if (!app.client) return;
    const outPath = await choosePath('固定 two_tables_v1 输出模板包', 'two_tables_v1-output-template.zip');
    if (!outPath) return;
    setBusy(true);
    setNotice(null);
    try {
      const result = await app.client.templateDownload(outPath, 'output', undefined, 'two_tables_v1');
      setNotice(`已生成输出模板：${result.out}（固定 two_tables_v1）`);
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const regenerateTables = async () => {
    if (!detail || !app.client || !app.activeWorkspaceId || busy) return;
    setRegenerateOpen(false);
    setRegenerating(true);
    setBusy(true);
    setNotice(null);
    try {
      const saved = await app.runLocalAnalysis({
        from: detail.period.from,
        to: detail.period.to,
        grain: detail.grain,
      });
      setNotice(saved
        ? '已按当前数据重新生成两表，并保存为新的 revision；旧 revision 仍保留在历史记录中。'
        : '本次未生成新 revision，请根据页面错误提示修正数据源后重试。');
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setRegenerating(false);
      setBusy(false);
    }
  };

  if (!hasData || !detail) {
    return (
      <div>
        <div className="screen-head">
          <div>
            <div className="eyebrow">03 · 生成结果</div>
            <h2 className="screen-title">两表展示 / 下载</h2>
            <div className="sub">完成原始数据分析后，在这里查看和下载固定 two_tables_v1 输出。</div>
          </div>
          <div className="head-controls">
            <button type="button" className="outline" onClick={() => void downloadOutputTemplate()} disabled={busy || !app.client}>
              下载输出模板
            </button>
          </div>
        </div>
        <EmptyState title="尚未生成两表" hint="请先在数据源页面完成扫描、映射和本地分析。" />
      </div>
    );
  }

  const ready = detail.two_tables_ready === true;
  const activeSeries = [...detail.series].sort((a, b) => {
    const valueDifference = (valueCount(b.actual) + valueCount(b.plan)) - (valueCount(a.actual) + valueCount(a.plan));
    return valueDifference || a.metric_label.localeCompare(b.metric_label, 'zh-CN');
  });
  const scan = app.data.scan;
  const sourceChangedSinceAnalysis = Boolean(
    scan?.scanned_at && detail.captured_at && scan.scanned_at > detail.captured_at,
  );

  return (
    <div data-testid="tables-screen" aria-busy={regenerating}>
      <div className="screen-head">
        <div>
          <div className="eyebrow">03 · 生成结果</div>
          <h2 className="screen-title">两表展示 / 下载</h2>
          <div className="sub">
            {detail.period.from} 至 {detail.period.to} · revision {detail.analysis_revision_no} · 输出合同 two_tables_v1
          </div>
        </div>
        <div className="head-controls">
          <button type="button" className="outline" onClick={() => void downloadOutputTemplate()} disabled={busy || !app.client}>
            下载输出模板
          </button>
          <button type="button" className="outline" onClick={() => setRegenerateOpen(true)} disabled={busy || !app.client} aria-busy={regenerating}>
            {regenerating ? <><span className="generation-spinner inline-generation-spinner" aria-hidden="true" />正在生成两表…</> : '重新生成两表'}
          </button>
          <button type="button" className="outline" onClick={() => void download('bundle')} disabled={!ready || busy}>
            下载两表包
          </button>
        </div>
      </div>

      {notice ? <div className="notice workspace-empty" role="status"><span>{notice}</span></div> : null}
      {regenerating ? (
        <div className="status-banner" role="status" aria-live="polite" aria-busy="true" data-testid="tables-regeneration-progress">
          <span className="generation-spinner" aria-hidden="true" />
          <div>
            <strong>正在重新生成两表</strong>
            <br />
            <span>正在读取当前数据、本地计算并保存新 revision，请稍候…</span>
          </div>
        </div>
      ) : null}
      {sourceChangedSinceAnalysis ? (
        <div className="notice warning" role="status" data-testid="tables-stale-source-notice">
          <strong>原始数据已重新扫描，当前两表仍是 revision {detail.analysis_revision_no}</strong>
          <span>
            受影响期间：{detail.period.from} 至 {detail.period.to}；本次扫描覆盖：{scan?.coverage.from} 至 {scan?.coverage.to}。
            {scan?.pending_inputs?.length ? ` 待补/待确认：${scan.pending_inputs.join('、')}。` : ''}
            {' '}请点击“重新生成两表”保存新的 revision；旧版本仍可在历史查询中查看。
          </span>
        </div>
      ) : null}
      <div className={`status-banner ${ready ? '' : 'warning'}`}>
        <span className="status-dot" />
        <div>
          <strong>{ready ? '核算表与推移表已生成' : '两表尚未完整生成'}</strong>
          <br />
          <span>{ready ? '以下内容由本地 Rust 核算结果生成；缺失值保留待补状态，不调用模型。' : '缺少任一表时不能下载，也不能生成任务单。'}</span>
        </div>
      </div>

      <div className="table-tabs" role="tablist" aria-label="两表类型">
        <button type="button" role="tab" aria-selected={tab === 'accounting'} aria-controls="accounting-table-panel" id="accounting-table-tab" className={`table-tab${tab === 'accounting' ? ' active' : ''}`} onClick={() => setTab('accounting')}>
          核算表
        </button>
        <button type="button" role="tab" aria-selected={tab === 'trend'} aria-controls="trend-table-panel" id="trend-table-tab" className={`table-tab${tab === 'trend' ? ' active' : ''}`} onClick={() => setTab('trend')}>
          推移表
        </button>
      </div>

      {tab === 'accounting' ? (
        <div className="panel" role="tabpanel" id="accounting-table-panel" aria-labelledby="accounting-table-tab" data-testid="accounting-table-panel">
          <div className="panel-head"><span className="panel-title">核算表 · 目标/参考差异</span><span className="tag green">生成结果</span></div>
          <div className="table-wrap">
            <table className="data-table two-tables-table">
              <thead><tr><th>指标</th><th>目标 / 参考值</th><th>实绩</th><th>差异</th><th>环比</th><th>同比</th></tr></thead>
              <tbody>
                {detail.metrics.map((metric) => (
                  <tr key={metric.metric_id}>
                    <td>{metric.metric_label}</td>
                    <td><TargetValue metric={metric} /></td>
                    <td>{formatMetricValue(metric.actual_value, metric.unit, metric.unit_signature)}</td>
                    <td className={diffTone(favorableOf(metric.direction, metric.delta))}>{formatMetricDifference(metric.delta, metric.unit, metric.unit_signature)}</td>
                    <td className={diffTone(favorableOf(metric.direction, metric.mom?.availability_status === 'available' && metric.mom.change_rate != null ? metric.mom.absolute_change ?? metric.mom.change_rate : null))}>{metric.mom?.availability_status !== 'available' || metric.mom.change_rate == null ? '不可比/待补' : formatRatioPercent(metric.mom.change_rate)}</td>
                    <td className={diffTone(favorableOf(metric.direction, metric.yoy?.availability_status === 'available' && metric.yoy.change_rate != null ? metric.yoy.absolute_change ?? metric.yoy.change_rate : null))}>{metric.yoy?.availability_status !== 'available' || metric.yoy.change_rate == null ? '不可比/待补' : formatRatioPercent(metric.yoy.change_rate)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="panel-footnote">金额保留 2 位、百分数保留 1 位；pp 表示百分点。参考值不代表已确认目标。</div>
          <div className="panel-footnote">附加值与经营收益按不同名称展示；人工费、目标、工时或期初缺失时保留待补，不按 0 计算。</div>
          <div className="panel-actions"><button type="button" className="primary" onClick={() => void download('accounting')} disabled={!ready || busy}>下载核算表</button></div>
        </div>
      ) : (
        <div className="panel" role="tabpanel" id="trend-table-panel" aria-labelledby="trend-table-tab" data-testid="trend-table-panel">
          <div className="panel-head"><span className="panel-title">推移表 · 期间序列</span><span className="tag blue">本地聚合</span></div>
          <div className="table-wrap">
            <table className="data-table two-tables-table">
              <thead><tr><th>指标</th><th>实绩/计划桶</th><th>实绩序列</th><th>计划序列</th></tr></thead>
              <tbody>
                {activeSeries.map((series) => (
                  <tr key={series.metric_key}>
                    <td>{series.metric_label}</td>
                    <td>{valueCount(series.actual)}/{valueCount(series.plan)}</td>
                    <td>{seriesText(series, series.actual, '实绩')}</td>
                    <td>{seriesText(series, series.plan, '计划')}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="panel-footnote">“实绩/计划桶”只统计已识别数值，不是日期桶数量；显示“待补”时，请到数据源确认文件用途和关键值列。金额按期间累计；比率与单位时间指标使用累计分子、分母重算。</div>
          <div className="panel-actions"><button type="button" className="primary" onClick={() => void download('trend')} disabled={!ready || busy}>下载推移表</button></div>
        </div>
      )}
      {regenerateOpen && detail ? (
        <div className="modal-scrim" data-testid="tables-regenerate-confirmation">
          <div className="modal" role="dialog" aria-modal="true" aria-labelledby="tables-regenerate-title">
            <div className="modal-head"><h3 id="tables-regenerate-title">重新生成当前期间的两表</h3></div>
            <div className="modal-body">
              <p>期间：{detail.period.from} 至 {detail.period.to}</p>
              <p>系统会读取当前已扫描的数据，在本机重新计算核算表和推移表，并保存为新的 revision。</p>
              <p className="panel-note">旧 revision 不会被覆盖，可在“历史查询”中继续查看。缺失字段仍会显示“待补”，不会按 0 计算。</p>
            </div>
            <div className="modal-actions">
              <button type="button" className="outline" onClick={() => setRegenerateOpen(false)}>取消</button>
              <button type="button" className="primary" onClick={() => void regenerateTables()} disabled={busy}>生成新 revision</button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
