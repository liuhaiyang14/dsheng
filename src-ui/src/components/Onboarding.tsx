/**
 * 首次启动引导 / 新建企业档案向导。
 *
 * 场景一：核心已连接且没有任何档案（首次使用）——自动弹出。
 * 场景二：顶栏「＋ 新建档案」——手动打开。
 * 创建走 workspace_create（含 initial_period_policy），创建后自动切换为当前档案。
 */

import { useState } from 'react';
import { useApp } from '../state/store';

const CADENCE_OPTIONS = [
  { value: 'day', label: '日度' },
  { value: 'week', label: '周度' },
  { value: 'month', label: '月度' },
  { value: 'quarter', label: '季度' },
] as const;

function todayIso(): string {
  const d = new Date();
  const m = `${d.getMonth() + 1}`.padStart(2, '0');
  const day = `${d.getDate()}`.padStart(2, '0');
  return `${d.getFullYear()}-${m}-${day}`;
}

export function Onboarding(props: { open: boolean; onClose: () => void }) {
  const app = useApp();
  const [name, setName] = useState('');
  const [cadence, setCadence] = useState<'day' | 'week' | 'month' | 'quarter'>('month');
  const [effectiveFrom, setEffectiveFrom] = useState(todayIso());
  const [timezone, setTimezone] = useState('Asia/Shanghai');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  if (!props.open) return null;

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setError('请填写企业名称（仅本机识别用）。');
      return;
    }
    if (!effectiveFrom) {
      setError('请设置周期策略生效日期。');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await app.createWorkspace({
        name: trimmed,
        cadence,
        effective_from: effectiveFrom,
        timezone_id: timezone,
        week_start_day: 1,
        fiscal_year_start_month: 1,
      });
      props.onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="onboarding-scrim" role="dialog" aria-label="新建企业档案">
      <div className="onboarding-card" data-testid="onboarding-card">
        <div className="eyebrow">WELCOME · 企业档案</div>
        <h3>创建你的第一份企业档案</h3>
        <p className="onboarding-lead">
          企业档案是本机里给一家企业单独建的资料夹（不是云端账号）。
          先建档案，再配置 Provider 与导入数据；不同档案的数据完全隔离。
        </p>
        <div className="onboarding-form">
          <label>
            企业名称
            <input
              className="select"
              value={name}
              placeholder="例如：示例制造企业"
              onChange={(e) => setName(e.target.value)}
              data-testid="onboarding-name"
            />
          </label>
          <label>
            默认核算周期
            <select
              className="select"
              value={cadence}
              onChange={(e) => setCadence(e.target.value as typeof cadence)}
              data-testid="onboarding-cadence"
            >
              {CADENCE_OPTIONS.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </select>
          </label>
          <label>
            生效日期
            <input
              className="select"
              type="date"
              value={effectiveFrom}
              onChange={(e) => setEffectiveFrom(e.target.value)}
              data-testid="onboarding-effective-from"
            />
          </label>
          <label>
            时区
            <select className="select" value={timezone} onChange={(e) => setTimezone(e.target.value)}>
              <option value="Asia/Shanghai">Asia/Shanghai (UTC+8)</option>
              <option value="UTC">UTC</option>
              <option value="America/Los_Angeles">America/Los_Angeles</option>
            </select>
          </label>
        </div>
        <p className="onboarding-hint">
          周起始日（周一）与财年起始月（一月）暂用默认值，创建后可在「系统设置」中
          以新版本修改；旧期间不会被改写。
        </p>
        {error ? (
          <div className="onboarding-error" data-testid="onboarding-error">
            {error}
          </div>
        ) : null}
        <div className="onboarding-actions">
          {app.workspaces.length > 0 ? (
            <button className="outline" onClick={props.onClose} disabled={busy}>
              取消
            </button>
          ) : null}
          <button className="primary" onClick={() => void submit()} disabled={busy} data-testid="onboarding-create">
            {busy ? '创建中…' : '创建并开始'}
          </button>
        </div>
      </div>
    </div>
  );
}
