import { useEffect, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { ImportDropzone } from '../components/ImportDropzone';
import { Tag } from '../components/bits';
import { FIXTURE_DEMO_ANNOTATION, type FieldMappingRow } from '../api';

const SOURCE_PROFILE_LABELS: Record<string, string> = {
  generic: '未指定行业模板',
  manufacturing_demo: '制造业示例模板',
  manufacturing_erp_v1: '制造业经营数据模板',
  xsd_trade_v1: '贸易/制造业单据模板（当前示例）',
  service_demo: '服务业示例模板',
};

const COMMON_METRIC_LABELS = '不含税销售额、订单成本、订单毛利、订单毛利率、入库金额、收款金额、付款金额、开票金额、经营费用、人工费、附加值（扣人工前）、经营收益（扣人工后）、有效工时、单位时间附加值、回款率和付款率';

function sourceProfileLabel(profileId?: string | null) {
  if (!profileId) return SOURCE_PROFILE_LABELS.generic;
  return SOURCE_PROFILE_LABELS[profileId] ?? '当前行业模板';
}

function humanizeSourceNote(note: string | null, sheetCount: number) {
  if (!note) return `${sheetCount} 个工作表已读取，等待字段确认`;
  return note
    .replace(/generic_table/gi, '未归类业务表')
    .replace(/generic/gi, '未指定模板')
    .replace(/table/gi, '业务表');
}

/** 数据源屏：目录扫描结果与字段映射（只读呈现核心返回）。 */
export function SourceScreen() {
  const app = useApp();
  // 当前提供的示例数据使用 xsd_trade_v1；未指定行业模板仍可由用户显式选择。
  const [sourceProfile, setSourceProfile] = useState('xsd_trade_v1');
  const [mappings, setMappings] = useState<FieldMappingRow[]>([]);
  const [scanBusy, setScanBusy] = useState(false);
  const [scanNote, setScanNote] = useState<string | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [diag, setDiag] = useState<string[]>([]);
  const lastDir = app.sourceDirectory;
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

  // 重新进入页面时沿用当前扫描绑定的模板，避免下拉框显示默认值而覆盖实际口径。
  useEffect(() => {
    const profileId = app.data.scan?.source_profile?.profile_id;
    if (profileId) setSourceProfile(profileId);
  }, [app.data.scan?.source_profile?.profile_id]);

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
      const r = await invoke('template_download', {
        outPath: '/tmp/dsheng-selftest-source-template.zip',
        package: 'source',
        profile: sourceProfile === 'generic' ? null : sourceProfile,
        contract: null,
      });
      pushDiag(`template_download → ${JSON.stringify(r).slice(0, 100)}`);
    } catch (e) {
      pushDiag(`template_download 失败: ${String(e).slice(0, 120)}`);
    }
    if (app.activeWorkspaceId) {

      const withTimeout = <T,>(p: Promise<T>, ms: number, tag: string) =>
        Promise.race([
          p,
          new Promise<never>((_, rej) => setTimeout(() => rej(new Error(`${tag} 超时 ${ms}ms`)), ms)),
        ]);
      try {
        const r = (await withTimeout(
          invoke('scan', {
            workspaceId: app.activeWorkspaceId,
            directoryLabel: '/tmp',
            sourceProfile: sourceProfile === 'generic' ? null : sourceProfile,
            generic: sourceProfile === 'generic',
          }),
          8000,
          'scan(/tmp)',
        )) as { ok?: boolean; error?: unknown };
        pushDiag(`scan(/tmp) → ok=${r?.ok}${r?.ok ? '' : ' ' + JSON.stringify(r?.error).slice(0, 90)}`);
      } catch (e) {
        pushDiag(`scan(/tmp) 失败/超时: ${String(e).slice(0, 120)}`);
      }
      try {
        const r = (await withTimeout(
          invoke('scan', {
            workspaceId: app.activeWorkspaceId,
            directoryLabel: '/tmp',
            sourceProfile: sourceProfile === 'generic' ? null : sourceProfile,
            generic: sourceProfile === 'generic',
          }),
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
        app.client.scan(
          app.activeWorkspaceId,
          path,
          sourceProfile === 'generic' ? undefined : sourceProfile,
          sourceProfile === 'generic',
        ),
        new Promise<never>((_, rej) =>
          setTimeout(() => rej(new Error('核心未在 15 秒内返回（scan 挂起）')), 15000),
        ),
      ]);
      app.rememberSourceDirectory(path);
      pushDiag(`scan 返回：${summary.files.length} 文件 / ${summary.total_rows} 行`);
      setScanNote(`解析完成：${summary.files.length} 个文件 / ${summary.total_rows} 行。待核对的表可在下方确认；完成后到「经营分析」运行分析。`);
      // 刷新时显式携带真实摘要（闭包竞态修复：不再依赖先 setState 再读旧 state）
      await app.reloadWorkspaceData({ keepScan: summary });
    } catch (err) {
      setScanNote(null);
      setScanError(err instanceof Error ? err.message : String(err));
    } finally {
      setScanBusy(false);
    }
  };

  /** 一键注册常用指标并重扫（对齐 CLI 验证流；让推移表按指标列识别）。 */
  const registerAndRescan = async () => {
    if (!app.client || !app.activeWorkspaceId) {
      setScanError('请先创建企业档案。');
      return;
    }
    setScanBusy(true);
    setScanError(null);
    try {
      const r = await app.client.metricRegisterDefaults(app.activeWorkspaceId);
      pushDiag(`指标注册：新增 ${r.registered}，已存在跳过 ${(r.skipped_already_present ?? []).length}`);
      if (lastDir) {
        await runScan(lastDir, '注册后自动重扫（推移表将按指标列识别）');
      } else {
        setScanNote('指标已注册；请重新选择目录扫描一次，让推移表按指标列识别。');
      }
    } catch (err) {
      setScanError(err instanceof Error ? err.message : String(err));
    } finally {
      setScanBusy(false);
    }
  };

  /** generic source 必须先明确确认字段映射，确认记录绑定本次目录与 profile hash。 */
  const confirmGenericMapping = async () => {
    if (!app.client || !app.activeWorkspaceId || !lastDir || !scan?.source_profile) {
      setScanError('缺少数据目录或输入模板，无法确认字段。');
      return;
    }
    setScanBusy(true);
    setScanError(null);
    try {
      await app.client.sourceMappingSet(
        app.activeWorkspaceId,
        lastDir,
        scan.source_profile.hash,
        [],
      );
      const summary = await app.client.scan(
        app.activeWorkspaceId,
        lastDir,
        scan.source_profile.mode === 'generic'
          ? undefined
          : (scan.source_profile.profile_ref ?? scan.source_profile.profile_id),
        scan.source_profile.mode === 'generic',
      );
      await app.reloadWorkspaceData({ keepScan: summary });
      setScanNote('字段映射已确认；现在可以到「经营分析」运行本地分析。');
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
      out = await save({ title: '保存原始数据模板包', defaultPath: 'source-template-v1.zip' });
    } else {
      out = window.prompt('保存原始数据模板包的路径（演示/测试通道）', '/tmp/source-template-v1.zip');
    }
    if (!out) return;
    setScanBusy(true);
    setScanError(null);
    try {
      const r = await app.client.templateDownload(
        out,
        'source',
        sourceProfile === 'generic' ? undefined : sourceProfile,
      );
      setScanNote(`原始数据模板已写入 ${r.out}（${r.bytes} 字节）；两表会由原始数据生成，不作为导入文件。`);
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
          <div className="eyebrow">01 · 数据源</div>
          <h2 className="screen-title">数据源</h2>
          <div className="sub" data-testid="source-subtitle">
            {hasData
              ? `${workspaceName} · 读取本地目录，核对期间、字段与数据口径。`
              : `${workspaceName} · 尚未选择数据目录，请先导入文件并确认字段映射。`}
          </div>
        </div>
        <div className="head-controls">
          <button type="button" className="outline" onClick={() => void downloadTemplate()} disabled={scanBusy}>下载原始数据模板</button>
        </div>
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
        <div className="panel source-intake-panel">
          <div className="panel-head">
            <span className="panel-title">导入原始经营数据</span>
            <span className="panel-note" data-testid="source-scan-status">{scanBusy ? '正在扫描…' : scanError ? '本次扫描失败' : scan ? '已扫描 · 本地解析' : '未扫描'}</span>
          </div>
          <div className="source-profile-row">
            <label className="period-field">原始数据模板
              <select
                className="select"
                aria-label="原始数据模板"
                value={sourceProfile}
                disabled={scanBusy}
                onChange={(event) => setSourceProfile(event.target.value)}
              >
                <option value="generic">未指定行业模板（需确认字段）</option>
                <option value="xsd_trade_v1">贸易/制造业单据模板（当前示例）</option>
                <option value="manufacturing_demo">制造业示例模板</option>
                <option value="service_demo">服务业示例模板</option>
              </select>
            </label>
            <span className="panel-note">扫描会记录模板版本；核算表、推移表只能作为生成结果查看。</span>
          </div>
          <p className="panel-note source-profile-metrics">常用指标：不含税销售额、订单成本、订单毛利率、经营费用、人工费、附加值（扣人工前）、经营收益（扣人工后）、有效工时和回款率。</p>
          <ImportDropzone busy={scanBusy} onPath={(path, note) => void runScan(path, note)} onDiag={pushDiag} />
          <div className="source-directory-summary">
            <div data-testid="source-directory">{lastDir ? `有效数据目录：${lastDir}` : '尚未成功扫描数据目录'}</div>
            {lastDir ? <button type="button" className="outline" disabled={scanBusy} onClick={() => void runScan(lastDir)}>重新扫描</button> : null}
            {scan ? <p>数据覆盖：{scan.coverage.from} 至 {scan.coverage.to} · {scan.files.length} 个文件 · {scan.total_rows} 行 · 模板：{sourceProfileLabel(scan.source_profile?.profile_id)}{scanError ? '（上次成功扫描）' : ''}</p> : <p>选择目录或拖入文件夹后，识别结果会显示在这里。</p>}
          </div>
          {scanNote ? <div className="notice" data-testid="scan-note"><b>导入说明</b><span>{scanNote}</span></div> : null}
          {scanError ? <div className="notice" role="alert" data-testid="scan-error"><b>扫描失败</b><span>{scanError}</span></div> : null}
          {scan ? <>
            <div className="panel-head"><span className="panel-title">已识别文件</span><span className="panel-note">{scan.files.length} 个文件 · {scan.total_rows} 行</span></div>
            <div className="file-list">
              {scan.files.length === 0 ? <p className="panel-note">此目录未识别到可导入的文件。</p> : scan.files.map((f) => (
                <div className="file" key={f.file_id}>
                  <div><div className="file-name">{f.file_label}</div><div className="file-meta">{humanizeSourceNote(f.note, f.sheet_count)} · {f.row_count} 行</div></div>
                  {f.mapping_status === 'mapped' ? <Tag tone="green">映射完成</Tag> : <Tag tone="gold">待核对</Tag>}
                </div>
              ))}
            </div>
          </> : null}
          <details className="source-technical-details">
            <summary>连接自检（需要排查问题时使用）</summary>
            <button type="button" className="outline" onClick={() => void runSelfTest()} data-testid="self-test-button">运行桥接自检</button>
            <div data-testid="diag-lines">{diag.map((line, i) => <div key={i}>{line}</div>)}</div>
          </details>
        </div>
      ) : null}

      {scan && (scan.files ?? []).some((f) => f.mapping_status === 'pending_review') ? (() => {
        const pendingFiles = (scan.files ?? []).filter((f) => f.mapping_status === 'pending_review');
        const previewFiles = pendingFiles.slice(0, 3).map((f) => f.file_label).join('、');
        const remainingCount = pendingFiles.length - Math.min(pendingFiles.length, 3);
        return (
        <div className="notice pending-review-guide" data-testid="pending-review-guide">
          <b>需要确认字段</b>
          <div className="pending-review-content">
            <p>
              {pendingFiles.length} 个文件中有部分工作表尚未归入常用指标，原始数据已保留。
              {previewFiles ? ` 当前包括：${previewFiles}${remainingCount > 0 ? ` 等 ${remainingCount} 个文件` : ''}。` : ''}
            </p>
            <p className="pending-review-metrics">常用指标包括：{COMMON_METRIC_LABELS}。确认后会重新扫描并生成可分析期间。</p>
            <details className="pending-review-files">
              <summary>查看待确认文件（{pendingFiles.length}）</summary>
              <ul>{pendingFiles.map((f) => <li key={f.file_id}>{f.file_label} · {f.row_count} 行</li>)}</ul>
            </details>
          </div>
          <div className="pending-review-actions">
            {scan.source_profile?.mode === 'generic' ? <button
              type="button"
              className="primary"
              disabled={scanBusy || !lastDir}
              onClick={() => void confirmGenericMapping()}
              data-testid="confirm-source-mapping-button"
            >
              {scanBusy ? '处理中…' : '确认当前字段识别'}
            </button> : null}
            <button
              type="button"
              className="primary"
              disabled={scanBusy}
              onClick={() => void registerAndRescan()}
              data-testid="register-metrics-button"
            >
              {scanBusy ? '处理中…' : '一键注册常用指标并重扫'}
            </button>
          </div>
        </div>
        );
      })() : null}

      {!hasData ? (
        <EmptyState
          title="此档案尚未导入数据"
          hint="用上方任一方式导入后，这里会显示识别到的文件与字段映射。其他企业的本地数据不会复制到这里。"
        />
      ) : (
        <>
          <div className="panel" style={{ marginTop: 14 }}>
            <div className="panel-head">
              <span className="panel-title">字段映射</span>
              <span className="panel-note">所选分析期间的字段识别结果</span>
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
                缺失数据保持待补，不会按 0 参与计算。修改数据口径后，运行本地分析才会形成新版本。
              </div>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
