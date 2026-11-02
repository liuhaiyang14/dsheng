import { EmptyState } from '../components/EmptyState';

/** S5 交付：框架选择 + 显式生成 + 报告详情。 */
export function DiagnosisScreen() {
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">03 · issue diagnosis</div>
          <h2 className="screen-title">经营问题诊断</h2>
          <div className="sub">S5 阶段交付框架选择、显式生成与报告详情。</div>
        </div>
      </div>
      <EmptyState title="诊断屏建设中（S5）" hint="进入页面绝不自动生成诊断报告。" />
    </div>
  );
}
