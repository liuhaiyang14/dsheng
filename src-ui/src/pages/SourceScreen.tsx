import { useEffect, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { ImportDropzone } from '../components/ImportDropzone';
import { Tag } from '../components/bits';
import { FIXTURE_DEMO_ANNOTATION, type FieldMappingRow } from '../api';

/** 数据源屏：目录扫描结果与字段映射（只读呈现核心返回）。 */
export function SourceScreen() {
  const app = useApp();
  const [mappings, setMappings] = useState<FieldMappingRow[]>([]);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanNote, setScanNote] = useState<string | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [diag, setDiag] = useState<string[]>([]);
  const hasData = workspaceHasData(app.data);
  const workspaceName = app.activeWorkspace?.name ?? '当前企业';

  useEffect(() => {
    let cancelled = false;
    if (!app.client || !app.activeWorkspaceId || !app.data.currentDetail) {
      setMappings([]);
      return;
    }
    app.client
      .fieldMappings(app.activeWorkspaceId, app.data.currentDetail.run_id)
      .then((rows) => {
        if (!cancelled) setMappings(rows);
      })
      .catch(() => {
        if (!cancelled) setMappings([]);
      });
    return () => {
      cancelled = true;
    };
    // 仅在切换档案/重载数据后刷新映射。
  }, [app.client, app.activeWorkspaceId, app.data.currentDetail]);

  const scan = app.data.scan;

  const pushDiag = (line: string) =>
    setDiag((prev) => [...prev.slice(-10), `${new Date().toLocaleTimeString()} ${line}`]);

  /** 自检：逐项直测核心命令并显示原始返回（一次性定位桥接问题）。 */
  const runSelfTest = async () => {
    setDiag([]);
    pushDiag('自检开始…');
    const w = window as unknown as {
      __TAURI__?: { core?: { invoke?: (cmd: string, args?: unknown) => Promise<unknown> } };
    };
    const invoke = w.__TAURI__?.core?.invoke;
    if (!invoke) {
      pushDiag('✗ __TAURI__.core.invoke 不存在（未连接核心）');
      return;
    }
    pushDiag('✓ __TAURI__.core.invoke 存在');
    try {
      const r = await invoke('ping');
      pushDiag(`ping → ${JSON.stringify(r).slice(0, 80)}`);
    } catch (e) {
      pushDiag(`ping 失败: ${e instanceof Error ? e.message : String(e)}`);
    }
    try {
      const r = await invoke('workspace_list');
      pushDiag(`workspace_list → ok=${JSON.stringify((r as { ok?: boolean })?.ok)}`);
    } catch (e) {
      pushDiag(`workspace_list 失败: ${String(e).slice(0, 100)}`);
    }
    try {
      const r = await invoke('template_download', { outPath: '/tmp/dsheng-selftest-template.zip' });
      pushDiag(`template_download → ${JSON.stringify(r).slice(0, 100)}`);
    } catch (e) {
      pushDiag(`template_download 失败: ${String(e).slice(0, 120)}`);
    }
    if (app.activeWorkspaceId) {
      // 探测目录用 /tmp 快路径即可定位层级问题；真实目录由用户经选择器/手动输入传入
      const homeDir = '';
      const withTimeout = <T,>(p: Promise<T>, ms: number, tag: string) =>
        Promise.race([
          p,
          new Promise<never>((_, rej) => setTimeout(() => rej(new Error(`${tag} 超时 ${ms}ms`)), ms)),
        ]);
      try {
        const r = (await withTimeout(
          invoke('scan', { workspaceId: app.activeWorkspaceId, directoryLabel: '/tmp' }),
          8000,
          'scan(/tmp)',
        )) as { ok?: boolean; error?: unknown };
        pushDiag(`scan(/tmp) → ok=${r?.ok}${r?.ok ? '' : ' ' + JSON.stringify(r?.error).slice(0, 90)}`);
      } catch (e) {
        pushDiag(`scan(/tmp) 失败/超时: ${String(e).slice(0, 120)}`);
      }
      try {
        const r = (await withTimeout(
          invoke('scan', { workspaceId: app.activeWorkspaceId, directoryLabel: `${homeDir}/Desktop/ds` }),
          15000,
          'scan(ds)',
        )) as { ok?: boolean; data?: { files?: unknown[] }; error?: unknown };
        pushDiag(
          `scan(ds) → ok=${r?.ok}${r?.ok ? ` files=${(r?.data?.files ?? []).length}` : ' ' + JSON.stringify(r?.error).slice(0, 90)}`,
        );
      } catch (e) {
        pushDiag(`scan(ds) 失败/超时: ${String(e).slice(0, 120)}`);
      }
    }
    pushDiag('自检完成（以上为原始结果）');
  };

  /** 解析目录（本地扫描，无 Provider 调用）。 */
  const runScan = async (path: string, note?: string) => {
    if (!app.client || !app.activeWorkspaceId) {
      setScanError('请先创建企业档案（右上角「＋ 新建档案」）。');
      return;
    }
    setScanBusy(true);
    setScanError(null);
    setScanNote(note ?? `已选择：${path}；本地解析中…`);
    pushDiag(`scan 调用：workspace=${app.activeWorkspaceId} dir=${path}`);
    try {
      const summary = await Promise.race([
        app.client.scan(app.activeWorkspaceId, path),
        new Promise<never>((_, rej) =>
          setTimeout(() => rej(new Error('核心未在 15 秒内返回（scan 挂起）')), 15000),
        ),
      ]);
      pushDiag(`scan 返回：${summary.files.length} 文件 / ${summary.total_rows} 行`);
      await app.reloadWorkspaceData();
    } catch (err) {
      setScanError(err instanceof Error ? err.message : String(err));
    } finally {
      setScanBusy(false);
    }
  };

  /** 下载空白导入模板包（本地生成，无网络）。 */
  const downloadTemplate = async () => {
    if (!app.client) {
      setScanError('未连接核心，无法生成模板。');
      return;
    }
    let out: string | null = null;
    const w = window as unknown as {
      __TAURI__?: { dialog?: { save?: (o: { title?: string; defaultPath?: string }) => Promise<string | null> } };
    };
    const save = w.__TAURI__?.dialog?.save;
    if (typeof save === 'function') {
      out = await save({ title: '保存导入模板包', defaultPath: 'import-template-v1.zip' });
    } else {
      out = window.prompt('保存模板包的路径（演示/测试通道）', '/tmp/import-template-v1.zip');
    }
    if (!out) return;
    setScanBusy(true);
    setScanError(null);
    try {
      const r = await app.client.templateDownload(out);
      setScanNote(`模板包已写入 ${r.out}（${r.bytes} 字节）；含核算表/推移表/任务单三份空白工作簿与字段说明。`);
    } catch (err) {
      setScanError(err instanceof Error ? err.message : String(err));
    } finally {
      setScanBusy(false);
    }
  };

  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">01 · source intake</div>
          <h2 className="screen-title">数据源</h2>
          <div className="sub" data-testid="source-subtitle">
            {hasData
              ? `${workspaceName} · 读取本地目录，核对期间、字段与数据口径。`
              : `${workspaceName} · 尚未选择数据目录，请先导入文件并确认字段映射。`}
          </div>
        </div>
        {hasData ? (
          <div className="head-controls">
            <button
              type="button"
              className="outline"
              onClick={() => void downloadTemplate()}
              disabled={scanBusy}
            >
              下载导入模板
            </button>
            <button
              type="button"
              className="primary"
              onClick={() => void app.reloadWorkspaceData()}
            >
              重新扫描
            </button>
          </div>
        ) : null}
      </div>

      {app.mode === 'fixture' ? (
        <div className="notice">
          <b>演示数据说明</b>
          <span>
            以下企业名、目录、文件、金额和人员岗位均为{FIXTURE_DEMO_ANNOTATION}
            ，用于展示完整流程，不来自真实业务文件。正式版会在调用模型前展示将发送的文件、工作表、行数与内容范围，并要求确认。
          </span>
        </div>
      ) : null}

      {app.activeWorkspace ? (
        <>
          <ImportDropzone
            busy={scanBusy}
            onPath={(path, note) => void runScan(path, note)}
            onDiag={pushDiag}
          />
          <div style={{ display: 'flex', gap: 8, alignItems: 'center', margin: '8px 0' }}>
            <button type="button" className="outline" onClick={() => void runSelfTest()} data-testid="self-test-button">
              桥接自检
            </button>
            {diag.length > 0 ? (
              <div
                style={{ fontSize: 11, color: 'var(--muted)', lineHeight: 1.9, flex: 1 }}
                data-testid="diag-lines"
              >
                {diag.map((l, i) => (
                  <div key={i}>{l}</div>
                ))}
              </div>
            ) : null}
          </div>
          {scanNote ? (
            <div className="notice" data-testid="scan-note" style={{ marginTop: 12 }}>
              <b>导入说明</b>
              <span>{scanNote}</span>
            </div>
          ) : null}
          {scanError ? (
            <div className="notice" data-testid="scan-error" style={{ marginTop: 12 }}>
              <b>扫描失败</b>
              <span>{scanError}</span>
            </div>
          ) : null}
        </>
      ) : null}

      {!hasData ? (
        <EmptyState
          title="此档案尚未导入数据"
          hint="用上方任一方式导入后，这里会显示识别到的文件与字段映射。其他企业的本地数据不会复制到这里。"
        />
      ) : (
        <>
          <div className="source-grid">
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">本地目录</span>
                <span className="panel-note">
                  <span className="status-dot" />
                  已扫描 · 只读
                </span>
              </div>
              <div className="dropzone">
                <div>
                  <div className="folder">⌂</div>
                  <p>{scan?.directory_label}</p>
                  <small>
                    核算周期：{scan?.coverage.granularity === 'day' ? '日度' : scan?.coverage.granularity} ·
                    数据覆盖：{scan?.coverage.from} 至 {scan?.coverage.to}
                  </small>
                </div>
              </div>
              <div className="path">
                {app.mode === 'fixture'
                  ? `示例路径：~/Documents/经营复盘/${workspaceName}/2026-10（演示值）`
                  : '文件保留在原位置，不复制进数据库；重扫使用当前目录内容。'}
              </div>
            </div>
            <div className="panel">
              <div className="panel-head">
                <span className="panel-title">已识别文件</span>
                <span className="panel-note">
                  {scan?.files.length ?? 0} 个文件 · {scan?.total_rows ?? 0} 行
                </span>
              </div>
              <div className="file-list">
                {(scan?.files ?? []).map((f) => (
                  <div className="file" key={f.file_id}>
                    <div>
                      <div className="file-name">{f.file_label}</div>
                      <div className="file-meta">
                        {f.note} · {f.row_count} 行
                      </div>
                    </div>
                    {f.mapping_status === 'mapped' ? (
                      <Tag tone="green">映射完成</Tag>
                    ) : (
                      <Tag tone="gold">待核对</Tag>
                    )}
                  </div>
                ))}
              </div>
            </div>
          </div>

          <div className="panel" style={{ marginTop: 14 }}>
            <div className="panel-head">
              <span className="panel-title">字段映射</span>
              <span className="panel-note">自动识别 · 可手动修正 · 由核心保存版本化草稿</span>
            </div>
            <div className="table-wrap">
              <table className="data-table">
                <thead>
                  <tr>
                    <th>工具字段</th>
                    <th>来源表 / 字段</th>
                    <th>识别结果</th>
                    <th>用途</th>
                  </tr>
                </thead>
                <tbody>
                  {mappings.map((m) => (
                    <tr key={m.tool_field}>
                      <td>{m.tool_field}</td>
                      <td>{m.source_label}</td>
                      <td>
                        {m.recognized.includes('已匹配') ? (
                          <Tag tone="green">{m.recognized}</Tag>
                        ) : (
                          <Tag tone="gold">{m.recognized}</Tag>
                        )}
                      </td>
                      <td>{m.usage}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              <div className="table-foot">
                缺失工时为「待补」，不会按 0 参与计算。映射修改走核心 mapping set（版本化草稿），重算才进入新分析版本。
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
