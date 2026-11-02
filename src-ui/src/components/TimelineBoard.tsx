/**
 * 任务时间线（呈现层排序规则，task-lifecycle §6/计划 §9.4）：
 * - 按真实日期（date/occurred_at 的日期部分）倒序排列；
 * - 缺日期的条目不参与日期排序，统一归入底部「未设置」分组，显示“未设置”；
 * - 不按导入/行顺序推断。
 */

import type { TimelineEntryView } from '../api';

const KIND_LABELS: Record<TimelineEntryView['kind'], string> = {
  planned_start: '计划开始',
  due: '截止节点',
  completed: '完成节点',
  progress_event: '进度事件',
};

/** 排序：有日期者按日期字符串倒序（ISO 日期可直接字典序比较）；无日期者殿后。 */
export function sortTimelineEntries(entries: TimelineEntryView[]): TimelineEntryView[] {
  const dated = entries
    .filter((e) => e.date != null)
    .sort((a, b) => (a.date! < b.date! ? 1 : a.date! > b.date! ? -1 : 0));
  const undated = entries.filter((e) => e.date == null);
  return [...dated, ...undated];
}

export function TimelineBoard({ entries }: { entries: TimelineEntryView[] }) {
  const sorted = sortTimelineEntries(entries);
  if (sorted.length === 0) {
    return <div className="panel-note" style={{ padding: '10px 0' }}>暂无进度记录</div>;
  }
  return (
    <div data-testid="timeline-board">
      {sorted.map((entry, i) => (
        <div
          className="timeline-item"
          key={`${entry.task_id}-${entry.kind}-${entry.date ?? 'none'}-${i}`}
        >
          {entry.task_title} · {KIND_LABELS[entry.kind]} · {entry.detail}
          <small>
            {entry.date ??
              (entry.occurred_at ? entry.occurred_at.slice(0, 10) : '未设置')}
            {entry.date == null && entry.occurred_at == null
              ? ''
              : entry.occurred_at
                ? ` ${entry.occurred_at.slice(11, 16)} UTC`
                : ''}{' '}
            · 日期{entry.date == null ? '未设置（不参与排序）' : '已设置'}
          </small>
        </div>
      ))}
    </div>
  );
}
