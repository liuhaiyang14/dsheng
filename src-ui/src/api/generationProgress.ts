/**
 * 生成进度事件订阅（S5 流式可观测）。
 *
 * 桌面运行时：window.__TAURI__.event.listen('generation://progress') 订阅
 * 壳层 Emitter 广播（payload 见 GenerationProgressEvent）；返回取消订阅函数。
 * 无 Tauri 运行时（fixture/浏览器预览）：返回 no-op 订阅——调用方退化为
 * 30 秒本地 tick 占位提示（GenerationProgressPanel）。
 */

import type { GenerationProgressEvent } from './types';

interface TauriEventListener {
  listen(
    event: string,
    handler: (e: { payload: GenerationProgressEvent }) => void,
  ): Promise<() => void>;
}

type TauriWithEvents = { event?: TauriEventListener } | undefined;

export const GENERATION_PROGRESS_EVENT = 'generation://progress';

/** 订阅生成进度事件；返回取消订阅。无 Tauri 时为 no-op（退化 tick）。 */
export function subscribeGenerationProgress(
  listener: (event: GenerationProgressEvent) => void,
): () => void {
  const tauri: TauriWithEvents =
    typeof window !== 'undefined'
      ? ((window as unknown as { __TAURI__?: TauriWithEvents }).__TAURI__ ?? undefined)
      : undefined;
  const api = tauri?.event;
  if (!api || typeof api.listen !== 'function') {
    return () => {};
  }
  let disposed = false;
  let unlisten: (() => void) | null = null;
  api.listen(GENERATION_PROGRESS_EVENT, (e) => listener(e.payload))
    .then((fn) => {
      if (disposed) {
        fn();
      } else {
        unlisten = fn;
      }
    })
    .catch(() => {
      /* 订阅失败保持退化（tick 提示）；进度是尽力而为的观测通道 */
    });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
