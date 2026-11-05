import { useCallback, useEffect, useRef, useState } from 'react';
import type { DshengClient, GenerationProgressEvent } from '../api';
import { listenGenerationProgress } from '../api/generationProgress';

export type GenerationPhase = 'idle' | 'starting' | 'running' | 'stopping' | 'succeeded' | 'failed' | 'cancelled';
export interface GenerationState {
  phase: GenerationPhase;
  latest: GenerationProgressEvent | null;
  elapsedMs: number;
  lastActivityAt: number | null;
  canRetryConnection: boolean;
  error: string | null;
  message: string | null;
}
const initialState: GenerationState = {
  phase: 'idle', latest: null, elapsedMs: 0, lastActivityAt: null,
  canRetryConnection: false, error: null, message: null,
};
const activePhase = (phase: GenerationPhase) => phase === 'starting' || phase === 'running' || phase === 'stopping';
const errorText = (error: unknown) => error instanceof Error ? error.message : String(error);

interface Inflight {
  confirmationId: string;
  workspaceId: string;
  client: DshengClient;
  startedAt: number;
  invoked: boolean;
  finished: boolean;
  cancelled: boolean;
  stopRequested: boolean;
  receivedBytes: number;
  receivedChunks: number;
  attemptId?: string;
  cancelPromise?: Promise<void>;
  unsubscribe?: () => void;
}

