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
it('示例数据默认使用中文模板名称，不把内部 profile 标识展示给用户', async () => {
  const client = new FixtureClient();
  await openSource(client);
  expect(screen.getByLabelText('原始数据模板')).toHaveValue('xsd_trade_v1');
  expect(screen.getByText('贸易/制造业单据模板（当前示例）')).toBeVisible();
  await scan('/tmp/example-source');
  expect(screen.getByText(/模板：贸易\/制造业单据模板（当前示例）/)).toBeVisible();
  expect(screen.queryByText(/generic/)).toBeNull();
  expect(screen.getByTestId('open-field-review-button')).toBeVisible();
  expect(screen.queryByTestId('confirm-source-mapping-button')).toBeNull();
});

it('待确认文件提供逐项确认入口，保存后调用映射确认并重新扫描', async () => {
  const client = new FixtureClient();
  const mappingSpy = vi.spyOn(client, 'sourceMappingSet');
  const scanSpy = vi.spyOn(client, 'scan');
  await openSource(client);
  await scan('/tmp/example-source');

  fireEvent.click(screen.getByTestId('open-field-review-button'));
  expect(screen.getByTestId('field-review-panel')).toBeVisible();
  const fileLabel = '示例费用人工_202610.xlsx';
  const fieldSelect = screen.getByLabelText(`确认字段：${fileLabel}`);
  expect(fieldSelect).toHaveValue('operating_expense');
  fireEvent.change(fieldSelect, { target: { value: 'labor_cost' } });
  fireEvent.change(screen.getByLabelText(`确认列：${fileLabel}`), { target: { value: '金额' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));

  await waitFor(() => expect(mappingSpy).toHaveBeenCalledTimes(1));
  const mappings = mappingSpy.mock.calls[0][3] as Array<Record<string, string>>;
  expect(mappings).toEqual(expect.arrayContaining([
    expect.objectContaining({
      target_role: 'labor_cost',
      source_label: fileLabel,
      column: '金额',
    }),
  ]));
  await waitFor(() => expect(scanSpy).toHaveBeenCalledTimes(2));
  expect(screen.queryByTestId('field-review-panel')).toBeNull();
  expect(screen.getByTestId('scan-note')).toHaveTextContent('字段确认已保存并重新扫描');
});
it('保留自定义 profile JSON 路径并按 profile 模式扫描', async () => {
  const client = new FixtureClient();
  const scanSpy = vi.spyOn(client, 'scan');
  await openSource(client);
  fireEvent.change(screen.getByLabelText('原始数据模板'), { target: { value: 'custom' } });
  fireEvent.change(screen.getByLabelText('自定义模板路径'), { target: { value: '/tmp/xsd-profile.json' } });
  await scan('/tmp/custom-source');
  expect(scanSpy).toHaveBeenCalledWith(WS_DEMO_ID, '/tmp/custom-source', '/tmp/xsd-profile.json', false);
  expect(screen.getByText(/模板：当前行业模板/)).toBeVisible();
});
it('单一导入区从未扫描到成功，跨页保留扫描结果，重新打开后保留有效目录', async () => {
  const client = new FixtureClient();
  const view = await openSource(client);
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('未扫描');
  expect(screen.queryByText('已识别文件')).toBeNull();
  expect(screen.queryByText('本地目录')).toBeNull();
  expect(screen.getAllByTestId('import-dropzone')).toHaveLength(1);
  expect(screen.getByTestId('self-test-button')).not.toBeVisible();
  // 该用例专门验证未指定模板时仍需显式确认字段。
  fireEvent.change(screen.getByLabelText('原始数据模板'), { target: { value: 'generic' } });
  await scan('/tmp/real-business-directory');
  expect(screen.getByTestId('source-scan-status')).toHaveTextContent('已扫描');
  expect(screen.getByText('已识别文件')).toBeVisible();
  expect(screen.getByTestId('pending-review-guide')).toHaveTextContent('常用指标包括');
  expect(screen.getByRole('button', { name: '确认当前字段识别' })).toBeVisible();
  expect(screen.queryByText('确认通用字段映射')).toBeNull();
  expect(screen.queryByText(/generic/)).toBeNull();
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
