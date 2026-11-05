import { describe, expect, it } from 'vitest';
import { formatRatioPercent, groupDecimal, shiftDecimal } from '../api/format';

describe('十进制数字的展示精度', () => {
  it.each([
    ['0.153', '15.3%'],
    ['0.05', '5%'],
    ['-0.05', '-5%'],
    ['0.0005', '0.05%'],
    ['-0.0005', '-0.05%'],
    ['0', '0%'],
    ['1', '100%'],
    ['0.1234567890123456789012345678', '12.34567890123456789012345678%'],
  ])('将比率 %s 展示为 %s，不补前导零或丢失小数', (input, expected) => {
    expect(formatRatioPercent(input)).toBe(expected);
  });

  it('保留大数、负号及原始金额精度', () => {
    expect(groupDecimal('-12345678901234567890.12345678'))
      .toBe('-12,345,678,901,234,567,890.12345678');
    expect(shiftDecimal('123.4', -4)).toBe('0.01234');
    expect(formatRatioPercent(null)).toBeNull();
  });
});
