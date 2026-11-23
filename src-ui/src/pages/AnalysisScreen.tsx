import { useMemo, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { TargetValue, hasConfirmedTarget, targetBasisLabel, targetKindLabel } from '../components/TargetValue';
import { PERIOD_GRAIN_LABELS as GRAIN_LABELS } from '../components/AnalysisPeriodBar';
import { LocalAnalysisForm } from '../components/LocalAnalysisForm';
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
  formatMetricDifference,
  formatMetricNumber,
  metricDisplayUnit,
  isZeroDecimal,
  diffTone,
  favorableOf,
  type MetricRowView,
} from '../api';

function metricStatus(m: MetricRowView): { tone: 'red' | 'gold' | 'green' | 'gray'; label: string } {
  if (m.actual_value == null) return { tone: 'gray', label: '实绩待补' };
  if (m.target_value == null) return { tone: 'gray', label: '未设目标' };
  if (m.delta == null) return { tone: 'gray', label: '差异待补' };
  if (!hasConfirmedTarget(m)) return { tone: 'gray', label: m.target_basis && m.target_basis.kind !== 'unconfirmed_legacy' ? '参考比较' : '来源待确认' };
  if (m.status === 'pending') return { tone: 'gray', label: '数据待补' };
  if (isZeroDecimal(m.delta)) return { tone: 'gray', label: '持平' };
  const favorable = favorableOf(m.direction, m.delta);
  if (favorable === 'unfavorable') return { tone: 'red', label: '未达目标' };
  if (favorable === 'neutral') return { tone: 'gray', label: '仅作比较' };
  return { tone: 'green', label: '达标' };
}

