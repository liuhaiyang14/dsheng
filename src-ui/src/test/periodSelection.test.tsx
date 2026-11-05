// 全局期间与本地读取回归；仅合成本地数据，不调用 Provider。
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { AppProvider, useApp } from '../state/store';
import { FixtureClient } from '../api/fixtureClient';
import { RUN_2026_10, WS_DEMO_ID, WS_EMPTY_ID, fixtureHistory, fixtureTasks } from '../api/fixtureData';
import type { HistoryDetailView } from '../api';

const SEPTEMBER = fixtureHistory[1].run_id;
const directoryKey = `dsheng.source-directory.${WS_DEMO_ID}`;
beforeEach(() => window.localStorage.removeItem(directoryKey));
afterEach(() => { cleanup(); vi.restoreAllMocks(); window.localStorage.removeItem(directoryKey); });

async function openApp(client = new FixtureClient(), mode: 'fixture' | 'tauri' = 'fixture') {
  render(<App runtime={{ mode, client }} />);
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  return client;
}
function navigate(name: string) {
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: new RegExp(name) }));
}

it('按期间日期选择缺省结果，跨页共享选择和历史版本，刷新不回到最近更新记录', async () => {
  const client = new FixtureClient();
  const list = client.historyList.bind(client);
  vi.spyOn(client, 'historyList').mockImplementation(async (ws) => (await list(ws)).reverse());
  const show = vi.spyOn(client, 'historyShow');
  const taskList = vi.spyOn(client, 'taskList');
  const diagnosisList = vi.spyOn(client, 'diagnosisList');
  await openApp(client);
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-10-01 至 2026-10-28');
  fireEvent.change(screen.getByTestId('saved-period-select'), { target: { value: SEPTEMBER } });
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-09-30'));
  expect(taskList).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER);
  navigate('经营问题诊断');
  await waitFor(() => expect(diagnosisList).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER));
  navigate('历史查询');
  const row = within(screen.getByTestId('history-table')).getByText('2026-09-01 至 2026-09-30').closest('tr')!;
  fireEvent.click(within(row).getByRole('button', { name: 'r1' }));
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('分析版本 1（历史快照）'));
  navigate('经营分析');
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-09-30');
  fireEvent.click(screen.getByRole('button', { name: '刷新所选期间' }));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  expect(show).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER, 1);
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('历史快照');
  navigate('Provider 设置');
  expect(screen.getByText('此页配置属于当前企业，不随查看期间过滤。')).toBeVisible();
});

it('任务和时间线只含所选run任务，默认时间线范围来自所选期间', async () => {
  const client = new FixtureClient();
  const selectedTask = { ...structuredClone(fixtureTasks[0]), origin_run_id: SEPTEMBER };
  vi.spyOn(client, 'taskList').mockImplementation(async (_ws, run) => run === SEPTEMBER ? [selectedTask] : []);
  const timeline = vi.spyOn(client, 'taskTimeline').mockResolvedValue([
    { task_id: selectedTask.task_id, task_title: '该期间任务', kind: 'due', date: '2026-09-20', occurred_at: null, detail: '保留' },
    { task_id: 'other-run-task', task_title: '其他期间任务', kind: 'due', date: '2026-09-20', occurred_at: null, detail: '过滤' },
  ]);
  await openApp(client);
  fireEvent.change(screen.getByTestId('saved-period-select'), { target: { value: SEPTEMBER } });
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  navigate('任务单');
  expect(await screen.findByTestId('task-list-table')).toHaveTextContent(selectedTask.title);
  expect(await screen.findByTestId('timeline-board')).toHaveTextContent('该期间任务');
  expect(screen.getByTestId('timeline-board')).not.toHaveTextContent('其他期间任务');
  expect(timeline).toHaveBeenLastCalledWith({ workspace_id: WS_DEMO_ID, from: '2026-09-01', to: '2026-09-30', grain: 'month' });
  expect(screen.getByText(/任务单展示与所选期间关联的任务及其最新进度/)).toBeVisible();
});

