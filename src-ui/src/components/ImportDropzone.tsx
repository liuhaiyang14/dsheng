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

type EnvCheck = { tauri: boolean; dialog: boolean; dragDrop: boolean };

function tauriWindow(): TauriWindowApi | null {
  const w = window as unknown as { __TAURI__?: { window?: { getCurrentWindow?: () => TauriWindowApi } } };
  try {
    return w.__TAURI__?.window?.getCurrentWindow?.() ?? null;
  } catch {
    return null;
  }
}

function tauriEvent(): { listen(name: string, handler: (e: { payload: TauriDragDropEvent }) => void): Promise<() => void> } | null {
  const w = window as unknown as { __TAURI__?: { event?: { listen?: unknown } } };
  type ListenFn = (n: string, h: (e: { payload: TauriDragDropEvent }) => void) => Promise<() => void>;
  const listen = w.__TAURI__?.event?.listen as ListenFn | undefined;
  return typeof listen === 'function' ? { listen } : null;
}

function tauriDialog(): TauriDialogApi | null {
  const w = window as unknown as { __TAURI__?: { dialog?: TauriDialogApi } };
  const d = w.__TAURI__?.dialog;
  return d && typeof d.open === 'function' ? d : null;
}

/** 直连插件命令（绕过全局 API 桥；2026-11-04 真机问题：选择器能开但返回值不回 JS）。 */
async function directDialogOpen(options: {
  directory: boolean;
  title?: string;
}): Promise<string | string[] | null | undefined> {
  const w = window as unknown as {
    __TAURI__?: { core?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> } };
  };
  const invoke = w.__TAURI__?.core?.invoke;
  if (typeof invoke !== 'function') return undefined; // 无桥可用（交给下一层）
  const r = await invoke('plugin:dialog|open', { options });
  return r as string | string[] | null;
}

function envCheck(): EnvCheck {
  const w = window as unknown as { __TAURI__?: unknown };
  return {
    tauri: w.__TAURI__ != null,
    dialog: tauriDialog() != null,
    dragDrop: tauriWindow() != null || tauriEvent() != null,
  };
}

export function ImportDropzone(props: {
  busy: boolean;
  onPath: (path: string, note?: string) => void;
  /** 诊断行回调：把路径获取每一步的结果实时显示给用户/开发。 */
  onDiag?: (line: string) => void;
}) {
  const [hover, setHover] = useState(false);
  const [manual, setManual] = useState('');
  const [manualMode, setManualMode] = useState(false);
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

  // Tauri 桌面：窗口级拖放事件（真实路径）——onDragDropEvent 与 event.listen 双通道
  useEffect(() => {
    let unlisteners: Array<() => void> = [];
    let alive = true;
    const onEvent = (event: TauriDragDropEvent) => {
      if (event.type === 'enter' || event.type === 'over') setHover(true);
      if (event.type === 'leave') setHover(false);
      if (event.type === 'drop') {
        setHover(false);
        handlePaths(event.paths ?? []);
      }
    };
    const w = tauriWindow();
    if (w) {
      void w.onDragDropEvent(onEvent)
        .then((fn) => {
          if (alive) unlisteners.push(fn);
          else fn();
        })
        .catch(() => undefined);
    }
    const ev = tauriEvent();
    if (ev) {
      void ev
        .listen('tauri://drag-drop', (e) => onEvent(e.payload))
        .then((fn) => {
          if (alive) unlisteners.push(fn);
          else fn();
        })
        .catch(() => undefined);
    }
    return () => {
      alive = false;
      unlisteners.forEach((fn) => fn());
      unlisteners = [];
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pickDirectory = async () => {
    setError(null);
    // 第 1 层：直连插件命令（最低层，返回值不经全局桥）
    try {
      const r1 = await directDialogOpen({ directory: true, title: '选择经营数据目录' });
      if (typeof r1 === 'string' && r1) {
        props.onDiag?.(`目录选择（直连）：${r1}`);
        props.onPath(r1);
        return;
      }
      if (r1 === null) {
        props.onDiag?.('目录选择（直连）：返回 null（用户取消或桥接空返回）');
        return;
      }
    } catch (err) {
      props.onDiag?.(`目录选择（直连）失败：${err instanceof Error ? err.message : String(err)}`);
    }
    // 第 2 层：全局桥 dialog.open
    const dialog = tauriDialog();
    if (dialog) {
      try {
        const dir = await dialog.open({ directory: true, title: '选择经营数据目录' });
        props.onDiag?.(`目录选择（全局桥）：${JSON.stringify(dir)}`);
        if (typeof dir === 'string' && dir) {
          props.onPath(dir);
          return;
        }
      } catch (err) {
        props.onDiag?.(`目录选择（全局桥）失败：${err instanceof Error ? err.message : String(err)}`);
      }
    }
    // 第 3 层：手动路径输入
    setManualMode(true);
    setError('系统选择器返回值未达界面（见诊断行）；请在下方手动输入目录路径后点「解析此目录」。');
  };

  const env = envCheck();

  return (
    <div className="panel import-panel" data-testid="import-panel">
      <div className="panel-head">
        <span className="panel-title">导入经营数据</span>
        <span className="panel-note" data-testid="import-env">
          环境：__TAURI__ {env.tauri ? '✓' : '✗'} · 目录选择器 {env.dialog ? '✓' : '✗'} · 拖放 {env.dragDrop ? '✓' : '✗'}
        </span>
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
      {manualMode || !env.dialog ? (
        <div className="manual-path" data-testid="manual-path-row">
          <input
            className="select"
            value={manual}
            placeholder="输入数据目录绝对路径，例如 /Users/你/Documents/经营复盘/2026-10"
            onChange={(e) => setManual(e.target.value)}
            data-testid="manual-path-input"
          />
          <button
            type="button"
            className="primary"
            disabled={props.busy || !manual.trim()}
            onClick={() => manual.trim() && props.onPath(manual.trim())}
            data-testid="manual-path-scan"
          >
            {props.busy ? '解析中…' : '解析此目录'}
          </button>
        </div>
      ) : null}
      {error ? <div className="onboarding-error">{error}</div> : null}
    </div>
  );
}
