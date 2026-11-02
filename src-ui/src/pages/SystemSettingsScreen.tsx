import { EmptyState } from '../components/EmptyState';

/** S7 交付：企业档案/周期 cadence/生效日期等系统设置。 */
export function SystemSettingsScreen() {
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">06 · system settings</div>
          <h2 className="screen-title">系统设置</h2>
          <div className="sub">S7 阶段交付企业档案与周期策略设置。</div>
        </div>
      </div>
      <EmptyState title="系统设置建设中（S7）" hint="周期修改保存后建立新的 period policy 版本。" />
    </div>
  );
}
