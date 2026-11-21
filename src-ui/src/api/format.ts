import type { UnitSignature } from './types';

/**
 * 展示层格式化工具。
 *
 * 硬约束：decimal 字符串不经 Number/parseFloat 参与计算或比较；
 * 本模块只做「字符串 → 展示字符串」的格式化（half-even 舍入、分组、百分号、正负号），
 * 不产生任何回写业务层的数值。
 */

/** 判断 DecimalString 是否为负（纯字符串前缀判断）。 */
export function isNegativeDecimal(value: string): boolean {
  return value.startsWith('-');
}

/**
 * 千位分组（仅展示）。对整数部分按 3 位插入分隔符，保留符号与小数部分；
 * 输入必须是规范 DecimalString。
 */
export function groupDecimal(value: string): string {
  const negative = isNegativeDecimal(value);
  const body = negative ? value.slice(1) : value;
  const [intPart, fracPart] = body.split('.');
  const grouped = intPart.replace(/\B(?=(\d{3})+(?!\d))/g, ',');
  return `${negative ? '-' : ''}${grouped}${fracPart !== undefined ? `.${fracPart}` : ''}`;
}

/** half-even 展示舍入；只处理字符串/BigInt，绝不更改业务值。 */
export function displayDecimal(value: string | null | undefined, places: number, trim = false): string | null {
  if (value == null || !/^-?\d+(?:\.\d+)?$/.test(value)) return null;
  const negative = isNegativeDecimal(value);
  const [integer, fraction = ''] = (negative ? value.slice(1) : value).split('.');
  const kept = fraction.slice(0, places).padEnd(places, '0');
  const discarded = fraction.slice(places);
  let digits = BigInt(integer + kept);
  const halfway = discarded[0] === '5' && !/[1-9]/.test(discarded.slice(1));
  if (discarded[0] > '5' || (discarded[0] === '5' && (!halfway || digits % 2n !== 0n))) digits += 1n;
  const raw = digits.toString().padStart(places + 1, '0');
  const whole = places ? raw.slice(0, -places) : raw;
  const fractionOut = places ? raw.slice(-places) : '';
  const decimal = trim ? fractionOut.replace(/0+$/, '') : fractionOut;
  return groupDecimal(`${negative && digits !== 0n ? '-' : ''}${whole}${decimal ? `.${decimal}` : ''}`);
}

/** 金额固定两位；百分数固定一位。 */
export function formatMoney(value: string | null): string | null {
  const text = displayDecimal(value, 2);
  return text == null ? null : `¥ ${text}`;
}

export function isZeroDecimal(value: string): boolean {
  return /^-?0+(?:\.0+)?$/.test(value);
}

export function formatSigned(value: string | null): string | null {
  const text = displayDecimal(value, 2);
  return text == null ? null : text.startsWith('-') || isZeroDecimal(text.replaceAll(',', '')) ? text : `+${text}`;
}

/** 输入本身已是百分数值；任务进度保留原精度，避免把未完成的 99.99% 显示为 100%。 */
export function formatPercent(value: string | null): string | null {
  const text = displayDecimal(value, 1);
  return text == null ? null : `${text}%`;
}

/**
 * 小数字符串 ×10^n（纯字符串小数点移位，n 为正右移）。
 * 用于把合同 ratio 口径（0.15 = 15%）转为百分数展示，不经 Number。
 */
export function shiftDecimal(value: string, places: number): string {
  if (places === 0 || value === '0') return value;
  const negative = isNegativeDecimal(value);
  const body = negative ? value.slice(1) : value;
  const [intPart, fracPart = ''] = body.split('.');
  let digits = intPart + fracPart;
  let point = intPart.length + places;
  while (point < 1) {
    digits = `0${digits}`;
    point += 1;
  }
  if (point >= digits.length) {
    digits = `${digits}${'0'.repeat(point - digits.length)}`;
  }
  const newInt = digits.slice(0, point).replace(/^0+(?=\d)/, '') || '0';
  const newFrac = digits.slice(point).replace(/0+$/, '');
  const out = newFrac ? `${newInt}.${newFrac}` : newInt;
  const zeroed = out === '' || /^0+$/.test(out) ? '0' : out;
  return negative && !/^0+$/.test(zeroed) ? `-${zeroed}` : zeroed;
}

