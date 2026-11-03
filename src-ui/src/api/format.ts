/**
 * 展示层格式化工具。
 *
 * 硬约束：decimal 字符串不经 Number/parseFloat 参与计算或比较；
 * 本模块只做「字符串 → 展示字符串」的格式化（分组、百分号、正负号），
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

/** 金额展示：¥ + 分组。 */
export function formatMoney(value: string | null): string | null {
  if (value == null) return null;
  return `¥ ${groupDecimal(value)}`;
}

/** 带符号展示：正数加 +，负数保留 -。 */
export function formatSigned(value: string | null): string | null {
  if (value == null) return null;
  return isNegativeDecimal(value) ? groupDecimal(value) : `+${groupDecimal(value)}`;
}

/** 百分比展示：数值本身已是百分数值字符串。 */
export function formatPercent(value: string | null): string | null {
  if (value == null) return null;
  return `${groupDecimal(value)}%`;
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
  const newInt = digits.slice(0, point);
  let newFrac = digits.slice(point).replace(/0+$/, '');
  const out = newFrac ? `${newInt}.${newFrac}` : newInt;
  const zeroed = out === '' || /^0+$/.test(out) ? '0' : out;
  return negative && !/^0+$/.test(zeroed) ? `-${zeroed}` : zeroed;
}

/** ratio（如 0.153）→ 百分数展示（15.3%）。 */
export function formatRatioPercent(value: string | null): string | null {
  if (value == null) return null;
  return `${groupDecimal(shiftDecimal(value, 2))}%`;
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
  if (delta == null) return null;
  const negative = isNegativeDecimal(delta);
  if (direction === 'neutral') return 'neutral';
  if (direction === 'up') return negative ? 'unfavorable' : 'favorable';
  return negative ? 'favorable' : 'unfavorable';
}

/** 差异展示配色：按 favorable（有利绿/不利红/中性黄/缺值灰）。 */
export function diffTone(favorable: Favorable): 'bad' | 'good' | 'warn' | 'muted' {
  switch (favorable) {
    case 'favorable':
      return 'good';
    case 'unfavorable':
      return 'bad';
    case 'neutral':
      return 'warn';
    default:
      return 'muted';
  }
}

/** 通用数值展示（含单位），null → '—'。 */
export function formatMetricValue(
  value: string | null,
  unit: string | null,
): string {
  if (value == null) return '—';
  const text = groupDecimal(value);
  return unit ? `${text} ${unit}` : text;
}

/** 截断 64 位 hash 供展示。 */
export function shortHash(hash: string): string {
  return hash.length > 12 ? `${hash.slice(0, 8)}…` : hash;
}
