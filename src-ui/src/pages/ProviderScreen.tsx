import { EmptyState } from '../components/EmptyState';

/** S7 交付：掩码 Key、明文存储风险披露、无连接测试。 */
export function ProviderScreen() {
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">05 · provider</div>
          <h2 className="screen-title">Provider 设置</h2>
          <div className="sub">S7 阶段交付 Provider 配置界面。</div>
        </div>
      </div>
      <EmptyState title="Provider 设置建设中（S7）" hint="保存配置不发起任何模型连接测试。" />
    </div>
  );
}
