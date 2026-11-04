import { useEffect, useMemo, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { ChartPanel } from '../components/ChartPanel';
import {
  buildChartOption,
  CHART_TYPE_LABELS,
  type ChartType,
} from '../components/chartOptions';
import {
  FIXTURE_DEMO_ANNOTATION,
  formatMetricValue,
  formatRatioPercent,
  formatSigned,
  groupDecimal,
  diffTone,
  favorableOf,
  type AnalysisRunView,
  type Cadence,
  type MetricRowView,
} from '../api';

const GRAIN_LABELS: Record<Cadence, string> = {
  day: '日度',
  week: '周度',
  month: '月度',
  quarter: '季度',
};

/** 核心不返回 status_tag；按 favorable/可比性本地推导（§1.1 语义）。 */
function deriveStatusTag(m: MetricRowView): NonNullable<MetricRowView['status_tag']> {
  if (m.mom == null && m.yoy == null) return 'incomplete';
  const favorable = favorableOf(m.direction, m.delta);
  if (favorable === 'unfavorable') return 'focus';
  if (m.delta == null || m.target_value == null) return 'watch';
  return 'ok';
}

const STATUS_TAGS: Record<NonNullable<MetricRowView['status_tag']>, { tone: 'red' | 'gold' | 'green' | 'gray'; label: string }> = {
  focus: { tone: 'red', label: '重点' },
  watch: { tone: 'gold', label: '关注' },
  ok: { tone: 'green', label: '达标' },
  incomplete: { tone: 'gray', label: '数据不完整' },
};

/** 合同 reason 码 → 用户可读原因（不改合同，仅展示层翻译）。 */
const REASON_LABELS: Record<string, string> = {
  missing_base: '缺少上期/同期数据，暂无法比较',
  missing_current: '本期数据缺失，暂无法比较',
  zero_base: '基期为 0，仅显示绝对差异',
  negative_base: '基期为负数，仅显示绝对差异',
  coverage_incomplete: '期间覆盖不完整',
  definition_mismatch: '指标口径与基期不一致',
  unit_mismatch: '单位口径不一致',
  scope_mismatch: '业务范围不一致',
  calendar_mismatch: '周期日历不一致',
  source_conflict: '来源冲突未解决',
  configuration_required: '指标还需配置（口径/公式）',
  unsupported_formula: '公式暂不支持自动计算',
  precision_unverified: '数值精度未能核验',
  invalid_input: '输入数据无效',
};

/** 比较值展示：可比较才显示数值；否则显示 —（原因汇总在表下方说明区）。 */
function CompareCell({
  compare,
}: {
  compare: MetricRowView['mom'] | MetricRowView['yoy'];
}) {
  if (compare == null) {
    return (
      <td className="muted" title="无可比基期">
        —
      </td>
    );
  }
  if (compare.availability_status === 'available' && compare.change_rate != null) {
    const tone = compare.change_rate.startsWith('-') ? 'bad' : 'good';
    return <td className={tone}>{formatRatioPercent(compare.change_rate)}</td>;
  }
  return (
    <td className="muted" title={compare.reason ? REASON_LABELS[compare.reason] ?? '无可比基期' : undefined}>
      —
    </td>
  );
}

export function AnalysisScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const historyDetail = app.data.currentDetail;
  const [metricKey, setMetricKey] = useState<string>('');
  const [chartType, setChartType] = useState<ChartType>('trend');
  const [showDataTable, setShowDataTable] = useState(false);
  /** 粒度切换后的本地重算结果（由核心 analyze 计算；UI 不聚合）。 */
  const [analyzed, setAnalyzed] = useState<AnalysisRunView | null>(null);

  const detail = analyzed ?? historyDetail;
  const activeGrain = detail?.grain ?? 'day';

  // 切换企业/期间后丢弃旧档案的重算结果，避免跨档案混用。
  useEffect(() => {
    setAnalyzed(null);
  }, [historyDetail?.run_id, app.activeWorkspaceId]);

  const changeGrain = (next: Cadence) => {
    if (!app.client || !app.activeWorkspaceId || !historyDetail) return;
    if (next === historyDetail.grain && analyzed == null) return;
    // 粒度切换 = 显式本地重算（analyze 无 Provider 调用），由核心返回新聚合。
    app.client
      .analyze({
        workspace_id: app.activeWorkspaceId,
        directory_label: '',
        from: historyDetail.period.from,
        to: historyDetail.period.to,
        grain: next,
      })
      .then((run) => setAnalyzed(run))
      .catch((err: unknown) => {
        app.clearError();
        // 展示核心错误 envelope 的 message
        window.alert(err instanceof Error ? err.message : String(err));
      });
  };
  const series = useMemo(() => {
    if (!detail) return null;
    return detail.series.find((s) => s.metric_key === metricKey) ?? detail.series[0] ?? null;
  }, [detail, metricKey]);

  const chartUnavailable =
    series != null &&
    ((chartType === 'composition' && series.composition == null) ||
      (chartType === 'variance' && series.delta == null));

  if (!hasData || !detail) {
    return (
      <div>
        <ScreenHead hasData={hasData} />
        <EmptyState
          title="尚无分析期间"
          hint="完成数据源导入与字段映射后，由本地核心计算生成分析结果；本页不做任何计算。"
        />
      </div>
    );
  }

  const topMetrics = detail.metrics.slice(0, 4);

  return (
    <div>
      <ScreenHead hasData={hasData} />
      {app.mode === 'fixture' ? (
        <div className="notice" data-testid="analysis-demo-note">
          <b>演示数据说明</b>
          <span>
            以下指标与序列均为{FIXTURE_DEMO_ANNOTATION}。正式版的指标数值、聚合与比较
            全部由本地 Rust 核心计算，UI 只呈现返回结果。
          </span>
        </div>
      ) : null}

      <div className="metric-grid" data-testid="metric-cards">
        {topMetrics.map((m) => (
          <div className="metric" key={m.metric_id}>
            <div className="label">{m.metric_label}</div>
            <div className="value">
              {m.actual_value == null ? '—' : groupDecimal(m.actual_value)}
              {m.unit === '%' && m.actual_value != null ? '%' : ''}
            </div>
            <div className={`delta ${diffTone(favorableOf(m.direction, m.delta))}`}>
              {m.delta != null && m.target_value != null
                ? `${m.delta.startsWith('-') ? '↓ 低于' : '↑ 高于'}目标 ${groupDecimal(m.delta.replace('-', ''))}${m.unit === '%' ? 'pp' : ` ${m.unit}`}`
                : '未设目标'}
            </div>
            <div className="metric-sub">
              <span>
                环比{' '}
                {m.mom?.availability_status === 'available' && m.mom.change_rate != null
                  ? `${formatRatioPercent(m.mom.change_rate)}`
                  : '不可比'}
              </span>
              <span>
                同比{' '}
                {m.yoy?.availability_status === 'available' && m.yoy.change_rate != null
                  ? `${formatRatioPercent(m.yoy.change_rate)}`
                  : '不可比'}
              </span>
            </div>
          </div>
        ))}
      </div>

      <div className="analysis-grid">
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">核算表 · 预实差异</span>
            <span className="panel-note">
              {detail.period.from} 至 {detail.period.to} · 公式由本地 Rust 核算
            </span>
          </div>
          <div className="table-wrap">
            <table className="data-table" data-testid="metric-table">
              <thead>
                <tr>
                  <th>指标</th>
                  <th>目标</th>
                  <th>实绩</th>
                  <th>差异</th>
                  <th>环比</th>
                  <th>同比</th>
                  <th>状态</th>
                </tr>
              </thead>
              <tbody>
                {detail.metrics.map((m) => {
                  const tag = STATUS_TAGS[m.status_tag ?? deriveStatusTag(m)];
                  return (
                    <tr key={m.metric_id}>
                      <td>
                        {m.metric_label}
                        {m.unit ? `（${m.unit}）` : ''}
                      </td>
                      <td>{formatMetricValue(m.target_value, null)}</td>
                      <td>{formatMetricValue(m.actual_value, null)}</td>
                      <td className={diffTone(favorableOf(m.direction, m.delta))}>
                        {m.delta != null ? formatSigned(m.delta) : '—'}
                      </td>
                      <CompareCell compare={m.mom} />
                      <CompareCell compare={m.yoy} />
                      <td>
                        <Tag tone={tag.tone}>{tag.label}</Tag>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <div className="table-foot">
              无可比数据时显示原因，不把缺失值当 0；数值为核心返回的十进制字符串。
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">异常信号</span>
            <span className="panel-note">按差异贡献排序（核心返回）</span>
          </div>
          {detail.metrics
            .filter((m) => m.delta != null)
            .slice(0, 3)
            .map((m) => (
              <div className="bar-row compact" key={m.metric_id}>
                <div className="bar-label">
                  <span>
                    {m.metric_label}
                    {m.delta != null && m.delta.startsWith('-') ? '低于目标' : '高于目标'}
                  </span>
                  <b className={diffTone(favorableOf(m.direction, m.delta))}>
                    {formatSigned(m.delta)}
                    {m.unit === '%' ? 'pp' : ''}
                  </b>
                </div>
                <div className="bar-track">
                  <div
                    className={`bar-fill ${(m.status_tag ?? deriveStatusTag(m)) === 'watch' ? 'gold' : ''}`}
                    style={{ width: (m.status_tag ?? deriveStatusTag(m)) === 'focus' ? '72%' : '40%' }}
                  />
                </div>
              </div>
            ))}
          <div className="root-card">
            <h4>环比／同比说明</h4>
            <p>
              {detail.metrics
                .filter(
                  (m) => m.mom?.availability_status !== 'available' || m.yoy?.availability_status !== 'available',
                )
                .map((m) => {
                  const parts: string[] = [];
                  if (m.mom != null && m.mom.availability_status !== 'available') {
                    parts.push(`环比${REASON_LABELS[m.mom.reason ?? 'missing_base'] ?? '缺少可比基期'}`);
                  }
                  if (m.yoy != null && m.yoy.availability_status !== 'available') {
                    parts.push(`同比${REASON_LABELS[m.yoy.reason ?? 'missing_base'] ?? '缺少可比基期'}`);
                  }
                  return `${m.metric_label}：${parts.length ? parts.join('，') : '无可比基期'}`;
                })
                .join('；') || '本期全部指标可比。'}
            </p>
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <span className="panel-title">经营图表</span>
          <span className="panel-note">
            {app.mode === 'fixture'
              ? '演示序列（虚构数据）；正式版仅绘制核心解析的真实期间，缺桶断线不插值'
              : '仅绘制核心解析的真实期间，缺桶断线不插值'}
          </span>
        </div>
        <div className="chart-toolbar">
          <select
            className="select"
            aria-label="图表指标"
            value={series?.metric_key ?? ''}
            onChange={(e) => setMetricKey(e.target.value)}
            data-testid="chart-metric-select"
          >
            {detail.series.map((s) => (
              <option key={s.metric_key} value={s.metric_key}>
                {s.metric_label}
              </option>
            ))}
          </select>
          <select
            className="select"
            aria-label="图表类型"
            value={chartType}
            onChange={(e) => setChartType(e.target.value as ChartType)}
            data-testid="chart-type-select"
          >
            {(Object.keys(CHART_TYPE_LABELS) as ChartType[]).map((t) => (
              <option key={t} value={t}>
                {CHART_TYPE_LABELS[t]}
              </option>
            ))}
          </select>
          <select
            className="select"
            aria-label="聚合粒度"
            value={activeGrain}
            onChange={(e) => changeGrain(e.target.value as Cadence)}
            data-testid="grain-select"
          >
            {(Object.keys(GRAIN_LABELS) as Cadence[]).map((g) => (
              <option key={g} value={g}>
                {GRAIN_LABELS[g]}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="outline"
            onClick={() => setShowDataTable((v) => !v)}
          >
            {showDataTable ? '收起数据表' : '查看数据表'}
          </button>
        </div>
        {chartUnavailable ? (
          <div className="empty-state" style={{ margin: 14 }}>
            当前指标没有{CHART_TYPE_LABELS[chartType]}所需的数据（核心未返回该序列）；
            不在 UI 层推算或插值。
          </div>
        ) : series ? (
          <ChartPanel
            option={buildChartOption(series, chartType)}
            ariaLabel={`${series.metric_label}${CHART_TYPE_LABELS[chartType]}（${GRAIN_LABELS[activeGrain]}）`}
          />
        ) : null}
        {showDataTable && series ? (
          <div className="table-wrap" style={{ margin: '0 15px 15px' }}>
            <table className="data-table" data-testid="chart-data-table">
              <thead>
                <tr>
                  <th>期间桶</th>
                  <th>实绩</th>
                  <th>计划</th>
                  <th>差异</th>
                </tr>
              </thead>
              <tbody>
                {series.buckets.map((b, i) => (
                  <tr key={b}>
                    <td>{b}</td>
                    <td>{series.actual[i] ?? '缺桶'}</td>
                    <td>{series.plan[i] ?? '—'}</td>
                    <td>{series.delta ? (series.delta[i] ?? '缺桶') : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function ScreenHead({ hasData }: { hasData: boolean }) {
  const app = useApp();
  return (
    <div className="screen-head">
      <div>
        <div className="eyebrow">02 · operating review</div>
        <h2 className="screen-title">经营分析</h2>
        <div className="sub">
          {hasData
            ? `${app.activeWorkspace?.name} · ${app.data.currentDetail?.period.from} 至 ${app.data.currentDetail?.period.to} · ${GRAIN_LABELS[app.data.currentDetail?.grain ?? 'day']}聚合，可切换粒度`
            : `${app.activeWorkspace?.name} · 尚无分析期间`}
        </div>
      </div>
    </div>
  );
}
