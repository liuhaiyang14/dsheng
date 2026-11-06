// 本地 UI 回归：延迟 client 与事件通道，无网络/Provider。
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { TauriClient } from '../api/tauriClient';
import type { DiagnosisGenerationReceipt, GenerationProgressEvent, TaskCandidateView } from '../api/types';

afterEach(() => { cleanup(); vi.useRealTimers(); vi.unstubAllGlobals(); });

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function events(ready?: Promise<() => void>) {
  const listeners = new Set<(event: { payload: GenerationProgressEvent }) => void>();
  vi.stubGlobal('__TAURI__', { event: { listen: vi.fn(async (_name, listener) => {
    listeners.add(listener);
    if (ready) await ready;
    return () => { listeners.delete(listener); };
  }) } });
  return (event: GenerationProgressEvent) => act(() => { listeners.forEach((listener) => listener({ payload: event })); });
}

async function diagnosisPreview(client: FixtureClient) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getAllByText('经营问题诊断')[0].closest('button')!);
  fireEvent.click(await screen.findByTestId('generate-diagnosis-button'));
  return screen.findByTestId('preview-modal');
}

it('确认发送后实时进度和可用停止按钮在同一弹窗内', async () => {
  events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  const dialog = screen.getByRole('dialog');
  await waitFor(() => expect(within(dialog).getByTestId('generation-progress-panel')).toBeTruthy());
  expect(within(dialog).getByRole('button', { name: '停止生成' })).not.toBeDisabled();
  expect(within(dialog).queryByText('取消（不发送）')).toBeNull();
});

const receipt: DiagnosisGenerationReceipt = { attempt_id: 'att-1', session_id: 'session-1', turn_id: 'turn-1', turn_state: 'patched', report_id: 'late-report' };
function progress(confirmationId: string, patch: Partial<GenerationProgressEvent> = {}): GenerationProgressEvent {
  return { confirmation_id: confirmationId, workspace_id: '00000000-0000-4000-8000-0000000000a1',
    attempt_id: 'att-1', stage: 'waiting', bytes: 0, chunk_seq: 0, elapsed_ms: 0, ...patch };
}

it('先等待事件监听就绪；准备阶段停止不会发送请求', async () => {
  const ready = deferred<() => void>();
  events(ready.promise);
  const client = new FixtureClient();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  expect(confirm).not.toHaveBeenCalled();
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
  await act(async () => ready.resolve(() => {}));
  expect(confirm).not.toHaveBeenCalled();
});

it('仅呈现本次事件的思考文本及正文计数，发送失败保留内容和终态', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const id = confirm.mock.calls[0][0];
  emit(progress('another-confirmation', { bytes: 999_999, stage: 'saving' }));
  expect(screen.getByTestId('generation-progress-stats')).not.toHaveTextContent('999,999');
  expect(screen.getByTestId('generation-reasoning-text')).toBeEmptyDOMElement();
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('0 字符');
  emit(progress(id, { bytes: 2048, chunk_seq: 5, elapsed_ms: 12345, stage: 'reasoning',
    content_chars: 20, reasoning_delta: '先核对期间。\n<img src=x onerror=alert(1)>' }));
  const dialog = screen.getByRole('dialog');
  expect(within(dialog).getByTestId('generation-progress-stage')).toHaveTextContent('模型正在思考');
  expect(within(dialog).getByTestId('generation-content-count')).toHaveTextContent('20 字符');
  expect(within(dialog).getByTestId('generation-progress-stats')).not.toHaveTextContent('2,048');
  expect(dialog).not.toHaveTextContent('字节');
  expect(dialog).not.toHaveTextContent('输出活动');
  expect(dialog).not.toHaveTextContent('思考活动');
  expect(screen.getByTestId('generation-reasoning-text')).toHaveTextContent('先核对期间。');
  expect(screen.getByTestId('generation-reasoning-text').querySelector('img')).toBeNull();
  expect(screen.getByTestId('generation-reasoning-text')).toHaveTextContent('<img src=x onerror=alert(1)>');
  await act(async () => pending.reject(new Error('服务返回无效结果')));
  expect(screen.getByRole('dialog')).toHaveTextContent('生成失败');
  expect(screen.getByRole('alert')).toHaveTextContent('服务返回无效结果');
  expect(screen.getByRole('button', { name: '关闭' })).toBeEnabled();
  expect(confirm).toHaveBeenCalledOnce();
});

