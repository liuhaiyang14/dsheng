// 测试层次：UI 纯组件/状态测试 + 静态约束扫描（无网络、无 Tauri）。
// S8 收口门禁：decimal 字符串不经 Number/parseFloat、页面显示层高精度呈现。
import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { groupDecimal } from '../api/format';

const SRC_ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');

function listSourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir)) {
    if (name === 'test' || name.startsWith('.') || name.endsWith('.css')) continue;
    const full = join(dir, name);
    if (statSync(full).isDirectory()) {
      out.push(...listSourceFiles(full));
    } else if (/\.(ts|tsx)$/.test(name)) {
      out.push(full);
    }
  }
  return out;
}

describe('S8 静态门禁：decimal 字符串不经 Number/parseFloat', () => {
  it('src 业务代码中不存在 parseFloat( 调用', () => {
    const offenders: string[] = [];
    for (const file of listSourceFiles(SRC_ROOT)) {
      const text = readFileSync(file, 'utf8');
      if (/parseFloat\(/.test(text)) {
        offenders.push(file);
      }
    }
    expect(offenders).toEqual([]);
  });

  it('Number( 仅允许出现在 SystemSettingsScreen 的离散整数换算（周起始/财年月）', () => {
    const offenders: string[] = [];
    for (const file of listSourceFiles(SRC_ROOT)) {
      const text = readFileSync(file, 'utf8');
      const uses = text.match(/Number\(/g)?.length ?? 0;
      if (uses === 0) continue;
      if (file.endsWith('SystemSettingsScreen.tsx')) {
        // 周起始日/财年起始月为离散整数（合同允许 JSON integer），最多 2 处。
        expect(uses).toBeLessThanOrEqual(2);
        continue;
      }
      offenders.push(file);
    }
    expect(offenders).toEqual([]);
  });
});

describe('S8 显示层：decimal 字符串原样分组（不经 Number）', () => {
  it('28 位有效数字与 6 位小数完整保留', () => {
    expect(groupDecimal('12345678901234567890123456.123456')).toBe(
      '12,345,678,901,234,567,890,123,456.123456',
    );
    expect(groupDecimal('-0.000001')).toBe('-0.000001');
    expect(groupDecimal('0')).toBe('0');
  });
});