it('无run也可扫描后显式首次分析，传真实目录与自定义日期粒度并选中新run', async () => {
  const client = new FixtureClient();
  const historyList = client.historyList.bind(client);
  const analyze = client.analyze.bind(client);
  let generatedRun: string | null = null;
  vi.spyOn(client, 'historyList').mockImplementation(async (ws) => generatedRun ? (await historyList(ws)).filter((h) => h.run_id === generatedRun) : []);
  const analyzeSpy = vi.spyOn(client, 'analyze').mockImplementation(async (input) => {
    const run = await analyze(input); generatedRun = run.run_id; return run;
  });
  await openApp(client);
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  fireEvent.change(screen.getByTestId('manual-path-input'), { target: { value: '/tmp/business-source' } });
  fireEvent.click(screen.getByTestId('manual-path-scan'));
  await waitFor(() => expect(screen.getByTestId('source-scan-status')).toHaveTextContent('已扫描'));
  navigate('经营分析');
  fireEvent.change(screen.getByLabelText('分析开始日期'), { target: { value: '2026-09-01' } });
  fireEvent.change(screen.getByLabelText('分析结束日期'), { target: { value: '2026-11-04' } });
  fireEvent.change(screen.getByLabelText('分析聚合粒度'), { target: { value: 'week' } });
  expect(analyzeSpy).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-11-04 · 周度'));
  expect(analyzeSpy).toHaveBeenCalledWith({ workspace_id: WS_DEMO_ID, directory_label: '/tmp/business-source', from: '2026-09-01', to: '2026-11-04', grain: 'week' });
  expect(screen.getByTestId('saved-period-select')).toHaveValue(generatedRun);
  expect(await screen.findByTestId('metric-table')).toBeVisible();
});

function SelectionProbe() {
  const app = useApp();
  return <>
    <button onClick={() => void app.selectRun(SEPTEMBER)}>select-september</button>
    <button onClick={() => void app.selectRun(RUN_2026_10)}>select-october</button>
    <button onClick={() => void app.switchWorkspace(WS_EMPTY_ID)}>empty-workspace</button>
    <span data-testid="probe-run">{app.data.currentDetail?.run_id ?? 'none'}</span>
    <span data-testid="probe-workspace">{app.activeWorkspaceId}</span>
  </>;
}
it('迟到的期间请求不能覆盖更新选择，也不能写入另一个企业的界面', async () => {
  const client = new FixtureClient();
  const realShow = client.historyShow.bind(client);
  let complete!: (detail: HistoryDetailView) => void;
  vi.spyOn(client, 'historyShow').mockImplementation((ws, run, revision) => run === SEPTEMBER
    ? new Promise((resolve) => { complete = resolve; }) : realShow(ws, run, revision));
  render(<AppProvider runtime={{ mode: 'fixture', client }}><SelectionProbe /></AppProvider>);
  await waitFor(() => expect(screen.getByTestId('probe-run')).toHaveTextContent(RUN_2026_10));
  fireEvent.click(screen.getByText('select-september'));
  await waitFor(() => expect(complete).toBeDefined());
  fireEvent.click(screen.getByText('select-october'));
  await waitFor(() => expect(screen.getByTestId('probe-run')).toHaveTextContent(RUN_2026_10));
  await act(async () => complete(await realShow(WS_DEMO_ID, SEPTEMBER)));
  expect(screen.getByTestId('probe-run')).toHaveTextContent(RUN_2026_10);
  fireEvent.click(screen.getByText('select-september'));
  await act(async () => {});
  fireEvent.click(screen.getByText('empty-workspace'));
  await waitFor(() => expect(screen.getByTestId('probe-workspace')).toHaveTextContent(WS_EMPTY_ID));
  await act(async () => complete(await realShow(WS_DEMO_ID, SEPTEMBER)));
  expect(screen.getByTestId('probe-run')).toHaveTextContent('none');
});

it('期间切换清除未发送诊断预览，桌面已保存报告不误导为可续问', async () => {
  const client = new FixtureClient();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await openApp(client, 'tauri');
  navigate('经营问题诊断');
  await screen.findByTestId('diagnosis-report-detail');
  expect(screen.getByTestId('continue-diagnosis-button')).toBeDisabled();
  expect(screen.getByText('报告已保存')).toBeVisible();
  expect(screen.getByText('当前版本暂不支持对已保存报告继续追问，已有内容可查看和下载。')).toBeVisible();
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  await screen.findByTestId('preview-modal');
  fireEvent.change(screen.getByTestId('saved-period-select'), { target: { value: SEPTEMBER } });
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01'));
  expect(screen.queryByTestId('preview-modal')).toBeNull();
  expect(confirm).not.toHaveBeenCalled();
});

