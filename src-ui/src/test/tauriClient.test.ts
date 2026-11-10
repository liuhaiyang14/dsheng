import { afterEach, describe, expect, it, vi } from 'vitest';
import { TauriClient } from '../api/tauriClient';
import type { WorkspaceCreateInput, WorkspaceView } from '../api/types';

const workspace: WorkspaceView = {
  workspace_id: '00000000-0000-4000-8000-000000000101',
  name: '首次创建企业',
  config_revision: 1,
  active_provider_id: null,
  current_period_policy_id: '00000000-0000-4000-8000-000000000102',
  created_at: '2026-11-10T00:00:00Z',
  updated_at: '2026-11-10T00:00:00Z',
};

const createInput: WorkspaceCreateInput = {
  schema_version: '1',
  operation: 'workspace_create',
  name: workspace.name,
  initial_period_policy: {
    cadence: 'month',
    week_start_day: 1,
    fiscal_year_start_month: 1,
    timezone_id: 'Asia/Shanghai',
    effective_from: '2026-11-01',
  },
};

afterEach(() => {
  delete window.__TAURI__;
  vi.restoreAllMocks();
});

describe('TauriClient workspace contract', () => {
  it('解包 workspace_create 后的 workspace，并可用其 id 读取空历史', async () => {
    const invoke = vi.fn()
      .mockResolvedValueOnce({
        schema_version: '1',
        ok: true,
        data: { created: true, workspace, initial_period_policy: {} },
      })
      .mockResolvedValueOnce({
        schema_version: '1',
        ok: true,
        data: { runs: [] },
      });
    window.__TAURI__ = { core: { invoke } };
    const client = new TauriClient();

    const created = await client.workspaceCreate(createInput);
    const history = await client.historyList(created.workspace_id);

    expect(created).toEqual(workspace);
    expect(history).toEqual([]);
    expect(invoke).toHaveBeenNthCalledWith(1, 'workspace_create', { input: createInput });
    expect(invoke).toHaveBeenNthCalledWith(2, 'history_list', {
      workspaceId: workspace.workspace_id,
    });
  });

  it('解包 workspace_update 的 workspace 视图', async () => {
    const invoke = vi.fn().mockResolvedValue({
      schema_version: '1',
      ok: true,
      data: { workspace },
    });
    window.__TAURI__ = { core: { invoke } };
    const client = new TauriClient();

    const updated = await client.workspaceUpdate({
      schema_version: '1',
      operation: 'workspace_update',
      workspace_id: workspace.workspace_id,
      expected_revision: 1,
      patch: { name: '更新后的企业' },
    });

    expect(updated).toEqual(workspace);
  });

  it('解包周期和 Provider 设置命令的嵌套视图', async () => {
    const policy = {
      period_policy_id: workspace.current_period_policy_id,
      workspace_id: workspace.workspace_id,
      version: 1,
      policy: createInput.initial_period_policy,
      effective_to: null,
      created_at: workspace.created_at,
    };
    const provider = {
      provider_id: '00000000-0000-4000-8000-000000000103',
      workspace_id: workspace.workspace_id,
      revision: 1,
      config: {
        name: '本地模型', protocol: 'chat_completions' as const,
        base_url: 'http://localhost:11434/v1', model: 'demo', stream: true,
        timeouts: { connect_seconds: 15, response_seconds: 180, total_seconds: 240 },
        budget: { context_window_tokens: 524288, output_reserve_tokens: 8192 },
      },
      key_present: false,
      created_at: workspace.created_at,
      updated_at: workspace.updated_at,
    };
    const invoke = vi.fn()
      .mockResolvedValueOnce({ schema_version: '1', ok: true, data: { policy } })
      .mockResolvedValueOnce({ schema_version: '1', ok: true, data: { provider } })
      .mockResolvedValueOnce({ schema_version: '1', ok: true, data: { provider } });
    window.__TAURI__ = { core: { invoke } };
    const client = new TauriClient();

    await expect(client.periodSet({
      schema_version: '1', operation: 'period_policy_set',
      workspace_id: workspace.workspace_id, expected_revision: 1,
      policy: createInput.initial_period_policy,
    })).resolves.toEqual(policy);
    await expect(client.providerSet({
      schema_version: '1', operation: 'provider_upsert',
      workspace_id: workspace.workspace_id, provider_id: null, expected_revision: null,
      config: provider.config, key_action: 'keep',
    })).resolves.toEqual(provider);
    await expect(client.providerClearKey({
      schema_version: '1', operation: 'provider_clear_key',
      workspace_id: workspace.workspace_id, provider_id: provider.provider_id,
      expected_revision: provider.revision, confirm_local: true,
    })).resolves.toEqual(provider);
  });
});
