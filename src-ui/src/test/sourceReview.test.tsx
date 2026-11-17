import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { SourceReviewPanel } from '../components/SourceReviewPanel';
import type { ScanFileInfo } from '../api';

const files: ScanFileInfo[] = [{
  file_id: 'sales', file_label: '销售出库.xlsx', row_count: 10, sheet_count: 1, mapping_status: 'pending_review', note: null,
  review: { kind: 'records', mappings: [
    { metric_key: 'sales_net', value_fields: ['含税售价'], date_fields: ['业务日期'], missing_fields: [], source_tax_basis: 'inclusive', requires_confirmation: false, status_field: '单据状态', accepted_statuses: ['已出库'] },
    { metric_key: 'order_cost', value_fields: ['采购成本'], date_fields: ['业务日期'], missing_fields: [], source_tax_basis: 'confirm_at_runtime', requires_confirmation: false, accepted_statuses: [] },
  ] },
}, { file_id: 'account', file_label: '资金账户.xlsx', row_count: 2, sheet_count: 1, mapping_status: 'pending_review', note: null, review: { kind: 'auxiliary', mappings: [] } }];
afterEach(cleanup);
function open(input = files, generic = false) {
  const save = vi.fn().mockResolvedValue(undefined);
  render(<SourceReviewPanel files={input} generic={generic} busy={false} onSave={save} onClose={vi.fn()} />);
  return save;
}
it('按实际用途列出数值、日期、状态条件，辅助资料不伪装为收入', async () => {
  const save = open();
  expect(screen.getByText('含税售价')).toBeVisible();
  expect(screen.getByText('采购成本')).toBeVisible();
  expect(screen.getByText('单据状态：已出库')).toBeVisible();
  expect(screen.getByText('辅助资料，仅保留供查阅，不参与核算')).toBeVisible();
  expect(screen.queryByLabelText('确认字段：销售出库.xlsx')).toBeNull();
  fireEvent.change(screen.getByLabelText('确认状态：销售出库.xlsx'), { target: { value: 'confirmed' } });
  fireEvent.change(screen.getByLabelText('确认状态：资金账户.xlsx'), { target: { value: 'confirmed' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0]).toEqual(expect.arrayContaining([
    expect.objectContaining({ source_label: '销售出库.xlsx', target_role: 'profile_mapping', tax_treatments: {} }),
    expect.objectContaining({ source_label: '资金账户.xlsx', target_role: 'reference_only' }),
  ]));
});
it('销售与成本的税口径分别确认，含税无默认税率且不能强选已不含税', async () => {
  const save = open();
  fireEvent.change(screen.getByLabelText('确认状态：销售出库.xlsx'), { target: { value: 'confirmed' } });
  const salesBasis = screen.getByLabelText('税口径：销售出库.xlsx：不含税销售额');
  expect(within(salesBasis).queryByText('金额已不含税')).toBeNull();
  fireEvent.change(salesBasis, { target: { value: 'inclusive' } });
  const rate = screen.getByLabelText('税率：销售出库.xlsx：不含税销售额');
  expect(rate).toHaveValue('');
  fireEvent.change(rate, { target: { value: '13' } });
  fireEvent.change(screen.getByLabelText('税口径：销售出库.xlsx：订单成本'), { target: { value: 'exclusive' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0][0].tax_treatments).toEqual({ sales_net: { basis: 'inclusive', rate_percent: '13' }, order_cost: { basis: 'exclusive' } });
});
it('选择含税但留空税率时只保存文件确认，明确零税率才保存零', async () => {
  const save = open();
  fireEvent.change(screen.getByLabelText('确认状态：销售出库.xlsx'), { target: { value: 'confirmed' } });
  fireEvent.change(screen.getByLabelText('税口径：销售出库.xlsx：不含税销售额'), { target: { value: 'inclusive' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0][0].tax_treatments).toEqual({});
  fireEvent.change(screen.getByLabelText('税率：销售出库.xlsx：不含税销售额'), { target: { value: '0' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save.mock.calls[1][0][0].tax_treatments).toEqual({ sales_net: { basis: 'inclusive', rate_percent: '0' } });
});
it('已确认文件再次编辑保留各指标税口径，缺列仍明确提示', () => {
  const confirmed = structuredClone(files);
  confirmed[0].mapping_status = 'mapped';
  confirmed[0].review!.confirmation = { target_role: 'profile_mapping', tax_treatments: { sales_net: { basis: 'inclusive', rate_percent: '13' } } };
  confirmed[0].review!.mappings[1].missing_fields = ['采购成本'];
  open(confirmed);
  expect(screen.getByLabelText('税率：销售出库.xlsx：不含税销售额')).toHaveValue('13');
  expect(screen.getByLabelText('税口径：销售出库.xlsx：订单成本')).toHaveValue('');
  expect(screen.getByText(/缺少关键列：采购成本/)).toBeVisible();
});

it('实际模板的收款、付款和入库指标显示正确中文，通用列只提供核心支持的角色', () => {
  const cash = structuredClone(files[0]);
  cash.review!.mappings = ['receipts_amount', 'payments_amount', 'inbound_amount'].map((metric_key) => ({ metric_key, value_fields: ['金额'], date_fields: ['业务日期'], missing_fields: [], source_tax_basis: 'not_applicable', requires_confirmation: false, accepted_statuses: [] }));
  const generic: ScanFileInfo = { file_id: 'generic', file_label: '通用经营表.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'pending_review', note: null, review: { kind: 'generic', mappings: [] } };
  open([cash, generic], true);
  expect(screen.getByText('用于收款金额')).toBeVisible();
  expect(screen.getByText('用于付款金额')).toBeVisible();
  expect(screen.getByText('用于采购入库金额')).toBeVisible();
  const options = within(screen.getByLabelText('确认字段：通用经营表.xlsx')).getAllByRole('option').map((option) => (option as HTMLOptionElement).value);
  expect(options).toEqual(['', 'period', 'metric_label', 'actual', 'planned', 'annual_month_plan']);
  expect(options).not.toContain('cash_received');
  expect(options).not.toContain('cash_paid');
  expect(options).not.toContain('actual_value');
});

it('模板识别的标准表直接确认用途，不要求手填列，也不覆盖已识别列', async () => {
  const standard: ScanFileInfo = { file_id: 'standard', file_label: '标准经营数据.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'pending_review', note: null, review: { kind: 'generic', mappings: [] } };
  const save = open([standard]);
  expect(screen.getByText(/期间、指标名称、实际值等列由模板自动识别/)).toBeVisible();
  expect(screen.queryByLabelText('确认列：标准经营数据.xlsx')).toBeNull();
  expect(screen.queryByLabelText('确认字段：标准经营数据.xlsx')).toBeNull();
  fireEvent.change(screen.getByLabelText('确认状态：标准经营数据.xlsx'), { target: { value: 'confirmed' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0][0]).toMatchObject({ target_role: 'profile_mapping', recognized: '已确认' });
  expect(save.mock.calls[0][0][0]).not.toHaveProperty('column');
});

it('显式通用模式仍要求列字母，拒绝中文表头并规范化合法输入', async () => {
  const file: ScanFileInfo = { file_id: 'g', file_label: '通用经营数据.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'pending_review', note: null, review: { kind: 'generic', mappings: [] } };
  const save = open([file], true);
  fireEvent.change(screen.getByLabelText('确认状态：通用经营数据.xlsx'), { target: { value: 'confirmed' } });
  fireEvent.change(screen.getByLabelText('确认字段：通用经营数据.xlsx'), { target: { value: 'actual' } });
  fireEvent.change(screen.getByLabelText('确认列：通用经营数据.xlsx'), { target: { value: '实际值' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  expect(screen.getByRole('alert')).toHaveTextContent('不能填写中文表头');
  expect(save).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText('确认列：通用经营数据.xlsx'), { target: { value: ' c ' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0][0]).toMatchObject({ target_role: 'actual', column: 'C' });
});

it('已确认的通用列重新打开时沿用保存值，不要求重新选择空字段', async () => {
  const file: ScanFileInfo = { file_id: 'g', file_label: '已确认通用表.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'mapped', note: null, review: { kind: 'generic', mappings: [], confirmation: { target_role: 'actual', column: 'C', recognized: 'C' } } };
  const save = open([file], true);
  expect(screen.getByLabelText('确认字段：已确认通用表.xlsx')).toHaveValue('actual');
  expect(screen.getByLabelText('确认列：已确认通用表.xlsx')).toHaveValue('C');
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0][0]).toMatchObject({ target_role: 'actual', column: 'C' });
});

it('已确认自动识别的通用文件留空保持原状，不能发送空数组误确认剩余文件', () => {
  const file: ScanFileInfo = { file_id: 'g', file_label: '已确认自动表.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'mapped', note: null, review: { kind: 'generic', mappings: [] } };
  const pending = { ...file, file_id: 'p', file_label: '未确认表.xlsx', mapping_status: 'pending_review' as const };
  const save = vi.fn(), close = vi.fn();
  render(<SourceReviewPanel files={[file, pending]} generic busy={false} onSave={save} onClose={close} />);
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  expect(screen.queryByRole('alert')).toBeNull();
  expect(save).not.toHaveBeenCalled();
  expect(close).toHaveBeenCalledOnce();
});

it('通用表展示所有已保存列修正，切换字段恢复其列并只提交当前字段', async () => {
  const file: ScanFileInfo = { file_id: 'g', file_label: '多列修正.xlsx', row_count: 3, sheet_count: 1, mapping_status: 'mapped', note: null,
    review: { kind: 'generic', mappings: [], confirmation: { target_role: 'actual', column: 'D', sheet: '经营数据' }, column_overrides: [
      { role: 'actual', column: 'D', sheet: '经营数据' }, { role: 'period', column: 'E', sheet: '经营数据' },
    ] } };
  const save = open([file], true);
  const saved = screen.getByLabelText('已保存的列修正：多列修正.xlsx');
  expect(saved).toHaveTextContent('实际值');
  expect(saved).toHaveTextContent('D 列 · 工作表：经营数据');
  expect(saved).toHaveTextContent('分析期间');
  expect(saved).toHaveTextContent('E 列 · 工作表：经营数据');
  expect(screen.getByText(/本次仅修改所选字段的列修正/)).toBeVisible();
  expect(screen.getByLabelText('确认列：多列修正.xlsx')).toHaveValue('D');
  fireEvent.change(screen.getByLabelText('确认字段：多列修正.xlsx'), { target: { value: 'period' } });
  expect(screen.getByLabelText('确认列：多列修正.xlsx')).toHaveValue('E');
  fireEvent.change(screen.getByLabelText('确认字段：多列修正.xlsx'), { target: { value: 'planned' } });
  expect(screen.getByLabelText('确认列：多列修正.xlsx')).toHaveValue('');
  fireEvent.change(screen.getByLabelText('确认字段：多列修正.xlsx'), { target: { value: 'period' } });
  fireEvent.change(screen.getByLabelText('确认列：多列修正.xlsx'), { target: { value: 'F' } });
  fireEvent.click(screen.getByTestId('save-field-review-button'));
  await waitFor(() => expect(save).toHaveBeenCalledOnce());
  expect(save.mock.calls[0][0]).toHaveLength(1);
  expect(save.mock.calls[0][0][0]).toMatchObject({ target_role: 'period', column: 'F', sheet: '经营数据' });
  expect(save.mock.calls[0][0][0]).not.toHaveProperty('tax_treatments');
});
