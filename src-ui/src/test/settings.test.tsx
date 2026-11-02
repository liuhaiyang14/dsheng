// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri）。
// S7 设置屏：掩码与披露、无“测试连接”、保存不发请求、周期=新版本语义、历史快照。
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';

afterEach(cleanup);

async function openScreen(label: string) {
  const client = new FixtureClient();
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  (screen.getAllByText(label)[0].closest('button') as HTMLButtonElement).click();
  return client;
}

describe('S7 Provider 设置', () => {
  it('掩码 Key + 明文存储风险披露 + 无“测试连接”按钮 + 保存不发请求', async () => {
    const client = new FixtureClient();
    const setSpy = vi.spyOn(client, 'providerSet');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    (screen.getAllByText('Provider 设置')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByTestId('provider-risk-disclosure');

    // 明文存储风险披露
    const disclosure = screen.getByTestId('provider-risk-disclosure');
    expect(disclosure.textContent).toContain('明文');
    expect(disclosure.textContent).toContain('本机 SQLite');
    expect(disclosure.textContent).toContain('不发');

    // 无“测试连接”按钮（任何变体）
    expect(screen.queryByText('测试连接')).toBeNull();
    expect(screen.queryByText(/测试连接/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /测试/ })).toBeNull();

    // 掩码呈现：key_present 状态，不回显 Key
    await waitFor(() => {
      expect(screen.getByTestId('key-presence').textContent).toContain('key_present = true');
    });
    const keyInput = screen.getByTestId('provider-key-input') as HTMLInputElement;
    expect(keyInput.value).toBe('');
    expect(keyInput.getAttribute('type')).toBe('password');

    // 保存（keep 模式，无 Key 通道）→ 调 providerSet，无网络
    fireEvent.click(screen.getByText('保存 Provider'));
    await waitFor(() => {
      expect(setSpy).toHaveBeenCalledTimes(1);
    });
    const [input, apiKey] = setSpy.mock.calls[0];
    expect(input.key_action).toBe('keep');
    expect(apiKey).toBeUndefined();
    expect(JSON.stringify(input)).not.toContain('demo-local-key');
    await waitFor(() => {
      expect(screen.getByTestId('provider-notice').textContent).toContain('未发起任何连接测试');
    });
  });

  it('输入新 Key 走 replace + 单独通道；清空 Key 有确认', async () => {
    const client = new FixtureClient();
    const setSpy = vi.spyOn(client, 'providerSet');
    const clearSpy = vi.spyOn(client, 'providerClearKey');
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    (screen.getAllByText('Provider 设置')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByTestId('provider-key-input');
    fireEvent.change(screen.getByTestId('provider-key-input'), {
      target: { value: 'new-demo-key' },
    });
    fireEvent.click(screen.getByText('保存 Provider'));
    await waitFor(() => expect(setSpy).toHaveBeenCalledTimes(1));
    expect(setSpy.mock.calls[0][0].key_action).toBe('replace');
    expect(setSpy.mock.calls[0][1]).toBe('new-demo-key');
    expect(JSON.stringify(setSpy.mock.calls[0][0])).not.toContain('new-demo-key');

    fireEvent.click(screen.getByText('清空 Key'));
    await waitFor(() => expect(clearSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      expect(screen.getByTestId('key-presence').textContent).toContain('key_present = false');
    });
    confirmSpy.mockRestore();
  });

  it('空档案 Provider 列表为空并显示未配置', async () => {
    const client = new FixtureClient();
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    const select = screen.getByTestId('workspace-select') as HTMLSelectElement;
    select.value = '00000000-0000-4000-8000-0000000000b2';
    fireEvent.change(select);
    await waitFor(() => expect(screen.getByTestId('workspace-empty-notice')).toBeTruthy());
    (screen.getAllByText('Provider 设置')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByText('未配置');
    expect(screen.getByTestId('key-presence').textContent).toContain('key_present = false');
  });
});

describe('S7 系统设置', () => {
  it('保存周期设置建立新版本并提示旧期间不改写', async () => {
    const client = new FixtureClient();
    const setSpy = vi.spyOn(client, 'periodSet');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    (screen.getAllByText('系统设置')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByTestId('save-policy-button');
    expect(screen.getByTestId('policy-version-note').textContent).toContain('新的 period_policy 版本');
    fireEvent.change(screen.getByTestId('cadence-select'), { target: { value: 'week' } });
    fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '2026-11-01' } });
    fireEvent.click(screen.getByTestId('save-policy-button'));
    await waitFor(() => {
      expect(setSpy).toHaveBeenCalledTimes(1);
    });
    expect(setSpy.mock.calls[0][0].policy.cadence).toBe('week');
    await waitFor(() => {
      expect(screen.getByTestId('settings-notice').textContent).toContain('v3');
    });
    // 版本历史表出现闭合版本
    await waitFor(() => {
      expect(screen.getByTestId('policy-history-table').textContent).toContain('已闭合');
    });
  });

  it('生效日期缺失时阻止保存', async () => {
    const client = new FixtureClient();
    const setSpy = vi.spyOn(client, 'periodSet');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    (screen.getAllByText('系统设置')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByTestId('save-policy-button');
    fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '' } });
    fireEvent.click(screen.getByTestId('save-policy-button'));
    await waitFor(() => {
      expect(screen.getByTestId('settings-notice').textContent).toContain('生效日期');
    });
    expect(setSpy).not.toHaveBeenCalled();
  });
});

describe('S7 历史查询', () => {
  it('期间列表 + 查看当前快照（revision/任务快照号）', async () => {
    const client = await openScreen('历史查询');
    const showSpy = vi.spyOn(client, 'historyShow');
    await screen.findByTestId('history-table');
    expect(screen.getByTestId('history-table').textContent).toContain('2026-10-01 至 2026-10-28');
    const viewButtons = screen.getAllByTestId(/history-view-/);
    fireEvent.click(viewButtons[0]);
    await waitFor(() => expect(showSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      expect(screen.getByTestId('history-detail').textContent).toContain('任务快照 #');
    });
    expect(screen.getByTestId('history-detail').textContent).toContain('分析 revision');
  });

  it('空档案显示空状态', async () => {
    const client = new FixtureClient();
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    const select = screen.getByTestId('workspace-select') as HTMLSelectElement;
    select.value = '00000000-0000-4000-8000-0000000000b2';
    fireEvent.change(select);
    await waitFor(() => expect(screen.getByTestId('workspace-empty-notice')).toBeTruthy());
    (screen.getAllByText('历史查询')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByText('尚无已保存期间');
  });
});
