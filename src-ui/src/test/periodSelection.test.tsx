// 全局期间与本地读取回归；仅合成本地数据，不调用 Provider。
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { AppProvider, useApp } from '../state/store';
import { FixtureClient } from '../api/fixtureClient';
import { RUN_2026_10, WS_DEMO_ID, WS_EMPTY_ID, fixtureHistory, fixtureTasks, fixturePeriodPolicies, fixtureScan } from '../api/fixtureData';
import type { HistoryDetailView, HistoryEntryView } from '../api';

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

/** 明确构造同一粒度的本地结果，不能让生产筛选为了旧 fixture 放宽口径。 */
function dailyPeriodsClient() {
  const client = new FixtureClient();
  const list = client.historyList.bind(client);
  const show = client.historyShow.bind(client);
  vi.spyOn(client, 'historyList').mockImplementation(async (ws) => (await list(ws)).map((h) => ({ ...h, cadence: 'day' as const })));
  vi.spyOn(client, 'historyShow').mockImplementation(async (...args) => ({ ...await show(...args), grain: 'day' }));
  return client;
}

it('页眉只列设置粒度的已保存期间，切换跨页共享且仅本地读取最新结果', async () => {
  const client = dailyPeriodsClient();
  const show = vi.mocked(client.historyShow);
  const analyze = vi.spyOn(client, 'analyze');
  const taskList = vi.spyOn(client, 'taskList');
  const diagnosisList = vi.spyOn(client, 'diagnosisList');
  await openApp(client);
  expect(screen.getByTestId('analysis-period-bar').closest('header')).toHaveClass('topbar');
  expect(screen.queryByLabelText('分析聚合粒度')).toBeNull();
  expect(screen.queryByLabelText('分析开始日期')).toBeNull();
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-10-01 至 2026-10-28');
  fireEvent.change(screen.getByTestId('saved-period-select'), { target: { value: SEPTEMBER } });
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-09-30'));
  expect(taskList).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER);
  navigate('经营问题诊断');
  await waitFor(() => expect(diagnosisList).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER));
  navigate('历史查询');
  expect(screen.queryByRole('button', { name: /^r[0-9]+$/ })).toBeNull();
  const row = within(screen.getByTestId('history-table')).getByText('2026年9月1日—2026年9月30日').closest('tr')!;
  fireEvent.click(within(row).getByRole('button', { name: '查看当前' }));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  navigate('经营分析');
  expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-09-30');
  fireEvent.click(screen.getByRole('button', { name: '刷新所选期间' }));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  expect(show).toHaveBeenLastCalledWith(WS_DEMO_ID, SEPTEMBER);
  expect(show.mock.calls.every((call) => call[2] == null)).toBe(true);
  expect(analyze).not.toHaveBeenCalled();
  navigate('Provider 设置');
  expect(screen.getByTestId('saved-period-select')).toHaveValue(SEPTEMBER);
});

it('任务和时间线只含所选run任务，默认时间线范围来自所选期间', async () => {
  const client = dailyPeriodsClient();
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
  expect(timeline).toHaveBeenLastCalledWith({ workspace_id: WS_DEMO_ID, from: '2026-09-01', to: '2026-09-30', grain: 'day' });
});

it('无结果时先扫描，再到经营分析显式运行，使用真实目录与系统设置粒度', async () => {
  const client = new FixtureClient();
  await client.periodSet({ schema_version: '1', operation: 'period_policy_set', workspace_id: WS_DEMO_ID,
    expected_revision: 4, policy: { ...fixturePeriodPolicies[1].policy, cadence: 'week', effective_from: '2026-11-01' } });
  const historyList = client.historyList.bind(client);
  const analyze = client.analyze.bind(client);
  let generatedRun: string | null = null;
  vi.spyOn(client, 'historyList').mockImplementation(async (ws) => generatedRun ? (await historyList(ws)).filter((h) => h.run_id === generatedRun) : []);
  const analyzeSpy = vi.spyOn(client, 'analyze').mockImplementation(async (input) => {
    const run = await analyze(input); generatedRun = run.run_id; return run;
  });
  await openApp(client);
  expect(screen.queryByRole('button', { name: '运行本地分析' })).toBeNull();
  expect(screen.getByTestId('saved-period-select')).toBeDisabled();
  fireEvent.change(screen.getByTestId('manual-path-input'), { target: { value: '/tmp/business-source' } });
  fireEvent.click(screen.getByTestId('manual-path-scan'));
  await waitFor(() => expect(screen.getByTestId('source-scan-status')).toHaveTextContent('已扫描'));
  fireEvent.click(screen.getByTestId('confirm-source-mapping-button'));
  await waitFor(() => expect(screen.queryByTestId('confirm-source-mapping-button')).toBeNull());
  navigate('经营分析');
  expect(screen.queryByLabelText('分析开始日期')).toBeNull();
  expect(screen.queryByLabelText('分析结束日期')).toBeNull();
  fireEvent.change(screen.getByLabelText('分析期间'), { target: { value: 'week:2026-10-05:2026-10-11' } });
  expect(screen.queryByLabelText('分析聚合粒度')).toBeNull();
  expect(analyzeSpy).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-10-05 至 2026-10-11'));
  expect(analyzeSpy).toHaveBeenCalledWith({ workspace_id: WS_DEMO_ID, directory_label: '/tmp/business-source', from: '2026-10-05', to: '2026-10-11', grain: 'week', generic: true, source_profile: undefined });
  expect(screen.getByTestId('saved-period-select')).toHaveValue(generatedRun);
  expect(await screen.findByTestId('metric-table')).toBeVisible();
});

