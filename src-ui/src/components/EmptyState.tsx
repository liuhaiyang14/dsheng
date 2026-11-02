import { PanelNote } from '../components/bits';

/** 通用空状态。 */
export function EmptyState({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="empty-state" data-testid="empty-state">
      <strong>{title}</strong>
      {hint ? <p style={{ margin: '8px 0 0' }}>{hint}</p> : null}
      {children}
    </div>
  );
}

/** 未连接核心时的全局空状态（无 Tauri 环境且未显式启用演示数据层）。 */
export function CoreMissingState() {
  return (
    <div className="app-window" data-testid="core-missing">
      <aside className="side">
        <div className="brand">
          <div className="brand-mark">两</div>
          <h1>两表一单</h1>
          <p>本地经营分析工具</p>
        </div>
        <div className="side-foot">
          未连接本地核心。
          <br />
          桌面版由 Tauri 壳接入核心后可用；本页面不使用网络数据。
        </div>
      </aside>
      <main className="main">
        <header className="topbar">
          <div className="crumb">企业档案 — / <strong>未连接核心</strong></div>
          <div className="top-actions">
            <span className="core-missing">未连接核心</span>
          </div>
        </header>
        <section className="content">
          <EmptyState
            title="未连接本地核心"
            hint="当前未运行在 Tauri 桌面环境中，也没有显式启用演示数据层。本工具的业务计算、数据读取与保存全部由本地 Rust 核心完成，UI 不从网络获取任何数据，也不会静默使用虚构数据。"
          >
            <p style={{ marginTop: 12 }}>
              <PanelNote>
                集成说明：由 Tauri 壳加载本前端并暴露 window.__TAURI__ 后自动连接；
                页面开发可显式追加 ?dsheng-fixtures=1 启用标注过的虚构演示数据层。
              </PanelNote>
            </p>
          </EmptyState>
        </section>
      </main>
    </div>
  );
}
