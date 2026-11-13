/**
 * 生成两表屏：两表只来自当前 analysis revision 的本地计算结果。
 * 页签切换核算表/推移表，下载调用核心 export_tables；这里不读取 source 文件，
 * 也不发起 Provider 请求。
 */
import { useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { groupDecimal } from '../api';

type TableTab = 'accounting' | 'trend';

function display(value: string | null | undefined): string {
  return value == null ? '待补' : groupDecimal(value);
}

export function TablesScreen() {
  const app = useApp();
  const detail = app.data.currentDetail;
  const hasData = workspaceHasData(app.data);
  const [tab, setTab] = useState<TableTab>('accounting');
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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
  const activeSeries = detail.series;

  return (
    <div data-testid="tables-screen">
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
          <button type="button" className="outline" onClick={() => void download('bundle')} disabled={!ready || busy}>
            下载两表包
          </button>
        </div>
      </div>

      {notice ? <div className="notice workspace-empty" role="status"><span>{notice}</span></div> : null}
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
          <div className="panel-head"><span className="panel-title">核算表 · 期间预实</span><span className="tag green">生成结果</span></div>
          <div className="table-wrap">
            <table className="data-table">
              <thead><tr><th>指标</th><th>目标</th><th>实绩</th><th>差异</th><th>环比</th><th>同比</th></tr></thead>
              <tbody>
                {detail.metrics.map((metric) => (
                  <tr key={metric.metric_id}>
                    <td>{metric.metric_label}</td>
                    <td>{display(metric.target_value)}</td>
                    <td>{display(metric.actual_value)}</td>
                    <td>{display(metric.delta)}</td>
                    <td>{metric.mom?.change_rate == null ? '不可比/待补' : display(metric.mom.change_rate)}</td>
                    <td>{metric.yoy?.change_rate == null ? '不可比/待补' : display(metric.yoy.change_rate)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="panel-footnote">附加值与经营收益按不同名称展示；人工费、目标、工时或期初缺失时保留待补，不按 0 计算。</div>
          <div className="panel-actions"><button type="button" className="primary" onClick={() => void download('accounting')} disabled={!ready || busy}>下载核算表</button></div>
        </div>
      ) : (
        <div className="panel" role="tabpanel" id="trend-table-panel" aria-labelledby="trend-table-tab" data-testid="trend-table-panel">
          <div className="panel-head"><span className="panel-title">推移表 · 期间序列</span><span className="tag blue">本地聚合</span></div>
          <div className="table-wrap">
            <table className="data-table">
              <thead><tr><th>指标</th><th>有效桶</th><th>实绩序列</th><th>计划序列</th></tr></thead>
              <tbody>
                {activeSeries.map((series) => (
                  <tr key={series.metric_key}>
                    <td>{series.metric_label}</td>
                    <td>{series.buckets.length || '待补'}</td>
                    <td>{series.actual.map((v, i) => `${series.buckets[i] ?? '—'}：${display(v)}`).join('；') || '待补'}</td>
                    <td>{series.plan.map((v, i) => `${series.buckets[i] ?? '—'}：${display(v)}`).join('；') || '待补'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="panel-footnote">金额按期间累计；比率与单位时间指标使用累计分子、分母重算。有效桶不足时显示不可计算原因。</div>
          <div className="panel-actions"><button type="button" className="primary" onClick={() => void download('trend')} disabled={!ready || busy}>下载推移表</button></div>
        </div>
      )}
    </div>
  );
}