async function monthClient(withExistingOctober = false, monthStartDay = 1) {
  const client = new FixtureClient();
  await client.periodSet({ schema_version: '1', operation: 'period_policy_set', workspace_id: WS_DEMO_ID,
    expected_revision: 4, policy: { ...fixturePeriodPolicies[1].policy, cadence: 'month', month_start_day: monthStartDay, effective_from: '2026-11-01' } });
  if (withExistingOctober) {
    const partial: HistoryEntryView = { ...structuredClone(fixtureHistory[0]), run_id: 'october-partial', cadence: 'month',
      period: { from: '2026-10-01', to: '2026-10-05' }, updated_at: '2026-10-20T00:00:00Z' };
    const list = client.historyList.bind(client);
    const show = client.historyShow.bind(client);
    vi.spyOn(client, 'historyList').mockImplementation(async (ws) => [...await list(ws), ...(ws === WS_DEMO_ID ? [partial] : [])]);
    vi.spyOn(client, 'historyShow').mockImplementation(async (ws, run, revision) => run === partial.run_id
      ? { ...await show(ws, RUN_2026_10), run_id: partial.run_id, grain: 'month', period: partial.period }
      : show(ws, run, revision));
  }
  return client;
}

async function openLocalAnalysis(client: FixtureClient) {
  window.localStorage.setItem(directoryKey, '/tmp/local-analysis-source');
  await openApp(client);
  navigate('经营分析');
  const summary = screen.queryByText('重新分析数据');
  if (summary) fireEvent.click(summary);
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(screen.getByLabelText('分析期间')).toBeEnabled());
  // generic_table candidates are intentionally gated until the user confirms
  // the directory/profile mapping; mirror that explicit product action here.
  navigate('数据源');
  const mappingButton = screen.queryByTestId('confirm-source-mapping-button');
  if (mappingButton) {
    fireEvent.click(screen.getByTestId('confirm-source-mapping-button'));
    await waitFor(() => expect(screen.queryByTestId('confirm-source-mapping-button')).toBeNull());
  }
  navigate('经营分析');
  await waitFor(() => expect(screen.getByLabelText('分析期间')).toBeEnabled());
}

it('只导入月初几天时，月度下拉仍提交该月自然区间，首次分析直接运行', async () => {
  const client = await monthClient();
  const summary = { ...structuredClone(fixtureScan[WS_DEMO_ID]), coverage: { from: '2026-10-01', to: '2026-10-05', granularity: 'day' as const },
    available_periods: [{ from: '2026-10-01', to: '2026-10-31', grain: 'month' as const }],
    available_period_policy_id: (await client.workspaceList())[0].current_period_policy_id };
  vi.spyOn(client, 'scan').mockResolvedValue(summary);
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  const choices = within(screen.getByLabelText('分析期间')).getAllByRole('option');
  expect(choices).toHaveLength(1);
  expect(choices[0]).toHaveTextContent('2026年10月');
  expect(screen.queryByLabelText('分析开始日期')).toBeNull();
  expect(screen.queryByLabelText('分析结束日期')).toBeNull();
  expect(analyze).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  expect(screen.queryByTestId('analysis-overwrite-confirmation')).toBeNull();
  await waitFor(() => expect(analyze).toHaveBeenCalledOnce());
  expect(analyze).toHaveBeenCalledWith({ workspace_id: WS_DEMO_ID, directory_label: '/tmp/local-analysis-source',
    from: '2026-10-01', to: '2026-10-31', grain: 'month', generic: true, source_profile: undefined });
});

