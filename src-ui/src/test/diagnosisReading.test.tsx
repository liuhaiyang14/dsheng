// 本地读侧回归：列表摘要与完整报告分离；不调用或模拟 Provider。
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureDiagnosisReports, WS_DEMO_ID, WS_EMPTY_ID } from '../api/fixtureData';
import type { DiagnosisReport } from '../api';

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

function summary(report: DiagnosisReport) {
  return {
    report_id: report.report_id, workspace_id: report.workspace_id, run_id: report.run_id,
    analysis_revision_no: report.analysis_revision_no, framework: report.framework,
    problem_statement: report.scope.problem_statement, report_revision_no: report.revision_no,
    state: 'active', stop_reason: report.stop_reason, can_continue: report.can_continue,
    active_warning_count: report.warnings.filter((w) => w.status === 'active').length,
    created_at: report.created_at, updated_at: report.updated_at,
  };
}

async function openDiagnosis(client: FixtureClient) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  fireEvent.click(screen.getByRole('button', { name: /经营问题诊断/ }));
  await screen.findByTestId('generate-diagnosis-button');
}

it('真实摘要列表通过 show 加载完整 5WHY，重新进入仍显示已有发现', async () => {
  const client = new FixtureClient();
  vi.spyOn(client, 'diagnosisList').mockResolvedValue(
    fixtureDiagnosisReports.map(summary),
  );
  const show = vi.spyOn(client, 'diagnosisShow');
  await openDiagnosis(client);
  const report = fixtureDiagnosisReports[0];
  const detail = await screen.findByTestId('diagnosis-report-detail');
  expect(detail.textContent).toContain(report.findings[0].answer);
  expect(show).toHaveBeenCalledWith(report.workspace_id, report.report_id, report.revision_no);
  fireEvent.click(screen.getByRole('button', { name: /数据源/ }));
  fireEvent.click(screen.getByRole('button', { name: /经营问题诊断/ }));
  expect((await screen.findByTestId('diagnosis-report-detail')).textContent).toContain(report.findings[0].answer);
});

it('报告详情读取失败显示可重试错误，导航保持可用', async () => {
  const client = new FixtureClient();
  const show = vi.spyOn(client, 'diagnosisShow').mockRejectedValueOnce(new Error('本地报告读取失败'));
  await openDiagnosis(client);
  expect(await screen.findByRole('alert')).toHaveTextContent('本地报告读取失败');
  expect(screen.getByRole('navigation')).toBeVisible();
  expect(screen.queryByTestId('diagnosis-report-detail')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '重新加载报告' }));
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[0].summary);
  expect(show).toHaveBeenCalledTimes(2);
});

it('列表读取失败不会误报没有报告，可重新加载已有报告', async () => {
  const client = new FixtureClient();
  vi.spyOn(client, 'diagnosisList').mockRejectedValueOnce(new Error('记录不可读'));
  await openDiagnosis(client);
  expect(await screen.findByRole('alert')).toHaveTextContent('记录不可读');
  expect(screen.queryByText('本期还没有诊断报告。选择框架并确认问题表述后，点击「生成诊断报告」。')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: '重新加载记录' }));
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[0].summary);
});

it('切换报告后迟到的旧正文不能覆盖新选择', async () => {
  const client = new FixtureClient();
  let resolveFirst!: (report: DiagnosisReport) => void;
  vi.spyOn(client, 'diagnosisShow').mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }));
  await openDiagnosis(client);
  await waitFor(() => expect(resolveFirst).toBeDefined());
  fireEvent.click(screen.getAllByTestId(/diagnosis-item-/)[1]);
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[1].summary);
  await act(async () => { resolveFirst(fixtureDiagnosisReports[0]); });
  expect(screen.getByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[1].summary);
  expect(screen.getByTestId('diagnosis-report-detail')).not.toHaveTextContent(fixtureDiagnosisReports[0].summary);
});

it('企业切换后旧请求失效，切回会重新读取而不采用迟到结果', async () => {
  const client = new FixtureClient();
  let resolveFirst!: (report: DiagnosisReport) => void;
  const show = vi.spyOn(client, 'diagnosisShow').mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }));
  await openDiagnosis(client);
  await waitFor(() => expect(resolveFirst).toBeDefined());
  fireEvent.change(screen.getByTestId('workspace-select'), { target: { value: WS_EMPTY_ID } });
  await screen.findByText('尚无可用分析期间');
  const stale = structuredClone(fixtureDiagnosisReports[0]);
  stale.summary = '不应显示的旧请求';
  await act(async () => { resolveFirst(stale); });
  expect(screen.queryByTestId('diagnosis-report-detail')).toBeNull();
  fireEvent.change(screen.getByTestId('workspace-select'), { target: { value: WS_DEMO_ID } });
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[0].summary);
  expect(screen.queryByText(stale.summary)).toBeNull();
  expect(show).toHaveBeenCalledTimes(2);
});

it('页面渲染异常被隔离并记入日志，重试可恢复且无需重启应用', async () => {
  const client = new FixtureClient();
  const broken = { ...fixtureDiagnosisReports[0], period: undefined } as unknown as DiagnosisReport;
  vi.spyOn(client, 'diagnosisShow').mockResolvedValueOnce(broken);
  const errorLog = vi.spyOn(console, 'error').mockImplementation(() => undefined);
  const nativeLog = vi.fn().mockResolvedValue(undefined);
  vi.stubGlobal('__TAURI__', { core: { invoke: nativeLog } });
  await openDiagnosis(client);
  expect(await screen.findByRole('alert')).toHaveTextContent('页面暂时无法显示');
  expect(screen.getByRole('navigation')).toBeVisible();
  expect(errorLog).toHaveBeenCalled();
  expect(nativeLog).toHaveBeenCalledWith('js_log', expect.objectContaining({ message: expect.stringContaining('page-render:') }));
  fireEvent.click(screen.getByRole('button', { name: '重新加载页面' }));
  expect(await screen.findByTestId('diagnosis-report-detail')).toHaveTextContent(fixtureDiagnosisReports[0].summary);
  fireEvent.click(screen.getByRole('button', { name: /数据源/ }));
  expect(screen.getByTestId('crumb-screen')).toHaveTextContent('数据源');
});
