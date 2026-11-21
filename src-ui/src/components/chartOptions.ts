/**
 * ECharts 图表 option 构建（纯函数）。
 *
 * 硬约束：序列数据保持 DecimalString 原样传入 ECharts（由 ECharts
 * 自行解析数值渲染），TS 层不做 Number/parseFloat 转换或运算；
 * 缺桶以 null 传入且 connectNulls=false —— 断线不插值。
 * 图表数据仅用于展示，绝不回写业务层。
 */

import type { MetricRowView, MetricSeriesView } from '../api/types';
import { favorableOf, formatMetricNumber, formatMetricValue, metricDisplayUnit } from '../api/format';

export type ChartType = 'trend' | 'plan-actual' | 'variance' | 'composition';

export const CHART_TYPE_LABELS: Record<ChartType, string> = {
  trend: '趋势折线',
  'plan-actual': '计划 / 实绩柱状',
  variance: '逐桶差异',
  composition: '分类构成',
};

const RED = '#a83c32';
const BLUE = '#2f6370';
const GOLD = '#b68b41';
const MUTED = '#687278';
const LINE = '#e4e8e4';

const bucketLabel = (bucket: string): string => bucket.slice(5).replace('-', '/');

type ChartPoint = { dataIndex: number; seriesIndex: number };
type SeriesKey = 'actual' | 'plan' | 'delta';
const SERIES_LABELS: Record<SeriesKey, string> = { actual: '实绩', plan: '计划', delta: '差异（实绩-计划）' };

/** tooltip 从原始序列取精确字符串，不使用图表引擎转换后的浮点值。 */
function seriesTooltip(series: MetricSeriesView, keys: SeriesKey[]) {
  return {
    trigger: 'axis',
    renderMode: 'richText',
    formatter: (params: ChartPoint | ChartPoint[]) => {
      const points = Array.isArray(params) ? params : [params];
      const bucket = series.buckets[points[0]?.dataIndex] ?? '';
      return [bucket, ...points.map((point) => {
        const key = keys[point.seriesIndex];
        return `${SERIES_LABELS[key]}：${formatMetricValue(series[key]?.[point.dataIndex], series.unit, series.unit_signature, key === 'delta')}`;
      })].join('\n');
    },
  };
}

/** 刻度由 ECharts 生成；仅转为展示字符串，业务序列保持原值。 */
function valueAxis(series: MetricSeriesView, difference = false) {
  const unit = metricDisplayUnit(series.unit, series.unit_signature);
  return {
    type: 'value',
    axisLabel: {
      color: MUTED,
      fontSize: 12,
      formatter: (value: number) => formatMetricNumber(value.toLocaleString('en-US', { useGrouping: false, maximumFractionDigits: 20 }), series.unit, series.unit_signature, difference),
    },
    splitLine: { lineStyle: { color: LINE } },
    name: difference && unit === '%' ? '百分点' : unit,
    nameTextStyle: { color: MUTED, fontSize: 12 },
  };
}

/** 趋势折线：实绩实线 + 计划虚线；null 断线。 */
export function buildTrendOption(series: MetricSeriesView) {
  return {
    grid: { left: 14, right: 22, top: 18, bottom: 32, containLabel: true },
    tooltip: seriesTooltip(series, ['actual', 'plan']),
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 12, hideOverlap: true },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: valueAxis(series),
    series: [
      {
        name: '实绩',
        type: 'line',
        data: series.actual,
        connectNulls: false,
        lineStyle: { color: RED, width: 3 },
        itemStyle: { color: RED },
        symbolSize: 5,
      },
      {
        name: '计划',
        type: 'line',
        data: series.plan,
        connectNulls: false,
        lineStyle: { color: BLUE, width: 2, type: 'dashed' },
        itemStyle: { color: BLUE },
        symbol: 'none',
      },
    ],
  };
}

/** 计划/实绩分组柱状。 */
export function buildPlanActualOption(series: MetricSeriesView) {
  return {
    grid: { left: 14, right: 22, top: 26, bottom: 32, containLabel: true },
    tooltip: seriesTooltip(series, ['plan', 'actual']),
    legend: { top: 0, textStyle: { color: MUTED, fontSize: 12 } },
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 12, hideOverlap: true },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: valueAxis(series),
    series: [
      {
        name: '计划',
        type: 'bar',
        data: series.plan,
        itemStyle: { color: BLUE, opacity: 0.75 },
        barGap: '12%',
      },
      {
        name: '实绩',
        type: 'bar',
        data: series.actual,
        itemStyle: { color: RED },
      },
    ],
  };
}

/** 逐桶差异柱状按改善方向着色；未知方向与零差异保持中性。 */
export function buildVarianceOption(series: MetricSeriesView, direction: MetricRowView['direction'] = 'neutral') {
  const diff = series.delta ?? [];
  return {
    grid: { left: 14, right: 22, top: 18, bottom: 32, containLabel: true },
    tooltip: seriesTooltip(series, ['delta']),
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 12, hideOverlap: true },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: valueAxis(series, true),
    series: [
      {
        name: '差异（实绩-计划）',
        type: 'bar',
        data: diff,
        connectNulls: false,
        itemStyle: {
          color: (params: { dataIndex: number }) => {
            const favorable = favorableOf(direction, diff[params.dataIndex] ?? null);
            return favorable === 'favorable' ? GREENISH : favorable === 'unfavorable' ? RED : MUTED;
          },
        },
      },
    ],
  };
}

const GREENISH = '#3e705b';

/** 分类构成饼图。 */
export function buildCompositionOption(series: MetricSeriesView) {
  return {
    tooltip: {
      trigger: 'item',
      renderMode: 'richText',
      formatter: (params: { dataIndex: number }) => {
        const item = series.composition?.[params.dataIndex];
        // 旧比例指标的构成值没有独立单位，不能冒充比率；只展示有限精度数值。
        const ratio = metricDisplayUnit(series.unit, series.unit_signature) === '%';
        return `${item?.name ?? series.metric_label}：${formatMetricValue(item?.value, ratio ? null : series.unit, ratio ? null : series.unit_signature)}`;
      },
    },
    legend: { bottom: 0, textStyle: { color: MUTED, fontSize: 12 } },
    series: [
      {
        name: series.metric_label,
        type: 'pie',
        radius: ['36%', '66%'],
        center: ['50%', '44%'],
        data: series.composition ?? [],
        label: { color: MUTED, fontSize: 12 },
        itemStyle: { borderColor: '#fff', borderWidth: 1 },
      },
    ],
    color: [RED, BLUE, GOLD, '#8a9a5b', '#6a5d7b'],
  };
}

export function buildChartOption(series: MetricSeriesView, type: ChartType, direction?: MetricRowView['direction']) {
  switch (type) {
    case 'trend':
      return buildTrendOption(series);
    case 'plan-actual':
      return buildPlanActualOption(series);
    case 'variance':
      return buildVarianceOption(series, direction);
    case 'composition':
      return buildCompositionOption(series);
  }
}
