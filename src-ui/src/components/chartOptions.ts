/**
 * ECharts 图表 option 构建（纯函数）。
 *
 * 硬约束：序列数据保持 DecimalString 原样传入 ECharts（由 ECharts
 * 自行解析数值渲染），TS 层不做 Number/parseFloat 转换或运算；
 * 缺桶以 null 传入且 connectNulls=false —— 断线不插值。
 * 图表数据仅用于展示，绝不回写业务层。
 */

import type { MetricSeriesView } from '../api/types';

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

/** 趋势折线：实绩实线 + 计划虚线；null 断线。 */
export function buildTrendOption(series: MetricSeriesView) {
  return {
    grid: { left: 56, right: 18, top: 18, bottom: 46 },
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 10, interval: 3 },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: {
      type: 'value',
      axisLabel: { color: MUTED, fontSize: 10 },
      splitLine: { lineStyle: { color: LINE } },
      name: series.unit,
      nameTextStyle: { color: MUTED, fontSize: 10 },
    },
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
    grid: { left: 56, right: 18, top: 26, bottom: 46 },
    tooltip: { trigger: 'axis' },
    legend: { top: 0, textStyle: { color: MUTED, fontSize: 10 } },
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 10, interval: 3 },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: {
      type: 'value',
      axisLabel: { color: MUTED, fontSize: 10 },
      splitLine: { lineStyle: { color: LINE } },
    },
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

/** 逐桶差异柱状（正负不同色）；diff 序列缺失时不可用。 */
export function buildVarianceOption(series: MetricSeriesView) {
  const diff = series.diff ?? [];
  return {
    grid: { left: 56, right: 18, top: 18, bottom: 46 },
    tooltip: { trigger: 'axis' },
    xAxis: {
      type: 'category',
      data: series.buckets.map(bucketLabel),
      axisLabel: { color: MUTED, fontSize: 10, interval: 3 },
      axisLine: { lineStyle: { color: LINE } },
    },
    yAxis: {
      type: 'value',
      axisLabel: { color: MUTED, fontSize: 10 },
      splitLine: { lineStyle: { color: LINE } },
    },
    series: [
      {
        name: '差异（实绩-计划）',
        type: 'bar',
        data: diff,
        connectNulls: false,
        itemStyle: {
          color: (params: { value: string | null }) =>
            params.value != null && params.value.startsWith('-') ? RED : GREENISH,
        },
      },
    ],
  };
}

const GREENISH = '#3e705b';

/** 分类构成饼图。 */
export function buildCompositionOption(series: MetricSeriesView) {
  return {
    tooltip: { trigger: 'item' },
    legend: { bottom: 0, textStyle: { color: MUTED, fontSize: 10 } },
    series: [
      {
        name: series.metric_label,
        type: 'pie',
        radius: ['36%', '66%'],
        center: ['50%', '44%'],
        data: series.composition ?? [],
        label: { color: MUTED, fontSize: 10 },
        itemStyle: { borderColor: '#fff', borderWidth: 1 },
      },
    ],
    color: [RED, BLUE, GOLD, '#8a9a5b', '#6a5d7b'],
  };
}

export function buildChartOption(series: MetricSeriesView, type: ChartType) {
  switch (type) {
    case 'trend':
      return buildTrendOption(series);
    case 'plan-actual':
      return buildPlanActualOption(series);
    case 'variance':
      return buildVarianceOption(series);
    case 'composition':
      return buildCompositionOption(series);
  }
}
