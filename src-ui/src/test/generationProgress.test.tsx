// 测试层次：UI 纯组件测试（fixture 通道注入事件序列；无网络、无 Tauri）。
// S5 生成进度状态面板：事件渲染 / 30s tick 占位提示 / 取消按钮调用 /
// 终态停止（active=false 卸载）；FixtureClient.generationCancel 记录断言。
import { cleanup, fireEvent, render, screen, act } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { GenerationProgressPanel } from '../components/GenerationProgressPanel';
import { FixtureClient } from '../api/fixtureClient';
import type { GenerationProgressEvent } from '../api/types';

afterEach(cleanup);

interface Channel {
  subscribe: (listener: (event: GenerationProgressEvent) => void) => () => void;
  emit: (event: GenerationProgressEvent) => void;
  listeners: number;
}

/** fixture 事件通道：收集订阅者，测试侧推进事件序列。 */
function eventChannel(): Channel {
  const listeners = new Set<(event: GenerationProgressEvent) => void>();
  const state = { listeners: 0 };
  return {
    get listeners() {
      return state.listeners;
    },
    subscribe(listener) {
      listeners.add(listener);
      state.listeners = listeners.size;
      return () => {
        listeners.delete(listener);
        state.listeners = listeners.size;
      };
    },
    emit(event) {
      listeners.forEach((l) => l(event));
    },
  };
}

function chunk(
  attemptId: string,
  seq: number,
  bytes: number,
  elapsedMs: number,
): GenerationProgressEvent {
  return {
    attempt_id: attemptId,
    chunk_seq: seq,
    bytes,
    elapsed_ms: elapsedMs,
    stage: 'receiving',
  };
}

describe('S5 生成进度面板（事件渲染）', () => {
  it('事件序列驱动 阶段/字节/块数/耗时 展示（多块累积）', async () => {
    const channel = eventChannel();
    render(
      <GenerationProgressPanel
        active
        subscribe={channel.subscribe}
        tickMs={30_000}
      />,
    );
    // 初始：等待响应占位
    expect(screen.getByTestId('generation-progress-stage').textContent).toContain(
      '等待响应',
    );
    expect(
      screen.getByTestId('generation-progress-stats').textContent,
    ).toContain('已接收 0 B');
    // 第 1 块
    act(() => {
      channel.emit(chunk('att-1', 1, 512, 1_200));
    });
    expect(
      screen.getByTestId('generation-progress-stats').textContent,
    ).toContain('已接收 512 B');
    expect(
      screen.getByTestId('generation-progress-stats').textContent,
    ).toContain('第 1 块');
    expect(
      screen.getByTestId('generation-progress-stats').textContent,
    ).toContain('耗时 1s');
    // 第 2 块（KB 展示 + 块数/耗时递增）
    act(() => {
      channel.emit(chunk('att-1', 7, 409_600, 65_000));
    });
    const stats = screen.getByTestId('generation-progress-stats').textContent ?? '';
    expect(stats).toContain('400.0 KB');
    expect(stats).toContain('第 7 块');
    expect(stats).toContain('耗时 65s');
    expect(screen.getByTestId('generation-progress-stage').textContent).toContain(
      'receiving',
    );
  });

  it('取消按钮以事件 payload 的 attempt_id 调用既有 cancel；已请求后禁用', async () => {
    const channel = eventChannel();
    const onCancel = vi.fn();
    render(
      <GenerationProgressPanel active subscribe={channel.subscribe} onCancel={onCancel} />,
    );
    // 无事件：取消按钮禁用（拿不到 attempt_id）
    const button = screen.getByTestId(
      'generation-progress-cancel',
    ) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    act(() => {
      channel.emit(chunk('att-cancel', 3, 9_216, 4_000));
    });
    expect(button.disabled).toBe(false);
    fireEvent.click(button);
    expect(onCancel).toHaveBeenCalledTimes(1);
    expect(onCancel).toHaveBeenCalledWith('att-cancel');
    // 已请求取消：按钮转入已请求态并禁用（重复点击不生效）
    expect(button.disabled).toBe(true);
    expect(button.textContent).toContain('已请求取消');
    fireEvent.click(button);
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it('30 秒本地 tick：无事件显示“仍在等待”，有事件后显示“仍在接收中”', () => {
    vi.useFakeTimers();
    try {
      const channel = eventChannel();
      render(
        <GenerationProgressPanel active subscribe={channel.subscribe} tickMs={30_000} />,
      );
      expect(screen.getByTestId('generation-progress-hint').textContent).toContain(
        '等待响应',
      );
      act(() => {
        vi.advanceTimersByTime(30_000);
      });
      expect(screen.getByTestId('generation-progress-hint').textContent).toContain(
        '仍在等待响应…（已等待约 30s）',
      );
      // 事件到达后：接收中提示
      act(() => {
        channel.emit(chunk('att-t', 1, 100, 500));
      });
      expect(screen.getByTestId('generation-progress-hint').textContent).toContain(
        '正在接收响应流',
      );
      act(() => {
        vi.advanceTimersByTime(30_000);
      });
      expect(screen.getByTestId('generation-progress-hint').textContent).toContain(
        '仍在接收中',
      );
    } finally {
      vi.useRealTimers();
    }
  });

  it('终态停止：active=false 卸载面板并取消订阅', () => {
    const channel = eventChannel();
    const { rerender } = render(
      <GenerationProgressPanel active subscribe={channel.subscribe} />,
    );
    expect(screen.getByTestId('generation-progress-panel')).toBeTruthy();
    rerender(
      <GenerationProgressPanel active={false} subscribe={channel.subscribe} />,
    );
    expect(screen.queryByTestId('generation-progress-panel')).toBeNull();
    expect(channel.listeners).toBe(0);
  });
});

describe('S5 取消通道（FixtureClient）', () => {
  it('generationCancel 记录 attempt 并返回既有取消语义结果', async () => {
    const client = new FixtureClient();
    const result = await client.generationCancel('00000000-0000-4000-8000-0000000000a1', 'att-fx');
    expect(result.cancelled).toBe(true);
    expect(result.attempt_id).toBe('att-fx');
    expect(result.remote_billing_note).toContain('仅本地取消');
    expect(client.cancelledAttempts).toEqual(['att-fx']);
  });
});