it('历史版本禁止误对当前版本发起生成，显式切回当前版本后恢复', async () => {
  const client = new FixtureClient();
  const preview = vi.spyOn(client, 'diagnosisGeneratePreview');
  await openApp(client);
  navigate('历史查询');
  const row = within(screen.getByTestId('history-table')).getByText('2026-10-01 至 2026-10-28').closest('tr')!;
  fireEvent.click(within(row).getByRole('button', { name: 'r1' }));
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('历史快照'));
  navigate('经营问题诊断');
  expect(screen.getByTestId('generate-diagnosis-button')).toBeDisabled();
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  expect(preview).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '查看该期间当前版本' }));
  await waitFor(() => expect(screen.getByTestId('generate-diagnosis-button')).not.toBeDisabled());
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('分析版本 2（该期间当前版本）');
});

it('生成发送期间锁定上下文，结束后恢复选择和导航', async () => {
  const client = new FixtureClient();
  const realConfirm = client.diagnosisGenerateConfirm.bind(client);
  let finish!: () => void;
  const blocked = new Promise<void>((resolve) => { finish = resolve; });
  vi.spyOn(client, 'diagnosisGenerateConfirm').mockImplementation(async (id) => {
    const receipt = await realConfirm(id); await blocked; return receipt;
  });
  await openApp(client);
  navigate('经营问题诊断');
  fireEvent.click(await screen.findByTestId('generate-diagnosis-button'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await waitFor(() => expect(screen.getByTestId('saved-period-select')).toBeDisabled());
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  expect(within(screen.getByRole('navigation')).getByRole('button', { name: /数据源/ })).toBeDisabled();
  expect(screen.getByRole('button', { name: '刷新所选期间' })).toBeDisabled();
  await act(async () => finish());
  await waitFor(() => expect(screen.getByTestId('saved-period-select')).not.toBeDisabled());
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-10-01');
});

it('待应用任务候选保留在原期间，显式忽略后才解除上下文锁', async () => {
  const client = new FixtureClient();
  await openApp(client);
  navigate('任务单');
  fireEvent.click(await screen.findByTestId('generate-task-button'));
  fireEvent.click(await screen.findByTestId('request-task-preview'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await screen.findByTestId('candidate-panel');
  expect(screen.getByTestId('saved-period-select')).toBeDisabled();
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '全部忽略' }));
  await waitFor(() => expect(screen.getByTestId('saved-period-select')).not.toBeDisabled());
});

it('诊断预览只发送当前指标真实证据并去重，缺证据时阻止请求', async () => {
  const client = new FixtureClient();
  const preview = vi.spyOn(client, 'diagnosisGeneratePreview');
  const show = client.historyShow.bind(client);
  let noEvidence = false;
  vi.spyOn(client, 'historyShow').mockImplementation(async (...args) => {
    const value = await show(...args);
    for (const metric of value.metrics) {
      if (metric.mom) metric.mom.evidence_ids = noEvidence ? [] : ['cell:source-current:Sheet1:A9', 'cell:source-current:Sheet1:A10'];
      if (metric.yoy) metric.yoy.evidence_ids = noEvidence ? [] : ['cell:source-current:Sheet1:A9'];
    }
    return value;
  });
  await openApp(client);
  navigate('经营问题诊断');
  fireEvent.click(await screen.findByTestId('generate-diagnosis-button'));
  await screen.findByTestId('preview-modal');
  expect(preview).toHaveBeenCalledWith(expect.objectContaining({ evidence_ids: ['cell:source-current:Sheet1:A9', 'cell:source-current:Sheet1:A10'] }));
  fireEvent.click(screen.getByRole('button', { name: '取消（不发送）' }));
  noEvidence = true;
  fireEvent.click(screen.getByRole('button', { name: '刷新所选期间' }));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  expect(await screen.findByTestId('diagnosis-notice')).toHaveTextContent('本期间暂无可用来源证据');
  expect(preview).toHaveBeenCalledTimes(1);
});
