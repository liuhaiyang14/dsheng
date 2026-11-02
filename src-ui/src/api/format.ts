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

/** 差异展示：down=低于目标（红），up=高于目标（绿），neutral=中性。 */
export function diffTone(direction: 'up' | 'down' | 'neutral' | null): 'bad' | 'good' | 'warn' | 'muted' {
  switch (direction) {
    case 'down':
      return 'bad';
    case 'up':
      return 'good';
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
