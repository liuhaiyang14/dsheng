/**
 * 系统设置屏（S7）：企业档案 + 周期策略。
 *
 * 语义：修改周期（cadence/周起始/财年/时区/生效日期）保存 = 建立新的
 * period_policy 版本；旧期间沿用原口径，不重写历史。
 */

import { useEffect, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { Tag } from '../components/bits';
import type { Cadence } from '../api';

const CADENCE_LABELS: Record<Cadence, string> = {
  day: '日度',
  week: '周度',
  month: '月度',
  quarter: '季度',
};

export function SystemSettingsScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const policies = app.data.policies;
  const currentPolicy = policies.find((p) => p.effective_to === null) ?? policies[policies.length - 1] ?? null;

  const [name, setName] = useState('');
  const [cadence, setCadence] = useState<Cadence>('month');
  const [effectiveFrom, setEffectiveFrom] = useState('');
  const [weekStartDay, setWeekStartDay] = useState('1');
  const [fiscalYearStart, setFiscalYearStart] = useState('1');
  const [timezone, setTimezone] = useState('Asia/Shanghai');
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    setName(app.activeWorkspace?.name ?? '');
    if (currentPolicy) {
      setCadence(currentPolicy.policy.cadence);
      setWeekStartDay(String(currentPolicy.policy.week_start_day));
      setFiscalYearStart(String(currentPolicy.policy.fiscal_year_start_month));
      setTimezone(currentPolicy.policy.timezone_id);
      setEffectiveFrom(currentPolicy.policy.effective_from);
    }
  }, [app.activeWorkspace?.workspace_id, currentPolicy?.period_policy_id]);

  const saveWorkspaceName = async () => {
    if (!app.client || !app.activeWorkspace) return;
    if (!name.trim()) {
      setNotice('企业名称不能为空。');
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      await app.client.workspaceUpdate({
        schema_version: '1',
        operation: 'workspace_update',
        workspace_id: app.activeWorkspace.workspace_id,
        expected_revision: app.activeWorkspace.config_revision,
        patch: { name: name.trim() },
      });
      setNotice('企业名称已保存（仅本机识别名，不修改原始表格）。');
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const savePolicy = async () => {
    if (!app.client || !app.activeWorkspace) return;
    if (!effectiveFrom) {
      setNotice('请设置新周期策略的生效日期。');
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const view = await app.client.periodSet({
        schema_version: '1',
        operation: 'period_policy_set',
        workspace_id: app.activeWorkspace.workspace_id,
        expected_revision: app.activeWorkspace.config_revision,
        policy: {
          cadence,
          week_start_day: Number(weekStartDay),
          fiscal_year_start_month: Number(fiscalYearStart),
          timezone_id: timezone,
          effective_from: effectiveFrom,
        },
      });
      setNotice(
        `已建立周期策略版本 v${view.version}（${CADENCE_LABELS[cadence]}，自 ${effectiveFrom} 生效）。旧期间继续按原版本查询，不会自动拆分或改写。`,
      );
      await app.reloadWorkspaceData();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">06 · system settings</div>
          <h2 className="screen-title">系统设置</h2>
          <div className="sub">
            {app.activeWorkspace?.name} · 管理当前企业档案的周期、日期边界与本机数据。
          </div>
        </div>
      </div>

      <div className="setting-intro">
        <strong>“企业档案”是什么？</strong>
        它是本机里给一家企业单独建的一份资料夹，不是云端账号。每份档案有自己的数据目录、
        字段对应关系、核算周期、指标算法、Provider、历史分析和任务记录。切换顶部的企业档案后，
        只能看到该企业自己的内容；不同企业的数据不会混在一起。
      </div>

      {notice ? (
        <div className="notice workspace-empty" data-testid="settings-notice">
          <span>{notice}</span>
        </div>
      ) : null}

      <div className="panel" style={{ maxWidth: 940 }}>
        <div className="panel-head">
          <span className="panel-title">企业档案与核算周期</span>
          <span className="panel-note">
            当前：{app.activeWorkspace?.name} · 策略 v{currentPolicy?.version ?? 1} ·
            workspace revision {app.activeWorkspace?.config_revision ?? '—'}
          </span>
        </div>
        <div className="mapping">
          <div className="mapping-row">
            <div>
              <label>企业名称</label>
              <small className="setting-help">只是本机中的识别名称，不会修改原始表格。</small>
            </div>
            <div style={{ display: 'flex', gap: 8 }}>
              <input
                className="select"
                value={name}
                onChange={(e) => setName(e.target.value)}
                aria-label="企业名称"
                data-testid="workspace-name-input"
              />
              <button type="button" className="outline" onClick={() => void saveWorkspaceName()} disabled={busy}>
                保存名称
              </button>
            </div>
          </div>
          <div className="mapping-row">
            <div>
              <label>默认核算周期</label>
              <small className="setting-help">决定新分析默认按日、周、月或季度归并数据。</small>
            </div>
            <select
              className="select"
              value={cadence}
              onChange={(e) => setCadence(e.target.value as Cadence)}
              aria-label="默认核算周期"
              data-testid="cadence-select"
            >
              {(Object.keys(CADENCE_LABELS) as Cadence[]).map((c) => (
                <option key={c} value={c}>
                  {CADENCE_LABELS[c]}
                </option>
              ))}
            </select>
          </div>
          <div className="mapping-row">
            <div>
              <label>新策略生效日期</label>
              <small className="setting-help">修改周期从哪天开始使用新版本；旧期间沿用原口径。</small>
            </div>
            <input
              className="select"
              type="date"
              value={effectiveFrom}
              onChange={(e) => setEffectiveFrom(e.target.value)}
              aria-label="新策略生效日期"
              data-testid="effective-from-input"
            />
          </div>
          <div className="mapping-row">
            <div>
              <label>周起始日</label>
              <small className="setting-help">周度汇总的第一天；也影响任务周视图的日期分组。</small>
            </div>
            <select className="select" value={weekStartDay} onChange={(e) => setWeekStartDay(e.target.value)} aria-label="周起始日">
              <option value="1">周一</option>
              <option value="0">周日</option>
            </select>
          </div>
          <div className="mapping-row">
            <div>
              <label>财年起始月</label>
              <small className="setting-help">决定年度累计和财年同比的起点。</small>
            </div>
            <select className="select" value={fiscalYearStart} onChange={(e) => setFiscalYearStart(e.target.value)} aria-label="财年起始月">
              <option value="1">一月</option>
              <option value="4">四月</option>
              <option value="7">七月</option>
              <option value="10">十月</option>
            </select>
          </div>
          <div className="mapping-row">
            <div>
              <label>时区</label>
              <small className="setting-help">决定业务日期归属和进度事件显示时间。</small>
            </div>
            <select className="select" value={timezone} onChange={(e) => setTimezone(e.target.value)} aria-label="时区">
              <option value="Asia/Shanghai">Asia/Shanghai (UTC+8)</option>
              <option value="UTC">UTC</option>
              <option value="America/Los_Angeles">America/Los_Angeles</option>
            </select>
          </div>
          <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
            <button type="button" className="primary" onClick={() => void savePolicy()} disabled={busy} data-testid="save-policy-button">
              保存周期设置（建立新版本）
            </button>
            <span className="panel-note" data-testid="policy-version-note">
              保存将建立新的 period_policy 版本；历史期间不重算、不改写。
            </span>
          </div>
        </div>
      </div>

      <h3 className="setting-group-title">周期策略版本历史</h3>
      <div className="panel" style={{ maxWidth: 940 }}>
        <div className="table-wrap">
          <table className="data-table" data-testid="policy-history-table">
            <thead>
              <tr>
                <th>版本</th>
                <th>周期</th>
                <th>生效区间</th>
                <th>周起始 / 财年 / 时区</th>
                <th>创建于</th>
                <th>状态</th>
              </tr>
            </thead>
            <tbody>
              {policies.map((p) => (
                <tr key={p.period_policy_id}>
                  <td>v{p.version}</td>
                  <td>{CADENCE_LABELS[p.policy.cadence]}</td>
                  <td>
                    {p.policy.effective_from} 至 {p.effective_to ?? '当前'}
                  </td>
                  <td>
                    周{p.policy.week_start_day === 1 ? '一' : '日'} · {p.policy.fiscal_year_start_month} 月 ·{' '}
                    {p.policy.timezone_id}
                  </td>
                  <td>{p.created_at.slice(0, 10)}</td>
                  <td>
                    {p.effective_to == null ? (
                      <Tag tone="green">当前版本</Tag>
                    ) : (
                      <Tag tone="gray">已闭合</Tag>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>

      <h3 className="setting-group-title">这份企业档案包含什么</h3>
      <div className="panel" style={{ maxWidth: 940 }}>
        <div className="mapping">
          <div className="mapping-row">
            <div>
              <label>数据目录</label>
              <small className="setting-help">来源文件所在目录；原文件仍留在原位置，只读取不复制。</small>
            </div>
            <span>{app.data.scan?.directory_label || '尚未设置'}</span>
          </div>
          <div className="mapping-row">
            <div>
              <label>字段映射</label>
              <small className="setting-help">把表格列对应到工具字段；映射决定哪些值参与计算。</small>
            </div>
            <span>{hasData ? `${app.data.scan?.files.length ?? 0} 个文件 · 版本化草稿` : '尚未配置'}</span>
          </div>
          <div className="mapping-row">
            <div>
              <label>Provider 连接</label>
              <small className="setting-help">为当前企业选择模型地址与本地 API Key。</small>
            </div>
            <span>
              {app.data.providers.length > 0
                ? `${app.data.providers[0].config.name} · ${app.data.providers[0].config.model}`
                : '未配置'}
            </span>
          </div>
          <div className="mapping-row">
            <div>
              <label>历史分析与任务</label>
              <small className="setting-help">按期间保存分析结果、版本快照、任务与进度事件。</small>
            </div>
            <span>
              {app.data.history.length} 个期间 · {app.data.tasks.length} 项当前任务
            </span>
          </div>
          <div className="mapping-row">
            <div>
              <label>本机存储与隔离</label>
              <small className="setting-help">SQLite 放在当前用户应用数据目录；按企业档案隔离。</small>
            </div>
            <span>本机保存 · 原始 XLSX 不复制进数据库</span>
          </div>
        </div>
      </div>

      <div className="footer-note">周期策略变更不迁移旧期间口径；重算需在历史页显式执行。</div>
    </div>
  );
}