it('思考增量去重并隔离其他请求，正文按后端字符计数，关闭后新请求清空', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const next = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm')
    .mockReturnValueOnce(pending.promise).mockReturnValueOnce(next.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const id = confirm.mock.calls[0][0];
  emit(progress(id, { stage: 'reasoning', chunk_seq: 1, reasoning_delta: '核对🙂' }));
  emit(progress(id, { stage: 'reasoning', chunk_seq: 1, reasoning_delta: '核对🙂' }));
  emit(progress(id, { stage: 'reasoning', chunk_seq: 2, workspace_id: 'other-workspace', reasoning_delta: '错误企业' }));
  emit(progress(id, { stage: 'reasoning', chunk_seq: 2, attempt_id: 'other-attempt', reasoning_delta: '错误请求' }));
  emit(progress(id, { stage: 'content', chunk_seq: 2, content_chars: 3, reasoning_delta: '，再看差异。' }));
  emit(progress(id, { stage: 'reasoning', chunk_seq: 1, reasoning_delta: '迟到内容' }));
  expect(screen.getByTestId('generation-reasoning-text')).toHaveTextContent('核对🙂，再看差异。');
  expect(screen.getByTestId('generation-reasoning-text').textContent).toBe('核对🙂，再看差异。');
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('3 字符');
  emit(progress(id, { stage: 'validating', chunk_seq: 2 }));
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('3 字符');
  await act(async () => pending.reject(new Error('本地测试终态')));
  emit(progress(id, { stage: 'reasoning', chunk_seq: 3, reasoning_delta: '终态后的内容' }));
  expect(screen.getByTestId('generation-reasoning-text').textContent).toBe('核对🙂，再看差异。');
  fireEvent.click(screen.getByRole('button', { name: '关闭' }));
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledTimes(2));
  expect(screen.getByTestId('generation-reasoning-text')).toBeEmptyDOMElement();
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('0 字符');
  await act(async () => next.reject(new Error('本地测试结束')));
});

it('思考区跟随新内容，用户上翻后保持阅读位置，可回到最新内容', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const id = confirm.mock.calls[0][0];
  const viewport = screen.getByTestId('generation-reasoning-text');
  let contentHeight = 500;
  Object.defineProperty(viewport, 'scrollHeight', { get: () => contentHeight });
  Object.defineProperty(viewport, 'clientHeight', { value: 100 });
  emit(progress(id, { stage: 'reasoning', chunk_seq: 1, reasoning_delta: '第一段\n' }));
  expect(viewport.scrollTop).toBe(500);
  viewport.scrollTop = 50;
  fireEvent.scroll(viewport);
  contentHeight = 600;
  emit(progress(id, { stage: 'reasoning', chunk_seq: 2, reasoning_delta: '第二段' }));
  expect(viewport.scrollTop).toBe(50);
  fireEvent.click(screen.getByRole('button', { name: '回到最新内容' }));
  expect(viewport.scrollTop).toBe(600);
  expect(screen.queryByRole('button', { name: '回到最新内容' })).toBeNull();
  await act(async () => pending.reject(new Error('本地测试结束')));
});

it('非流式结束时接收正文计数，没有返回思考则展示空白区域', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const id = confirm.mock.calls[0][0];
  emit(progress(id, { stage: 'receiving', bytes: 5000, chunk_seq: 1 }));
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('0 字符');
  emit(progress(id, { stage: 'content', bytes: 5000, chunk_seq: 2, content_chars: 1200 }));
  emit(progress(id, { stage: 'saving', bytes: 5000, chunk_seq: 2, content_chars: 1200 }));
  await act(async () => pending.resolve(receipt));
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('1,200 字符');
  expect(screen.getByTestId('generation-reasoning-text')).toBeEmptyDOMElement();
});

it('首个输出前可以登记停止，停止成功丢弃迟到报告并等待请求结束才解锁', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  const cancel = vi.spyOn(client, 'generationCancel');
  const list = vi.spyOn(client, 'diagnosisList');
  await diagnosisPreview(client);
  const reads = list.mock.calls.length;
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  expect(cancel).not.toHaveBeenCalled();
  emit(progress('wrong-request'));
  expect(cancel).not.toHaveBeenCalled();
  emit(progress(confirm.mock.calls[0][0]));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith('00000000-0000-4000-8000-0000000000a1', 'att-1'));
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  await act(async () => pending.resolve(receipt));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
  expect(screen.getByTestId('workspace-select')).toBeEnabled();
  expect(list).toHaveBeenCalledTimes(reads);
  expect(screen.queryByTestId('diagnosis-item-late-report')).toBeNull();
});

