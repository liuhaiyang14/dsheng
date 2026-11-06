// 本地 UI 夹具回归，不调用 Provider；生产持久化另由 Rust/Tauri 回归验证。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureTasks, fixtureWorkspaces, RUN_2026_10, WS_DEMO_ID } from '../api/fixtureData';
import { TaskCreateDialog } from '../components/TaskCreateDialog';
import { TaskDrawer } from '../components/TaskDrawer';
import { normalizeTaskDecimal } from '../components/taskForm';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

const change = (name: string, value: string) => fireEvent.change(screen.getByLabelText(name), { target: { value } });

describe('人工任务与行动项', () => {
  it('直接新增本地任务与多个行动项，打开详情且不发起模型生成', async () => {
    const client = new FixtureClient();
    const create = vi.spyOn(client, 'taskCreate');
    const generate = vi.spyOn(client, 'taskGeneratePreview');
    const confirm = vi.spyOn(client, 'taskGenerateConfirm');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    fireEvent.click(screen.getAllByText('任务单')[0].closest('button')!);
    fireEvent.click(await screen.findByTestId('manual-create-task-button'));
    const dialog = await screen.findByRole('dialog', { name: '新增任务' });
    change('任务名称', '人工核对客户交付');
    change('任务说明', '按实际情况补充');
    change('目标数值', '0.00');
    change('数值依据', '已确认的零差错目标');
    change('行动内容 1', '核对交付清单');
    change('新行动 1 负责人', '运营负责人');
    change('新行动 1 开始日期', '2026-11-02');
    change('新行动 1 截止日期', '2026-11-05');
    change('新行动 1 验收标准', '逐笔核对并留档');
    fireEvent.click(within(dialog).getByText('新增行动项'));
    change('行动内容 2', '确认后续处理安排');
    fireEvent.click(within(dialog).getByText('创建任务'));
    const drawer = await screen.findByTestId('task-drawer');
    expect(within(drawer).getByLabelText('任务名称')).toHaveValue('人工核对客户交付');
    expect(within(drawer).getByLabelText('行动 1 负责人')).toHaveValue('运营负责人');
    expect(within(drawer).getByTestId('drawer-step-count')).toHaveTextContent('0 / 2');
    expect(within(drawer).getByTestId('task-smart-check')).toHaveTextContent('暂无 SMART 评价');
    const input = create.mock.calls[0][0];
    expect(input.content.target_value).toBe('0');
    expect(input.source_note).toBe('已确认的零差错目标');
    expect(input.cycle_role).toBe('plan');
    expect(input.steps[0]).toMatchObject({ action: '核对交付清单', due_date: '2026-11-05', done_criteria: '逐笔核对并留档' });
    expect(generate).not.toHaveBeenCalled(); expect(confirm).not.toHaveBeenCalled();
    expect((await client.taskList(WS_DEMO_ID, RUN_2026_10)).some((task) => task.title === '人工核对客户交付')).toBe(true);
  });

  it('保存响应丢失后保留草稿并以同一操作重试，不重复创建', async () => {
    const client = new FixtureClient();
    const detail = await client.historyShow(WS_DEMO_ID, RUN_2026_10);
    const original = client.taskCreate.bind(client);
    let first = true;
    const create = vi.spyOn(client, 'taskCreate').mockImplementation(async (input) => {
      const result = await original(input);
      if (first) { first = false; throw new Error('本地响应未收到，请重试'); }
      return result;
    });
    const onCreated = vi.fn();
    render(<TaskCreateDialog client={client} workspaceId={WS_DEMO_ID} detail={detail} onClose={vi.fn()} onCreated={onCreated} />);
    change('任务名称', '响应重试任务'); change('行动内容 1', '核对记录');
    fireEvent.click(screen.getByText('创建任务'));
    await screen.findByText('本地响应未收到，请重试');
    expect(screen.getByLabelText('任务名称')).toHaveValue('响应重试任务');
    fireEvent.click(screen.getByText('创建任务'));
    await waitFor(() => expect(onCreated).toHaveBeenCalledTimes(1));
    expect(create.mock.calls[0][0].mutation_id).toBe(create.mock.calls[1][0].mutation_id);
    expect((await client.taskList(WS_DEMO_ID)).filter((task) => task.title === '响应重试任务')).toHaveLength(1);
  });

  it('修改任务及行动细节保留人工进度，数值按字符串规范化并标记旧评价待复核', async () => {
    const client = new FixtureClient();
    const source = fixtureTasks[0];
    await client.taskUpdate({ schema_version: '1', workspace_id: WS_DEMO_ID, task_id: source.task_id, expected_task_revision: source.task_revision_no, mutation_id: crypto.randomUUID(), operation: { kind: 'progress', progress_mode: 'manual', progress: '40', status: 'in_progress', adjustment_reason: '人工确认完成比例', blocked_reason: null } });
    const task = await client.taskShow(WS_DEMO_ID, source.task_id);
    const update = vi.spyOn(client, 'taskUpdate');
    render(<TaskDrawer task={task} workspace={fixtureWorkspaces[0]} client={client} onClose={vi.fn()} onSaved={vi.fn()} periodEnd="2026-12-31" />);
    change('任务名称', '人工修订的任务');
    change('行动步骤 1', '按实际流程核对');
    change('行动 1 负责人', '复核岗');
    change('行动 1 验收标准', '记录复核结论');
    change('实绩数值', '12345678901234567890.120000');
    change('人工调整依据（改数值必填）', '实际核对结果');
    fireEvent.click(screen.getByText('保存任务'));
    await waitFor(() => expect(screen.getByTestId('task-save-status')).toHaveTextContent('已保存'));
    const stored = await client.taskShow(WS_DEMO_ID, task.task_id);
    expect(stored.progress).toBe(task.progress); expect(stored.progress_mode).toBe('manual');
    expect(stored.steps[0]).toMatchObject({ action: '按实际流程核对', owner: '复核岗', done_criteria: '记录复核结论' });
    expect(stored.actual_value).toBe('12345678901234567890.12');
    expect(stored.smart_check.smart_status).toBe('stale');
    expect(update.mock.calls[0][0].operation).toMatchObject({ kind: 'content', source_note: '实际核对结果', patch: { actual_value: '12345678901234567890.12' } });
    expect(document.querySelector('.drawer-head .meta')).toHaveTextContent('本期检视');
  });

  it('取消重开不改变勾选；新增未保存行动不能提前标完成，新增保存也需确认重开', async () => {
    const client = new FixtureClient();
    const base = fixtureTasks.find((task) => task.progress === '33.33')!;
    await client.taskUpdate({ schema_version: '1', workspace_id: WS_DEMO_ID, task_id: base.task_id, expected_task_revision: base.task_revision_no, mutation_id: crypto.randomUUID(), operation: { kind: 'content', step_operations: base.steps.map((step) => ({ op: 'update', step_id: step.step_id, patch: { status: 'completed' } })) } });
    const task = await client.taskShow(WS_DEMO_ID, base.task_id);
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(false);
    const update = vi.spyOn(client, 'taskUpdate');
    render(<TaskDrawer task={task} workspace={fixtureWorkspaces[0]} client={client} onClose={vi.fn()} onSaved={vi.fn()} />);
    fireEvent.click(screen.getByLabelText('完成步骤 1'));
    expect(screen.getByLabelText('完成步骤 1')).toBeChecked();
    expect(screen.getByTestId('task-save-status')).toHaveTextContent('当前任务无待保存更改');
    fireEvent.click(screen.getByText('添加步骤'));
    const index = task.steps.length + 1;
    expect(screen.getByLabelText(`完成步骤 ${index}`)).toBeDisabled();
    change(`行动步骤 ${index}`, '补充人工发现的遗漏');
    fireEvent.click(screen.getByText('保存任务'));
    expect(confirm).toHaveBeenCalledTimes(2);
    expect(update).not.toHaveBeenCalled();
    expect(screen.getByLabelText(`行动步骤 ${index}`)).toHaveValue('补充人工发现的遗漏');
    confirm.mockReturnValue(true);
    fireEvent.click(screen.getByText('保存任务'));
    await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
    expect((await client.taskShow(WS_DEMO_ID, task.task_id)).status).toBe('in_progress');
  });

  it('只读上下文不能保存或修改，准确使用保存的任务计划期间', async () => {
    const client = new FixtureClient();
    const task = await client.taskShow(WS_DEMO_ID, fixtureTasks[0].task_id);
    const { unmount } = render(<TaskDrawer task={task} workspace={fixtureWorkspaces[0]} client={client} onClose={vi.fn()} onSaved={vi.fn()} readOnly />);
    expect(screen.getByLabelText('任务名称')).toBeDisabled();
    expect(screen.getByText('保存任务')).toBeDisabled();
    unmount();
    const detail = await client.historyShow(WS_DEMO_ID, RUN_2026_10);
    detail.task_plan_period = { from: '2027-02-28', to: '2027-03-30' };
    render(<TaskCreateDialog client={client} workspaceId={WS_DEMO_ID} detail={detail} onClose={vi.fn()} onCreated={vi.fn()} />);
    expect(screen.getByText(/任务期间：2027-02-28 至 2027-03-30/)).toBeInTheDocument();
    expect(screen.getByLabelText('新行动 1 截止日期')).toHaveAttribute('max', '2027-03-30');
  });
});

it('金额规范化保留完整精度，兼容小数尾零和负零，不接受超出精度的数值', () => {
  expect(normalizeTaskDecimal(' 12345678901234567890.120000 ')).toBe('12345678901234567890.12');
  expect(normalizeTaskDecimal('100.00')).toBe('100');
  expect(normalizeTaskDecimal('-0.000')).toBe('0');
  expect(normalizeTaskDecimal('0.1234567')).toBeNull();
  expect(normalizeTaskDecimal('1e6')).toBeNull();
});
