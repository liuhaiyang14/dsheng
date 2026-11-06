// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri）。
// S7 设置屏：掩码与披露、无“测试连接”、保存不发请求、周期=新版本语义、历史快照。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { RUN_2026_10, WS_DEMO_ID, fixtureHistory, fixturePeriodPolicies } from '../api/fixtureData';

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
  it('真实策略以9999年结束时仍按档案绑定ID标记当前版本', async () => {
    const client = new FixtureClient();
    const list = client.periodList.bind(client);
    vi.spyOn(client, 'periodList').mockImplementation(async (workspace) => (await list(workspace)).map((policy) => ({
      ...policy, effective_to: policy.effective_to ?? '9999-12-31',
    })));
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    fireEvent.click(screen.getAllByText('系统设置')[0].closest('button')!);
    const table = await screen.findByTestId('policy-history-table');
    const current = within(table).getByText('v2').closest('tr')!;
    expect(current).toHaveTextContent('当前版本');
    expect(current).toHaveTextContent('至 当前');
    expect(current).not.toHaveTextContent('已闭合');
    expect(within(table).getByText('v1').closest('tr')).toHaveTextContent('已闭合');
  });

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

  it('月度起始日提供1至31并保存31，短月规则清楚，重载后保留设置', async () => {
    const client = new FixtureClient();
    const set = vi.spyOn(client, 'periodSet');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    fireEvent.click(screen.getAllByText('系统设置')[0].closest('button')!);
    expect(screen.queryByLabelText('月度起始日')).toBeNull();
    fireEvent.change(await screen.findByTestId('cadence-select'), { target: { value: 'month' } });
    const start = screen.getByLabelText('月度起始日');
    expect(start).toHaveValue('1');
    expect(start.querySelectorAll('option')).toHaveLength(31);
    expect(screen.getByText(/短月没有该日时取月末/)).toHaveTextContent('之后月份仍按原日号');
    fireEvent.change(start, { target: { value: '31' } });
    fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '2026-11-01' } });
    fireEvent.click(screen.getByTestId('save-policy-button'));
    await waitFor(() => expect(set).toHaveBeenCalledOnce());
    expect(set.mock.calls[0][0].policy).toMatchObject({ cadence: 'month', month_start_day: 31 });
    await waitFor(() => expect(screen.getByTestId('settings-notice')).toHaveTextContent('v3'));
    expect(screen.getByLabelText('月度起始日')).toHaveValue('31');
    expect(screen.getByTestId('policy-history-table')).toHaveTextContent('每月 31 日');
    expect((await client.periodList(WS_DEMO_ID)).find((policy) => policy.version === 3)?.policy.month_start_day).toBe(31);
  });

  it('旧月度策略缺少起始日时默认1，日周季隐藏该字段且切回月度保留草稿', async () => {
    const client = new FixtureClient();
    await client.periodSet({ schema_version: '1', operation: 'period_policy_set', workspace_id: WS_DEMO_ID,
      expected_revision: 4, policy: { ...fixturePeriodPolicies[1].policy, cadence: 'month', effective_from: '2026-11-01' } });
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    fireEvent.click(screen.getAllByText('系统设置')[0].closest('button')!);
    expect(await screen.findByLabelText('月度起始日')).toHaveValue('1');
    fireEvent.change(screen.getByLabelText('月度起始日'), { target: { value: '15' } });
    for (const cadence of ['day', 'week', 'quarter']) {
      fireEvent.change(screen.getByTestId('cadence-select'), { target: { value: cadence } });
      expect(screen.queryByLabelText('月度起始日')).toBeNull();
    }
    fireEvent.change(screen.getByTestId('cadence-select'), { target: { value: 'month' } });
    expect(screen.getByLabelText('月度起始日')).toHaveValue('15');
  });

  it('切换系统粒度后页眉选择匹配数据，无匹配数据时清空当前结果', async () => {
    const client = new FixtureClient();
    const analyze = vi.spyOn(client, 'analyze');
    const show = vi.spyOn(client, 'historyShow');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await waitFor(() => expect(screen.getByTestId('saved-period-select')).toHaveValue(RUN_2026_10));
    expect(screen.getByTestId('saved-period-select').querySelectorAll('option')).toHaveLength(1);
    fireEvent.click(screen.getAllByText('系统设置')[0].closest('button')!);
    fireEvent.change(await screen.findByTestId('cadence-select'), { target: { value: 'month' } });
    fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '2026-11-01' } });
    fireEvent.click(screen.getByTestId('save-policy-button'));
    await waitFor(() => expect(screen.getByTestId('saved-period-select')).toHaveValue(fixtureHistory[1].run_id));
    expect(screen.getByTestId('saved-period-select')).toHaveTextContent('2026年9月');
    expect(screen.getByTestId('saved-period-select')).not.toHaveTextContent('2026年10月');
    expect(show).toHaveBeenLastCalledWith(WS_DEMO_ID, fixtureHistory[1].run_id);
    fireEvent.change(screen.getByTestId('cadence-select'), { target: { value: 'week' } });
    fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '2026-12-01' } });
    fireEvent.click(screen.getByTestId('save-policy-button'));
    await waitFor(() => expect(screen.getByTestId('saved-period-select')).toBeDisabled());
    await waitFor(() => expect(screen.getByTestId('saved-period-select')).toHaveTextContent('暂无已保存数据'));
    expect(screen.getByTestId('saved-period-select')).toHaveValue('');
    expect(screen.getByTestId('current-period-label')).toBeEmptyDOMElement();
    fireEvent.click(screen.getAllByText('经营分析')[0].closest('button')!);
    await screen.findByText('尚无分析期间');
    expect(screen.queryByTestId('metric-table')).toBeNull();
    expect(analyze).not.toHaveBeenCalled();
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
  it('期间列表仅含设置粒度并查看最后保存结果', async () => {
    const client = await openScreen('历史查询');
    const showSpy = vi.spyOn(client, 'historyShow');
    await screen.findByTestId('history-table');
    expect(screen.getByTestId('history-table').textContent).toContain('2026年10月1日—2026年10月28日');
    const viewButtons = screen.getAllByTestId(/history-view-/);
    fireEvent.click(viewButtons[0]);
    await waitFor(() => expect(showSpy).toHaveBeenCalledTimes(1));
    await waitFor(() => {
      expect(screen.getByTestId('history-detail').textContent).toContain('任务快照');
    });
    expect(screen.getByTestId('history-detail').textContent).toContain('最新分析结果');
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