it('停止失败保留运行态和上下文锁，允许再次停止', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  const cancel = vi.spyOn(client, 'generationCancel')
    .mockImplementationOnce(() => { throw new Error('暂时无法停止'); });
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  emit(progress(confirm.mock.calls[0][0]));
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  await screen.findByText('停止失败：暂时无法停止');
  expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled();
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  await waitFor(() => expect(cancel).toHaveBeenCalledTimes(2));
  await act(async () => pending.reject(new Error('CANCELLED')));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
});

it('任务生成使用同一弹窗，取消成功不加载迟到候选', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<TaskCandidateView>();
  const confirm = vi.spyOn(client, 'taskGenerateConfirm').mockReturnValue(pending.promise);
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getAllByText('任务单')[0].closest('button')!);
  fireEvent.click(await screen.findByTestId('generate-task-button'));
  fireEvent.click(await screen.findByTestId('request-task-preview'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  expect(within(screen.getByRole('dialog')).getByTestId('generation-progress-panel')).toBeTruthy();
  emit(progress(confirm.mock.calls[0][0]));
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  await waitFor(() => expect(screen.getByRole('dialog')).toHaveTextContent('停止已确认'));
  await act(async () => pending.resolve({ tasks: [] } as unknown as TaskCandidateView));
  expect(screen.queryByTestId('candidate-panel')).toBeNull();
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
});

it('继续分析也在弹窗内显示生成进度和停止入口', async () => {
  events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisContinueConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByText('取消（不发送）'));
  fireEvent.change(document.getElementById('diagnosis-supplement')!, { target: { value: '补充本地合成事实' } });
  fireEvent.click(screen.getByTestId('continue-diagnosis-button'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const dialog = screen.getByRole('dialog');
  expect(dialog).toHaveTextContent('继续分析');
  expect(within(dialog).getByRole('button', { name: '停止生成' })).toBeEnabled();
  await act(async () => pending.resolve(receipt));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('生成完成');
});

it('期间刷新已在进行时，生成入口和预览确认都不可发送', async () => {
  events();
  const client = new FixtureClient();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await diagnosisPreview(client);
  const history = await client.historyList('00000000-0000-4000-8000-0000000000a1');
  const refresh = deferred<typeof history>();
  vi.spyOn(client, 'historyList').mockReturnValueOnce(refresh.promise);
  fireEvent.click(screen.getByRole('button', { name: '刷新所选期间' }));
  expect(screen.getByTestId('generate-diagnosis-button')).toBeDisabled();
  expect(screen.getByTestId('preview-confirm')).toBeDisabled();
  fireEvent.click(screen.getByTestId('preview-confirm'));
  expect(confirm).not.toHaveBeenCalled();
  expect(screen.getByText('取消（不发送）')).toBeEnabled();
  await act(async () => refresh.resolve(history));
  await waitFor(() => expect(screen.getByTestId('preview-confirm')).toBeEnabled());
});

it('事件监听失败不派发请求，即使具备按确认编号停止能力；显式重试成功后才发送', async () => {
  let firstProgressSubscription = true;
  const listen = vi.fn(async (name: string) => {
    if (name === 'generation://progress' && firstProgressSubscription) {
      firstProgressSubscription = false;
      throw new Error('事件不可用');
    }
    return () => {};
  });
  vi.stubGlobal('__TAURI__', { event: { listen } });
  const stop = vi.fn(async () => ({ cancelled: true, attempt_id: null, remote_billing_note: '已产生的用量以服务商记录为准' }));
  const client = Object.assign(new FixtureClient(), { generationCancelByConfirmation: stop });
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await screen.findByText('无法连接实时状态通道，本次请求尚未发送。请重试连接。');
  expect(confirm).not.toHaveBeenCalled();
  expect(stop).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '重试连接并发送' }));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  await waitFor(() => expect(stop).toHaveBeenCalledWith('00000000-0000-4000-8000-0000000000a1', confirm.mock.calls[0][0]));
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  await act(async () => pending.resolve(receipt));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
});

it('旧客户端既无可用事件通道也无确认编号停止能力时，不发起不可停止的请求', async () => {
  vi.stubGlobal('__TAURI__', { event: { listen: vi.fn().mockRejectedValue(new Error('事件不可用')) } });
  const client = new FixtureClient();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await screen.findByText('无法连接实时状态通道，本次请求尚未发送。请重试连接。');
  expect(confirm).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: '关闭' })).toBeEnabled();
});

