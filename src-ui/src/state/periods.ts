import type { Cadence, DateRange, HistoryEntryView, PeriodPolicyView } from '../api';

export const PERIOD_GRAIN_LABELS: Record<Cadence, string> = {
  day: '日度', week: '周度', month: '月度', quarter: '季度',
};

export function currentPeriodPolicy(policies: PeriodPolicyView[], currentId?: string) {
  return policies.find((p) => p.period_policy_id === currentId)
    ?? [...policies].sort((a, b) => b.version - a.version)[0] ?? null;
}

function dateLabel(date: string) {
  const [year, month, day] = date.split('-');
  return `${year}年${Number(month)}月${Number(day)}日`;
}

function configuredMonthWindow(period: DateRange, startDay: number) {
  const from = new Date(`${period.from}T00:00:00Z`);
  if (Number.isNaN(from.getTime())) return false;
  const anchor = (year: number, month: number) => new Date(Date.UTC(year, month,
    Math.min(startDay, new Date(Date.UTC(year, month + 1, 0)).getUTCDate())));
  const start = anchor(from.getUTCFullYear(), from.getUTCMonth());
  const next = anchor(from.getUTCFullYear(), from.getUTCMonth() + 1);
  const end = new Date(next.getTime() - 86400000);
  return start.toISOString().slice(0, 10) === period.from && end.toISOString().slice(0, 10) === period.to;
}

function isMonthlyPeriod(period: DateRange, startDay: number) {
  if (period.from.slice(0, 7) === period.to.slice(0, 7) || configuredMonthWindow(period, startDay)) return true;
  // 历史记录可能采用旧起始日；仅识别完整有效月窗口，不按新设置改写历史范围。
  for (let day = 1; day <= 31; day += 1) {
    if (day !== startDay && configuredMonthWindow(period, day)) return true;
  }
  return false;
}

/** 缩写实际保存范围，跨月或多日汇总保留范围，不虚构独立日/月结果。 */
export function periodLabel(period: DateRange, cadence: Cadence, monthStartDay = 1) {
  if (cadence === 'month' && isMonthlyPeriod(period, monthStartDay)) {
    return `${period.from.slice(0, 4)}年${Number(period.from.slice(5, 7))}月`;
  }
  if (cadence === 'quarter') {
    const start = Math.floor((Number(period.from.slice(5, 7)) - 1) / 3);
    const end = Math.floor((Number(period.to.slice(5, 7)) - 1) / 3);
    if (period.from.slice(0, 4) === period.to.slice(0, 4) && start === end) {
      return `${period.from.slice(0, 4)}年第${start + 1}季度`;
    }
  }
  if (period.from === period.to) return dateLabel(period.from);
  return `${dateLabel(period.from)}—${dateLabel(period.to)}`;
}

/** 区间身份与显示文字分开；同月的部分日期和整月属于同一个月。 */
export function periodIdentity(period: DateRange, cadence: Cadence, monthStartDay = 1) {
  if (cadence === 'month' && isMonthlyPeriod(period, monthStartDay)) {
    return `month:${period.from.slice(0, 7)}`;
  }
  if (cadence === 'quarter') {
    const quarter = (date: string) => `${date.slice(0, 4)}:${Math.floor((Number(date.slice(5, 7)) - 1) / 3)}`;
    if (quarter(period.from) === quarter(period.to)) return `quarter:${quarter(period.from)}`;
  }
  return `${cadence}:${period.from}:${period.to}`;
}

/** 每个粒度/区间仅呈现最后一次分析，原始版本存储不在浏览流程中删除。 */
export function latestPeriods(history: HistoryEntryView[], monthStartDay = 1) {
  const periods = new Map<string, HistoryEntryView>();
  for (const entry of [...history].sort((a, b) => b.updated_at.localeCompare(a.updated_at)
    || b.current_revision_no - a.current_revision_no || a.run_id.localeCompare(b.run_id))) {
    const key = periodIdentity(entry.period, entry.cadence, monthStartDay);
    if (!periods.has(key) && entry.summary.metric_count > 0) periods.set(key, entry);
  }
  return [...periods.values()].sort((a, b) => b.period.to.localeCompare(a.period.to)
    || b.period.from.localeCompare(a.period.from) || a.run_id.localeCompare(b.run_id));
}

export function savedPeriods(history: HistoryEntryView[], cadence: Cadence | undefined, monthStartDay = 1) {
  return latestPeriods(history, monthStartDay).filter((h) => h.cadence === cadence);
}