function differenceCaption(m: MetricRowView): string {
  if (m.target_value == null) return '未设目标';
  if (m.delta == null) return '差异待补';
  const kind = targetKindLabel(m);
  if (isZeroDecimal(m.delta)) return `与${kind}持平`;
  const amount = formatMetricValue(m.delta.replace(/^-/, ''), m.unit, m.unit_signature, true);
  return `${m.delta.startsWith('-') ? '↓ 低于' : '↑ 高于'}${kind} ${amount}`;
}

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
  direction,
}: {
  direction: MetricRowView['direction'];
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
    const tone = diffTone(favorableOf(direction, compare.absolute_change ?? compare.change_rate));
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
  const detail = app.data.currentDetail;
  const [metricKey, setMetricKey] = useState<string>('');
  const [chartType, setChartType] = useState<ChartType>('trend');
  const [showDataTable, setShowDataTable] = useState(false);
  const series = useMemo(() => {
    if (!detail) return null;
    const selected = detail.series.find((s) => s.metric_key === metricKey);
    if (selected) return selected;
    return [...detail.series].sort((a, b) => {
      const aValues = a.actual.filter((value) => value != null).length + a.plan.filter((value) => value != null).length;
      const bValues = b.actual.filter((value) => value != null).length + b.plan.filter((value) => value != null).length;
      return bValues - aValues;
    })[0] ?? null;
  }, [detail, metricKey]);

  const chartUnavailable =
    series != null &&
    ((chartType === 'composition' && series.composition == null) ||
      (chartType === 'variance' && series.delta == null));

  if (!hasData || !detail) {
    return (
      <div>
        <ScreenHead hasData={hasData} />
      <LocalAnalysisForm />
        <EmptyState
          title="尚无分析期间"
          hint="选择上方月份并点击“生成并保存期间”。首次保存后，这里会显示分析结果，页眉可切换已保存月份。"
        />
      </div>
    );
  }

  const topMetrics = detail.metrics.slice(0, 4);

  return (
    <div>
      <ScreenHead hasData={hasData} />
      <LocalAnalysisForm />
      {app.mode === 'fixture' ? (
        <div className="notice" data-testid="analysis-demo-note">
          <b>演示数据说明</b>
          <span>
            以下指标与序列均为{FIXTURE_DEMO_ANNOTATION}。正式版的指标数值、聚合与比较
            全部由本地计算，页面只呈现已保存的结果。
          </span>
        </div>
      ) : null}

      <div className="metric-grid" data-testid="metric-cards">
        {topMetrics.map((m) => (
          <div className="metric" key={m.metric_id}>
            <div className="label">{m.metric_label}</div>
            <div className="value" title={formatMetricValue(m.actual_value, m.unit, m.unit_signature)}>
              <span className="value-number">{formatMetricNumber(m.actual_value, m.unit, m.unit_signature)}</span>
              {metricDisplayUnit(m.unit, m.unit_signature) !== '%' && m.actual_value != null ? <span className="metric-unit">{metricDisplayUnit(m.unit, m.unit_signature)}</span> : null}
            </div>
            <div className={`delta ${diffTone(favorableOf(m.direction, m.delta))}`}>
              {differenceCaption(m)}
            </div>
            <small className="target-basis">{targetBasisLabel(m)}</small>
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
            <span className="panel-title">核算表 · 目标/参考差异</span>
            <span className="panel-note">
              {detail.period.from} 至 {detail.period.to} · 已按当前口径核算
            </span>
          </div>
          <div className="table-wrap">
            <table className="data-table metric-table" data-testid="metric-table">
              <thead>
                <tr>
                  <th>指标</th>
                  <th>目标 / 参考值</th>
                  <th>实绩</th>
                  <th>差异</th>
                  <th>环比</th>
                  <th>同比</th>
                  <th>状态</th>
                </tr>
              </thead>
              <tbody>
                {detail.metrics.map((m) => {
                  const tag = metricStatus(m);
                  return (
                    <tr key={m.metric_id}>
                      <td>
                        {m.metric_label}
                        {m.unit ? `（${metricDisplayUnit(m.unit, m.unit_signature)}）` : ''}
                      </td>
                      <td><TargetValue metric={m} /></td>
                      <td>{formatMetricValue(m.actual_value, m.unit, m.unit_signature)}</td>
                      <td className={diffTone(favorableOf(m.direction, m.delta))}>
                        {formatMetricDifference(m.delta, m.unit, m.unit_signature)}
                      </td>
                      <CompareCell compare={m.mom} direction={m.direction} />
                      <CompareCell compare={m.yoy} direction={m.direction} />
                      <td>
                        <Tag tone={tag.tone}>{tag.label}</Tag>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
            <div className="table-foot">
              金额保留 2 位、百分数保留 1 位；pp 表示百分点。参考值不代表已确认目标，缺失值不当作 0。
            </div>
          </div>
        </div>

        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">目标/参考差异</span>
            <span className="panel-note">按指标顺序展示</span>
          </div>
          {detail.metrics
            .filter((m) => m.delta != null)
            .slice(0, 3)
            .map((m) => (
              <div className="bar-row compact" key={m.metric_id}>
                <div className="bar-label">
                  <span>{m.metric_label}</span>
                  <b className={diffTone(favorableOf(m.direction, m.delta))}>{differenceCaption(m)}</b>
                </div>
                <small className="target-basis">{targetBasisLabel(m)}</small>
              </div>
            ))}
          <div className="root-card">
            <h4>环比／同比说明</h4>
            <ul className="compare-notes">
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
                  return (
                    <li key={m.metric_id}>
                      <b>{m.metric_label}</b>
                      <span>{parts.length ? parts.join('；') : '无可比基期'}</span>
                    </li>
                  );
                })}
              {detail.metrics.every(
                (m) => m.mom?.availability_status === 'available' && m.yoy?.availability_status === 'available',
              ) ? <li>本期全部指标可比。</li> : null}
            </ul>
          </div>
        </div>
      </div>

      <div className="panel" style={{ marginTop: 14 }}>
        <div className="panel-head">
          <span className="panel-title">经营图表</span>
          <span className="panel-note">
            {app.mode === 'fixture'
              ? '演示序列（虚构数据）；正式版仅绘制真实期间，缺少期间不补值'
              : '仅绘制真实期间，缺少期间不补值'}
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
            当前指标没有{CHART_TYPE_LABELS[chartType]}所需的数据；
            缺少期间不会自动补值。
          </div>
        ) : series ? (
          <ChartPanel
            option={buildChartOption(series, chartType, detail.metrics.find((metric) => metric.metric_key === series.metric_key)?.direction)}
            ariaLabel={`${series.metric_label}${CHART_TYPE_LABELS[chartType]}（${GRAIN_LABELS[detail.grain]}）`}
          />
        ) : null}
        {showDataTable && series ? (
          <div className="table-wrap" style={{ margin: '0 15px 15px' }}>
            <table className="data-table series-table" data-testid="chart-data-table">
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
                    <td>{series.actual[i] == null ? '缺桶' : formatMetricValue(series.actual[i], series.unit, series.unit_signature)}</td>
                    <td>{series.plan[i] == null ? '—' : formatMetricValue(series.plan[i], series.unit, series.unit_signature)}</td>
                    <td>{series.delta ? (series.delta[i] == null ? '缺桶' : formatMetricValue(series.delta[i], series.unit, series.unit_signature, true)) : '—'}</td>
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
  const workspaceName = app.activeWorkspace?.name ?? '当前企业';
  const savedDetail = app.data.currentDetail;
  return (
    <div className="screen-head">
      <div>
        <div className="eyebrow">02 · 经营分析</div>
        <h2 className="screen-title">经营分析</h2>
        <div className="sub">
          {hasData && savedDetail
            ? `${workspaceName} · ${savedDetail.period.from} 至 ${savedDetail.period.to} · 在页眉切换历史期间`
            : `${workspaceName} · 尚未保存分析期间 · 选择月份并运行本地分析后保存`}
        </div>
      </div>
    </div>
  );
}
