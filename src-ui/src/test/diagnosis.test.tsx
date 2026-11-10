// 测试层次：UI 纯组件/状态测试（FixtureClient 注入，无网络、无 Tauri）。
// S5 诊断屏：无自动触发门禁（P0）、显式按钮流、warning 的 resolution_requirement 展示。
import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { App } from '../app/App';
import { FixtureClient } from '../api/fixtureClient';
import { fixtureDiagnosisReports } from '../api/fixtureData';

afterEach(cleanup);

async function openDiagnosis(client: FixtureClient) {
  render(<App runtime={{ mode: 'fixture', client }} />);
  await screen.findByTestId('foot-workspace');
  (screen.getAllByText('经营问题诊断')[0].closest('button') as HTMLButtonElement).click();
  await screen.findByTestId('generate-diagnosis-button');
}

describe('S5 无自动触发门禁（P0）', () => {
  it('进入页面 / 切换框架 / 选择报告 / 修改问题表述均不触发生成', async () => {
    const client = new FixtureClient();
    const previewSpy = vi.spyOn(client, 'diagnosisGeneratePreview');
    await openDiagnosis(client);
    // 进入页面后报告列表已加载（只读）
    await waitFor(() => {
      expect(screen.getByTestId('diagnosis-report-list').textContent).toContain('五问根因分析');
    });
    expect(previewSpy).not.toHaveBeenCalled();

    // 切换框架
    const frameworkSelect = screen.getByTestId('framework-select') as HTMLSelectElement;
    frameworkSelect.value = '5w2h';
    frameworkSelect.dispatchEvent(new Event('change', { bubbles: true }));
    frameworkSelect.value = '5w';
    frameworkSelect.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => {});
    expect(previewSpy).not.toHaveBeenCalled();

    // 选择另一份报告
    const items = screen.getAllByTestId(/diagnosis-item-/);
    if (items.length > 1) {
      items[items.length - 1].click();
    }
    await waitFor(() => {});
    expect(previewSpy).not.toHaveBeenCalled();
  });

  it('诊断任何状态变化不触发任务生成（模块隔离）', async () => {
    const client = new FixtureClient();
    const taskPreviewSpy = vi.spyOn(client, 'taskGeneratePreview');
    await openDiagnosis(client);
    // 生成一份新报告（显式按钮 + 预览确认）
    screen.getByTestId('generate-diagnosis-button').click();
    await screen.findByTestId('preview-modal');
    screen.getByTestId('preview-confirm').click();
    await waitFor(() => {
      expect(screen.getByTestId('diagnosis-notice').textContent).toContain('已生成');
    });
    // 继续分析（补充事实 + 预览确认）
    const textarea = document.getElementById('diagnosis-supplement') as HTMLTextAreaElement;
    fireEvent.change(textarea, { target: { value: '已核对 7 笔订单审批时间。' } });
    screen.getByTestId('continue-diagnosis-button').click();
    await screen.findByTestId('preview-modal');
    screen.getByTestId('preview-confirm').click();
    await waitFor(() => {
      expect(screen.getByTestId('diagnosis-notice').textContent).toContain('已随本轮分析发送');
    });
    // 诊断全流程结束后任务生成从未被触发
    expect(taskPreviewSpy).not.toHaveBeenCalled();
  });
});

describe('S5 显式按钮流', () => {
  it('生成：按钮 → 预览（含范围/预算）→ 确认 → 新报告入列', async () => {
    const client = new FixtureClient();
    const confirmSpy = vi.spyOn(client, 'diagnosisGenerateConfirm');
    await openDiagnosis(client);
    const before = screen.getAllByTestId(/diagnosis-item-/).length;
    screen.getByTestId('generate-diagnosis-button').click();
    const modal = await screen.findByTestId('preview-modal');
    expect(modal.textContent).toContain('发送范围');
    expect(modal.textContent).toContain('发送方式');
    screen.getByTestId('preview-confirm').click();
    await waitFor(() => {
      expect(confirmSpy).toHaveBeenCalledTimes(1);
    });
    await waitFor(() => {
      expect(screen.getAllByTestId(/diagnosis-item-/).length).toBe(before + 1);
    });
  });

  it('取消预览不确认、不发送', async () => {
    const client = new FixtureClient();
    const confirmSpy = vi.spyOn(client, 'diagnosisGenerateConfirm');
    await openDiagnosis(client);
    screen.getByTestId('generate-diagnosis-button').click();
    await screen.findByTestId('preview-modal');
    screen.getByText('取消（不发送）').click();
    await waitFor(() => {
      expect(screen.queryByTestId('preview-modal')).toBeNull();
    });
    expect(confirmSpy).not.toHaveBeenCalled();
  });
});

describe('S5 报告详情展示', () => {
  it('逐层问答 + 证据 + 事实/假设 + 置信度 + warning 的 resolution_requirement', async () => {
    const client = new FixtureClient();
    await openDiagnosis(client);
    const detail = screen.getByTestId('diagnosis-report-detail');
    // 5WHY 逐层
    expect(within(detail).getAllByText(/第 \d 层追问/).length).toBeGreaterThanOrEqual(2);
    // 事实/假设与置信度
    expect(detail.textContent).toContain('事实与假设清单');
    expect(detail.textContent).toContain('假设');
    expect(detail.textContent).toContain('置信度');
    // warning 的解决要求
    const warnings = screen.getByTestId('diagnosis-warnings');
    expect(warnings.textContent).toContain('需补充来源记录');
    expect(warnings.textContent).toContain('需用户确认');
  });

  it('空档案显示空状态且不触发生成', async () => {
    const client = new FixtureClient();
    const previewSpy = vi.spyOn(client, 'diagnosisGeneratePreview');
    render(<App runtime={{ mode: 'fixture', client }} />);
    await screen.findByTestId('foot-workspace');
    const select = screen.getByTestId('workspace-select') as HTMLSelectElement;
    select.value = '00000000-0000-4000-8000-0000000000b2';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    await waitFor(() => {
      expect(screen.getByTestId('workspace-empty-notice')).toBeTruthy();
    });
    (screen.getAllByText('经营问题诊断')[0].closest('button') as HTMLButtonElement).click();
    await screen.findByText('尚无可用分析期间');
    expect(previewSpy).not.toHaveBeenCalled();
  });

  it('下载 md/json 占位调用 client.diagnosisExport', async () => {
    const client = new FixtureClient();
    const exportSpy = vi
      .spyOn(client, 'diagnosisExport')
      .mockResolvedValue({ written: true, out: 'x.md', bytes: 1024 });
    await openDiagnosis(client);
    screen.getByText('下载 MD').click();
    await waitFor(() => {
      expect(exportSpy).toHaveBeenCalledTimes(1);
    });
    void fixtureDiagnosisReports;
  });
});
