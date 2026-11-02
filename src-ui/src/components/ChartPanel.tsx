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
    // 导出由本地核心完成（CLI export chart）；此处仅提供占位说明。
    window.alert(
      '导出 PNG/PDF 由本地核心完成（dsheng export --chart）；当前演示环境不产生文件。',
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
          {ariaLabel}（当前环境无 canvas，未绘制）
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
