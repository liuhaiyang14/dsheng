// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri）。
// S6 任务屏：显式生成门禁、预览可选引用诊断、详情抽屉进度规则呈现、
// 类别中文标签、时间线排序（缺日期不排）。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureAnalysisRun, fixtureTasks, fixtureWorkspaces } from '../api/fixtureData';
import { sortTimelineEntries, TimelineBoard } from '../components/TimelineBoard';
import { TaskDrawer } from '../components/TaskDrawer';
import type { TaskCandidateView, TaskView, TimelineEntryView, WorkspaceView } from '../api';

afterEach(cleanup);
vi.stubGlobal('crypto', {
  ...(typeof crypto !== 'undefined' ? crypto : {}),
  randomUUID: () => '00000000-0000-4000-8000-0000000000f' + Math.floor(Math.random() * 9),
});

async function openTasks(client: FixtureClient) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getAllByText('任务单')[0].closest('button') as HTMLButtonElement);
  await screen.findByTestId('generate-task-button');
}

async function sendTasks(client: FixtureClient) {
  await openTasks(client);
  fireEvent.click(screen.getByTestId('generate-task-button'));
  await screen.findByTestId('generate-param-modal');
  fireEvent.click(screen.getByTestId('request-task-preview'));
  await screen.findByTestId('preview-modal');
  fireEvent.click(screen.getByTestId('preview-confirm'));
}

async function generateFlow(client: FixtureClient) {
  await sendTasks(client);
  await screen.findByTestId('candidate-panel');
  fireEvent.click(screen.getByTestId('generation-close'));
  await waitFor(() => expect(screen.queryByTestId('preview-modal')).toBeNull());
}

