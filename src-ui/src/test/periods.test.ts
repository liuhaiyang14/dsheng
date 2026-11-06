// 期间标签与结果选择：仅合成元数据，无网络或业务计算。
import { expect, it } from 'vitest';
import { currentPeriodPolicy, latestPeriods, periodIdentity, periodLabel, savedPeriods } from '../state/periods';
import { fixtureHistory, fixturePeriodPolicies } from '../api/fixtureData';
import type { HistoryEntryView } from '../api';

it('月度和单日显示自然日期，跨月与多日结果明确保留真实区间', () => {
  expect(periodLabel({ from: '2026-10-01', to: '2026-10-31' }, 'month')).toBe('2026年10月');
  expect(periodLabel({ from: '2026-10-01', to: '2026-10-01' }, 'day')).toBe('2026年10月1日');
  expect(periodLabel({ from: '2026-09-01', to: '2026-10-31' }, 'month')).toBe('2026年9月1日—2026年10月31日');
  expect(periodLabel({ from: '2026-10-01', to: '2026-10-28' }, 'day')).toBe('2026年10月1日—2026年10月28日');
  expect(periodLabel({ from: '2026-10-01', to: '2026-12-31' }, 'quarter')).toBe('2026年第4季度');
  expect(periodLabel({ from: '2026-12-01', to: '2027-01-31' }, 'quarter')).toBe('2026年12月1日—2027年1月31日');
});

it('同粒度同区间按更新时间选最后一份有数据的结果，版本号不覆盖更新时间', () => {
  const old: HistoryEntryView = { ...structuredClone(fixtureHistory[0]), current_revision_no: 99 };
  const newest: HistoryEntryView = { ...structuredClone(old), run_id: 'newer-result', current_revision_no: 1, updated_at: '2026-11-01T00:00:00Z' };
  const empty: HistoryEntryView = { ...structuredClone(newest), run_id: 'empty-result', updated_at: '2026-11-02T00:00:00Z', summary: { metric_count: 0, task_count: 1, chart_count: 0 } };
  const inputs = [old, fixtureHistory[1], empty, newest];
  expect(latestPeriods(inputs).map((h) => h.run_id)).toEqual([newest.run_id, fixtureHistory[1].run_id]);
  expect(savedPeriods(inputs, 'day').map((h) => h.run_id)).toEqual([newest.run_id]);
  expect(savedPeriods(inputs, 'month').map((h) => h.run_id)).toEqual([fixtureHistory[1].run_id]);
  expect(savedPeriods(inputs, 'week')).toEqual([]);
  expect(savedPeriods(inputs, undefined)).toEqual([]);
  expect(inputs[0]).toBe(old);
});

it('月度1至5日和1至31日都归同月，只展示最后分析结果', () => {
  const complete: HistoryEntryView = { ...structuredClone(fixtureHistory[0]), cadence: 'month', period: { from: '2026-10-01', to: '2026-10-31' } };
  const partial: HistoryEntryView = { ...structuredClone(complete), run_id: 'partial-month', period: { from: '2026-10-01', to: '2026-10-05' }, updated_at: '2026-11-01T00:00:00Z' };
  expect(savedPeriods([complete, partial], 'month')).toEqual([partial]);
  expect(periodLabel(partial.period, partial.cadence)).toBe('2026年10月');
});

it('当前周期优先档案绑定ID，兼容9999结束日，缺绑定时回退最高版本', () => {
  const old = { ...structuredClone(fixturePeriodPolicies[0]), effective_to: '9999-12-31' };
  const current = { ...structuredClone(fixturePeriodPolicies[1]), effective_to: '9999-12-31' };
  expect(currentPeriodPolicy([current, old], current.period_policy_id)).toBe(current);
  expect(currentPeriodPolicy([current, old], old.period_policy_id)).toBe(old);
  expect(currentPeriodPolicy([old, current])).toBe(current);
  expect(currentPeriodPolicy([old, current], 'missing-id')).toBe(current);
  expect(currentPeriodPolicy([])).toBeNull();
});

it('跨月和短月核算期按起点月份显示，旧起始日的记录也参与同月最新去重', () => {
  expect(periodLabel({ from: '2026-01-02', to: '2026-02-01' }, 'month', 2)).toBe('2026年1月');
  expect(periodLabel({ from: '2026-01-31', to: '2026-02-27' }, 'month', 31)).toBe('2026年1月');
  expect(periodLabel({ from: '2026-02-28', to: '2026-03-30' }, 'month', 31)).toBe('2026年2月');
  expect(periodLabel({ from: '2024-02-29', to: '2024-03-30' }, 'month', 31)).toBe('2024年2月');
  expect(periodIdentity({ from: '2026-10-01', to: '2026-10-05' }, 'month', 31)).toBe('month:2026-10');
  expect(periodIdentity({ from: '2026-10-31', to: '2026-11-29' }, 'month', 2)).toBe('month:2026-10');
  const old = { ...structuredClone(fixtureHistory[0]), cadence: 'month' as const,
    period: { from: '2026-10-31', to: '2026-11-29' } };
  const latest = { ...old, run_id: 'latest-custom-month', updated_at: '2026-12-01T00:00:00Z',
    period: { from: '2026-10-02', to: '2026-11-01' } };
  expect(savedPeriods([old, latest], 'month', 2).map((h) => h.run_id)).toEqual([latest.run_id]);
});
