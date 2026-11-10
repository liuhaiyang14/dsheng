import { AppProvider, useApp, SCREEN_LABELS, workspaceHasData, type ScreenId } from '../state/store';
import { useState } from 'react';
import { CoreMissingState } from '../components/EmptyState';
import { Onboarding } from '../components/Onboarding';
import { AnalysisPeriodBar } from '../components/AnalysisPeriodBar';
import { PageErrorBoundary } from '../components/PageErrorBoundary';
import { ErrorBanner } from '../components/bits';
import { SourceScreen } from '../pages/SourceScreen';
import { AnalysisScreen } from '../pages/AnalysisScreen';
import { DiagnosisScreen } from '../pages/DiagnosisScreen';
import { TaskScreen } from '../pages/TaskScreen';
import { HistoryScreen } from '../pages/HistoryScreen';
import { ProviderScreen } from '../pages/ProviderScreen';
import { SystemSettingsScreen } from '../pages/SystemSettingsScreen';

const NAV_ITEMS: { id: ScreenId; icon: string }[] = [
  { id: 'source', icon: '↳' },
  { id: 'analysis', icon: '⌁' },
  { id: 'diagnosis', icon: '⌕' },
  { id: 'task', icon: '□' },
  { id: 'history', icon: '◷' },
];

const NAV_SETTINGS: { id: ScreenId; icon: string }[] = [
  { id: 'provider', icon: '⚙' },
  { id: 'system-settings', icon: '⌘' },
];

function Shell() {
  const [showCreate, setShowCreate] = useState(false);
  const app = useApp();

  if (!app.connected) {
    return <CoreMissingState />;
  }

  const hasData = workspaceHasData(app.data);

  return (
    <div className="app-window">
      <aside className="side">
        <div className="brand">
          <div className="brand-mark">两</div>
          <h1>两表一单</h1>
          <p>本地经营分析工具</p>
        </div>
        <nav className="nav" aria-label="主导航">
          {NAV_ITEMS.map((item) => (
            <button
              key={item.id}
              type="button"
              className={app.screen === item.id ? 'active' : ''}
              onClick={() => app.setScreen(item.id)}
              disabled={app.contextLockReason != null}
              aria-current={app.screen === item.id ? 'page' : undefined}
            >
              <span className="icon">{item.icon}</span>
              {SCREEN_LABELS[item.id]}
            </button>
          ))}
          <div className="nav-section">
            <div className="nav-section-label">设置</div>
            {NAV_SETTINGS.map((item) => (
              <button
                key={item.id}
                type="button"
                className={`nav-subitem${app.screen === item.id ? ' active' : ''}`}
                onClick={() => app.setScreen(item.id)}
              disabled={app.contextLockReason != null}
                aria-current={app.screen === item.id ? 'page' : undefined}
              >
                <span className="icon">{item.icon}</span>
                {SCREEN_LABELS[item.id]}
              </button>
            ))}
          </div>
        </nav>
        <div className="side-foot">
          当前企业档案
          <br />
          <span className="foot-workspace" data-testid="foot-workspace">
            {app.activeWorkspace?.name ?? '—'}
          </span>
          <br />
          <br />
          {app.mode === 'fixture'
            ? '全量虚构演示数据。正式版仅在本机运行；模型发送前逐次确认范围。'
            : '数据仅保存在本机；模型发送前逐次确认范围。'}
          <br />
          <span className="foot-version" data-testid="foot-version">
            v0.1.12 · powered by 海洋
          </span>
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <div className="crumb">
            企业档案{' '}
            <select
              className="select"
              aria-label="切换企业档案"
              value={app.activeWorkspaceId ?? ''}
              onChange={(e) => void app.switchWorkspace(e.target.value)}
              disabled={app.loading || app.contextLockReason != null}
              data-testid="workspace-select"
            >
              {app.workspaces.map((w) => (
                <option key={w.workspace_id} value={w.workspace_id}>
                  {w.name}
                </option>
              ))}
            </select>{' '}
            / <strong data-testid="crumb-screen">{SCREEN_LABELS[app.screen]}</strong>
          </div>
          <div className="top-actions">
            <AnalysisPeriodBar key={app.activeWorkspaceId} />
            <button
              className="outline"
              onClick={() => setShowCreate(true)}
              disabled={app.contextLockReason != null}
              data-testid="create-workspace-button"
            >
              ＋ 新建档案
            </button>
            {app.mode === 'fixture' ? (
              <span className="demo-stamp" data-testid="demo-stamp">
                虚构演示数据
              </span>
            ) : null}
          </div>
        </header>
        <section className="content">
          <ErrorBanner />
          {!hasData && app.screen !== 'provider' && app.screen !== 'system-settings' ? (
            <div className="notice workspace-empty" data-testid="workspace-empty-notice">
              <b>当前企业档案还没有数据</b>
              <span>
                该档案是独立的空档案。请先选择本地目录并完成字段映射；
                它不会显示或读取其他企业的分析、任务、历史和 Provider。
              </span>
            </div>
          ) : null}
          <PageErrorBoundary key={app.screen === 'provider' || app.screen === 'system-settings' ? `${app.screen}:${app.activeWorkspaceId}` : `${app.screen}:${app.activeWorkspaceId}:${app.selectedRun?.runId ?? ''}:${app.selectedRun?.revision ?? 'current'}:${app.data.currentDetail?.analysis_revision_no ?? ''}`}>
            {app.screen === 'source' ? <SourceScreen /> : null}
            {app.screen === 'analysis' ? <AnalysisScreen /> : null}
            {app.screen === 'diagnosis' ? <DiagnosisScreen /> : null}
            {app.screen === 'task' ? <TaskScreen /> : null}
            {app.screen === 'history' ? <HistoryScreen /> : null}
            {app.screen === 'provider' ? <ProviderScreen /> : null}
            {app.screen === 'system-settings' ? <SystemSettingsScreen /> : null}
          </PageErrorBoundary>
        </section>
        <Onboarding
          open={
            showCreate || (app.connected && app.bootstrapped && app.workspaces.length === 0)
          }
          onClose={() => setShowCreate(false)}
        />
      </main>
    </div>
  );
}

export function App({
  runtime,
}: {
  /** 测试注入（fixture/mock client）；缺省按运行时规则选择。 */
  runtime?: Parameters<typeof AppProvider>[0]['runtime'];
}) {
  return (
    <AppProvider runtime={runtime}>
      <Shell />
    </AppProvider>
  );
}
