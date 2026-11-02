// 测试层次：UI 纯组件/状态测试（FixtureClient 内存模拟，无网络、无 Tauri）。
// S2：API 合同层行为——envelope 解包、workspace 隔离、revision 冲突、
// 显式确认门禁、decimal 字符串不经 Number。
import { beforeEach, describe, expect, it } from 'vitest';
import { FixtureClient } from '../api/fixtureClient';
import {
  DEMO_NOW,
  RUN_2026_10,
  WS_DEMO_ID,
  WS_EMPTY_ID,
  fixtureTasks,
} from '../api/fixtureData';
import { groupDecimal } from '../api/format';
import type { TaskEditInput } from '../api/types';

let client: FixtureClient;

beforeEach(() => {
  client = new FixtureClient();
});

function editInput(
  taskId: string,
  revision: number,
  operation: TaskEditInput['operation'],
): TaskEditInput {
  return {
    schema_version: '1',
    workspace_id: WS_DEMO_ID,
    task_id: taskId,
    expected_task_revision: revision,
    mutation_id: '00000000-0000-4000-8000-0000000000ff',
    operation,
  };
}

describe('S2 FixtureClient：合同行为模拟', () => {
  it('workspace 隔离：空档案不返回示例企业任务/报告/历史', async () => {
    const demoTasks = await client.taskList(WS_DEMO_ID);
    const emptyTasks = await client.taskList(WS_EMPTY_ID);
    expect(demoTasks.length).toBeGreaterThan(0);
    expect(emptyTasks).toEqual([]);
    const emptyHistory = await client.historyList(WS_EMPTY_ID);
    expect(emptyHistory).toEqual([]);
    const emptyReports = await client.diagnosisList(WS_EMPTY_ID, RUN_2026_10);
    expect(emptyReports).toEqual([]);
  });

  it('跨 workspace 读取任务返回 WORKSPACE_MISMATCH', async () => {
    const demoTask = fixtureTasks[0];
    await expect(
      client.taskShow(WS_EMPTY_ID, demoTask.task_id),
    ).rejects.toMatchObject({
      code: 'WORKSPACE_MISMATCH',
    });
  });

  it('expected_task_revision 不一致返回 REVISION_CONFLICT', async () => {
    const task = await client.taskShow(WS_DEMO_ID, fixtureTasks[0].task_id);
    await expect(
      client.taskUpdate(
        editInput(task.task_id, task.task_revision_no + 5, {
          kind: 'content',
          patch: { title: '改名' },
        }),
      ),
    ).rejects.toMatchObject({ code: 'REVISION_CONFLICT' });
  });

  it('数值 patch 无 source_note 被拒绝（DATA_VALIDATION_FAILED）', async () => {
    const task = await client.taskShow(WS_DEMO_ID, fixtureTasks[0].task_id);
    await expect(
      client.taskUpdate(
        editInput(task.task_id, task.task_revision_no, {
          kind: 'content',
          patch: { target_value: '215000' },
        }),
      ),
    ).rejects.toMatchObject({ code: 'DATA_VALIDATION_FAILED' });
  });

  it('诊断生成必须显式 preview→confirm，confirm 一次后不可重复消费', async () => {
    const preview = await client.diagnosisGeneratePreview({
      workspace_id: WS_DEMO_ID,
      run_id: RUN_2026_10,
      framework: '5w',
      problem: '结算收益差异如何界定？',
    });
    expect(preview.operation).toBe('diagnosis_generate');
    expect(preview.request_count_maximum).toBe(1);
    const before = await client.diagnosisList(WS_DEMO_ID, RUN_2026_10);
    const report = await client.diagnosisGenerateConfirm(preview.confirmation_id);
    expect(report.framework).toBe('5w');
    const after = await client.diagnosisList(WS_DEMO_ID, RUN_2026_10);
    expect(after.length).toBe(before.length + 1);
    await expect(
      client.diagnosisGenerateConfirm(preview.confirmation_id),
    ).rejects.toMatchObject({ code: 'CONFIRMATION_STALE' });
  });

  it('空档案生成诊断/任务被拒绝且不产生任何候选或报告', async () => {
    await expect(
      client.diagnosisGeneratePreview({
        workspace_id: WS_EMPTY_ID,
        run_id: RUN_2026_10,
        framework: '5why',
        problem: 'x',
      }),
    ).rejects.toMatchObject({ code: 'DATA_VALIDATION_FAILED' });
    await expect(
      client.taskGeneratePreview({ workspace_id: WS_EMPTY_ID, run_id: RUN_2026_10 }),
    ).rejects.toMatchObject({ code: 'DATA_VALIDATION_FAILED' });
  });

  it('任务生成引用不属于本 run 的诊断报告返回 STALE_DIAGNOSIS_REFERENCE', async () => {
    await expect(
      client.taskGeneratePreview({
        workspace_id: WS_DEMO_ID,
        run_id: RUN_2026_10,
        diagnosis_report_id: '00000000-0000-4000-8000-000000000bad',
      }),
    ).rejects.toMatchObject({ code: 'STALE_DIAGNOSIS_REFERENCE' });
  });

  it('auto 进度规则模拟：1/3 → 33.33；block 优先；manual 需原因', async () => {
    const task = fixtureTasks.find((t) => t.progress === '33.33')!;
    const view = await client.taskShow(WS_DEMO_ID, task.task_id);
    expect(view.progress).toBe('33.33');

    const blocked = fixtureTasks.find((t) => t.status === 'blocked')!;
    const blockedView = await client.taskShow(WS_DEMO_ID, blocked.task_id);
    expect(blockedView.status).toBe('blocked');

    await expect(
      client.taskUpdate(
        editInput(view.task_id, view.task_revision_no, {
          kind: 'progress',
          progress_mode: 'manual',
          progress: '50',
          status: 'in_progress',
          adjustment_reason: '   ',
          blocked_reason: null,
        }),
      ),
    ).rejects.toMatchObject({ code: 'DATA_VALIDATION_FAILED' });
  });

  it('步骤全部勾选后 auto 进度为 100 且状态 completed；重开写 reopened', async () => {
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    const view = await client.taskShow(WS_DEMO_ID, source.task_id);
    const updated = await client.taskUpdate(
      editInput(view.task_id, view.task_revision_no, {
        kind: 'content',
        step_operations: view.steps.map((s) => ({
          op: 'update' as const,
          step_id: s.step_id,
          patch: { status: 'completed' as const },
        })),
      }),
    );
    expect(updated.progress).toBe('100');
    expect(updated.status).toBe('completed');
    expect(updated.completed_at).not.toBeNull();

    // 提交与计算不一致的 auto 状态（3 步全完成却请求 in_progress）被拒绝
    await expect(
      client.taskUpdate(
        editInput(updated.task_id, updated.task_revision_no, {
          kind: 'progress',
          progress_mode: 'auto',
          progress: null,
          status: 'in_progress',
          adjustment_reason: null,
          blocked_reason: null,
        }),
      ),
    ).rejects.toMatchObject({ code: 'STATE_PROGRESS_MISMATCH' });

    // 取消勾选一步：completed → in_progress 自动重开并写 reopened 事件
    const afterUncheck = await client.taskUpdate(
      editInput(updated.task_id, updated.task_revision_no, {
        kind: 'content',
        step_operations: [
          { op: 'update', step_id: updated.steps[2].step_id, patch: { status: 'not_started' } },
        ],
      }),
    );
    expect(afterUncheck.progress).toBe('66.67');
    expect(afterUncheck.status).toBe('in_progress');
    expect(afterUncheck.completed_at).toBeNull();
    expect(afterUncheck.events.some((e) => e.kind === 'reopened')).toBe(true);
  });

  it('删除最后一步返回 AT_LEAST_ONE_STEP', async () => {
    const source = fixtureTasks.find((t) => t.progress === '33.33')!;
    const view = await client.taskShow(WS_DEMO_ID, source.task_id);
    let current = view;
    for (const step of [...view.steps]) {
      if (current.steps.length > 1) {
        current = await client.taskUpdate(
          editInput(current.task_id, current.task_revision_no, {
            kind: 'content',
            step_operations: [{ op: 'delete', step_id: step.step_id }],
          }),
        );
      }
    }
    await expect(
      client.taskUpdate(
        editInput(current.task_id, current.task_revision_no, {
          kind: 'content',
          step_operations: [
            { op: 'delete', step_id: current.steps[0].step_id },
          ],
        }),
      ),
    ).rejects.toMatchObject({ code: 'AT_LEAST_ONE_STEP' });
  });

  it('provider 保存不发请求且 Key 不出现在返回值；无默认 Provider 时生成预览被拒', async () => {
    const providers = await client.providerList(WS_DEMO_ID);
    expect(providers).toHaveLength(1);
    expect(JSON.stringify(providers)).not.toContain('demo-local-key');
    expect(providers[0].key_present).toBe(true);

    // 取消默认 Provider 后（无 fallback），生成预览返回 PROVIDER_REQUIRED
    const ws = (await client.workspaceList()).find(
      (w) => w.workspace_id === WS_DEMO_ID,
    )!;
    await client.providerUse({
      schema_version: '1',
      operation: 'provider_use',
      workspace_id: WS_DEMO_ID,
      provider_id: null,
      expected_revision: ws.config_revision,
    });
    await expect(
      client.diagnosisGeneratePreview({
        workspace_id: WS_DEMO_ID,
        run_id: RUN_2026_10,
        framework: '5why',
        problem: 'x',
      }),
    ).rejects.toMatchObject({ code: 'PROVIDER_REQUIRED' });
  });

  it('period 保存创建新版本并闭合旧版本区间', async () => {
    const before = await client.periodList(WS_DEMO_ID);
    const ws = (await client.workspaceList()).find(
      (w) => w.workspace_id === WS_DEMO_ID,
    )!;
    const created = await client.periodSet({
      schema_version: '1',
      operation: 'period_policy_set',
      workspace_id: WS_DEMO_ID,
      expected_revision: ws.config_revision,
      policy: {
        cadence: 'week',
        week_start_day: 1,
        fiscal_year_start_month: 1,
        timezone_id: 'Asia/Shanghai',
        effective_from: '2026-11-01',
      },
    });
    expect(created.version).toBe(before.length + 1);
    const after = await client.periodList(WS_DEMO_ID);
    expect(after.find((p) => p.period_policy_id === before[1].period_policy_id)?.effective_to).toBe('2026-11-01');
  });
});

describe('S2 展示格式化：decimal 字符串不经 Number', () => {
  it('groupDecimal 纯字符串分组，保留高精度数字', () => {
    expect(groupDecimal('184600')).toBe('184,600');
    expect(groupDecimal('-25400.5')).toBe('-25,400.5');
    expect(groupDecimal('1234567890123456789012345678.123456')).toBe(
      '1,234,567,890,123,456,789,012,345,678.123456',
    );
    // 28 位有效数字场景：若经 Number 会精度丢失，纯字符串则完整保留
    expect(groupDecimal('9999999999999999999999999999')).toBe(
      '9,999,999,999,999,999,999,999,999,999',
    );
  });
});

void DEMO_NOW;
