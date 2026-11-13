import '@testing-library/jest-dom/vitest';

// vitest 全局环境见 vite.config.ts 的 test 段（jsdom + globals）。

// Node 25 在传入无效 --localstorage-file 时会给 jsdom 注入一个不完整的
// localStorage 对象（存在属性但没有 Storage 方法）。桌面端不受影响，但会
// 让依赖本地目录记忆的 UI 回归测试在 beforeEach 就失败；测试环境补一个
// 确定性的内存实现，保持和浏览器 Storage API 相同的最小行为。
try {
  const storage = window.localStorage as Partial<Storage>;
  if (typeof storage.getItem !== 'function'
    || typeof storage.setItem !== 'function'
    || typeof storage.removeItem !== 'function'
    || typeof storage.clear !== 'function') {
    const values = new Map<string, string>();
    const shim: Storage = {
      get length() { return values.size; },
      clear() { values.clear(); },
      getItem(key: string) { return values.has(key) ? values.get(key)! : null; },
      key(index: number) { return [...values.keys()][index] ?? null; },
      removeItem(key: string) { values.delete(key); },
      setItem(key: string, value: string) { values.set(String(key), String(value)); },
    };
    Object.defineProperty(window, 'localStorage', { configurable: true, value: shim });
  }
} catch {
  // 某些 jsdom URL 配置会拒绝访问 localStorage；相关代码本身已有 try/catch。
}
