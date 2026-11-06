// 合成报告验证版本选择、重新生成及任务引用，不调用真实 Provider。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureDiagnosisReports } from '../api/fixtureData';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

async function fixtureWithVersions() {
  const client = new FixtureClient();
  const original = fixtureDiagnosisReports[0];
  const request = {
    workspace_id: original.workspace_id, run_id: original.run_id, framework: original.framework,
    problem: original.scope.problem_statement, evidence_ids: ['ev-1'],
  };
  const preview = await client.diagnosisGeneratePreview(request);
  const receipt = await client.diagnosisGenerateConfirm(preview.confirmation_id);
  const latestId = receipt.report_id!;
  const show = client.diagnosisShow.bind(client);
  vi.spyOn(client, 'diagnosisShow').mockImplementation(async (...args) => {
    const report = await show(...args);
    report.summary = report.report_id === original.report_id ? '第一版的原始结论' : '重新分析后的最新结论';
    return report;
  });
  return { client, original, latestId, request };
}

async function openDiagnosis(client: FixtureClient) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getByRole('button', { name: /经营问题诊断/ }));
  await screen.findByTestId('diagnosis-report-detail');
}

it('同报告列表只显示最新版，标题版本下拉读取原始历史正文且只读', async () => {
  const { client, original, latestId } = await fixtureWithVersions();
  const generate = vi.spyOn(client, 'diagnosisGeneratePreview');
  const continuePreview = vi.spyOn(client, 'diagnosisContinuePreview');
  const exported = vi.spyOn(client, 'diagnosisExport');
  await openDiagnosis(client);
  const list = screen.getByTestId('diagnosis-report-list');
  expect(within(list).getAllByRole('button')).toHaveLength(2);
  expect(list).toHaveTextContent('第 2 版');
  expect(screen.getByTestId('diagnosis-report-detail')).toHaveTextContent('重新分析后的最新结论');
  const versions = screen.getByLabelText('报告版本');
  expect(versions).toHaveValue(latestId);
  expect(within(versions).getAllByRole('option')).toHaveLength(2);

  fireEvent.change(versions, { target: { value: original.report_id } });
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent('第一版的原始结论');
  expect(screen.getByText('历史版本 · 仅供查看')).toBeInTheDocument();
  expect(screen.getByTestId('continue-diagnosis-button')).toBeDisabled();
  expect(screen.getByRole('button', { name: '停止并保留报告' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '下载 MD' }));
  await waitFor(() => expect(exported).toHaveBeenCalledWith(original.workspace_id, original.report_id, 'md', expect.any(String)));
  expect(generate).not.toHaveBeenCalled();
  expect(continuePreview).not.toHaveBeenCalled();

  fireEvent.change(screen.getByLabelText('报告版本'), { target: { value: latestId } });
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent('重新分析后的最新结论');
  expect(screen.queryByText('历史版本 · 仅供查看')).toBeNull();
});

it('重新生成的发送预览明确替换展示，失败保留旧版，成功后仅展示新版本', async () => {
  const { client, latestId, request } = await fixtureWithVersions();
  const confirm = vi.spyOn(client, 'diagnosisGenerateConfirm');
  await openDiagnosis(client);
  fireEvent.change(screen.getByLabelText('问题表述'), { target: { value: request.problem } });
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  const modal = await screen.findByTestId('preview-modal');
  expect(within(modal).getByTestId('diagnosis-replacement-notice')).toHaveTextContent('第 2 版');
  expect(modal).toHaveTextContent('第 3 版替换当前展示');
  expect(within(modal).getByTestId('preview-confirm')).toHaveTextContent('确认重新生成');
  expect(confirm).not.toHaveBeenCalled();
  confirm.mockRejectedValueOnce(new Error('合成保存失败'));
  fireEvent.click(within(modal).getByTestId('preview-confirm'));
  expect(await screen.findByRole('alert')).toHaveTextContent('合成保存失败');
  fireEvent.click(screen.getByTestId('generation-close'));
  expect(screen.getByLabelText('报告版本')).toHaveValue(latestId);
  expect(within(screen.getByTestId('diagnosis-report-list')).getAllByRole('button')).toHaveLength(2);

  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  fireEvent.click(await screen.findByTestId('preview-confirm'));
  await waitFor(() => expect(screen.getByLabelText('报告版本')).not.toHaveValue(latestId));
  expect(within(screen.getByTestId('diagnosis-report-list')).getAllByRole('button')).toHaveLength(2);
  expect(screen.getByLabelText('报告版本')).toHaveTextContent('第 3 版（最新）');
  expect(screen.getByLabelText('报告版本')).toHaveTextContent('第 2 版（历史）');
});

it('任务引用只能选择本期可引用的最新报告，历史版本和失效分析报告不出现', async () => {
  const { client, original, latestId, request } = await fixtureWithVersions();
  const baseList = await client.diagnosisList(request.workspace_id, request.run_id);
  const latest = baseList.find((report) => report.report_id === latestId)!;
  vi.spyOn(client, 'diagnosisList').mockResolvedValue([
    latest,
    { ...latest, report_id: original.report_id, is_latest: false },
    { ...latest, report_id: 'stale-analysis', referenceable: false },
  ]);
  const taskPreview = vi.spyOn(client, 'taskGeneratePreview');
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(screen.getByRole('button', { name: /任务单/ }));
  fireEvent.click(await screen.findByTestId('generate-task-button'));
  const dialog = await screen.findByRole('dialog', { name: '生成任务单参数' });
  fireEvent.click(within(dialog).getByRole('checkbox'));
  const select = within(dialog).getByLabelText('引用报告');
  expect(within(select).getAllByRole('option')).toHaveLength(1);
  expect(select).toHaveValue(latestId);
  expect(select).toHaveTextContent('第 2 版（最新）');
  fireEvent.click(within(dialog).getByTestId('request-task-preview'));
  await screen.findByTestId('preview-modal');
  expect(taskPreview).toHaveBeenLastCalledWith({ workspace_id: request.workspace_id, run_id: request.run_id, diagnosis_report_id: latestId });
});
