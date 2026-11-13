import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

async function openTables(client = new FixtureClient()) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  const nav = screen.getAllByText('两表展示/下载')[0].closest('button');
  expect(nav).toBeTruthy();
  fireEvent.click(nav!);
  await screen.findByTestId('tables-screen');
  return client;
}

describe('两表展示/下载页', () => {
  it('按页签呈现核算表与推移表，并显示待补而不填 0', async () => {
    await openTables();
    expect(screen.getByTestId('accounting-table-panel')).toBeVisible();
    expect(screen.getByText('附加值与经营收益按不同名称展示；人工费、目标、工时或期初缺失时保留待补，不按 0 计算。')).toBeTruthy();
    fireEvent.click(screen.getByRole('tab', { name: '推移表' }));
    expect(screen.getByTestId('trend-table-panel')).toBeVisible();
    expect(screen.getByRole('tab', { name: '推移表' })).toHaveAttribute('aria-controls', 'trend-table-panel');
  });

  it('导出核算表、两表包和固定 two_tables_v1 输出模板都走客户端门面', async () => {
    vi.stubGlobal('prompt', vi.fn().mockReturnValue('/tmp/two-tables.xlsx'));
    const client = await openTables();
    const exportSpy = vi.spyOn(client, 'exportTables');
    const templateSpy = vi.spyOn(client, 'templateDownload');

    fireEvent.click(screen.getByRole('button', { name: '下载核算表' }));
    await waitFor(() => expect(exportSpy).toHaveBeenCalledWith(expect.objectContaining({ kind: 'accounting' })));

    fireEvent.click(screen.getByRole('button', { name: '下载两表包' }));
    await waitFor(() => expect(exportSpy).toHaveBeenCalledWith(expect.objectContaining({ kind: 'bundle' })));

    fireEvent.click(screen.getByRole('button', { name: '下载输出模板' }));
    await waitFor(() => expect(templateSpy).toHaveBeenCalledWith(
      '/tmp/two-tables.xlsx', 'output', undefined, 'two_tables_v1',
    ));
  });

  it('两表未就绪时显式显示任务门禁并禁用生成按钮', async () => {
    const client = new FixtureClient();
    const originalHistoryShow = client.historyShow.bind(client);
    vi.spyOn(client, 'historyShow').mockImplementation(async (workspaceId, runId, revision) => ({
      ...await originalHistoryShow(workspaceId, runId, revision),
      two_tables_ready: false,
    }));
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    fireEvent.click(screen.getAllByText('任务单')[0].closest('button')!);
    await screen.findByTestId('task-two-tables-gate');
    expect(screen.getByTestId('generate-task-button')).toBeDisabled();
    expect(screen.getByText(/尚未同时生成核算表和推移表/)).toBeTruthy();
  });
});
