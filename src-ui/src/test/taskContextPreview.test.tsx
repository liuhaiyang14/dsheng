// 合成预览范围展示，无 Provider 调用。
import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { FixtureClient } from '../api/fixtureClient';
import { RUN_2026_10, WS_DEMO_ID } from '../api/fixtureData';
import { PreviewModal } from '../components/PreviewModal';
afterEach(cleanup);

it('发送前列出历史任务范围与所选诊断信息', async () => {
  const preview = await new FixtureClient().taskGeneratePreview({ workspace_id: WS_DEMO_ID, run_id: RUN_2026_10 });
  preview.task_context_summary = {
    prior_task_titles: ['核对交付排程', '回款跟进'], diagnosis_summary: '交付延迟待核实',
    diagnosis_framework: '5w', diagnosis_fact_count: 2, diagnosis_hypothesis_count: 1,
  };
  render(<PreviewModal preview={preview} title="生成任务" confirmLabel="确认发送" onConfirm={vi.fn()} onCancel={vi.fn()} />);
  expect(screen.getByTestId('preview-task-history')).toHaveTextContent('2 项正式任务');
  expect(screen.getByTestId('preview-task-history')).toHaveTextContent('回款跟进');
  expect(screen.getByTestId('preview-task-diagnosis')).toHaveTextContent('五W问题界定 · 交付延迟待核实');
  expect(screen.getByTestId('preview-task-diagnosis')).toHaveTextContent('事实及用户陈述 2 项');
  expect(screen.getByTestId('preview-task-diagnosis')).toHaveTextContent('假设及待确认内容 1 项');
});

it('首期没有历史记录或未选诊断时不冒称引用', async () => {
  const preview = await new FixtureClient().taskGeneratePreview({ workspace_id: WS_DEMO_ID, run_id: RUN_2026_10 });
  preview.task_context_summary = {
    prior_task_titles: [], diagnosis_summary: null, diagnosis_framework: null,
    diagnosis_fact_count: 0, diagnosis_hypothesis_count: 0,
  };
  render(<PreviewModal preview={preview} title="生成任务" confirmLabel="确认发送" onConfirm={vi.fn()} onCancel={vi.fn()} />);
  expect(screen.getByTestId('preview-task-history')).toHaveTextContent('没有相关历史任务');
  expect(screen.getByTestId('preview-task-diagnosis')).toHaveTextContent('未引用诊断报告');
});