it('可用期间包含当前查看月份时默认保留该月，选择新月份不会自动分析', async () => {
  const client = await monthClient();
  vi.spyOn(client, 'scan').mockResolvedValue({ ...structuredClone(fixtureScan[WS_DEMO_ID]),
    available_period_policy_id: (await client.workspaceList())[0].current_period_policy_id, available_periods: [
    { from: '2026-09-01', to: '2026-09-30', grain: 'month' },
    { from: '2026-10-01', to: '2026-10-31', grain: 'month' },
  ] });
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  expect(screen.getByLabelText('分析期间')).toHaveValue('month:2026-09-01:2026-09-30');
  expect(within(screen.getByLabelText('分析期间')).getAllByRole('option').map((option) => option.textContent)).toEqual(['2026年10月', '2026年9月']);
  fireEvent.change(screen.getByLabelText('分析期间'), { target: { value: 'month:2026-10-01:2026-10-31' } });
  expect(analyze).not.toHaveBeenCalled();
});

it('已有该月结果时必须确认覆盖，取消不分析，确认后只运行一次', async () => {
  const client = await monthClient(true);
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  expect(screen.getByLabelText('分析期间')).toHaveValue('month:2026-10-01:2026-10-31');
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  const modal = screen.getByRole('dialog', { name: '确认重新分析' });
  expect(modal).toHaveTextContent('重新分析 2026年10月');
  expect(modal).toHaveTextContent('将覆盖该期间已有分析记录，以本次数据为准');
  expect(analyze).not.toHaveBeenCalled();
  fireEvent.click(within(modal).getByRole('button', { name: '取消' }));
  expect(screen.queryByTestId('analysis-overwrite-confirmation')).toBeNull();
  expect(analyze).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  fireEvent.click(screen.getByRole('button', { name: '覆盖并重新分析' }));
  await waitFor(() => expect(analyze).toHaveBeenCalledOnce());
  expect(analyze).toHaveBeenCalledWith(expect.objectContaining({ from: '2026-10-01', to: '2026-10-31', grain: 'month' }));
});

it('自定义月度跨月窗口覆盖同月份记录时，确认展示实际起止日并原样提交', async () => {
  const client = await monthClient(true, 31);
  vi.spyOn(client, 'scan').mockResolvedValue({ ...structuredClone(fixtureScan[WS_DEMO_ID]),
    coverage: { from: '2026-10-31', to: '2026-11-05', granularity: 'day' },
    available_period_policy_id: (await client.workspaceList())[0].current_period_policy_id,
    available_periods: [{ from: '2026-10-31', to: '2026-11-29', grain: 'month' }] });
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  expect(screen.getByLabelText('分析期间')).toHaveValue('month:2026-10-31:2026-11-29');
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  const modal = screen.getByRole('dialog', { name: '确认重新分析' });
  expect(modal).toHaveTextContent('重新分析 2026年10月');
  expect(modal).toHaveTextContent('分析范围：2026-10-31 至 2026-11-29');
  expect(analyze).not.toHaveBeenCalled();
  fireEvent.click(within(modal).getByRole('button', { name: '覆盖并重新分析' }));
  await waitFor(() => expect(analyze).toHaveBeenCalledOnce());
  expect(analyze).toHaveBeenCalledWith(expect.objectContaining({ from: '2026-10-31', to: '2026-11-29', grain: 'month' }));
});

it('覆盖确认打开后切换查看期间会关闭旧确认，不能误分析新目标', async () => {
  const client = await monthClient(true);
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  fireEvent.click(screen.getByRole('button', { name: '运行本地分析' }));
  await screen.findByRole('dialog', { name: '确认重新分析' });
  fireEvent.change(screen.getByTestId('saved-period-select'), { target: { value: SEPTEMBER } });
  await waitFor(() => expect(screen.getByTestId('current-period-label')).toHaveTextContent('2026-09-01 至 2026-09-30'));
  expect(screen.queryByTestId('analysis-overwrite-confirmation')).toBeNull();
  expect(analyze).not.toHaveBeenCalled();
});

it.each(['available_periods', 'available_period_policy_id'] as const)('扫描缺少 %s 时不从coverage补齐日期或运行分析', async (missingField) => {
  const client = new FixtureClient();
  const summary = structuredClone(fixtureScan[WS_DEMO_ID]);
  delete summary[missingField];
  vi.spyOn(client, 'scan').mockResolvedValue(summary);
  const analyze = vi.spyOn(client, 'analyze');
  window.localStorage.setItem(directoryKey, '/tmp/legacy-summary');
  await openApp(client);
  navigate('经营分析');
  fireEvent.click(screen.getByText('重新分析数据'));
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '重新扫描数据' })).toBeEnabled());
  expect(screen.getByLabelText('分析期间')).toBeDisabled();
  expect(screen.getByLabelText('分析期间')).toHaveTextContent('暂无可分析期间');
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  expect(analyze).not.toHaveBeenCalled();
});

