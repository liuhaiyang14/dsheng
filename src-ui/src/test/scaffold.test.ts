// 测试层次：UI 纯组件/状态测试（无网络、无 Tauri、无 Provider）。
// S1 冒烟：脚手架可编译、测试环境可用。
import { describe, expect, it } from 'vitest';

describe('S1 脚手架冒烟', () => {
  it('vitest 环境可用', () => {
    expect(1 + 1).toBe(2);
  });
});
