import { useEffect, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { FIXTURE_DEMO_ANNOTATION, type FieldMappingRow } from '../api';

/** 数据源屏：目录扫描结果与字段映射（只读呈现核心返回）。 */
export function SourceScreen() {
  const app = useApp();
  const [mappings, setMappings] = useState<FieldMappingRow[]>([]);
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
              onClick={() => {
                window.alert(
                  '演示下载占位：空白模板包由本地核心导出（template download）；当前未连接核心时不产生文件。',
                );
              }}
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

      {!hasData ? (
        <EmptyState
          title="此档案尚未导入数据"
          hint="请先为该企业指定本地目录，再完成字段映射。其他企业的本地数据不会复制到这里。"
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
                示例路径：~/Documents/经营复盘/{workspaceName}/2026-10（演示值）
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