/** 三个模型入口共享的生命周期：先监听再发送，停止确认后丢弃迟到结果。 */
export function useGeneration(client: DshengClient | null, workspaceId: string | null) {
  const [state, setState] = useState<GenerationState>(initialState);
  const current = useRef<Inflight | null>(null);
  const stopOnUnmount = useRef<(request: Inflight) => Promise<void>>();
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      const request = current.current;
      request?.unsubscribe?.();
      if (request && !request.finished) void stopOnUnmount.current?.(request);
    };
  }, []);
  const update = useCallback((request: Inflight, change: Partial<GenerationState>) => {
    if (mounted.current && current.current === request) {
      setState((previous) => ({ ...previous, ...change, elapsedMs: Date.now() - request.startedAt }));
    }
  }, []);
  useEffect(() => {
    if (!activePhase(state.phase)) return;
    const timer = window.setInterval(() => {
      const request = current.current;
      if (request) update(request, {});
    }, 1000);
    return () => window.clearInterval(timer);
  }, [state.phase, update]);

  const stopRequest = useCallback(async (request: Inflight) => {
    if (request.finished || request.cancelled || request.cancelPromise) return;
    request.stopRequested = true;
    update(request, { phase: 'stopping', error: null, message: '正在停止本次生成…' });
    if (!request.invoked) {
      request.cancelled = true;
      update(request, { phase: 'cancelled', message: '已停止，尚未发送请求。' });
      return;
    }
    if (!request.client.generationCancelByConfirmation && !request.attemptId) {
      update(request, { message: '停止请求已登记，正在等待本次请求标识。' });
      return;
    }
    request.cancelPromise = Promise.resolve().then(async () => {
      try {
        const result = request.client.generationCancelByConfirmation
          ? await request.client.generationCancelByConfirmation(request.workspaceId, request.confirmationId)
          : await request.client.generationCancel(request.workspaceId, request.attemptId!);
        if (!result.cancelled) throw new Error('本次生成未能停止，可能已经完成。请等待结果，或再次尝试停止。');
        request.cancelled = true;
        update(request, { phase: 'stopping', message: '停止已确认，正在结束本次请求。' });
      } catch (error) {
        // 终态事件比迟到的取消 IPC 错误更明确，不能把已确认停止改回运行中。
        if (request.cancelled) return;
        request.stopRequested = false;
        update(request, { phase: 'running', error: `停止失败：${errorText(error)}`, message: '本次请求仍在进行，可以再次停止。' });
      } finally {
        request.cancelPromise = undefined;
      }
    });
    await request.cancelPromise;
  }, [update]);
  stopOnUnmount.current = stopRequest;

  const run = useCallback(async <T,>(confirmationId: string, send: () => Promise<T>): Promise<
    { status: 'succeeded'; value: T } | { status: 'failed' | 'cancelled' }
  > => {
    if (!client || !workspaceId || (current.current && !current.current.finished && !current.current.cancelled)) {
      return { status: 'failed' };
    }
    const request: Inflight = {
      confirmationId, workspaceId, client, startedAt: Date.now(), invoked: false,
      finished: false, cancelled: false, stopRequested: false, receivedBytes: 0, receivedChunks: 0,
    };
    current.current = request;
    setState({ ...initialState, phase: 'starting' });
    try {
      try {
        const desktopEvents = (window as unknown as { __TAURI__?: { event?: { listen?: unknown } } }).__TAURI__?.event;
        if (client.kind === 'tauri' && typeof desktopEvents?.listen !== 'function') {
          throw new Error('桌面事件通道不可用');
        }
        request.unsubscribe = await listenGenerationProgress((event) => {
          if (current.current !== request || request.finished || request.cancelled) return;
          if (event.confirmation_id !== confirmationId || (event.workspace_id && event.workspace_id !== workspaceId)) return;
          if (request.attemptId && request.attemptId !== event.attempt_id) return;
          request.attemptId = event.attempt_id;
          const newActivity = event.stage === 'receiving'
            && (event.bytes > request.receivedBytes || event.chunk_seq > request.receivedChunks);
          if (event.stage === 'receiving') {
            request.receivedBytes = Math.max(request.receivedBytes, event.bytes);
            request.receivedChunks = Math.max(request.receivedChunks, event.chunk_seq);
          }
          update(request, { latest: event, ...(newActivity ? { lastActivityAt: Date.now() } : {}) });
          if (event.stage === 'cancelled') {
            request.cancelled = true;
            update(request, { phase: 'stopping', message: '停止已确认，正在结束本次请求。' });
            return;
          }
          if (request.stopRequested) void stopRequest(request);
        });
      } catch {
        throw new Error('无法连接实时状态通道，本次请求尚未发送。请重试连接。');
      }
      if (request.cancelled || !mounted.current || current.current !== request) return { status: 'cancelled' };
      request.invoked = true;
      update(request, { phase: 'running' });
      const value = await send();
      await request.cancelPromise;
      if (request.cancelled || !mounted.current || current.current !== request) {
        update(request, { phase: 'cancelled', error: null, message: '本次生成已停止，迟到结果不会加载。' });
        return { status: 'cancelled' };
      }
      update(request, { phase: 'succeeded', error: null, message: request.stopRequested ? '请求已完成，停止未生效。' : '结果已保存，可以关闭弹窗查看。' });
      return { status: 'succeeded', value };
    } catch (error) {
      await request.cancelPromise;
      if (request.cancelled || /CANCELLED|CANCELED|已取消/.test(errorText(error))) {
        request.cancelled = true;
        update(request, { phase: 'cancelled', error: null, message: '本次生成已停止，迟到结果不会加载。' });
        return { status: 'cancelled' };
      }
      update(request, { phase: 'failed', canRetryConnection: !request.invoked,
        error: errorText(error), message: request.invoked ? '本次生成未完成，没有自动重试。' : '尚未发送模型请求。' });
      return { status: 'failed' };
    } finally {
      request.finished = true;
      request.unsubscribe?.();
    }
  }, [client, workspaceId, stopRequest, update]);

  const stop = useCallback(() => {
    if (current.current) void stopRequest(current.current);
  }, [stopRequest]);
  const reset = useCallback(() => {
    if (current.current && !current.current.finished && !current.current.cancelled) return;
    current.current?.unsubscribe?.();
    current.current = null;
    setState(initialState);
  }, []);
  return { state, active: activePhase(state.phase), run, stop, reset };
}

export type GenerationController = ReturnType<typeof useGeneration>;
