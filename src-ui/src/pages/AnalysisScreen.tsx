import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';

/** S4 交付：指标卡/表 + ECharts 图表。此文件在 S4 完成内容。 */
export function AnalysisScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">02 · operating review</div>
          <h2 className="screen-title">经营分析</h2>
          <div className="sub">
            {hasData
              ? `${app.activeWorkspace?.name} · 分析期间见下表 · 数据来自本地核心`
              : `${app.activeWorkspace?.name} · 尚无分析期间`}
          </div>
        </div>
      </div>
      {!hasData ? (
        <EmptyState title="尚无分析期间" hint="完成数据源导入与字段映射后，由本地核心计算生成分析结果。" />
      ) : (
        <EmptyState title="经营分析屏建设中（S4）" hint="指标卡、预实差异表与图表在 S4 阶段交付。" />
      )}
    </div>
  );
}
