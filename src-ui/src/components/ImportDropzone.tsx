/**
 * 数据导入入口：拖拽（Tauri 原生拖放事件→真实路径）＋ 选择目录（系统目录选择器）。
 * 取得路径后调用 scan（本地解析，不发请求），完成由父级 reload 呈现结果。
 *
 * 测试层次说明：fixture/浏览器环境无 Tauri 时退化为 HTML5 drop（仅文件名）与
 * 手填路径输入框；正式桌面环境走 window.__TAURI__（withGlobalTauri 注入）。
 */

import { useEffect, useRef, useState } from 'react';

interface TauriDragDropEvent {
  type: 'enter' | 'over' | 'drop' | 'leave';
  paths: string[];
  position: { x: number; y: number };
}

interface TauriWindowApi {
  onDragDropEvent(
    handler: (event: TauriDragDropEvent) => void,
  ): Promise<() => void>;
}

interface TauriDialogApi {
  open(options: { directory: boolean; title?: string }): Promise<string | string[] | null>;
}

function tauriWindow(): TauriWindowApi | null {
  const w = window as unknown as { __TAURI__?: { window?: { getCurrentWindow?: () => TauriWindowApi } } };
  try {
    return w.__TAURI__?.window?.getCurrentWindow?.() ?? null;
  } catch {
    return null;
  }
}

function tauriDialog(): TauriDialogApi | null {
  const w = window as unknown as { __TAURI__?: { dialog?: TauriDialogApi } };
  return w.__TAURI__?.dialog ?? null;
}

export function ImportDropzone(props: {
  busy: boolean;
  onPath: (path: string, note?: string) => void;
}) {
  const [hover, setHover] = useState(false);
  const [manual, setManual] = useState('');
  const [error, setError] = useState<string | null>(null);
  const dropRef = useRef<HTMLDivElement | null>(null);

  const handlePaths = (paths: string[]) => {
    if (paths.length === 0) return;
    const first = paths[0];
    // scan 以目录为单位：拖入文件夹→直接用；拖入文件→取其所在目录
    const guessDir = first.split('/').slice(0, -1).join('/') || '/';
    const note =
      paths.length > 1
        ? `收到 ${paths.length} 项，扫描其所在目录：${guessDir}`
        : first.endsWith('.xlsx') || first.endsWith('.xls') || first.endsWith('.csv')
          ? `扫描文件所在目录：${guessDir}`
          : undefined;
    props.onPath(guessDir, note);
  };

  // Tauri 桌面：窗口级拖放事件（真实路径）
  useEffect(() => {
    const w = tauriWindow();
    if (!w) return;
    let unlisten: (() => void) | null = null;
    let alive = true;
    void w.onDragDropEvent((event) => {
      if (event.type === 'enter' || event.type === 'over') setHover(true);
      if (event.type === 'leave') setHover(false);
      if (event.type === 'drop') {
        setHover(false);
        handlePaths(event.paths ?? []);
      }
    }).then((fn) => {
      if (alive) unlisten = fn;
      else fn();
    }).catch(() => undefined);
    return () => {
      alive = false;
      unlisten?.();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pickDirectory = async () => {
    setError(null);
    const dialog = tauriDialog();
    if (dialog) {
      try {
        const dir = await dialog.open({ directory: true, title: '选择经营数据目录' });
        if (typeof dir === 'string' && dir) props.onPath(dir);
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err));
      }
      return;
    }
    // 浏览器/测试退路：手动填路径
    const input = window.prompt('输入数据目录的绝对路径（演示/测试通道）', manual || '');
    if (input && input.trim()) {
      setManual(input.trim());
      props.onPath(input.trim());
    }
  };

  return (
    <div className="panel import-panel" data-testid="import-panel">
      <div className="panel-head">
        <span className="panel-title">导入经营数据</span>
        <span className="panel-note">两种方式任选 · 文件保留在原位置，不复制进数据库</span>
      </div>
      <div className="import-row">
        <div
          ref={dropRef}
          className={`dropzone interactive${hover ? ' hover' : ''}`}
          data-testid="import-dropzone"
          role="button"
          tabIndex={0}
          onClick={() => void pickDirectory()}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void pickDirectory();
          }}
          onDragOver={(e) => {
            e.preventDefault();
            if (!tauriWindow()) setHover(true);
          }}
          onDragLeave={() => {
            if (!tauriWindow()) setHover(false);
          }}
          onDrop={(e) => {
            // 浏览器/测试通道：HTML5 drop（无 Tauri 时）
            if (tauriWindow()) return;
            e.preventDefault();
            setHover(false);
            const names = Array.from(e.dataTransfer?.files ?? []).map((f) => f.name);
            if (names.length > 0) {
              props.onPath(
                `/演示路径/${names[0]}`,
                `测试通道收到文件：${names.join('、')}（桌面版拖拽使用真实路径）`,
              );
            }
          }}
        >
          <div className="folder">⇩</div>
          <p className="drop-title">把文件或文件夹拖到这里</p>
          <small>.xlsx / .xls / .csv · 支持多选，取所在目录扫描</small>
        </div>
        <div className="import-actions">
          <button
            type="button"
            className="primary"
            disabled={props.busy}
            onClick={() => void pickDirectory()}
            data-testid="pick-directory-button"
          >
            {props.busy ? '解析中…' : '选择目录…'}
          </button>
          <p className="import-hint">
            点击上方虚线框或本按钮均可打开系统目录选择器；
            拖拽直接使用文件真实路径。解析在本地完成，不发送任何数据。
          </p>
        </div>
      </div>
      {error ? <div className="onboarding-error">{error}</div> : null}
    </div>
  );
}
