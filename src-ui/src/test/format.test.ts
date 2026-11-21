import { describe, expect, it } from 'vitest';
import { displayDecimal, favorableOf, diffTone, formatMetricDifference, formatMetricValue, formatMoney, formatPercent, formatRatioPercent, formatSigned, groupDecimal, shiftDecimal } from '../api/format';

describe('十进制数字的展示精度', () => {
  it.each([
    ['0.153', '15.3%'],
    ['0.05', '5.0%'],
    ['-0.05', '-5.0%'],
    ['0.0005', '0.0%'],
    ['-0.0005', '0.0%'],
    ['0.0015', '0.2%'],
    ['0', '0.0%'],
    ['1', '100.0%'],
    ['0.1234567890123456789012345678', '12.3%'],
    ['0.944857673584', '94.5%'],
  ])('将比率 %s 展示为 %s，百分数保留一位 half-even', (input, expected) => {
    expect(formatRatioPercent(input)).toBe(expected);
  });

  it.each([
    ['455022.79245283018867924528302', '455,022.79'],
    ['690391.13274336283185840707965', '690,391.13'],
    ['1234567890123456789012345678.125', '1,234,567,890,123,456,789,012,345,678.12'],
    ['9999999999999999999999999999.995', '10,000,000,000,000,000,000,000,000,000.00'],
    ['1.225', '1.22'], ['1.235', '1.24'], ['1.225000001', '1.23'],
    ['-1.225', '-1.22'], ['-1.235', '-1.24'], ['-0.005', '0.00'],
    ['0', '0.00'], ['-0.00', '0.00'],
  ])('金额 %s 仅舍入展示为 %s，不经过浮点数', (input, expected) => {
    expect(displayDecimal(input, 2)).toBe(expected);
    expect(formatMoney(input)).toBe(`¥ ${expected}`);
    expect(formatMetricValue(input, '元')).toBe(`${expected} 元`);
  });

  it('按冻结的单位签名显示 ratio、单价、数量和时间，兼容旧单位文本', () => {
    expect(formatMetricValue('0.18647', '旧单位', { dimension: 'ratio', base_unit: '1' })).toBe('18.6%');
    expect(formatMetricValue('0.18647', '{"dimension":"ratio","base_unit":"1"}')).toBe('18.6%');
    expect(formatMetricValue('18.647', '%')).toBe('18.6%');
    expect(formatMetricValue('428.125', '元/时')).toBe('428.12 元/时');
    expect(formatMetricValue('428.135', '旧单位', { dimension: 'money_per_time', base_unit: '元/hour' })).toBe('428.14 元/hour');
    expect(formatMetricValue('1234.1234565', '件', { dimension: 'quantity', base_unit: '件' })).toBe('1,234.123456 件');
    expect(formatMetricValue('3.1234555', 'hour', { dimension: 'time', base_unit: 'hour' })).toBe('3.123456 hour');
    expect(formatMetricValue('3.100000', '件')).toBe('3.1 件');
    expect(formatMetricValue('3.1234567', null)).toBe('3.123457');
  });

  it('比例差异为百分点，环比同比为变化率，舍入成零不带负号或加号', () => {
    expect(formatMetricDifference('0.015', '1')).toBe('+1.5pp');
    expect(formatMetricDifference('-1.5', '%')).toBe('-1.5pp');
    expect(formatRatioPercent('0.015')).toBe('1.5%');
    expect(formatMetricDifference('-0.00001', '1')).toBe('0.0pp');
    expect(formatMetricDifference('0.004', '元')).toBe('0.00 元');
    expect(formatSigned('-0.004')).toBe('0.00');
    expect(formatSigned('12.125')).toBe('+12.12');
    expect(formatPercent('33.33')).toBe('33.3%');
  });

  it.each([null, undefined, '', 'NaN', 'Infinity', '1e3', '1,234', '--1'])('缺失或无效值 %s 不被当作零', (input) => {
    expect(displayDecimal(input, 2)).toBeNull();
    expect(formatMetricValue(input, '元')).toBe('—');
    expect(formatMetricValue(input, '1')).toBe('—');
    expect(formatRatioPercent(input ?? null)).toBeNull();
  });

  it('零差异中性，费用减少有利；缺失差异不判达标', () => {
    expect(favorableOf('up', '-0.00')).toBe('neutral');
    expect(favorableOf('down', '0')).toBe('neutral');
    expect(diffTone(favorableOf('down', '-20.1'))).toBe('good');
    expect(diffTone(favorableOf('up', '-20.1'))).toBe('bad');
    expect(diffTone(favorableOf('down', '0'))).toBe('muted');
    expect(favorableOf('up', null)).toBeNull();
    expect(favorableOf('up', '')).toBeNull();
  });

  it('保留大数、负号及原始金额精度', () => {
    expect(groupDecimal('-12345678901234567890.12345678'))
      .toBe('-12,345,678,901,234,567,890.12345678');
    expect(shiftDecimal('123.4', -4)).toBe('0.01234');
    expect(formatRatioPercent(null)).toBeNull();
  });
});
