/**
 * 经营问题诊断屏（S5）。
 *
 * 门禁：进入页面 / 切换框架 / 选择指标 / 选择报告等任何事件都不触发生成；
 * 只有用户显式点击「生成诊断报告」才创建预览，预览确认后才执行。
 * 诊断不创建、不修改任务。
 */

import { useCallback, useEffect, useMemo, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { GenerationProgressPanel } from '../components/GenerationProgressPanel';
import { PreviewModal } from '../components/PreviewModal';
import {
  CONFIDENCE_LABELS,
  DIAGNOSIS_FRAMEWORK_OPTIONS,
  FACT_STATUS_LABELS,
  RESOLUTION_REQUIREMENT_LABELS,
  groupDecimal,
  type ConfirmationPreview,
  type DiagnosisFramework,
  type DiagnosisReport,
  type DiagnosisFinding,
} from '../api';

interface PendingPreview {
  preview: ConfirmationPreview;
  kind: 'generate' | 'continue';
  label: string;
}

const STATUS_LABELS: Record<string, string> = {
  candidate: '候选结论',
  needs_confirmation: '待核实',
  resolved: '已核实',
};

function FindingBlock({ finding }: { finding: DiagnosisFinding }) {
  const title =
    'level' in finding
      ? `第 ${finding.level} 层追问`
      : `${finding.dimension}${'kind' in finding ? (finding.kind === 'proposal' ? '（行动建议）' : '（观察）') : ''}`;
  return (
    <div className="root-step">
      <span className="step-num">·</span>
      <strong>{title} · {finding.question}</strong>
      <p>答复：{finding.answer}</p>
      {'validation_question' in finding && finding.validation_question ? (
        <p>待验证：{finding.validation_question}</p>
      ) : null}
      {'missing_information' in finding && finding.missing_information.length > 0 ? (
        <p>信息缺口：{finding.missing_information.join('；')}</p>
      ) : null}
      <div className="evidence">
        事实状态：{FACT_STATUS_LABELS[finding.fact_status]} · 置信度：
        {CONFIDENCE_LABELS[finding.confidence]} · 状态：
        {STATUS_LABELS[finding.status] ?? finding.status}
        {finding.evidence_ids.length > 0
          ? ` · 证据：${finding.evidence_ids.join(', ')}`
          : ' · 无来源证据（需人工核实）'}
      </div>
    </div>
  );
}

export function DiagnosisScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const detail = app.data.currentDetail;

  const [reports, setReports] = useState<DiagnosisReport[]>([]);
  const [selectedReportId, setSelectedReportId] = useState<string | null>(null);
  const [framework, setFramework] = useState<DiagnosisFramework>('5why');
  const [problem, setProblem] = useState('');
  const [supplement, setSupplement] = useState('');
  const [pending, setPending] = useState<PendingPreview | null>(null);
  const [busy, setBusy] = useState(false);
  /** confirm 发送进行中（进度面板渲染窗口；终态或异常即关闭）。 */
  const [sending, setSending] = useState(false);
  const [stoppedLocal, setStoppedLocal] = useState<Set<string>>(new Set());
  const [notice, setNotice] = useState<string | null>(null);

  const runId = detail?.run_id ?? null;

  // 进入页面：只读加载报告列表；绝不生成。
  const loadReports = useCallback(async () => {
    if (!app.client || !app.activeWorkspaceId || !runId) {
      setReports([]);
      setSelectedReportId(null);
      return;
    }
    try {
      const list = await app.client.diagnosisList(app.activeWorkspaceId, runId);
      setReports(list);
      setSelectedReportId((prev) =>
        prev && list.some((r) => r.report_id === prev) ? prev : (list[0]?.report_id ?? null),
      );
    } catch {
      setReports([]);
    }
  }, [app.client, app.activeWorkspaceId, runId]);

  useEffect(() => {
    void loadReports();
  }, [loadReports]);

  const selected = useMemo(
    () => reports.find((r) => r.report_id === selectedReportId) ?? null,
    [reports, selectedReportId],
  );

  const activeMetric = detail?.metrics.find((m) => m.delta != null && m.target_value != null);
  const defaultProblem = useMemo(() => {
    if (!activeMetric) return '';
    // 默认问题表述：由指标与差异确定性拼接（呈现层组句，不做计算）。
    return `${activeMetric.metric_label}本期 ${groupDecimal(activeMetric.actual_value ?? '')} 低于目标 ${groupDecimal(activeMetric.target_value ?? '')}，差异 ${groupDecimal((activeMetric.delta ?? '').replace('-', ''))}${activeMetric.unit ?? ''}，需要定位原因`;
  }, [activeMetric]);

  const problemText = problem || defaultProblem;

  /** 显式按钮：生成诊断报告 → 预览。 */
  const requestGenerate = async () => {
    if (!app.client || !app.activeWorkspaceId || !runId) return;
    setBusy(true);
    setNotice(null);
    try {
      const preview = await app.client.diagnosisGeneratePreview({
        workspace_id: app.activeWorkspaceId,
        run_id: runId,
        framework,
        problem: problemText,
        focus_metric_id: activeMetric?.metric_id,
        // CLI 合同必填：当前 run 的来源证据（演示取指标关联单元格）
        evidence_ids: [`cell:demo:${activeMetric?.metric_key ?? '核算表'}:C7`],
      });
      setPending({ preview, kind: 'generate', label: '确认生成诊断报告（发送 1 次请求）' });
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  /** 显式按钮：继续分析 → 预览（含补充事实）。 */
  const requestContinue = async () => {
    if (!app.client || !app.activeWorkspaceId || !selected) return;
    if (supplement.trim().length === 0) {
      setNotice('请先补充一条现场事实或核实结果。');
      return;
    }
    setBusy(true);
    setNotice(null);
    try {
      const preview = await app.client.diagnosisContinuePreview({
        workspace_id: app.activeWorkspaceId,
        session_id: selected.generation_session_id,
        supplement_fact: supplement.trim(),
      });
      setPending({ preview, kind: 'continue', label: '确认继续分析（发送 1 次请求）' });
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const cancelGeneration = async (attemptId: string) => {
    if (!app.client || !app.activeWorkspaceId) return;
    try {
      await app.client.generationCancel(app.activeWorkspaceId, attemptId);
      setNotice('已请求取消本次生成（仅本地取消；不保证远端停止计费，迟到数据不提交）。');
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    }
  };

  const confirmPending = async () => {
    if (!app.client || !pending) return;
    setBusy(true);
    setSending(true);
    try {
      if (pending.kind === 'generate') {
        await app.client.diagnosisGenerateConfirm(pending.preview.confirmation_id);
        setNotice('诊断报告已生成并保存到当前企业档案。');
      } else {
        await app.client.diagnosisContinueConfirm(pending.preview.confirmation_id);
        setSupplement('');
        setNotice('补充事实已随本轮分析发送并记录为用户陈述（未核实）。');
      }
      await loadReports();
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    } finally {
      setSending(false);
      setBusy(false);
      setPending(null);
    }
  };

  const stopReport = () => {
    if (!selected) return;
    if (window.confirm(`停止并保留「${selected.scope.problem_statement.slice(0, 20)}…」？停止后仍可查看与下载。`)) {
      setStoppedLocal((prev) => new Set(prev).add(selected.report_id));
      setNotice('已停止并保留报告；现有内容仍可查看与下载。');
    }
  };

  const download = async (format: 'md' | 'json') => {
    if (!app.client || !app.activeWorkspaceId || !selected) return;
    try {
      const result = await app.client.diagnosisExport(
        app.activeWorkspaceId,
        selected.report_id,
        format,
        `诊断报告-${selected.report_id.slice(0, 8)}.${format}`,
      );
      setNotice(`已请求核心导出 ${format.toUpperCase()}：${result.out}`);
    } catch (err) {
      setNotice(err instanceof Error ? err.message : String(err));
    }
  };

  const isStopped = selected ? stoppedLocal.has(selected.report_id) || !selected.can_continue : false;

  if (!hasData || !detail || !runId) {
    return (
      <div>
        <div className="screen-head">
          <div>
            <div className="eyebrow">03 · issue diagnosis</div>
            <h2 className="screen-title">经营问题诊断</h2>
            <div className="sub">
              {app.activeWorkspace?.name} · 针对经营分析中的重点差异形成独立报告
            </div>
          </div>
        </div>
        <EmptyState
          title="尚无可用分析期间"
          hint="诊断报告基于已保存的经营分析差异生成；请先完成数据导入与分析。"
        />
      </div>
    );
  }

  return (
    <div>
      <div className="screen-head">
        <div>
          <div className="eyebrow">03 · issue diagnosis</div>
          <h2 className="screen-title">经营问题诊断</h2>
          <div className="sub">
            针对经营分析中的重点差异形成独立报告；报告不会自动生成或修改任务。
          </div>
        </div>
        <div className="head-controls">
          <select
            className="select"
            aria-label="诊断框架"
            value={framework}
            onChange={(e) => setFramework(e.target.value as DiagnosisFramework)}
            data-testid="framework-select"
          >
            {DIAGNOSIS_FRAMEWORK_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="primary"
            onClick={() => void requestGenerate()}
            disabled={busy}
            data-testid="generate-diagnosis-button"
          >
            生成诊断报告
          </button>
        </div>
      </div>

      <div className="notice">
        <b>框架说明</b>
        <span>
          {DIAGNOSIS_FRAMEWORK_OPTIONS.map((o) => `${o.label}：${o.purpose}`).join('；')}。
          切换框架只改变下一次生成的选择，不会自动生成；只有你点击「生成诊断报告」并确认预览才会发送。
        </span>
      </div>

      {notice ? (
        <div className="notice workspace-empty" data-testid="diagnosis-notice">
          <span>{notice}</span>
        </div>
      ) : null}

      <div className="panel" style={{ marginBottom: 14 }}>
        <div className="panel-head">
          <span className="panel-title">问题表述（可编辑）</span>
          <span className="panel-note">每份报告只锁定一个主问题，避免续问范围漂移</span>
        </div>
        <div className="panel-body">
          <textarea
            className="input"
            aria-label="问题表述"
            placeholder={defaultProblem || '描述需要诊断的经营问题'}
            value={problem}
            onChange={(e) => setProblem(e.target.value)}
            data-testid="problem-input"
          />
          <div className="panel-note" style={{ marginTop: 6 }}>
            当前将使用：{problemText || '（无，请填写）'}
            {activeMetric ? ` · 关联指标：${activeMetric.metric_label}` : ''}
          </div>
        </div>
      </div>

      <div className="diagnosis-layout">
        <div className="panel">
          <div className="panel-head">
            <span className="panel-title">本期诊断记录</span>
            <span className="panel-note">
              {detail.period.from} 至 {detail.period.to} · {reports.length} 份
            </span>
          </div>
          <div className="diagnosis-list" data-testid="diagnosis-report-list">
            {reports.length === 0 ? (
              <div className="panel-note" style={{ padding: '12px 0' }}>
                本期还没有诊断报告。选择框架并确认问题表述后，点击「生成诊断报告」。
              </div>
            ) : (
              reports.map((r) => (
                <button
                  type="button"
                  key={r.report_id}
                  className={`diagnosis-item${r.report_id === selectedReportId ? ' active' : ''}`}
                  onClick={() => {
                    setSelectedReportId(r.report_id);
                    setSupplement('');
                  }}
                  data-testid={`diagnosis-item-${r.report_id.slice(0, 8)}`}
                >
                  <strong>
                    {r.scope.problem_statement.slice(0, 18)}… · {r.framework.toUpperCase()}
                  </strong>
                  <small>
                    第 {r.revision_no} 版 · 生成于 {r.created_at.slice(0, 10)} ·{' '}
                    {r.can_continue ? '可继续' : '已停止'}
                  </small>
                </button>
              ))
            )}
          </div>
          <div className="root-card flush" style={{ margin: 0, borderTop: '1px solid #eceeeb' }}>
            <h4>与任务单的关系</h4>
            <p>
              诊断报告与任务单分别保存、分别导出；本页不会创建或修改任务。
              任务单页可在生成预览中选择是否引用已保存报告。
            </p>
          </div>
        </div>

        <div className="panel">
          {selected ? (
            <>
              <div className="diagnosis-report-head">
                <div>
                  <strong>{selected.scope.problem_statement}</strong>
                  <div className="diagnosis-report-meta">
                    {app.activeWorkspace?.name} · {selected.period.from ?? '—'} 至{' '}
                    {selected.period.to ?? '—'} · {selected.framework.toUpperCase()} · 第{' '}
                    {selected.revision_no} 版 · 分析 revision {selected.analysis_revision_no}
                  </div>
                </div>
                {isStopped ? (
                  <Tag tone="blue">已停止 · 保留报告</Tag>
                ) : (
                  <Tag tone="gold">进行中 · 可继续</Tag>
                )}
              </div>
              <div className="diagnosis-report-body" data-testid="diagnosis-report-detail">
                <div className="diagnosis-summary">{selected.summary}</div>
                {selected.findings.map((f) => (
                  <FindingBlock key={f.finding_id} finding={f} />
                ))}
                {selected.facts_and_hypotheses.length > 0 ? (
                  <div className="root-step">
                    <strong>事实与假设清单</strong>
                    {selected.facts_and_hypotheses.map((fh) => (
                      <p key={fh.item_id}>
                        [{FACT_STATUS_LABELS[fh.fact_status]}] {fh.statement}（置信度：
                        {CONFIDENCE_LABELS[fh.confidence]}）
                      </p>
                    ))}
                  </div>
                ) : null}
                {selected.warnings.length > 0 ? (
                  <div className="root-step" data-testid="diagnosis-warnings">
                    <strong>警告与解决要求</strong>
                    {selected.warnings.map((w) => (
                      <p key={w.warning_id}>
                        <Tag tone={w.status === 'active' ? 'red' : 'green'}>
                          {w.status === 'active' ? '未解决' : '已解决'}
                        </Tag>{' '}
                        {w.message} ·{' '}
                        <span className="evidence-ref">
                          {RESOLUTION_REQUIREMENT_LABELS[w.resolution_requirement]}
                        </span>
                        {w.resolution_note ? ` · ${w.resolution_note}` : ''}
                      </p>
                    ))}
                  </div>
                ) : null}
                {selected.clarification_questions.length > 0 ? (
                  <div className="root-step">
                    <strong>待澄清问题</strong>
                    {selected.clarification_questions.map((q) => (
                      <p key={q.question_id}>
                        {q.status === 'open' ? '❓' : '✔'} {q.question}
                        {q.answer_summary ? `（${q.answer_summary}）` : ''}
                      </p>
                    ))}
                  </div>
                ) : null}
                {selected.stop_reason ? (
                  <div className="root-step">
                    <strong>停止原因</strong>
                    <p>{selected.stop_reason}</p>
                  </div>
                ) : null}
              </div>
              <div className="diagnosis-followup">
                <label htmlFor="diagnosis-supplement">补充现场事实后继续</label>
                <textarea
                  id="diagnosis-supplement"
                  className="input"
                  placeholder="写入核实到的新事实、记录来源或现场反馈（将作为未核实的用户陈述记录）"
                  value={supplement}
                  onChange={(e) => setSupplement(e.target.value)}
                  disabled={isStopped}
                />
                <div className="diagnosis-actions">
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void requestContinue()}
                    disabled={busy || isStopped}
                    data-testid="continue-diagnosis-button"
                  >
                    继续分析
                  </button>
                  <button type="button" className="outline" onClick={stopReport} disabled={isStopped}>
                    停止并保留报告
                  </button>
                  <button type="button" className="outline" onClick={() => void download('md')}>
                    下载 MD
                  </button>
                  <button type="button" className="outline" onClick={() => void download('json')}>
                    下载 JSON
                  </button>
                  <span className="diagnosis-status">
                    {isStopped
                      ? '本报告已停止；内容仍可查看与下载'
                      : '每轮续问都会先预览发送范围，确认后才发送'}
                  </span>
                </div>
              </div>
            </>
          ) : (
            <div className="panel-body">
              <EmptyState
                title="暂无选中的诊断报告"
                hint="问题表述将默认由重点指标差异确定性生成；也可以自行修改后生成。"
              >
                <p style={{ marginTop: 10 }}>
                  <span className="muted">默认问题表述：</span>
                  {problemText || '（本期无可重点差异指标）'}
                </p>
              </EmptyState>
            </div>
          )}
        </div>
      </div>

      <div className="footer-note">
        报告独立保存到当前企业档案；本页不调用任务生成，任务单页亦不进行根因追问。
      </div>

      <GenerationProgressPanel
        active={sending}
        title="诊断生成发送中"
        onCancel={(attemptId) => void cancelGeneration(attemptId)}
      />

      {pending ? (
        <PreviewModal
          preview={pending.preview}
          title={pending.kind === 'generate' ? '生成诊断报告 · 发送预览' : '继续分析 · 发送预览'}
          confirmLabel={pending.label}
          onConfirm={() => void confirmPending()}
          onCancel={() => setPending(null)}
          busy={busy}
        />
      ) : null}
    </div>
  );
}