describe('S6 显式生成门禁（P0）', () => {
  it('进入任务页 / 勾选引用诊断报告 / 取消勾选 均不触发生成', async () => {
    const client = new FixtureClient();
    const previewSpy = vi.spyOn(client, 'taskGeneratePreview');
    await openTasks(client);
    await screen.findByTestId('task-list-table');
    expect(previewSpy).not.toHaveBeenCalled();
    const useBox = screen.getByTestId('use-diagnosis-context') as HTMLInputElement;
    fireEvent.click(useBox);
    fireEvent.click(useBox);
    await waitFor(() => {});
    expect(previewSpy).not.toHaveBeenCalled();
  });

  it('显式按钮流：参数 → 预览（可选引用报告）→ 确认 → 候选 → 应用 → 打开任务详情', async () => {
    const client = new FixtureClient();
    await generateFlow(client);
    // 候选面板出现两条候选
    const panel = screen.getByTestId('candidate-panel');
    expect(panel.textContent).toContain('建立高折扣订单预警阈值');
    fireEvent.click(screen.getByText('应用所选（本地确认）'));
    await waitFor(() => {
      expect(screen.getByTestId('task-notice').textContent).toContain('已应用');
    });
    await waitFor(() => {
      expect(screen.getByTestId('task-list-table').textContent).toContain(
        '建立高折扣订单预警阈值',
      );
    });
    fireEvent.click(within(screen.getByTestId('task-list-table')).getByText('建立高折扣订单预警阈值'));
    const drawer = await screen.findByTestId('task-drawer');
    expect(within(drawer).getByLabelText('任务名称')).toHaveValue('建立高折扣订单预警阈值');
    expect(within(drawer).getByTestId('task-smart-check')).toHaveTextContent('生成时未给出期限，保留待确认');
    expect(within(drawer).getAllByRole('textbox', { name: /行动步骤/ }).length).toBeGreaterThan(0);
    expect(screen.queryByText('页面暂时无法显示')).toBeNull();
  });

  it('取消预览不确认不发送', async () => {
    const client = new FixtureClient();
    const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm');
    await openTasks(client);
    fireEvent.click(screen.getByTestId('generate-task-button'));
    await screen.findByTestId('generate-param-modal');
    fireEvent.click(screen.getByTestId('request-task-preview'));
    await screen.findByTestId('preview-modal');
    fireEvent.click(screen.getByText('取消（不发送）'));
    await waitFor(() => expect(screen.queryByTestId('preview-modal')).toBeNull());
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it.each([
    ['仅返回保存回执', (candidate: TaskCandidateView) => ({
      candidate_generation_id: candidate.candidate_generation_id,
      candidate_revision_no: candidate.candidate_revision_no,
      task_count: candidate.tasks.length,
    })],
    ['任务缺少步骤列表', (candidate: TaskCandidateView) => ({
      ...candidate, tasks: candidate.tasks.map((task) => ({ ...task, steps: undefined })),
    })],
    ['任务标题是对象', (candidate: TaskCandidateView) => ({
      ...candidate, tasks: candidate.tasks.map((task) => ({ ...task, title: { text: task.title } })),
    })],
    ['结果属于其他分析', (candidate: TaskCandidateView) => ({ ...candidate, run_id: 'another-run' })],
  ])('%s时留在可关闭的错误弹窗，不崩溃、不重发、不应用', async (_label, malformed) => {
    const client = new FixtureClient();
    const original = client.taskGenerateConfirm.bind(client);
    const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm').mockImplementation(async (id) =>
      malformed(await original(id)) as unknown as TaskCandidateView);
    const applySpy = vi.spyOn(client, 'taskReconcileApply');
    await sendTasks(client);
    const dialog = screen.getByRole('dialog');
    await waitFor(() => expect(within(dialog).getByRole('alert')).toHaveTextContent('已收到生成结果'));
    expect(within(dialog).getByTestId('generation-progress-stage')).toHaveTextContent('生成失败');
    expect(within(dialog).getByRole('alert')).toHaveTextContent('请勿重复生成');
    expect(screen.queryByTestId('candidate-panel')).toBeNull();
    expect(screen.queryByText('页面暂时无法显示')).toBeNull();
    expect(screen.getByTestId('task-list-table')).toBeTruthy();
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(applySpy).not.toHaveBeenCalled();
    expect(within(dialog).queryByText('重试连接并发送')).toBeNull();
    fireEvent.click(within(dialog).getByRole('button', { name: '关闭' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(screen.getByTestId('generate-task-button')).not.toBeDisabled();
  });

  it('合法空候选明确显示无建议，不伪造任务或允许空应用', async () => {
    const client = new FixtureClient();
    const original = client.taskGenerateConfirm.bind(client);
    vi.spyOn(client, 'taskGenerateConfirm').mockImplementation(async (id) => ({ ...await original(id), tasks: [] }));
    await generateFlow(client);
    expect(screen.getByTestId('candidate-panel')).toHaveTextContent('本次没有生成可应用的任务建议');
    expect(screen.getByRole('button', { name: '应用所选（本地确认）' })).toBeDisabled();
    expect(screen.queryByText('页面暂时无法显示')).toBeNull();
  });
});

describe('S6 任务列表呈现', () => {
  it('类别显示中文标签；周期角色与进度呈现', async () => {
    const client = new FixtureClient();
    await openTasks(client);
    const table = screen.getByTestId('task-list-table');
    expect(table.textContent).toContain('销售最大化');
    expect(table.textContent).toContain('费用最小化');
    expect(table.textContent).toContain('时间最短化');
    expect(table.textContent).toContain('其他事项');
    expect(table.textContent).toContain('本期检视');
    expect(table.textContent).toContain('下期计划');
    // 进度为核心返回字符串：33.33 与 99.99 场景由组件测试单独覆盖
    expect(table.textContent).toContain('33.33% · 进行中');
    expect(table.textContent).toContain('40% · 有阻塞');
  });
});

describe('本地恢复已生成候选', () => {
  it('显式读取已存候选后可应用，并使用原指纹，不重新预览或请求模型', async () => {
    const client = new FixtureClient();
    // 仅预置 FixtureClient 的本地演示状态；没有网络或真实模型调用。
    const initialPreview = await client.taskGeneratePreview({
      workspace_id: fixtureWorkspaces[0].workspace_id, run_id: fixtureAnalysisRun.run_id,
    });
    await client.taskGenerateConfirm(initialPreview.confirmation_id);
    const previewSpy = vi.spyOn(client, 'taskGeneratePreview');
    const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm');
    const readSpy = vi.spyOn(client, 'taskCandidateLatest');
    const drySpy = vi.spyOn(client, 'taskReconcileDryRun');
    await openTasks(client);
    expect(readSpy).not.toHaveBeenCalled();
    expect(screen.queryByTestId('candidate-panel')).toBeNull();
    fireEvent.click(screen.getByTestId('read-saved-task-candidate'));
    const panel = await screen.findByTestId('candidate-panel');
    expect(panel).toHaveTextContent('建立高折扣订单预警阈值');
    expect(screen.getByTestId('task-notice')).toHaveTextContent('已读取本地保存的候选，未调用模型');
    expect(readSpy).toHaveBeenCalledWith(fixtureWorkspaces[0].workspace_id, fixtureAnalysisRun.run_id, fixtureAnalysisRun.current_revision_no);
    fireEvent.click(screen.getByRole('button', { name: '暂时收起' }));
    await waitFor(() => expect(screen.queryByTestId('candidate-panel')).toBeNull());
    expect(readSpy).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByTestId('read-saved-task-candidate'));
    await screen.findByTestId('candidate-panel');
    expect(readSpy).toHaveBeenCalledTimes(2);
    fireEvent.click(screen.getByRole('button', { name: '应用所选（本地确认）' }));
    await waitFor(() => expect(screen.getByTestId('task-notice')).toHaveTextContent('已应用'));
    expect(drySpy.mock.calls[0][0].input_fingerprint).toBe(initialPreview.analysis_input_fingerprint);
    expect(screen.getByTestId('task-list-table')).toHaveTextContent('建立高折扣订单预警阈值');
    expect(previewSpy).not.toHaveBeenCalled();
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it('无可恢复候选时明确提示，不生成替代结果', async () => {
    const client = new FixtureClient();
    const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm');
    await openTasks(client);
    fireEvent.click(screen.getByTestId('read-saved-task-candidate'));
    await waitFor(() => expect(screen.getByTestId('task-notice')).toHaveTextContent('本期没有可恢复的待应用结果'));
    expect(screen.queryByTestId('candidate-panel')).toBeNull();
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it('恢复接口返回不完整结果时不挂载坏候选，也不会重发', async () => {
    const client = new FixtureClient();
    vi.spyOn(client, 'taskCandidateLatest').mockResolvedValue({ candidate: {}, analysis_input_fingerprint: 'saved-fingerprint' } as never);
    const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm');
    await openTasks(client);
    fireEvent.click(screen.getByTestId('read-saved-task-candidate'));
    await waitFor(() => expect(screen.getByTestId('task-notice')).toHaveTextContent('任务候选数据不完整'));
    expect(screen.queryByText('页面暂时无法显示')).toBeNull();
    expect(screen.queryByTestId('candidate-panel')).toBeNull();
    expect(confirmSpy).not.toHaveBeenCalled();
  });

  it('生成预览缺少指纹时不能用时间戳替代并应用', async () => {
    const client = new FixtureClient();
    const original = client.taskGeneratePreview.bind(client);
    vi.spyOn(client, 'taskGeneratePreview').mockImplementation(async (request) => ({
      ...await original(request), analysis_input_fingerprint: '',
    }));
    const drySpy = vi.spyOn(client, 'taskReconcileDryRun');
    await generateFlow(client);
    fireEvent.click(screen.getByRole('button', { name: '应用所选（本地确认）' }));
    expect(screen.getByTestId('task-notice')).toHaveTextContent('候选缺少分析校验信息');
    expect(drySpy).not.toHaveBeenCalled();
  });
});

describe('S6 详情抽屉', () => {
  const workspace: WorkspaceView = {
    workspace_id: '00000000-0000-4000-8000-0000000000a1',
    name: '示例制造企业',
    config_revision: 4,
    active_provider_id: null,
    current_period_policy_id: 'x',
    created_at: '2026-09-01T02:00:00Z',
    updated_at: '2026-09-01T02:00:00Z',
  };

  it('auto 模式显示核心计算值（33.33）且不提交人工进度', async () => {
    const client = new FixtureClient();
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    const updateSpy = vi.spyOn(client, 'taskUpdate');
    render(
      <TaskDrawer
        task={structuredClone(source)}
        workspace={workspace}
        client={client}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByTestId('drawer-progress').textContent).toBe('33.33%');
    expect(screen.getByTestId('auto-progress-note').textContent).toContain('33.33');
    expect(screen.getByTestId('auto-progress-note').textContent).toContain('1/3');
    // auto 模式下无手工进度输入
    expect(screen.queryByTestId('manual-progress-input')).toBeNull();

    // 勾选全部步骤并保存 → 核心 auto 计算 100
    const checkboxes = screen.getAllByRole('checkbox', { name: /完成步骤/ });
    fireEvent.click(checkboxes[1]);
    fireEvent.click(checkboxes[2]);
    fireEvent.click(screen.getByText('保存任务'));
    await waitFor(() => {
      expect(screen.getByTestId('drawer-progress').textContent).toBe('100%');
    });
    const op = updateSpy.mock.calls[0][0].operation;
    if (op.kind !== 'content') throw new Error('应为 content 操作');
    expect(op.step_operations?.length).toBeGreaterThan(0);
  });

  it('99.99 上限：核心返回的封顶值原样呈现，不重算为 100', () => {
    const client = new FixtureClient();
    const task = structuredClone(fixtureTasks[0]) as TaskView;
    task.progress = '99.99';
    task.steps = Array.from({ length: 3 }, (_, i) => ({
      ...task.steps[0],
      step_id: `s-${i}`,
      status: i < 2 ? 'completed' : 'in_progress',
    }));
    render(
      <TaskDrawer task={task} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />,
    );
    expect(screen.getByTestId('drawer-progress').textContent).toBe('99.99%');
    expect(screen.getByTestId('drawer-step-count').textContent).toContain('2 / 3');
  });

  it('block 优先：blocked 任务显示阻塞理由', () => {
    const client = new FixtureClient();
    const blocked = fixtureTasks.find((t) => t.status === 'blocked')!;
    render(
      <TaskDrawer
        task={structuredClone(blocked)}
        workspace={workspace}
        client={client}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );
    expect(screen.getByTestId('drawer-progress').textContent).toBe('40%');
    expect(screen.getByText(/block 优先于其他状态展示/)).toBeTruthy();
    expect(screen.getByText(/财务系统接口授权未下发/)).toBeTruthy();
  });

  it('manual 必填原因；blocked 必填理由；完成必须 100（UI 预校验）', async () => {
    const client = new FixtureClient();
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    render(
      <TaskDrawer
        task={structuredClone(source)}
        workspace={workspace}
        client={client}
        onClose={() => {}}
        onSaved={() => {}}
      />,
    );
    fireEvent.change(screen.getByTestId('progress-mode-select'), { target: { value: 'manual' } });
    fireEvent.change(screen.getByTestId('manual-progress-input'), { target: { value: '60' } });
    // 原因为空 → 阻止
    fireEvent.click(screen.getByText('保存进度'));
    expect(screen.getByTestId('task-save-status').textContent).toContain('必须填写调整原因');
    fireEvent.change(document.getElementById('adjust-reason')!, {
      target: { value: '两步并一步推进' },
    });
    // completed 但进度非 100 → 阻止
    fireEvent.change(screen.getByTestId('progress-status-select'), {
      target: { value: 'completed' },
    });
    fireEvent.click(screen.getByText('保存进度'));
    expect(screen.getByTestId('task-save-status').textContent).toContain('100%');
    // 修正为 100 后保存成功
    fireEvent.change(screen.getByTestId('manual-progress-input'), { target: { value: '100' } });
    fireEvent.change(screen.getByTestId('progress-status-select'), {
      target: { value: 'completed' },
    });
    fireEvent.click(screen.getByText('保存进度'));
    await waitFor(() => {
      expect(screen.getByTestId('task-save-status').textContent).toContain('进度已保存');
    });
    // blocked 必填理由（manual 模式下调整原因也要同时满足）
    fireEvent.change(document.getElementById('adjust-reason')!, {
      target: { value: '外部依赖延期，整体评估下调' },
    });
    fireEvent.change(screen.getByTestId('progress-status-select'), {
      target: { value: 'blocked' },
    });
    fireEvent.click(screen.getByText('保存进度'));
    expect(screen.getByTestId('task-save-status').textContent).toContain('阻塞理由');
  });

  it('重开需确认；确认后核心写 reopened 并清空完成时间', async () => {
    const client = new FixtureClient();
    const confirmSpy = vi.spyOn(window, 'confirm').mockReturnValue(true);
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    const task = structuredClone(source);
    // 先构造已完成任务
    await client.taskUpdate({
      schema_version: '1',
      workspace_id: workspace.workspace_id,
      task_id: task.task_id,
      expected_task_revision: task.task_revision_no,
      mutation_id: '00000000-0000-4000-8000-0000000000aa',
      operation: {
        kind: 'content',
        step_operations: task.steps.map((s) => ({
          op: 'update' as const,
          step_id: s.step_id,
          patch: { status: 'completed' as const },
        })),
      },
    });
    const completed = await client.taskShow(workspace.workspace_id, task.task_id);
    render(
      <TaskDrawer task={completed} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />,
    );
    expect(screen.getByTestId('drawer-progress').textContent).toBe('100%');
    // 重开（manual 40% in_progress）
    fireEvent.change(screen.getByTestId('progress-mode-select'), { target: { value: 'manual' } });
    fireEvent.change(screen.getByTestId('manual-progress-input'), { target: { value: '40' } });
    fireEvent.change(document.getElementById('adjust-reason')!, {
      target: { value: '发现遗漏口径，重开补做' },
    });
    fireEvent.change(screen.getByTestId('progress-status-select'), {
      target: { value: 'in_progress' },
    });
    fireEvent.click(screen.getByText('保存进度'));
    await waitFor(() => {
      expect(screen.getByTestId('task-save-status').textContent).toContain('进度已保存');
    });
    expect(confirmSpy).toHaveBeenCalled();
    expect(screen.getByTestId('drawer-timeline').textContent).toContain('reopened');
    expect(within(screen.getByText('未完成').parentElement!).getByText('未完成')).toBeTruthy();
    confirmSpy.mockRestore();
  });

  it('SMART 校核展示 generated / stale', () => {
    const client = new FixtureClient();
    const generated = fixtureTasks.find(
      (t) => t.smart_check.smart_status === 'generated',
    )!;
    const stale = fixtureTasks.find((t) => t.smart_check.smart_status === 'stale')!;
    const { rerender } = render(
      <TaskDrawer task={structuredClone(generated)} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />,
    );
    expect(screen.getByText('SMART：生成时评价')).toBeTruthy();
    rerender(
      <TaskDrawer task={structuredClone(stale)} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />,
    );
    expect(screen.getByText(/内容已改待复核/)).toBeTruthy();
    expect(screen.getByTestId('task-smart-check')).toHaveTextContent(`任务版本 r${stale.smart_check.assessed_task_revision} 的历史评价`);
    expect(screen.getByTestId('task-smart-check')).toHaveTextContent('尚未复核当前内容');
  });

  it.each([
    ['缺失', undefined],
    ['为空', null],
    ['历史原始形状', { overall_goal: { criteria: { S: { status: 'met' } } }, subgoals: [] }],
    ['缺少检查列表', { smart_status: 'generated', assessed_task_revision: 1 }],
    ['检查列表不是数组', { smart_status: 'generated', assessed_task_revision: 1, checks: {} }],
    ['列表含空值', { smart_status: 'generated', assessed_task_revision: 1, checks: [null] }],
    ['非合同布尔值', { smart_status: 'generated', assessed_task_revision: 1, checks: [{ dimension: 'S', pass: true, note: '历史演示形状' }] }],
    ['未知状态', { smart_status: 'generated', assessed_task_revision: 1, checks: [{ dimension: 'S', status: 'unknown', note: null }] }],
    ['备注是对象', { smart_status: 'generated', assessed_task_revision: 1, checks: [{ dimension: 'S', status: 'met', note: {} }] }],
    ['空列表', { smart_status: 'generated', assessed_task_revision: 1, checks: [] }],
  ])('SMART %s时诚实提示不可用，详情、步骤和编辑仍可用', (_label, smart) => {
    const client = new FixtureClient();
    const source = structuredClone(fixtureTasks[0]);
    source.smart_check = smart as unknown as TaskView['smart_check'];
    render(<TaskDrawer task={source} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />);
    const section = screen.getByTestId('task-smart-check');
    expect(section).toHaveTextContent('SMART 评价暂不可用');
    expect(section).not.toHaveTextContent('通过');
    expect(section).not.toHaveTextContent('生成时评价');
    expect(screen.getByTestId('drawer-step-count')).toHaveTextContent(`${source.steps.length} 个步骤`);
    expect(screen.getByTestId('drawer-timeline')).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText('任务名称'), { target: { value: '仍可编辑任务' } });
    expect(screen.getByLabelText('任务名称')).toHaveValue('仍可编辑任务');
    expect(screen.getByRole('button', { name: '保存任务' })).toBeEnabled();
    expect(screen.getByTestId('task-save-status')).toHaveTextContent('有未保存更改');
  });

  it('按合同展示 SMART 三种状态，保留总目标与子目标重复维度及空备注', () => {
    const client = new FixtureClient();
    const source = structuredClone(fixtureTasks[0]);
    source.smart_check.checks = [
      { dimension: 'S', status: 'met', note: '总目标：核实回款' },
      { dimension: 'M', status: 'needs_confirmation', note: '总目标：数值待确认' },
      { dimension: 'A', status: 'not_applicable', note: null },
      { dimension: 'S', status: 'needs_confirmation', note: '子目标：补充客户清单' },
    ];
    render(<TaskDrawer task={source} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />);
    const section = within(screen.getByTestId('task-smart-check'));
    expect(section.getByText('通过 · 总目标：核实回款')).toBeInTheDocument();
    expect(section.getByText('待确认 · 总目标：数值待确认')).toBeInTheDocument();
    expect(section.getByText('不适用')).toBeInTheDocument();
    expect(section.getByText('待确认 · 子目标：补充客户清单')).toBeInTheDocument();
    expect(section.getAllByText('具体性 S')).toHaveLength(2);
  });

  it('步骤增删改排序：删除最后一步被 UI 阻止（AT_LEAST_ONE_STEP 预提示）', () => {
    const client = new FixtureClient();
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    render(
      <TaskDrawer task={structuredClone(source)} workspace={workspace} client={client} onClose={() => {}} onSaved={() => {}} />,
    );
    const removeButtons = screen.getAllByRole('button', { name: '删除步骤' });
    removeButtons.forEach((b) => fireEvent.click(b));
    // 3 步删到剩 1 步后，再删被 UI 阻止
    const remaining = screen.getAllByRole('button', { name: '删除步骤' });
    fireEvent.click(remaining[0]);
    expect(screen.getByTestId('task-save-status').textContent).toContain(
      '至少一条行动步骤',
    );
  });
});

describe('S6 时间线排序（组件测试）', () => {
  const entries: TimelineEntryView[] = [
    {
      task_id: 't1',
      task_title: 'A 任务',
      kind: 'planned_start',
      date: '2026-10-29',
      occurred_at: null,
      detail: '任务计划开始',
    },
    {
      task_id: 't2',
      task_title: 'B 任务',
      kind: 'progress_event',
      date: '2026-11-02',
      occurred_at: '2026-11-02T08:00:00Z',
      detail: '较早事件',
    },
    {
      task_id: 't3',
      task_title: 'C 任务',
      kind: 'due',
      date: null,
      occurred_at: null,
      detail: '无日期节点',
    },
    {
      task_id: 't4',
      task_title: 'D 任务',
      kind: 'completed',
      date: '2026-11-05',
      occurred_at: '2026-11-05T02:00:00Z',
      detail: '完成',
    },
  ];

  it('有日期条目倒序；缺日期归入底部且标注“未设置”，不按行序猜测', () => {
    const sorted = sortTimelineEntries(entries);
    expect(sorted.map((e) => e.date)).toEqual(['2026-11-05', '2026-11-02', '2026-10-29', null]);
    const { container } = render(<TimelineBoard entries={entries} />);
    const items = container.querySelectorAll('.timeline-item');
    expect(items[items.length - 1].textContent).toContain('未设置');
    expect(items[0].textContent).toContain('D 任务');
    expect(items[1].textContent).toContain('B 任务');
  });
});
