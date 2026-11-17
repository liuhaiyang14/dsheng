import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { WS_DEMO_ID } from '../api/fixtureData';
import { ImportDropzone } from '../components/ImportDropzone';
const directoryKey = `dsheng.source-directory.${WS_DEMO_ID}`;
beforeEach(() => { window.localStorage.removeItem(directoryKey); window.localStorage.removeItem(`dsheng.source-profile.${WS_DEMO_ID}`); });
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
  const rawScan = client.scan.bind(client);
  vi.spyOn(client, 'scan').mockImplementation(async (...args) => {
    const result = await rawScan(...args);
    return { ...result, files: result.files.map((file) => ({
      ...file,
      note: `${file.file_label}:generic_table/profile:generic_source(${file.row_count}行)`,
    })) };
  });
  await openSource(client);
  expect(screen.getByLabelText('原始数据模板')).toHaveValue('xsd_trade_v1');
  expect(screen.getByText('贸易/制造业单据模板（当前示例）')).toBeVisible();
  await scan('/tmp/example-source');
  expect(screen.getByText(/模板：贸易\/制造业单据模板（当前示例）/)).toBeVisible();
  expect(screen.queryByText(/generic/)).toBeNull();
  expect(screen.queryByText(/profile:/)).toBeNull();
  expect(screen.getByTestId('pending-review-guide')).toHaveTextContent('系统已经按当前输入模板猜测了文件用途');
  expect(screen.getByTestId('pending-review-guide')).toHaveTextContent('金额、日期等关键列');
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
  expect(screen.queryByLabelText(`确认字段：${fileLabel}`)).toBeNull();
  expect(screen.getByTestId('field-review-panel')).toHaveTextContent('金额');
  expect(screen.getByTestId('field-review-panel')).toHaveTextContent('业务日期');
  fireEvent.change(screen.getByLabelText(`确认状态：${fileLabel}`), { target: { value: 'confirmed' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));

  await waitFor(() => expect(mappingSpy).toHaveBeenCalledTimes(1));
  const mappings = mappingSpy.mock.calls[0][3] as Array<Record<string, string>>;
  expect(mappings).toEqual(expect.arrayContaining([
    expect.objectContaining({ target_role: 'profile_mapping', source_label: fileLabel }),
  ]));
  await waitFor(() => expect(scanSpy).toHaveBeenCalledTimes(2));
  expect(screen.queryByTestId('field-review-panel')).toBeNull();
  expect(screen.getByTestId('scan-note')).toHaveTextContent('字段确认已保存并重新扫描');
});

it('通用模板没有可分析期间时，提供示例模板重扫入口', async () => {
  const client = new FixtureClient();
  const rawScan = client.scan.bind(client);
  const scanSpy = vi.spyOn(client, 'scan').mockImplementation(async (...args) => {
    const result = await rawScan(...args);
    return args[3] ? { ...result, available_periods: [] } : result;
  });
  await openSource(client);
  fireEvent.change(screen.getByLabelText('原始数据模板'), { target: { value: 'generic' } });
  await scan('/tmp/detail-source');
  expect(screen.getByTestId('generic-period-guide')).toHaveTextContent('当前还没有可生成的分析期间');
  expect(screen.getByTestId('generic-period-guide')).toHaveTextContent('销售出库、采购入库、工资条');
  fireEvent.click(screen.getByRole('button', { name: '使用当前示例模板并重新扫描' }));
  await waitFor(() => expect(scanSpy).toHaveBeenLastCalledWith(
    WS_DEMO_ID,
    '/tmp/detail-source',
    'xsd_trade_v1',
    false,
  ));
  expect(screen.getByLabelText('原始数据模板')).toHaveValue('xsd_trade_v1');
});

it('profile 模板仍有待确认文件时，经营分析入口保持禁用并说明原因', async () => {
  const client = new FixtureClient();
  await openSource(client);
  await scan('/tmp/example-source');
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营分析/ }));
  fireEvent.click(screen.getByText('重新分析数据'));
  await waitFor(() => expect(screen.getByRole('button', { name: '运行本地分析' })).toBeDisabled());
  expect(screen.getByText(/部分原始文件的业务用途尚未确认/)).toBeVisible();
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
  expect(screen.getByTestId('pending-review-guide')).toHaveTextContent('确认需要换算的税口径');
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

