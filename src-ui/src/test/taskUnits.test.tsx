import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureTasks } from '../api/fixtureData';
import type { TaskView } from '../api';

afterEach(() => { cleanup(); vi.restoreAllMocks(); });

it('任务目标按自身单位显示，关联指标签名不改变百分数、万元、数量或缺省单位', async () => {
  const client = new FixtureClient();
  const original = client.historyShow.bind(client);
  vi.spyOn(client, 'historyShow').mockImplementation(async (...args) => {
    const detail = await original(...args);
    detail.metrics = detail.metrics.map((metric) => metric.metric_id === 'm-margin'
      ? { ...metric, unit: '1', unit_signature: { dimension: 'ratio', base_unit: '1' } }
      : { ...metric, unit_signature: { dimension: 'money', base_unit: '元' } });
    return detail;
  });
  const cases = [
    { metric_id: 'm-margin', target_value: '20', unit: '%', expected: '20.0%', editorUnit: '%' },
    { metric_id: 'm-settle', target_value: '2', unit: '万元', expected: '2.00 万元', editorUnit: '万元' },
    { metric_id: 'm-settle', target_value: '1.23456789', unit: '件', expected: '1.234568 件', editorUnit: '件' },
    { metric_id: 'm-settle', target_value: '2.1234567', unit: null, expected: '2.123457', editorUnit: '' },
    { metric_id: 'm-settle', target_value: '0.2', unit: '{"dimension":"ratio","base_unit":"1"}', expected: '20.0%', editorUnit: '1' },
  ];
  const tasks: TaskView[] = cases.map((item, index) => ({
    ...structuredClone(fixtureTasks[0]), task_id: `00000000-0000-4000-8000-0000000009${index}1`, title: `独立任务单位 ${index + 1}`,
    metric_id: item.metric_id, target_value: item.target_value, unit: item.unit,
    planned_value: item.target_value, actual_value: item.target_value, current_value: item.target_value,
  }));
  vi.spyOn(client, 'taskList').mockResolvedValue(tasks);
  const update = vi.spyOn(client, 'taskUpdate');
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  fireEvent.click(within(screen.getByRole('navigation')).getByRole('button', { name: /任务单/ }));
  const table = await screen.findByTestId('task-list-table');
  for (const [index, item] of cases.entries()) {
    const title = within(table).getByText(tasks[index].title);
    const cells = within(title.closest('tr')!).getAllByRole('cell');
    expect(cells[2]).toHaveTextContent(item.expected);
    expect(cells[2]).not.toHaveTextContent('2,000.0%');
    // 列表与详情都显示任务自己的单位；编辑框保持原始精度。
    fireEvent.click(title);
    const drawer = await screen.findByTestId('task-drawer');
    expect(within(drawer).getByLabelText('目标数值')).toHaveValue(item.target_value);
    expect(within(drawer).getByLabelText('数值单位')).toHaveValue(item.editorUnit);
    for (const label of ['预定数值', '实绩数值', '当前数值']) {
      expect(within(drawer).getByLabelText(label)).toHaveValue(item.target_value);
    }
    fireEvent.click(within(drawer).getByRole('button', { name: '关闭详情' }));
  }
  expect(update).not.toHaveBeenCalled();
  expect(tasks.map((task) => [task.target_value, task.unit])).toEqual(cases.map((item) => [item.target_value, item.unit]));
});