/** ratio（如 0.153）→ 百分数展示（15.3%）。 */
export function formatRatioPercent(value: string | null): string | null {
  if (value == null || !/^-?\d+(?:\.\d+)?$/.test(value)) return null;
  return formatPercent(shiftDecimal(value, 2));
}

/**
 * favorable 判定（data-period-import §1.1）：
 * favorable = (direction=up ∧ delta>0) ∨ (direction=down ∧ delta<0)；
 * neutral 不判；delta 缺失（任一操作数缺）时为 null。
 * 纯字符串符号判断，不经 Number。
 */
export type Favorable = 'favorable' | 'unfavorable' | 'neutral' | null;

export function favorableOf(
  direction: 'up' | 'down' | 'neutral',
  delta: string | null,
): Favorable {
  if (delta == null || !/^-?\d+(?:\.\d+)?$/.test(delta)) return null;
  const negative = isNegativeDecimal(delta);
  if (direction === 'neutral' || isZeroDecimal(delta)) return 'neutral';
  if (direction === 'up') return negative ? 'unfavorable' : 'favorable';
  return negative ? 'favorable' : 'unfavorable';
}

/** 差异展示配色：按 favorable（有利绿/不利红/中性与缺值灰）。 */
export function diffTone(favorable: Favorable): 'bad' | 'good' | 'warn' | 'muted' {
  switch (favorable) {
    case 'favorable':
      return 'good';
    case 'unfavorable':
      return 'bad';
    case 'neutral':
      return 'muted';
    default:
      return 'muted';
  }
}

/** 老视图有时把单位对象序列化在 unit 中；优先使用本版本的单位签名。 */
function displayUnit(unit: string | null, signature?: UnitSignature | null): { base: string; dimension?: string } {
  if (signature) return { base: signature.base_unit, dimension: signature.dimension };
  if (unit) {
    try {
      const parsed = JSON.parse(unit) as UnitSignature;
      if (typeof parsed?.base_unit === 'string') return { base: parsed.base_unit, dimension: parsed.dimension };
    } catch { /* 普通单位文本。 */ }
  }
  return { base: unit ?? '' };
}

export function metricDisplayUnit(unit: string | null, signature?: UnitSignature | null): string {
  const { base, dimension } = displayUnit(unit, signature);
  return dimension === 'ratio' || (!dimension && base === '1') ? '%' : base;
}

/** 金额两位、百分数一位；数量/时间及未知单位最多六位，不隐式当成金额。 */
export function formatMetricNumber(
  value: string | null | undefined,
  unit: string | null,
  signature?: UnitSignature | null,
  difference = false,
): string {
  if (value == null || !/^-?\d+(?:\.\d+)?$/.test(value)) return '—';
  const { base, dimension } = displayUnit(unit, signature);
  const ratio = dimension === 'ratio' || (!dimension && base === '1');
  if (ratio || base === '%') {
    const text = displayDecimal(ratio ? shiftDecimal(value, 2) : value, 1);
    return text == null ? '—' : `${text}${difference ? 'pp' : '%'}`;
  }
  const money = dimension?.startsWith('money') || (!dimension && /^(?:元|万元|亿元|CNY|RMB|USD|EUR|¥|￥|\$|€)(?:\s*\/.*)?$/.test(base));
  return displayDecimal(value, money ? 2 : 6, !money) ?? '—';
}

export function formatMetricValue(
  value: string | null | undefined,
  unit: string | null,
  signature?: UnitSignature | null,
  difference = false,
): string {
  const text = formatMetricNumber(value, unit, signature, difference);
  const suffix = metricDisplayUnit(unit, signature);
  return text === '—' || !suffix || suffix === '%' ? text : `${text} ${suffix}`;
}

export function formatMetricDifference(value: string | null, unit: string | null, signature?: UnitSignature | null): string {
  const text = formatMetricValue(value, unit, signature, true);
  return text === '—' || text.startsWith('-') || isZeroDecimal((text.match(/^-?[\d,.]+/)?.[0] ?? '').replaceAll(',', '')) ? text : `+${text}`;
}

/** 截断 64 位 hash 供展示。 */
export function shortHash(hash: string): string {
  return hash.length > 12 ? `${hash.slice(0, 8)}…` : hash;
}
