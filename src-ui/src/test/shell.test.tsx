// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri）。
// S3 应用壳：七屏导航、企业切换与空档案隔离（用例 34）、未连接核心空状态。
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { AppProvider, useApp } from '../state/store';
import { FixtureClient } from '../api/fixtureClient';
import { EmptyState } from '../components/EmptyState';
import type { ReactNode } from 'react';

afterEach(cleanup);

function renderWithFixture() {
  return render(<App runtime={{ mode: 'fixture', client: new FixtureClient() }} />);
}

function SwitchProbe() {
  const app = useApp();
  return (
    <div>
      <button type="button" onClick={() => void app.switchWorkspace('00000000-0000-4000-8000-0000000000b2')}>
        switch-empty
      </button>
      <button type="button" onClick={() => void app.switchWorkspace('00000000-0000-4000-8000-0000000000a1')}>
        switch-demo
      </button>
      <span data-testid="probe-tasks">{app.data.tasks.length}</span>
      <span data-testid="probe-history">{app.data.history.length}</span>
      <span data-testid="probe-providers">{app.data.providers.length}</span>
      <span data-testid="probe-screen">{app.screen}</span>
    </div>
  );
}

describe('S3 应用壳：导航与屏幕切换', () => {
  it('七屏导航均可达且面包屑跟随', async () => {
    renderWithFixture();
    await screen.findAllByText('数据源');
    const screens: [string, string][] = [
      ['经营分析', 'analysis'],
      ['经营问题诊断', 'diagnosis'],
      ['任务单', 'task'],
      ['历史查询', 'history'],
      ['Provider 设置', 'provider'],
      ['系统设置', 'system-settings'],
    ];
    for (const [label, testId] of screens) {
      const navButton = screen.getAllByText(label)[0].closest('button')!;
      navButton.click();
      // eslint-disable-next-line no-await-in-loop
      await waitFor(() => {
        expect(screen.getByTestId('crumb-screen').textContent).toBe(label);
      });
      void testId;
    }
    expect(screen.getByRole('navigation')).toBeTruthy();
  });
});

describe('S3 企业切换与空档案隔离（用例 34）', () => {
  it('切到空白档案：数据屏全空状态；切回原企业只见原企业数据', async () => {
    const { container } = render(
      <AppProvider runtime={{ mode: 'fixture', client: new FixtureClient() }}>
        <SwitchProbe />
      </AppProvider>,
    );
    // 初始：示例制造企业，有任务/历史/Provider
    await waitFor(() => {
      expect(container.querySelector('[data-testid="probe-tasks"]')!.textContent).not.toBe('0');
    });
    const demoTaskCount = container.querySelector('[data-testid="probe-tasks"]')!.textContent;
    const demoHistoryCount = container.querySelector('[data-testid="probe-history"]')!.textContent;
    expect(demoHistoryCount).not.toBe('0');
    expect(container.querySelector('[data-testid="probe-providers"]')!.textContent).not.toBe('0');

    // 切到空白档案
    container.querySelector<HTMLButtonElement>('button')!.click();
    await waitFor(() => {
      expect(container.querySelector('[data-testid="probe-tasks"]')!.textContent).toBe('0');
      expect(container.querySelector('[data-testid="probe-history"]')!.textContent).toBe('0');
      expect(container.querySelector('[data-testid="probe-providers"]')!.textContent).toBe('0');
    });

    // 切回原企业：只恢复该企业自己的数据（不混合空档案）
    const buttons = container.querySelectorAll('button');
    buttons[1].click();
    await waitFor(() => {
      expect(container.querySelector('[data-testid="probe-tasks"]')!.textContent).toBe(demoTaskCount);
      expect(container.querySelector('[data-testid="probe-history"]')!.textContent).toBe(demoHistoryCount);
    });
  });

  it('App 壳：空白档案显示空档案提示与各数据屏空状态；切回后消失', async () => {
    renderWithFixture();
    // 默认示例企业：无空档案提示
    await screen.findByTestId('foot-workspace');
    expect(screen.getByTestId('foot-workspace').textContent).toBe('示例制造企业');
    expect(screen.queryByTestId('workspace-empty-notice')).toBeNull();

    const select = screen.getByTestId('workspace-select') as HTMLSelectElement;
    select.value = '00000000-0000-4000-8000-0000000000b2';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => {
      expect(screen.getByTestId('workspace-empty-notice')).toBeTruthy();
    });
    expect(screen.getByTestId('foot-workspace').textContent).toBe('新建空白档案');
    expect(screen.getAllByTestId('empty-state').length).toBeGreaterThan(0);

    // 切回示例企业：空提示消失（扫描摘要不再由空目录扫描自动填充——
    // 真实结果只在用户显式导入后出现，防止覆盖真实扫描的修复）
    select.value = '00000000-0000-4000-8000-0000000000a1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => {
      expect(screen.queryByTestId('workspace-empty-notice')).toBeNull();
    });
    expect(screen.getByTestId('foot-workspace').textContent).toBe('示例制造企业');
  });
});

describe('S3 未连接核心空状态', () => {
  it('无 Tauri 且未显式启用 fixture 时显示「未连接核心」，不静默用假数据', () => {
    // 清掉可能存在的显式 fixture 开关
    window.localStorage.removeItem('dsheng_fixtures');
    const url = new URL(window.location.href);
    url.searchParams.delete('dsheng-fixtures');
    window.history.replaceState({}, '', url);
    vi.stubGlobal('__TAURI__', undefined);
    render(<App />);
    expect(screen.getByTestId('core-missing')).toBeTruthy();
    expect(screen.getByText('未连接本地核心')).toBeTruthy();
    // 不出现任何演示数据内容
    expect(screen.queryByText('示例制造企业')).toBeNull();
    expect(screen.queryByTestId('demo-stamp')).toBeNull();
    vi.unstubAllGlobals();
  });
});

void EmptyState;
void ({} as ReactNode);
