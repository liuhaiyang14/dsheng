import { EmptyState } from '../components/EmptyState';

/** S6 交付：任务列表 + 显式生成 + 详情抽屉 + 时间线。 */
export function TaskScreen() {
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">04 · action sheet</div>
          <h2 className="screen-title">任务单</h2>
          <div className="sub">S6 阶段交付任务列表、生成预览与详情抽屉。</div>
        </div>
      </div>
      <EmptyState title="任务屏建设中（S6）" hint="任务生成只能由用户显式点击触发。" />
    </div>
  );
}
