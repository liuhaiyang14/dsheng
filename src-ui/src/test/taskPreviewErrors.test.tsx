// 参数弹窗的请求/错误状态测试，使用合成报告和 FixtureClient，不调用 Provider。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureAnalysisRun, fixtureWorkspaces } from '../api/fixtureData';
import type { ConfirmationPreview, DiagnosisFramework } from '../api';

afterEach(cleanup);

const evidenceError = '所选诊断报告引用的证据不在本次可发送范围，请核对来源或取消报告引用';

async function setup() {
  const client = new FixtureClient();
  const request = { workspace_id: fixtureWorkspaces[0].workspace_id, run_id: fixtureAnalysisRun.run_id };
  const [base] = await client.diagnosisList(request.workspace_id, request.run_id);
  vi.spyOn(client, 'diagnosisList').mockResolvedValue((['5why', '5w', '5w2h'] as DiagnosisFramework[])
    .map((framework) => ({ ...base, framework, report_id: `report-${framework}` })));
  const preview = await client.taskGeneratePreview(request);
  const previewSpy = vi.spyOn(client, 'taskGeneratePreview');
  const confirmSpy = vi.spyOn(client, 'taskGenerateConfirm');
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getAllByText('任务单')[0].closest('button')!);
  fireEvent.click(await screen.findByTestId('generate-task-button'));
  const dialog = await screen.findByRole('dialog', { name: '生成任务单参数' });
  fireEvent.click(within(dialog).getByRole('checkbox'));
  return { client, request, preview, previewSpy, confirmSpy, dialog };
}

it.each(['5why', '5w'])('%s 引用预览失败在弹窗内提示，切换报告清除旧错误并可重试', async (framework) => {
  const { request, preview, previewSpy, confirmSpy, dialog } = await setup();
  previewSpy.mockRejectedValueOnce(new Error(evidenceError)).mockResolvedValue(preview);
  fireEvent.change(within(dialog).getByLabelText('引用报告'), { target: { value: `report-${framework}` } });
  fireEvent.click(within(dialog).getByTestId('request-task-preview'));

  expect(await within(dialog).findByRole('alert')).toHaveTextContent('所选报告的部分证据不在本次可发送范围');
  expect(within(dialog).getByRole('alert')).toHaveTextContent('请取消勾选“引用本期已保存的诊断报告”后继续，或选择其他可用报告');
  expect(screen.queryByTestId('task-notice')).toBeNull();
  expect(screen.queryByTestId('preview-modal')).toBeNull();
  expect(within(dialog).getByTestId('request-task-preview')).not.toBeDisabled();
  expect(previewSpy).toHaveBeenCalledTimes(1);
  expect(previewSpy).toHaveBeenLastCalledWith({ ...request, diagnosis_report_id: `report-${framework}` });
  expect(confirmSpy).not.toHaveBeenCalled();

  fireEvent.change(within(dialog).getByLabelText('引用报告'), { target: { value: 'report-5w2h' } });
  expect(within(dialog).queryByRole('alert')).toBeNull();
  expect(previewSpy).toHaveBeenCalledTimes(1);
  fireEvent.click(within(dialog).getByTestId('request-task-preview'));
  await screen.findByTestId('preview-modal');
  expect(screen.queryByTestId('generate-param-modal')).toBeNull();
  expect(previewSpy).toHaveBeenLastCalledWith({ ...request, diagnosis_report_id: 'report-5w2h' });
  expect(confirmSpy).not.toHaveBeenCalled();
});

it('取消报告引用清除错误，继续时不再携带报告；重开参数弹窗不留旧错误', async () => {
  const { request, preview, previewSpy, confirmSpy, dialog } = await setup();
  previewSpy.mockRejectedValueOnce(new Error(evidenceError)).mockResolvedValue(preview);
  fireEvent.click(within(dialog).getByTestId('request-task-preview'));
  await within(dialog).findByRole('alert');
  fireEvent.click(within(dialog).getByRole('button', { name: '取消' }));
  fireEvent.click(screen.getByTestId('generate-task-button'));
  const reopened = await screen.findByRole('dialog', { name: '生成任务单参数' });
  expect(within(reopened).queryByRole('alert')).toBeNull();

  previewSpy.mockRejectedValueOnce(new Error(evidenceError));
  fireEvent.click(within(reopened).getByTestId('request-task-preview'));
  await within(reopened).findByRole('alert');
  fireEvent.click(within(reopened).getByRole('checkbox'));
  expect(within(reopened).queryByRole('alert')).toBeNull();
  fireEvent.click(within(reopened).getByTestId('request-task-preview'));
  await screen.findByTestId('preview-modal');
  expect(previewSpy).toHaveBeenLastCalledWith({ ...request, diagnosis_report_id: undefined });
  expect(confirmSpy).not.toHaveBeenCalled();
});

it('准备预览时显示待处理状态并锁定请求参数，不重复提交或自动发送', async () => {
  const { preview, previewSpy, confirmSpy, dialog } = await setup();
  let resolve!: (value: ConfirmationPreview) => void;
  previewSpy.mockImplementation(() => new Promise((done) => { resolve = done; }));
  fireEvent.click(within(dialog).getByTestId('request-task-preview'));
  expect(dialog).toHaveAttribute('aria-busy', 'true');
  const next = within(dialog).getByRole('button', { name: '正在准备预览…' });
  expect(next).toBeDisabled();
  expect(within(dialog).getByLabelText('引用报告')).toBeDisabled();
  expect(within(dialog).getByRole('checkbox')).toBeDisabled();
  expect(within(dialog).getByRole('button', { name: '取消' })).toBeDisabled();
  expect(screen.getByRole('combobox', { name: '切换企业档案' })).toBeDisabled();
  fireEvent.click(next);
  expect(previewSpy).toHaveBeenCalledTimes(1);
  resolve(preview);
  await waitFor(() => expect(screen.queryByTestId('generate-param-modal')).toBeNull());
  expect(screen.getByTestId('preview-modal')).toBeInTheDocument();
  expect(confirmSpy).not.toHaveBeenCalled();
});
