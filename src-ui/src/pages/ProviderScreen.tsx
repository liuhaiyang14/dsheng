/**
 * Provider 设置屏（S7）。
 *
 * 合同（provider-config-delivery.md §1）：
 * - Key 明文保存在本机 SQLite，必须披露风险；界面只显示掩码（key_present），
 *   公开 JSON 无 Key/前缀/hash；
 * - 保存配置不做模型连接测试（本页没有“测试连接”按钮，保存不发任何请求）；
 * - Key 只经单独通道提交（key_action=replace + 独立字段，不进 JSON）；
 * - 无默认 Provider 时不自动 fallback；设默认需显式操作。
 */

import { useEffect, useState } from 'react';
import { useApp } from '../state/store';
import { Tag } from '../components/bits';
import type { ProviderView } from '../api';

const DEFAULT_TIMEOUTS = { connect_seconds: 15, response_seconds: 180, total_seconds: 240 };

export function ProviderScreen() {
  const app = useApp();
  const providers = app.data.providers;
  const [selectedId, setSelectedId] = useState<string>('');
  const [name, setName] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [model, setModel] = useState('');
  const [apiKeyInput, setApiKeyInput] = useState('');
  const [stream, setStream] = useState(false);
  const [ctxWindow, setCtxWindow] = useState(524288);
  const [outReserve, setOutReserve] = useState(8192);
  const [replaceKey, setReplaceKey] = useState(false);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const selected: ProviderView | null =
    providers.find((p) => p.provider_id === selectedId) ?? providers[0] ?? null;

  useEffect(() => {
    if (selected) {
      setName(selected.config.name);
      setBaseUrl(selected.config.base_url);
      setModel(selected.config.model);
      setStream(selected.config.stream);
      setCtxWindow(selected.config.budget.context_window_tokens);
      setOutReserve(selected.config.budget.output_reserve_tokens);
      setApiKeyInput('');
      setReplaceKey(false);
    } else {
      setName('');
      setBaseUrl('');
      setModel('');
      setStream(false);
      setCtxWindow(65536);
      setOutReserve(8192);
      setApiKeyInput('');
      setReplaceKey(false);
    }
  }, [selected?.provider_id, selected == null]);

  const save = async () => {
    if (!app.client || !app.activeWorkspace) {
      setNotice('请先创建企业档案（右上角「＋ 新建档案」）。');
      return;
    }
    setNotice(null);
    if (!name.trim() || !baseUrl.trim() || !model.trim()) {
      setNotice('名称、Base URL 和模型都需要填写。');
      return;
    }
    if (!/^https:\/\//.test(baseUrl.trim()) && !/^http:\/\/(localhost|127\.0\.0\.1|\[::1\])(:|\/|$)/.test(baseUrl.trim())) {
      setNotice('Base URL 仅允许 HTTPS；HTTP 只允许字面 localhost/127.0.0.1/[::1]。');
      return;
    }
    if (selected == null && replaceKey && apiKeyInput.trim().length === 0) {
      setNotice('已选择替换 Key，但未提供新的访问凭证。');
      return;
    }
    setBusy(true);
    try {
      await app.client.providerSet(
        {
          schema_version: '1',
          operation: 'provider_upsert',
          workspace_id: app.activeWorkspace.workspace_id,
          provider_id: selected?.provider_id ?? null,
          expected_revision: selected?.revision ?? null,
          config: {
            name: name.trim(),
            protocol: 'chat_completions',
            base_url: baseUrl.trim(),
            model: model.trim(),
            stream: stream,
            timeouts: selected?.config.timeouts ?? DEFAULT_TIMEOUTS, // 保持已有超时
            budget: { context_window_tokens: ctxWindow, output_reserve_tokens: outReserve },
          },
          key_action: replaceKey ? 'replace' : 'keep',
        },
        replaceKey ? apiKeyInput : undefined,
      );
      setApiKeyInput('');
      setReplaceKey(false);
      // 首个 Provider 自动设为默认
      if (app.activeWorkspace?.active_provider_id == null) {
        try {
          const list = await app.client.providerList(app.activeWorkspace.workspace_id);
          if (list.length === 1) {
            await app.client.providerUse({
              schema_version: '1',
              operation: 'provider_use',
              workspace_id: app.activeWorkspace.workspace_id,
              provider_id: list[0].provider_id,
              expected_revision: app.activeWorkspace.config_revision,
            });
          }
        } catch {
          // 自动设默认失败不阻塞保存
        }
      }
      setNotice('已保存到当前企业档案（未发起任何连接测试或模型请求）。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const setDefault = async () => {
    if (!app.client || !app.activeWorkspace || !selected) return;
    setBusy(true);
    setNotice(null);
    try {
      await app.client.providerUse({
        schema_version: '1',
        operation: 'provider_use',
        workspace_id: app.activeWorkspace.workspace_id,
        provider_id: selected.provider_id,
        expected_revision: app.activeWorkspace.config_revision,
      });
      setNotice('已设为当前企业档案的默认 Provider。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const clearKey = async () => {
    if (!app.client || !app.activeWorkspace || !selected) return;
    if (
      !window.confirm(
        '确认清空该 Provider 的 API Key？清空只修改本地配置，不发请求；发送预览将提示缺少 Key。',
      )
    ) {
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      await app.client.providerClearKey({
        schema_version: '1',
        operation: 'provider_clear_key',
        workspace_id: app.activeWorkspace.workspace_id,
        provider_id: selected.provider_id,
        expected_revision: selected.revision,
        confirm_local: true,
      });
      setNotice('已清空 Key（key_present=false）。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const removeProvider = async () => {
    if (!app.client || !app.activeWorkspace || !selected) return;
    if (!window.confirm(`确认删除 Provider「${selected.config.name}」？历史快照保留；若为默认，删除后 active_provider_id 置空。`)) {
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      await app.client.providerDelete({
        schema_version: '1',
        operation: 'provider_delete',
        workspace_id: app.activeWorkspace.workspace_id,
        provider_id: selected.provider_id,
        expected_revision: selected.revision,
        confirm_local: true,
      });
      setNotice('已删除。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const isActive = selected != null && app.activeWorkspace?.active_provider_id === selected.provider_id;

  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">05 · provider</div>
          <h2 className="screen-title">Provider 设置</h2>
          <div className="sub">
            {app.activeWorkspace?.name} · 配置模型服务连接并保存到当前企业的本地 SQLite。
          </div>
        </div>
      </div>

      <div className="setting-intro" data-testid="provider-intro">
        <strong>Provider 设置是做什么的？</strong>
        Base URL 决定请求发往哪里，模型名决定使用哪个模型，API Key 是访问凭证。
        界面始终掩码显示 Key；Key 不进入日志、提示词、导出或错误信息。
        保存配置不做模型连接测试；实际发送前会在预览中确认范围与大小。
      </div>
      <div className="setting-intro risk" data-testid="provider-risk-disclosure">
        <strong>明文存储风险披露</strong>
        API Key 以<b>明文</b>保存在本机 SQLite 数据库中；能读取当前用户数据库文件的人也能读取
        Key。本地备份同样包含明文 Key，请自行妥善保管数据目录。本页保存不发起任何网络请求。
      </div>

      {notice ? (
        <div className="notice workspace-empty" data-testid="provider-notice">
          <span>{notice}</span>
        </div>
      ) : null}

      <div className="panel" style={{ maxWidth: 820 }}>
        <div className="panel-head">
          <span className="panel-title">OpenAI 兼容 Provider</span>
          {selected ? (
            <span style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              {isActive ? <Tag tone="green">默认 Provider</Tag> : <Tag tone="gold">未设为默认</Tag>}
              <span className="panel-note">revision {selected.revision}</span>
            </span>
          ) : (
            <Tag tone="gray">未配置</Tag>
          )}
        </div>
        <div className="mapping">
          {providers.length > 1 ? (
            <div className="mapping-row">
              <div>
                <label>选择配置</label>
              </div>
              <select
                className="select"
                value={selected?.provider_id ?? ''}
                onChange={(e) => setSelectedId(e.target.value)}
              >
                {providers.map((p) => (
                  <option key={p.provider_id} value={p.provider_id}>
                    {p.config.name}（revision {p.revision}）
                  </option>
                ))}
              </select>
            </div>
          ) : null}
          <div className="mapping-row">
            <div>
              <label>配置名称</label>
              <small className="setting-help">在本机中区分不同 Provider；同企业内唯一。</small>
            </div>
            <input
              className="select"
              value={name}
              onChange={(e) => setName(e.target.value)}
              aria-label="配置名称"
            />
          </div>
          <div className="mapping-row">
            <div>
              <label>Base URL</label>
              <small className="setting-help">
                模型服务的实际兼容 Base URL；不要求以 /v1 结尾。仅允许 HTTPS，回环地址可用 HTTP。
              </small>
            </div>
            <input
              className="select"
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
              aria-label="Base URL"
              data-testid="provider-base-url"
            />
          </div>
          <div className="mapping-row">
            <div>
              <label>模型</label>
              <small className="setting-help">用于生成诊断报告和任务候选的模型名称。</small>
            </div>
            <input
              className="select"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              aria-label="模型名"
            />
          </div>
          <div className="mapping-row">
            <div>
              <label>API Key（掩码）</label>
              <small className="setting-help">
                {selected?.key_present
                  ? '已保存 Key（界面不回显，仅显示是否存在的掩码状态）。'
                  : '尚未保存 Key；保存配置可暂缺 Key，发送预览会提示缺少。'}
              </small>
            </div>
            <div>
              <input
                className="select mock-key"
                type="password"
                value={apiKeyInput}
                onChange={(e) => {
                  setApiKeyInput(e.target.value);
                  setReplaceKey(e.target.value.length > 0);
                }}
                placeholder={selected?.key_present ? '••••••••（已保存，输入新值替换）' : '输入访问凭证'}
                aria-label="API Key"
                data-testid="provider-key-input"
                autoComplete="new-password"
              />
              <div className="key-mask-note" data-testid="key-presence">
                {selected?.key_present ? 'key_present = true（掩码显示，不回显明文）' : 'key_present = false'}
              </div>
            </div>
          </div>
          <div className="mapping-row">
            <div>
              <label>流式输出</label>
              <small className="setting-help">开启后可实时看到模型输出进度（推荐开启，避免长时间等待无响应）。</small>
            </div>
            <select
              className="select"
              value={stream ? 'true' : 'false'}
              onChange={(e) => setStream(e.target.value === 'true')}
              data-testid="provider-stream"
            >
              <option value="true">开启（推荐）</option>
              <option value="false">关闭</option>
            </select>
          </div>
          <div className="mapping-row">
            <div>
              <label>上下文窗口（tokens）</label>
              <small className="setting-help">模型能处理的上下文大小。业务表格较大时建议 65536 以上。</small>
            </div>
            <input
              className="select"
              type="number"
              value={ctxWindow}
              min={8192}
              max={1048576}
              onChange={(e) => setCtxWindow(Number(e.target.value))}
              data-testid="provider-ctx-window"
            />
          </div>
          <div className="mapping-row">
            <div>
              <label>输出预留（tokens）</label>
              <small className="setting-help">为模型输出预留的空间，一般不需要改。</small>
            </div>
            <input
              className="select"
              type="number"
              value={outReserve}
              min={1024}
              max={65536}
              onChange={(e) => setOutReserve(Number(e.target.value))}
            />
          </div>
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
            <button type="button" className="primary" onClick={() => void save()} disabled={busy}>
              保存 Provider
            </button>
            {selected && !isActive ? (
              <button type="button" className="outline" onClick={() => void setDefault()} disabled={busy}>
                设为默认
              </button>
            ) : null}
            {selected ? (
              <>
                <button type="button" className="outline" onClick={() => void clearKey()} disabled={busy}>
                  清空 Key
                </button>
                <button type="button" className="outline" onClick={() => void removeProvider()} disabled={busy}>
                  删除
                </button>
              </>
            ) : null}
            <span className="panel-note">保存不发请求 · 首次保存自动设为默认</span>
          </div>
        </div>
      </div>

      <div className="footer-note">
        新建配置不自动设为默认；删除默认后 active_provider_id 置空，不会自动换服务。
      </div>
    </div>
  );
}
