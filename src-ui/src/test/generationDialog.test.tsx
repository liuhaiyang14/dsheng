// 本地 UI 回归：延迟 client 与事件通道，无网络/Provider。
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { TauriClient } from '../api/tauriClient';
import type { DiagnosisGenerationReceipt, GenerationProgressEvent, TaskCandidateView } from '../api/types';

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

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

it('只呈现本次关联事件的活动和计数，不展示原始思考；发送失败保留终态', async () => {
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
  emit({ ...progress(id, { bytes: 2048, chunk_seq: 5, elapsed_ms: 12345, stage: 'reasoning', content_chars: 20, reasoning_chars: 360 }),
    reasoning_content: '不应暴露的原始思考' } as GenerationProgressEvent);
  const dialog = screen.getByRole('dialog');
  expect(within(dialog).getByTestId('generation-progress-stage')).toHaveTextContent('模型正在思考');
  expect(within(dialog).getByTestId('generation-progress-stats')).toHaveTextContent('2,048');
  expect(dialog).toHaveTextContent('正文 20 字符');
  expect(dialog).toHaveTextContent('思考活动 360 字符');
  expect(dialog).not.toHaveTextContent('不应暴露的原始思考');
  await act(async () => pending.reject(new Error('服务返回无效结果')));
  expect(screen.getByRole('dialog')).toHaveTextContent('生成失败');
  expect(screen.getByRole('alert')).toHaveTextContent('服务返回无效结果');
  expect(screen.getByRole('button', { name: '关闭' })).toBeEnabled();
  expect(confirm).toHaveBeenCalledOnce();
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

it('事件监听失败时仍可按确认编号停止，且不需要首个attempt事件', async () => {
  vi.stubGlobal('__TAURI__', { event: { listen: vi.fn().mockRejectedValue(new Error('事件不可用')) } });
  const stop = vi.fn(async () => ({ cancelled: true, attempt_id: null, remote_billing_note: '已产生的用量以服务商记录为准' }));
  const client = Object.assign(new FixtureClient(), { generationCancelByConfirmation: stop });
  const pending = deferred<DiagnosisGenerationReceipt>();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm').mockReturnValue(pending.promise);
  await diagnosisPreview(client);
  fireEvent.click(screen.getByTestId('preview-confirm'));
  await waitFor(() => expect(confirm).toHaveBeenCalledOnce());
  expect(screen.getByRole('dialog')).toHaveTextContent('实时状态暂不可用');
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
  await screen.findByText('无法准备实时状态通道，请重试后再发送。');
  expect(confirm).not.toHaveBeenCalled();
  expect(screen.getByRole('button', { name: '关闭' })).toBeEnabled();
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
