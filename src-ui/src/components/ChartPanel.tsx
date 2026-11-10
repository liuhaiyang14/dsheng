/**
 * ECharts 包装组件：本地 npm 依赖（import 'echarts'），禁止 CDN。
 * jsdom 无 canvas，测试通过 vi.mock('echarts') 或只测 chartOptions 纯函数。
 */

import { useEffect, useRef } from 'react';
import * as echarts from 'echarts';

/** jsdom 等无 canvas 环境跳过真实渲染（测试里验证 option 构建即可）。 */
function canvasAvailable(): boolean {
  try {
    return document.createElement('canvas').getContext('2d') != null;
  } catch {
    return false;
  }
}

export function ChartPanel({
  option,
  ariaLabel,
}: {
  option: echarts.EChartsCoreOption;
  ariaLabel: string;
}) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<echarts.ECharts | null>(null);
  const canRender = canvasAvailable();

  useEffect(() => {
    if (!containerRef.current || !canRender) return;
    const chart = echarts.init(containerRef.current);
    chartRef.current = chart;
    return () => {
      chart.dispose();
      chartRef.current = null;
    };
  }, [canRender]);

  useEffect(() => {
    if (canRender) {
      chartRef.current?.setOption(option, { notMerge: true });
    }
  }, [option, canRender]);

  const exportPng = () => {
    // 当前页面只负责展示图表，导出由桌面版统一处理。
    window.alert(
      '当前环境暂不支持直接导出图表文件。',
    );
  };

  return (
    <div>
      {canRender ? (
        <div
          ref={containerRef}
          className="chart"
          role="img"
          aria-label={ariaLabel}
          data-testid="chart-panel"
        />
      ) : (
        <div
          className="chart"
          role="img"
          aria-label={ariaLabel}
          data-testid="chart-panel-stub"
          style={{ display: 'grid', placeItems: 'center', color: '#687278', fontSize: 11 }}
        >
          {ariaLabel}（当前设备暂不显示图表）
        </div>
      )}
      <div className="legend">
        <span>缺桶断线不插值</span>
        <span>页面与表格使用相同期间范围</span>
        <button type="button" className="outline" onClick={exportPng}>
          导出 PNG
        </button>
      </div>
    </div>
  );
}