it('重新打开应用后经营分析重扫沿用该企业自定义模板，不降级通用表', async () => {
  const client = new FixtureClient();
  const spy = vi.spyOn(client, 'scan');
  const view = await openSource(client);
  fireEvent.change(screen.getByLabelText('原始数据模板'), { target: { value: 'custom' } });
  fireEvent.change(screen.getByLabelText('自定义模板路径'), { target: { value: '/tmp/my-business-profile.json' } });
  await scan('/tmp/business-source');
  view.unmount();
  await openSource(client);
  expect(screen.getByLabelText('自定义模板路径')).toHaveValue('/tmp/my-business-profile.json');
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /经营分析/ }));
  fireEvent.click(screen.getByText('重新分析数据'));
  fireEvent.click(screen.getByRole('button', { name: '重新扫描数据' }));
  await waitFor(() => expect(spy).toHaveBeenLastCalledWith(WS_DEMO_ID, '/tmp/business-source', '/tmp/my-business-profile.json', false));
  await waitFor(() => expect(screen.getByTestId('workspace-select')).toBeEnabled());
});

it('扫描期间锁定企业和模板选择，完成后恢复操作', async () => {
  const client = new FixtureClient();
  const original = client.scan.bind(client);
  let complete!: () => void;
  vi.spyOn(client, 'scan').mockImplementation(async (...args) => { await new Promise<void>((resolve) => { complete = resolve; }); return original(...args); });
  await openSource(client);
  fireEvent.change(screen.getByTestId('manual-path-input'), { target: { value: '/tmp/business-source' } });
  fireEvent.click(screen.getByTestId('manual-path-scan'));
  expect(screen.getByTestId('workspace-select')).toBeDisabled();
  expect(screen.getByLabelText('原始数据模板')).toBeDisabled();
  await act(async () => complete());
  await waitFor(() => expect(screen.getByTestId('workspace-select')).toBeEnabled());
  expect(screen.getByTestId('source-directory')).toHaveTextContent('/tmp/business-source');
});

it.each(['manufacturing_demo', 'service_demo'])('%s 标准表确认用途时不要求列覆盖，重扫保留全部模板字段', async (profile) => {
  const client = new FixtureClient();
  const original = client.scan.bind(client);
  const scanSpy = vi.spyOn(client, 'scan').mockImplementation(async (...args) => {
    const summary = await original(...args);
    return { ...summary, files: summary.files.map((file) => ({ ...file, review: { kind: 'generic' as const, mappings: [], confirmation: file.review?.confirmation } })) };
  });
  const save = vi.spyOn(client, 'sourceMappingSet');
  await openSource(client);
  fireEvent.change(screen.getByLabelText('原始数据模板'), { target: { value: profile } });
  await scan(`/tmp/${profile}`);
  fireEvent.click(screen.getByTestId('open-field-review-button'));
  expect(screen.getAllByText('已按模板识别标准经营数据表')).toHaveLength(3);
  expect(screen.queryByLabelText(/确认列：/)).toBeNull();
  const panel = screen.getByTestId('field-review-panel');
  for (const select of within(panel).getAllByLabelText(/确认状态：/)) fireEvent.change(select, { target: { value: 'confirmed' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  const entries = save.mock.calls[0][3] as Array<Record<string, unknown>>;
  expect(entries).toHaveLength(3);
  for (const entry of entries) { expect(entry.target_role).toBe('profile_mapping'); expect(entry).not.toHaveProperty('column'); }
  await waitFor(() => expect(scanSpy).toHaveBeenCalledTimes(2));
  await waitFor(() => expect(screen.queryByTestId('field-review-panel')).toBeNull());
  expect(screen.getByTestId('open-field-review-button')).toHaveTextContent('查看 / 重新确认');
});
