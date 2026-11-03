/**
 * 历史查询屏（S7）：期间列表、版本与任务快照查看。
 * 历史查看标识为 run_id + analysis_revision_no + task_snapshot_no；
 * 任务后续推进不改变已选快照。
 */

import { useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import {
  TASK_STATUS_LABELS,
  groupDecimal,
  type HistoryDetailView,
} from '../api';

const CADENCE_LABELS: Record<string, string> = {
  day: '日度',
  week: '周度',
  month: '月度',
  quarter: '季度',
};

export function HistoryScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const history = app.data.history;
  const [detail, setDetail] = useState<HistoryDetailView | null>(null);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const view = async (runId: string, revision?: number) => {
    if (!app.client || !app.activeWorkspace) {
      setNotice('请先创建企业档案（右上角「＋ 新建档案」）。');
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const d = await app.client.historyShow(app.activeWorkspace!.workspace_id, runId, revision);
      setDetail(d);
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
          <div className="eyebrow">04 · local history</div>
          <h2 className="screen-title">历史查询</h2>
          <div className="sub">
            {app.activeWorkspace?.name} · 按企业、核算周期和期间检索已保存的分析结果。
          </div>
        </div>
      </div>

      {notice ? (
        <div className="notice workspace-empty">
          <span>{notice}</span>
        </div>
      ) : null}

      {!hasData || history.length === 0 ? (
        <EmptyState
          title="尚无已保存期间"
          hint="完成数据导入与分析后，期间结果按版本保存在当前企业档案内。"
        />
      ) : (
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">已保存期间</span>
            <span className="panel-note">
              同一企业 + 周期 + 期间重做会更新当前版本并保留快照；任务进度事件不丢失
            </span>
          </div>
          <div className="table-wrap">
            <table className="data-table" data-testid="history-table">
              <thead>
                <tr>
                  <th>期间</th>
                  <th>核算周期</th>
                  <th>当前版本</th>
                  <th>任务快照</th>
                  <th>更新于</th>
                  <th>结果</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                {history.map((h) => (
                  <tr key={h.run_id}>
                    <td>
                      {h.period.from} 至 {h.period.to}
                    </td>
                    <td>
                      {CADENCE_LABELS[h.cadence]} · 策略 v{h.policy_version}
                    </td>
                    <td>revision {h.current_revision_no}</td>
                    <td>#{h.task_snapshot_no}</td>
                    <td>{h.updated_at.replace('T', ' ').slice(0, 16)}</td>
                    <td>
                      {h.summary.metric_count} 指标 · {h.summary.task_count} 任务 ·{' '}
                      {h.summary.chart_count} 图表
                    </td>
                    <td>
                      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
                        <button
                          type="button"
                          className="outline"
                          onClick={() => void view(h.run_id)}
                          disabled={busy}
                          data-testid={`history-view-${h.run_id.slice(0, 8)}`}
                        >
                          查看当前
                        </button>
                        {Array.from({ length: h.current_revision_no }, (_, i) => i + 1)
                          .filter((r) => r !== h.current_revision_no)
                          .map((r) => (
                            <button
                              key={r}
                              type="button"
                              className="outline"
                              onClick={() => void view(h.run_id, r)}
                              disabled={busy}
                            >
                              r{r}
                            </button>
                          ))}
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      )}

      {detail ? (
        <div className="panel" style={{ marginTop: 14 }} data-testid="history-detail">
          <div className="panel-head">
            <span className="panel-title">
              快照 · run {detail.run_id.slice(0, 8)}… · 分析 revision{' '}
              {detail.analysis_revision_no} · 任务快照 #{detail.task_snapshot_no}
            </span>
            <span className="panel-note">
              捕获于 {detail.captured_at.replace('T', ' ').slice(0, 16)} UTC ·
              后续任务推进不改变本快照
            </span>
          </div>
          <div className="table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th>指标</th>
                  <th>目标</th>
                  <th>实绩</th>
                  <th>差异</th>
                </tr>
              </thead>
              <tbody>
                {detail.metrics.map((m) => (
                  <tr key={m.metric_id}>
                    <td>{m.metric_label}</td>
                    <td>{m.target_value != null ? groupDecimal(m.target_value) : '—'}</td>
                    <td>{m.actual_value != null ? groupDecimal(m.actual_value) : '—'}</td>
                    <td>{m.delta != null ? groupDecimal(m.delta) : '—'}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="panel-head" style={{ borderTop: '1px solid var(--line)' }}>
            <span className="panel-title">任务快照</span>
          </div>
          <div className="table-wrap">
            <table className="data-table">
              <thead>
                <tr>
                  <th>任务</th>
                  <th>状态</th>
                  <th>进度</th>
                  <th>任务版本</th>
                </tr>
              </thead>
              <tbody>
                {detail.tasks.map((t) => (
                  <tr key={t.task_id}>
                    <td>{t.title}</td>
                    <td>
                      <Tag
                        tone={
                          t.status === 'blocked'
                            ? 'red'
                            : t.status === 'completed'
                              ? 'green'
                              : 'gold'
                        }
                      >
                        {TASK_STATUS_LABELS[t.status]}
                      </Tag>
                    </td>
                    <td>{groupDecimal(t.progress)}%</td>
                    <td>r{t.task_revision_no}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      ) : null}

      <div className="root-card" style={{ maxWidth: 940 }}>
        <h4>周期变更记录</h4>
        <p>
          企业切换核算周期后，新分析采用新策略版本；已有结果仍保留，
          历史期间可按旧版本查看，显式重算才会产生新 revision。
        </p>
      </div>
    </div>
  );
}
