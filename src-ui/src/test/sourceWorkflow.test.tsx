import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { WS_DEMO_ID } from '../api/fixtureData';
import { ImportDropzone } from '../components/ImportDropzone';
const directoryKey = `dsheng.source-directory.${WS_DEMO_ID}`;
beforeEach(() => window.localStorage.removeItem(directoryKey));
afterEach(() => { cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals(); window.localStorage.removeItem(directoryKey); });
async function openSource(client: FixtureClient) {
  const view = render(<App runtime={{ mode: 'fixture', client }} />);
  await waitFor(() => expect(screen.getByTestId('workspace-select')).not.toBeDisabled());
  return view;
}
async function scan(path: string) {
  fireEvent.change(screen.getByTestId('manual-path-input'), { target: { value: path } });
  fireEvent.click(screen.getByTestId('manual-path-scan'));
  await waitFor(() => expect(screen.getByTestId('manual-path-scan')).not.toBeDisabled());
}
it('单一导入区从未扫描到成功，跨页保留扫描结果，重新打开后保留有效目录', async () => {
  const client = new FixtureClient();
  const view = await openSource(client);
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('未扫描');
  expect(screen.queryByText('已识别文件')).toBeNull();
  expect(screen.queryByText('本地目录')).toBeNull();
  expect(screen.getAllByTestId('import-dropzone')).toHaveLength(1);
  expect(screen.getByTestId('self-test-button')).not.toBeVisible();
  await scan('/tmp/real-business-directory');
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('已扫描');
  expect(screen.getByText('已识别文件')).toBeVisible();
  expect(screen.getByTestId('source-directory')).toHaveTextContent('/tmp/real-business-directory');
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /Provider 设置/ }));
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /数据源/ }));
  expect(screen.getByTestId('source-directory')).toHaveTextContent('/tmp/real-business-directory');
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('已扫描');
  view.unmount();
  await openSource(client);
  expect(screen.getByTestId('source-directory')).toHaveTextContent('/tmp/real-business-directory');
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('未扫描');
  expect(screen.queryByRole('button', { name: '运行本地分析' })).toBeNull();
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营分析/ }));
  fireEvent.click(screen.getByText('重新分析数据'));
  expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled();
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled());
  // generic_table candidates require an explicit mapping confirmation before
  // local analysis can create a revision.
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /数据源/ }));
  fireEvent.click(screen.getByTestId('confirm-source-mapping-button'));
  await waitFor(() => expect(screen.queryByTestId('confirm-source-mapping-button')).toBeNull());
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营分析/ }));
  await waitFor(() => expect(screen.getByRole('button', { name: '运行本地分析' })).not.toBeDisabled());
});
it('失败扫描显示真实失败状态，保留上次有效目录而不保存失败路径', async () => {
  const client = new FixtureClient();
  const reader = vi.spyOn(client, 'scan');
  await openSource(client);
  await scan('/tmp/valid-directory');
  reader.mockRejectedValueOnce(new Error('目录不存在'));
  await scan('/tmp/missing-directory');
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('本次扫描失败');
  expect(screen.getByTestId('scan-error')).toHaveTextContent('目录不存在');
  expect(screen.getByTestId('source-directory')).toHaveTextContent('/tmp/valid-directory');
  expect(window.localStorage.getItem(directoryKey)).toBe('/tmp/valid-directory');
});
it('原生拖放目录直接扫描该目录，拖放文件扫描其所在目录', async () => {
  let onDrop!: (event: { type: string; paths: string[] }) => void;
  vi.stubGlobal('__TAURI__', { window: { getCurrentWindow: () => ({ onDragDropEvent: (handler: typeof onDrop) => { onDrop = handler; return Promise.resolve(() => {}); } }) } });
  const onPath = vi.fn();
  render(<ImportDropzone busy={false} onPath={onPath} />);
  await act(async () => { onDrop({ type: 'drop', paths: ['/tmp/business'] }); });
  expect(onPath).toHaveBeenLastCalledWith('/tmp/business', undefined);
  await act(async () => { onDrop({ type: 'drop', paths: ['C:\\Business\\books.xlsx'] }); });
  expect(onPath).toHaveBeenLastCalledWith('C:\\Business', expect.any(String));
});
