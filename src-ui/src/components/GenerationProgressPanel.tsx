/**
 * 生成进度状态面板（流式输出生成链路 S5）。
 *
 * 确认发送（diagnosis/task confirm）后渲染：
 * - 桌面运行时：订阅 `generation://progress` 事件，显示 阶段/已接收字节/
 *   块数/耗时；事件 payload 的 attempt_id 用于调既有 cancel 语义（取消按钮）。
 * - 无 Tauri（fixture/浏览器）：退化为 30 秒本地 tick 占位提示
 *   （“仍在等待响应/仍在接收中”）；事件通道到达后同样展示。
 * - 终态停止：confirm promise 结束后由父级置 active=false 卸载面板。
 * - stdout 纯净约定不适用于 UI；本面板只写自身 DOM，不触控制台。
 *
 * 组件测试经 `subscribe` prop 注入事件序列（fixture 通道，无网络/无 Tauri）。
 */
import { useEffect, useState } from 'react';
import { subscribeGenerationProgress } from '../api/generationProgress';
import type { GenerationProgressEvent } from '../api/types';

export interface GenerationProgressPanelProps {
  /** 面板激活（confirm 发送中）；false 时不渲染（终态停止）。 */
  active: boolean;
  /** 标题（默认“生成发送中”）。 */
  title?: string;
  /** 取消回调（attempt_id 取自事件 payload；无事件时按钮禁用）。 */
  onCancel?: (attemptId: string) => void;
  /** 事件通道注入（测试；默认订阅 Tauri generation://progress）。 */
  subscribe?: (listener: (event: GenerationProgressEvent) => void) => () => void;
  /** 本地提示 tick 间隔（默认 30s；测试注入小值）。 */
  tickMs?: number;
}

const DEFAULT_TICK_MS = 30_000;

function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  if (bytes < 1024 * 1024) {
    return `${(bytes / 1024).toFixed(1)} KB`;
  }
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`;
}

export function GenerationProgressPanel({
  active,
  title = '生成发送中',
  onCancel,
  subscribe,
  tickMs = DEFAULT_TICK_MS,
}: GenerationProgressPanelProps) {
  const [latest, setLatest] = useState<GenerationProgressEvent | null>(null);
  const [ticks, setTicks] = useState(0);
  const [cancelled, setCancelled] = useState(false);

  useEffect(() => {
    if (!active) {
      return undefined;
    }
    setLatest(null);
    setTicks(0);
    setCancelled(false);
    const unsubscribe = (subscribe ?? subscribeGenerationProgress)((event) => {
      setLatest(event);
      setTicks(0); // 事件到达重置 tick 计数（提示重新从“正在接收”起算）
    });
    const timer = window.setInterval(() => setTicks((n) => n + 1), tickMs);
    return () => {
      unsubscribe();
      window.clearInterval(timer);
    };
  }, [active, subscribe, tickMs]);

  if (!active) {
    return null;
  }

  const hint = latest
    ? ticks > 0
      ? '仍在接收中…（流式响应进行中）'
      : '正在接收响应流…'
    : ticks > 0
      ? `仍在等待响应…（已等待约 ${Math.round((ticks * tickMs) / 1000)}s）`
      : '已发起请求，等待响应…';

  return (
    <div className="notice" data-testid="generation-progress-panel">
      <strong>{title}</strong>
      <div data-testid="generation-progress-stage">
        {latest ? `阶段：接收中（${latest.stage}）` : '阶段：等待响应'}
      </div>
      <div data-testid="generation-progress-stats">
        {latest
          ? `已接收 ${formatBytes(latest.bytes)} · 第 ${latest.chunk_seq} 块 · 耗时 ${Math.round(
              latest.elapsed_ms / 1000,
            )}s`
          : '已接收 0 B · 第 0 块 · 耗时 0s'}
      </div>
      <div className="muted" data-testid="generation-progress-hint">
        {hint}
      </div>
      <button
        type="button"
        className="outline"
        data-testid="generation-progress-cancel"
        disabled={!latest || cancelled || !onCancel}
        onClick={() => {
          if (latest && onCancel && !cancelled) {
            onCancel(latest.attempt_id);
            setCancelled(true);
          }
        }}
      >
        {cancelled ? '已请求取消（迟到数据不提交）' : '取消生成'}
      </button>
    </div>
  );
}