it('仅新的正文或思考重置输出时间，传输包装不计入正文且等待不自动停止', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  const cancel = vi.spyOn(client, 'generationCancel');
  await diagnosisPreview(client);
  vi.useFakeTimers({ toFake: ['Date', 'setInterval', 'clearInterval'] });
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await act(async () => { await Promise.resolve(); });
  const id = confirm.mock.calls[0][0];
  emit(progress(id, { stage: 'receiving', bytes: 100, chunk_seq: 1 }));
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('尚未收到模型输出');
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('0 字符');
  emit(progress(id, { stage: 'content', bytes: 200, chunk_seq: 2, content_chars: 1 }));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('正在接收正文');
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 0 秒');
  act(() => vi.advanceTimersByTime(3000));
  emit(progress(id, { stage: 'content', bytes: 200, chunk_seq: 2, content_chars: 1 }));
  emit(progress('another-request', { stage: 'receiving', bytes: 500, chunk_seq: 2 }));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('等待后续输出');
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 3 秒');
  emit(progress(id, { stage: 'waiting', bytes: 200, chunk_seq: 2 }));
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 3 秒');
  emit(progress(id, { stage: 'receiving', bytes: 300, chunk_seq: 3 }));
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 3 秒');
  expect(screen.getByTestId('generation-content-count')).toHaveTextContent('1 字符');
  emit(progress(id, { stage: 'content', bytes: 400, chunk_seq: 4, content_chars: 2 }));
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 0 秒');
  act(() => vi.advanceTimersByTime(3_600_000));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('等待后续输出');
  expect(screen.getByTestId('generation-last-output')).toHaveTextContent('距上次输出 3,600 秒');
  expect(screen.getByRole('button', { name: '停止生成' })).toBeEnabled();
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  expect(cancel).not.toHaveBeenCalled();
  emit(progress(id, { stage: 'validating', bytes: 400, chunk_seq: 4, content_chars: 2 }));
  expect(screen.getByTestId('generation-progress-hint')).toHaveTextContent('正在校验生成结果');
  expect(screen.getByTestId('generation-progress-hint')).not.toHaveTextContent('正在接收');
  emit(progress(id, { stage: 'saving', bytes: 400, chunk_seq: 4, content_chars: 2 }));
  expect(screen.getByTestId('generation-progress-hint')).toHaveTextContent('正在保存结果');
  await act(async () => pending.resolve(receipt));
});

it('桌面停止辅助按企业和确认编号调用本地命令并解包回执', async () => {
  const data = { cancelled: true, attempt_id: null, remote_billing_note: '本地停止已登记' };
  const invoke = vi.fn().mockResolvedValue({ schema_version: '1', ok: true, data });
  vi.stubGlobal('__TAURI__', { core: { invoke } });
  expect(await new TauriClient().generationCancelByConfirmation('workspace-1', 'confirmation-1')).toEqual(data);
  expect(invoke).toHaveBeenCalledWith('generation_cancel_by_confirmation', {
    workspaceId: 'workspace-1', confirmationId: 'confirmation-1',
  });
});

it('真实桌面客户端缺少事件API时，不把空订阅当作连接成功', async () => {
  vi.stubGlobal('__TAURI__', { core: { invoke: vi.fn() } });
  const client = new FixtureClient();
  Object.defineProperty(client, 'kind', { value: 'tauri' });
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await screen.findByText('无法连接实时状态通道，本次请求尚未发送。请重试连接。');
  expect(confirm).not.toHaveBeenCalled();
});

it('停止终态事件到达后，迟到取消IPC错误不能把界面改回运行', async () => {
  const emit = events();
  const client = new FixtureClient();
  const pending = deferred<DiagnosisGenerationReceipt>();
  const cancelPending = deferred<Awaited<ReturnType<FixtureClient['generationCancel']>>>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  const cancel = vi.spyOn(client, 'generationCancel').mockReturnValue(cancelPending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  const id = confirm.mock.calls[0][0];
  emit(progress(id));
  fireEvent.click(screen.getByRole('button', { name: '停止生成' }));
  await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
  emit(progress(id, { stage: 'cancelled' }));
  await act(async () => cancelPending.reject(new Error('取消回执迟到错误')));
  expect(screen.getByTestId('generation-progress-hint')).toHaveTextContent('停止已确认');
  expect(screen.queryByText('停止失败：取消回执迟到错误')).toBeNull();
  expect(screen.getByRole('button', { name: '正在停止…' })).toBeDisabled();
  await act(async () => pending.resolve(receipt));
  expect(screen.getByTestId('generation-progress-stage')).toHaveTextContent('已停止');
});
