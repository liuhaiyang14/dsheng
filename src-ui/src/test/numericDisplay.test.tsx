import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureAnalysisRun } from '../api/fixtureData';
import type { MetricRowView } from '../api';

beforeEach(() => {
  window.localStorage.clear();
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(null);
});
afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const explicit = { kind: 'explicit_target', label: '明确本期目标', is_reference: false } as const;
const base = fixtureAnalysisRun.metrics[0];
const metrics: MetricRowView[] = [
  { ...base, metric_id: 'reference', metric_key: 'reference', metric_label: '参考收入', target_value: '690391.13274336283185840707965', actual_value: '455022.79245283018867924528302', delta: '-235368.34029053264317916179663', target_basis: { kind: 'prior_year_actual', label: '去年同期实绩参考下限', is_reference: true, from: '2025-10-01', to: '2025-10-31' }, status_tag: 'ok' },
  { ...base, metric_id: 'ratio', metric_key: 'ratio', metric_label: '利润率', unit: '1', unit_signature: { dimension: 'ratio', base_unit: '1' }, target_value: '0.20', actual_value: '0.18', delta: '-0.02', target_basis: explicit, status_tag: 'ok', mom: { ...base.mom!, absolute_change: '0.05', change_rate: '0.944857673584' } },
  { ...base, metric_id: 'expense', metric_key: 'expense', metric_label: '费用', target_value: '100', actual_value: '80', delta: '-20', direction: 'down', target_basis: explicit, status_tag: 'focus', mom: { ...base.mom!, absolute_change: '-10', change_rate: '-0.111111111111' } },
  { ...base, metric_id: 'zero', metric_key: 'zero', metric_label: '持平收入', target_value: '100', actual_value: '100', delta: '-0.00', target_basis: explicit, status_tag: 'focus' },
  { ...base, metric_id: 'legacy', metric_key: 'legacy', metric_label: '历史收入', target_basis: undefined, status_tag: 'ok' },
  { ...base, metric_id: 'annual', metric_key: 'annual', metric_label: '计划收入', target_basis: { kind: 'annual_month_plan', label: '年度当月计划参考下限', is_reference: true }, status_tag: 'ok' },
  { ...base, metric_id: 'missing', metric_key: 'missing', metric_label: '待补指标', target_value: null, actual_value: null, delta: null, target_basis: undefined, status_tag: 'ok' },
  { ...base, metric_id: 'pending', metric_key: 'pending', metric_label: '覆盖不足指标', target_value: '100', actual_value: '120', delta: '20', target_basis: explicit, status: 'pending', pending_reasons: ['period_bucket_missing'] },
  { ...base, metric_id: 'delta-missing', metric_key: 'delta-missing', metric_label: '待补差异指标', target_basis: explicit, delta: null },
];

async function openAnalysis() {
  const client = new FixtureClient();
  const original = client.historyShow.bind(client);
  vi.spyOn(client, 'historyShow').mockImplementation(async (...args) => ({
    ...await original(...args), metrics: structuredClone(metrics),
    series: [{ ...fixtureAnalysisRun.series[0], metric_key: 'reference', metric_label: '参考收入', buckets: ['2026-10-01', '2026-10-02'], actual: [metrics[0].actual_value, null], plan: [metrics[0].target_value, null], delta: [metrics[0].delta, null] }],
  }));
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营分析/ }));
  await screen.findByTestId('metric-table');
  return client;
}

function metricRow(table: HTMLElement, label: string) {
  return within(table).getByText(label, { exact: false }).closest('tr')!;
}

describe('经营数值与目标来源回归', () => {
  it('卡片和核算表显示有限精度，按目标来源及方向派生状态，忽略历史 status_tag', async () => {
    await openAnalysis();
    const cards = screen.getByTestId('metric-cards');
    expect(within(cards).getByText('455,022.79')).toBeTruthy();
    expect(cards).not.toHaveTextContent('2830188679');
    expect(cards).toHaveTextContent('低于参考值 235,368.34 元');
    const table = screen.getByTestId('metric-table');
    const reference = metricRow(table, '参考收入');
    expect(reference).toHaveTextContent('690,391.13 元');
    expect(reference).toHaveTextContent('去年同期实绩参考下限 · 参考，待确认目标 · 2025-10-01 至 2025-10-31');
    expect(reference).toHaveTextContent('参考比较');
    expect(reference).not.toHaveTextContent('达标');
    const ratio = metricRow(table, '利润率');
    expect(ratio).toHaveTextContent('18.0%');
    expect(ratio).toHaveTextContent('-2.0pp');
    expect(ratio).toHaveTextContent('94.5%');
    expect(ratio).toHaveTextContent('未达目标');
    const expense = metricRow(table, '费用');
    expect(within(expense).getByText('-20.00 元')).toHaveClass('good');
    expect(within(expense).getByText('-11.1%')).toHaveClass('good');
    expect(expense).toHaveTextContent('达标');
    const zero = metricRow(table, '持平收入');
    expect(within(zero).getByText('0.00 元')).toHaveClass('muted');
    expect(within(zero).getByText('持平')).toBeTruthy();
    expect(metricRow(table, '历史收入')).toHaveTextContent('来源待确认');
    expect(metricRow(table, '计划收入')).toHaveTextContent('参考比较');
    const missing = metricRow(table, '待补指标');
    expect(missing).toHaveTextContent('实绩待补');
    expect(missing).not.toHaveTextContent('0.00');
    expect(metricRow(table, '覆盖不足指标')).toHaveTextContent('数据待补');
    expect(metricRow(table, '待补差异指标')).toHaveTextContent('差异待补');
    expect(screen.queryByText(/按差异贡献排序|异常信号/)).toBeNull();
    expect(document.querySelector('.bar-fill')).toBeNull();
  });

  it('图表数据表、两表、历史与诊断问题描述沿用同一显示和来源口径', async () => {
    const client = await openAnalysis();
    const analyze = vi.spyOn(client, 'analyze');
    fireEvent.click(screen.getByRole('button', { name: '查看数据表' }));
    expect(screen.getByTestId('chart-data-table')).toHaveTextContent('455,022.79 元');
    expect(screen.getByTestId('chart-data-table')).toHaveTextContent('缺桶');
    fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /两表展示/ }));
    const accounting = await screen.findByTestId('accounting-table-panel');
    expect(accounting).toHaveTextContent('455,022.79 元');
    expect(accounting).toHaveTextContent('去年同期实绩参考下限');
    expect(accounting).toHaveTextContent('-2.0pp');
    fireEvent.click(screen.getByRole('tab', { name: '推移表' }));
    expect(screen.getByTestId('trend-table-panel')).toHaveTextContent('2026-10-01：455,022.79 元');
    expect(screen.getByTestId('trend-table-panel')).toHaveTextContent('1 个期间待补');
    fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /历史查询/ }));
    await screen.findByTestId('history-table');
    expect(screen.getByText('455,022.79 元')).toBeTruthy();
    expect(screen.getByText(/去年同期实绩参考下限/)).toBeTruthy();
    fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营问题诊断/ }));
    const input = await screen.findByTestId('problem-input');
    expect(input).toHaveAttribute('placeholder', expect.stringContaining('参考值 690,391.13 元'));
    expect(input).toHaveAttribute('placeholder', expect.stringContaining('参考，待确认目标'));
    expect(analyze).not.toHaveBeenCalled();
    expect(metrics[0].actual_value).toBe('455022.79245283018867924528302');
  });
});
