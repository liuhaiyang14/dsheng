import { EmptyState } from '../components/EmptyState';

/** S7 交付：期间列表 / 版本 / 快照查看。 */
export function HistoryScreen() {
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">04 · local history</div>
          <h2 className="screen-title">历史查询</h2>
          <div className="sub">S7 阶段交付期间列表、版本与快照查看。</div>
        </div>
      </div>
      <EmptyState title="历史屏建设中（S7）" hint="同一企业 + 周期 + 期间重做会更新当前版本并保留快照。" />
    </div>
  );
}
