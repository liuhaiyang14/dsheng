import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './app/App';
import './app/app.css';

const container = document.getElementById('root');
if (!container) {
  throw new Error('缺少 #root 挂载点');
}

createRoot(container).render(
  <StrictMode>
    <App />
  </StrictMode>,
);

// JS 飞行记录仪：任何未捕获异常/拒绝写入 /tmp/dsheng-cmd.log（与 Rust 侧同文件）
{
  const w = window as unknown as {
    __TAURI__?: { core?: { invoke?: (c: string, a?: unknown) => Promise<unknown> } };
  };
  const log = (kind: string, detail: unknown) => {
    const invoke = w.__TAURI__?.core?.invoke;
    const msg = `${kind}: ${typeof detail === 'string' ? detail : JSON.stringify(String(detail)).slice(0, 500)}`;
    void invoke?.('js_log', { message: msg }).catch(() => undefined);
    console.error(msg);
  };
  window.addEventListener('error', (e) => log('error', `${e.message} @${e.filename}:${e.lineno}:${e.colno}`));
  window.addEventListener('unhandledrejection', (e) =>
    log('unhandledrejection', e.reason instanceof Error ? `${e.reason.message}\n${e.reason.stack ?? ''}` : String(e.reason)),
  );
}
