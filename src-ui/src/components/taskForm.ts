/** 金额只做字符串规范化，避免浮点转换丢失精度；空输入保持空字符串。 */
export function normalizeTaskDecimal(input: string): string | null {
  const text = input.trim();
  if (!text) return '';
  if (!/^-?\d+(?:\.\d{0,6})?$/.test(text)) return null;
  const negative = text.startsWith('-');
  const [rawWhole, rawFraction = ''] = (negative ? text.slice(1) : text).split('.');
  const whole = rawWhole.replace(/^0+(?=\d)/, '');
  const fraction = rawFraction.replace(/0+$/, '');
  const value = fraction ? `${whole}.${fraction}` : whole;
  return negative && value !== '0' ? `-${value}` : value;
}
