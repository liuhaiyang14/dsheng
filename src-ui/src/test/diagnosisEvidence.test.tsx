// 桌面只读证据辅助接口回归；不调用 Provider，不改变冻结 MetricRowView。
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { TauriClient } from '../api/tauriClient';
import { RUN_2026_10, WS_DEMO_ID } from '../api/fixtureData';

afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

async function openDiagnosis(client: FixtureClient) {
  render(<App runtime={{ mode: 'tauri', client }} />);
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营问题诊断/ }));
  await screen.findByTestId('generate-diagnosis-button');
}

it('不完整期间没有环比同比，桌面辅助接口仍能为当前版本准备诊断', async () => {
  const helper = vi.fn().mockResolvedValue(['cell:current:Sheet1:C9', 'cell:current:Sheet1:C9']);
  const client = Object.assign(new FixtureClient(), { diagnosisEvidence: helper });
  const historyShow = client.historyShow.bind(client);
  vi.spyOn(client, 'historyShow').mockImplementation(async (...args) => {
    const detail = await historyShow(...args);
    detail.metrics.forEach((metric) => { metric.mom = null; metric.yoy = null; });
    return detail;
  });
  const preview = vi.spyOn(client, 'diagnosisGeneratePreview');
  await openDiagnosis(client);
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  await screen.findByTestId('preview-modal');
  expect(helper).toHaveBeenCalledWith(WS_DEMO_ID, RUN_2026_10, 2, 'm-settle');
  expect(preview).toHaveBeenCalledWith(expect.objectContaining({ evidence_ids: ['cell:current:Sheet1:C9'] }));
});

it('辅助接口没有保存证据时明确提示重算，不能用比较旧证据或假锚点发起预览', async () => {
  const client = Object.assign(new FixtureClient(), { diagnosisEvidence: vi.fn().mockResolvedValue([]) });
  const preview = vi.spyOn(client, 'diagnosisGeneratePreview');
  await openDiagnosis(client);
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  expect(await screen.findByTestId('diagnosis-notice')).toHaveTextContent('该版本未保留可用的来源证据，请使用同一数据目录重新运行本地分析后再生成诊断');
  expect(preview).not.toHaveBeenCalled();
  expect(screen.queryByTestId('preview-modal')).toBeNull();
});

it('离开原页面后迟到的证据结果不再创建旧期间预览', async () => {
  let finish!: (ids: string[]) => void;
  const client = Object.assign(new FixtureClient(), { diagnosisEvidence: vi.fn(() => new Promise<string[]>((resolve) => { finish = resolve; })) });
  const preview = vi.spyOn(client, 'diagnosisGeneratePreview');
  await openDiagnosis(client);
  fireEvent.click(screen.getByTestId('generate-diagnosis-button'));
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /数据源/ }));
  await act(async () => finish(['cell:current:Sheet1:C9']));
  expect(preview).not.toHaveBeenCalled();
});

it('Tauri 辅助接口准确传递企业、run、分析版本和可选指标', async () => {
  const invoke = vi.fn().mockResolvedValue({ schema_version: '1', ok: true, data: ['cell:current:Sheet1:C9'] });
  vi.stubGlobal('__TAURI__', { core: { invoke } });
  const client = new TauriClient();
  await expect(client.diagnosisEvidence(WS_DEMO_ID, RUN_2026_10, 3, 'metric-id')).resolves.toEqual(['cell:current:Sheet1:C9']);
  expect(invoke).toHaveBeenLastCalledWith('diagnosis_evidence', { workspaceId: WS_DEMO_ID, runId: RUN_2026_10, revisionNo: 3, metricId: 'metric-id' });
  await client.diagnosisEvidence(WS_DEMO_ID, RUN_2026_10, 3);
  expect(invoke).toHaveBeenLastCalledWith('diagnosis_evidence', { workspaceId: WS_DEMO_ID, runId: RUN_2026_10, revisionNo: 3, metricId: null });
});
