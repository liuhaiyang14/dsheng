import { describe, expect, it } from 'vitest';
import { buildCompositionOption, buildPlanActualOption, buildTrendOption, buildVarianceOption } from '../components/chartOptions';
import type { MetricSeriesView } from '../api';

const series: MetricSeriesView = {
  metric_key: 'revenue', metric_label: '营业收入', unit: '元',
  unit_signature: { dimension: 'money', base_unit: '元' },
  buckets: ['2026-10-01', '2026-10-02', '2026-10-03'],
  actual: ['455022.79245283018867924528302', null, '12345678901234567890.125'],
  plan: ['690391.13274336283185840707965', null, '10'],
  delta: ['-235368.34029053264317916179663', null, '-0.00'],
  composition: [{ name: '分类甲', value: '455022.79245283018867924528302' }],
};

describe('图表展示和数据表口径一致', () => {
  it('tooltip 从原序列精确取值，保留缺桶且不改动原数组', () => {
    const before = structuredClone(series);
    const option = buildTrendOption(series);
    expect(option.series[0].data).toBe(series.actual);
    expect(option.series[0].connectNulls).toBe(false);
    expect(option.tooltip.formatter([{ dataIndex: 0, seriesIndex: 0 }, { dataIndex: 0, seriesIndex: 1 }]))
      .toBe('2026-10-01\n实绩：455,022.79 元\n计划：690,391.13 元');
    expect(option.tooltip.formatter({ dataIndex: 1, seriesIndex: 0 })).toContain('实绩：—');
    expect(option.tooltip.formatter({ dataIndex: 2, seriesIndex: 0 })).toContain('12,345,678,901,234,567,890.12 元');
    expect(option.yAxis.axisLabel.formatter(1200)).toBe('1,200.00');
    expect(buildPlanActualOption(series).tooltip.formatter({ dataIndex: 0, seriesIndex: 0 })).toContain('计划：690,391.13 元');
    expect(buildCompositionOption(series).tooltip.formatter({ dataIndex: 0 })).toBe('分类甲：455,022.79 元');
    expect(series).toEqual(before);
  });

  it('ratio 坐标/tooltip 为百分比，差异图为百分点，旧百分数单位不重复乘百', () => {
    const ratio: MetricSeriesView = {
      ...series, unit: '1', unit_signature: { dimension: 'ratio', base_unit: '1' },
      actual: ['0.944857673584'], plan: ['0.9'], delta: ['0.044857673584'],
    };
    const trend = buildTrendOption(ratio);
    expect(trend.tooltip.formatter({ dataIndex: 0, seriesIndex: 0 })).toContain('94.5%');
    expect(trend.yAxis.axisLabel.formatter(0.15)).toBe('15.0%');
    expect(trend.yAxis.name).toBe('%');
    const variance = buildVarianceOption(ratio);
    expect(variance.tooltip.formatter({ dataIndex: 0, seriesIndex: 0 })).toContain('4.5pp');
    expect(variance.yAxis.axisLabel.formatter(0.015)).toBe('1.5pp');
    expect(variance.yAxis.name).toBe('百分点');
    const legacy = buildTrendOption({ ...ratio, unit: '%', unit_signature: undefined, actual: ['94.4857673584'] });
    expect(legacy.tooltip.formatter({ dataIndex: 0, seriesIndex: 0 })).toContain('94.5%');
    // 比例构成的旧合同没有各分类单位，不擅自把绝对构成值标为百分比。
    expect(buildCompositionOption(ratio).tooltip.formatter({ dataIndex: 0 })).not.toContain('%');
  });

  it('费用下降为有利色，零、缺值及未知改善方向为中性色', () => {
    const expense = buildVarianceOption(series, 'down').series[0].itemStyle.color;
    const income = buildVarianceOption(series, 'up').series[0].itemStyle.color;
    const unknown = buildVarianceOption(series).series[0].itemStyle.color;
    expect(expense({ dataIndex: 0 })).toBe('#3e705b');
    expect(income({ dataIndex: 0 })).toBe('#a83c32');
    expect(expense({ dataIndex: 1 })).toBe('#687278');
    expect(expense({ dataIndex: 2 })).toBe('#687278');
    expect(unknown({ dataIndex: 0 })).toBe('#687278');
  });
});