it('重启后重新扫描失败显示错误，不从历史范围构造可分析选项', async () => {
  const client = new FixtureClient();
  vi.spyOn(client, 'scan').mockRejectedValue(new Error('本地目录不可读取'));
  const analyze = vi.spyOn(client, 'analyze');
  window.localStorage.setItem(directoryKey, '/tmp/missing-local-source');
  await openApp(client);
  navigate('经营分析');
  fireEvent.click(screen.getByText('重新分析数据'));
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  expect(await screen.findByRole('alert')).toHaveTextContent('本地目录不可读取');
  expect(screen.getByLabelText('分析期间')).toBeDisabled();
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  expect(analyze).not.toHaveBeenCalled();
});

it('系统粒度改变后旧扫描选项不能沿用，重新扫描成功才出现新粒度期间', async () => {
  const client = new FixtureClient();
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  expect(screen.getByLabelText('分析期间')).toHaveValue('day:2026-10-28:2026-10-28');
  navigate('系统设置');
  fireEvent.change(await screen.findByTestId('cadence-select'), { target: { value: 'month' } });
  fireEvent.change(screen.getByTestId('effective-from-input'), { target: { value: '2026-11-01' } });
  fireEvent.click(screen.getByTestId('save-policy-button'));
  await waitFor(() => expect(screen.getByTestId('saved-period-select')).toHaveValue(SEPTEMBER));
  navigate('经营分析');
  fireEvent.click(screen.getByText('重新分析数据'));
  expect(screen.getByLabelText('分析期间')).toBeDisabled();
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(screen.getByLabelText('分析期间')).toBeEnabled());
  expect(screen.getByLabelText('分析期间')).toHaveValue('month:2026-10-01:2026-10-31');
  expect(screen.getByLabelText('分析期间')).not.toHaveTextContent('10月28日');
  expect(analyze).not.toHaveBeenCalled();
});

it('即使粒度相同，周期策略版本改变也必须重新扫描才可分析', async () => {
  const client = new FixtureClient();
  const analyze = vi.spyOn(client, 'analyze');
  await openLocalAnalysis(client);
  expect(screen.getByLabelText('分析期间')).toBeEnabled();
  await act(async () => { await client.periodSet({ schema_version: '1', operation: 'period_policy_set', workspace_id: WS_DEMO_ID,
    expected_revision: 4, policy: { ...fixturePeriodPolicies[1].policy, week_start_day: 2, effective_from: '2026-11-01' } }); });
  fireEvent.click(screen.getByRole('button', { name: '刷新所选期间' }));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).toBeEnabled());
  expect(screen.getByLabelText('分析期间')).toBeDisabled();
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(screen.getByLabelText('分析期间')).toBeEnabled());
  expect(analyze).not.toHaveBeenCalled();
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
  const client = dailyPeriodsClient();
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
  const client = dailyPeriodsClient();
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

it('同粒度同区间只展示最后更新的结果，旧版本没有选择入口', async () => {
  const client = new FixtureClient();
  const originalShow = client.historyShow.bind(client);
  const newest: HistoryEntryView = { ...structuredClone(fixtureHistory[0]), run_id: 'latest-same-period', current_revision_no: 1, updated_at: '2026-11-01T00:00:00Z' };
  const empty: HistoryEntryView = { ...structuredClone(newest), run_id: 'empty-latest', updated_at: '2026-11-02T00:00:00Z', summary: { metric_count: 0, task_count: 0, chart_count: 0 } };
  vi.spyOn(client, 'historyList').mockResolvedValue([empty, fixtureHistory[1], fixtureHistory[0], newest]);
  const show = vi.spyOn(client, 'historyShow').mockImplementation(async (ws, run) => ({
    ...await originalShow(ws, RUN_2026_10), run_id: run, analysis_revision_no: run === newest.run_id ? 1 : 2,
  }));
  await openApp(client);
  const options = within(screen.getByTestId('saved-period-select')).getAllByRole('option');
  expect(options).toHaveLength(1);
  expect(options[0]).toHaveValue(newest.run_id);
  expect(show).toHaveBeenLastCalledWith(WS_DEMO_ID, newest.run_id);
  navigate('历史查询');
  expect(within(screen.getByTestId('history-table')).getAllByRole('row')).toHaveLength(2);
  expect(screen.queryByRole('button', { name: /^r[0-9]+$/ })).toBeNull();
  expect(screen.queryByRole('button', { name: '查看该期间当前版本' })).toBeNull();
  expect(screen.getByTestId('history-detail')).toHaveTextContent('最新分析结果');
  expect(show.mock.calls.every((call) => call.length === 2)).toBe(true);
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

it('待应用任务候选保留在原期间，显式收起后才解除上下文锁', async () => {
  const client = new FixtureClient();
  await openApp(client);
  navigate('任务单');
  fireEvent.click(await screen.findByTestId('generate-task-button'));
  fireEvent.click(await screen.findByTestId('request-task-preview'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await screen.findByTestId('candidate-panel');
  expect(screen.getByTestId('saved-period-select')).toBeDisabled();
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '暂时收起' }));
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
