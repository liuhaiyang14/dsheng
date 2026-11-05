// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri、无 canvas）。
// S4 经营分析屏：指标呈现、decimal 字符串原样展示、不可比原因、缺桶断线、粒度切换。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureAnalysisRun } from '../api/fixtureData';
import { buildChartOption } from '../components/chartOptions';
import type { DshengClient } from '../api';

afterEach(cleanup);
vi.mock('echarts', () => ({
  init: () => ({ setOption: vi.fn(), dispose: vi.fn() }),
}));

const DEMO = '00000000-0000-4000-8000-0000000000a1';

async function openAnalysis() {
  const client = new FixtureClient();
  const calls: string[] = [];
  const spyClient = new Proxy(client, {
    get(target, prop, receiver) {
      const value = Reflect.get(target, prop, receiver);
      if (typeof value === 'function' && prop === 'analyze') {
        return async (...args: unknown[]) => {
          calls.push('analyze');
          return (value as (...a: unknown[]) => Promise<unknown>)(...args);
        };
      }
      return value;
    },
  }) as FixtureClient & DshengClient;
  render(<App runtime={{ mode: 'fixture', client: spyClient }} />);
  await screen.findByTestId('foot-workspace');
  (screen.getAllByText('经营分析')[0].closest('button') as HTMLButtonElement).click();
  await screen.findByTestId('metric-table');
  return { calls };
}

describe('S4 经营分析屏', () => {
  it('渲染指标卡/预实差异表，数值为核心返回的十进制字符串原样展示', async () => {
    await openAnalysis();
    const cards = screen.getByTestId('metric-cards');
    expect(within(cards).getByText('184,600')).toBeTruthy();
    expect(Array.from(cards.querySelectorAll('.value')).map((node) => node.textContent)).toContain('92.4%');
    const table = screen.getByTestId('metric-table');
    // 表内保留目标 210000 → 分组展示，不重算
    expect(within(table).getAllByText('210,000').length).toBeGreaterThan(0);
    // 演示数据标注存在
    expect(screen.getByTestId('analysis-demo-note').textContent).toContain(
      '虚构演示数据',
    );
    expect(screen.getByTestId('analysis-demo-note').textContent).toContain(
      '本地 Rust 核心计算',
    );
  });

  it('不可比时单元格显示 —，表下方有平实文字说明', async () => {
    await openAnalysis();
    const table = screen.getByTestId('metric-table');
    // 合同语义（§1.4）：无合法基期 → 比较对象为 null；单元格仅显示 —
    expect(within(table).getAllByText('—').length).toBeGreaterThan(0);
    // 说明区用平实文字（不再出现技术码 missing_base）
    expect(screen.queryByText(/missing_base/)).toBeNull();
    expect(screen.getByText(/环比／同比说明/)).toBeTruthy();
  });

  it('粒度选择只修改草稿，显式运行本地分析才调用核心', async () => {
    window.localStorage.setItem(`dsheng.source-directory.${DEMO}`, '/tmp/dsheng-demo');
    const { calls } = await openAnalysis();
    const select = screen.getByTestId('grain-select') as HTMLSelectElement;
    select.value = 'week';
    fireEvent.change(select, { target: { value: 'week' } });
    expect(calls).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
    await waitFor(() => {
      expect(calls).toContain('analyze');
    });
    window.localStorage.removeItem(`dsheng.source-directory.${DEMO}`);
  });

  it('空档案显示空状态且不调用 analyze', async () => {
    const client = new FixtureClient();
    const analyzeSpy = vi.spyOn(client, 'analyze');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    const select = screen.getByTestId('workspace-select') as HTMLSelectElement;
    select.value = '00000000-0000-4000-8000-0000000000b2';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => {
      expect(screen.getByTestId('workspace-empty-notice')).toBeTruthy();
    });
    (screen.getAllByText('经营分析')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByText('尚无分析期间');
    expect(analyzeSpy).not.toHaveBeenCalled();
    void DEMO;
  });
});

describe('S4 图表 option：decimal 字符串与缺桶', () => {
  const settlement = fixtureAnalysisRun.series[0];

  it('趋势 option 原样保留 DecimalString 与 null（断线不插值）', () => {
    const option = buildChartOption(settlement, 'trend') as {
      series: { name: string; data: unknown; connectNulls: boolean }[];
    };
    const actual = option.series[0];
    expect(actual.connectNulls).toBe(false);
    expect(actual.data).toBe(settlement.actual);
    expect(actual.data).toContain(null);
    // 字符串数值不经 Number 转换
    expect(actual.data).toContain('6120');
  });

  it('构成 option 使用核心返回的构成数据', () => {
    const cash = fixtureAnalysisRun.series[2];
    const option = buildChartOption(cash, 'composition') as {
      series: { data: { name: string; value: string }[] }[];
    };
    expect(option.series[0].data).toEqual(cash.composition);
  });

  it('差异 option 逐桶差异保持字符串与 null', () => {
    const option = buildChartOption(settlement, 'variance') as {
      series: { data: unknown }[];
    };
    expect(option.series[0].data).toBe(settlement.delta);
    expect(option.series[0].data).toContain(null);
  });
});
