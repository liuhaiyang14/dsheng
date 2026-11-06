/**
 * 经营问题诊断屏（S5）。
 *
 * 门禁：进入页面 / 切换框架 / 选择指标 / 选择报告等任何事件都不触发生成；
 * 只有用户显式点击「生成诊断报告」才创建预览，预览确认后才执行。
 * 诊断不创建、不修改任务。
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { useApp, workspaceHasData } from '../state/store';
import { EmptyState } from '../components/EmptyState';
import { Tag } from '../components/bits';
import { GenerationDialog } from '../components/GenerationDialog';
import { useGeneration } from '../components/useGeneration';
import {
  CONFIDENCE_LABELS,
  DIAGNOSIS_FRAMEWORK_OPTIONS,
  FACT_STATUS_LABELS,
  RESOLUTION_REQUIREMENT_LABELS,
  groupDecimal,
  type ConfirmationPreview,
  type DiagnosisFramework,
  type DiagnosisReport,
  type DiagnosisReportSummaryView,
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

function friendlyError(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err);
  if (/PROVIDER_REQUIRED|No explicit\/default provider|未.*默认 Provider/i.test(raw)) {
    return `本档案尚未配置默认 Provider——请到「Provider 设置」填写 Base URL／模型／API Key（Key 仅存本机），并点「设为默认」后回来重试。（${raw}）`;
  }
  if (/PROVIDER_KEY_MISSING|no Key|未配置.*Key/i.test(raw)) {
    return `Provider 缺少 API Key——请到「Provider 设置」重新保存并填入 Key。（${raw}）`;
  }
  return raw;
}

export function DiagnosisScreen() {
  const app = useApp();
  const hasData = workspaceHasData(app.data);
  const continuationUnavailable = app.mode === 'tauri';
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  const detail = app.data.currentDetail;
  const historical = detail != null && app.data.history.some((h) => h.run_id === detail.run_id && h.current_revision_no !== detail.analysis_revision_no);

  const [reports, setReports] = useState<DiagnosisReportSummaryView[]>([]);
  const [reportDetail, setReportDetail] = useState<DiagnosisReport | null>(null);
  const [listLoading, setListLoading] = useState(false);
  const [reportLoading, setReportLoading] = useState(false);
  const [listError, setListError] = useState<string | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  const [listReload, setListReload] = useState(0);
  const [detailReload, setDetailReload] = useState(0);
  const [selectedReportId, setSelectedReportId] = useState<string | null>(null);
  const [selectedVersionId, setSelectedVersionId] = useState<string | null>(null);
  const [framework, setFramework] = useState<DiagnosisFramework>('5why');
  const [problem, setProblem] = useState('');
  const [supplement, setSupplement] = useState('');
  const [pending, setPending] = useState<PendingPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const generation = useGeneration(app.client, app.activeWorkspaceId);
  const sending = generation.active;
  const [stoppedLocal, setStoppedLocal] = useState<Set<string>>(new Set());
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    app.setContextLock(sending ? '正在生成诊断；请等待完成或在生成弹窗停止后再切换企业、期间或页面。' : null);
    return () => app.setContextLock(null);
  }, [app.setContextLock, sending]);

  const runId = detail?.run_id ?? null;

  // 列表是摘要，正文必须单独读取；所有请求都只读，不触发生成。
  useEffect(() => {
    let cancelled = false;
    setReports([]);
    setListError(null);
    if (!app.client || !app.activeWorkspaceId || !runId) {
      setSelectedReportId(null);
      setSelectedVersionId(null);
      setListLoading(false);
      return;
    }
    setListLoading(true);
    void app.client.diagnosisList(app.activeWorkspaceId, runId)
      .then((list) => {
        if (cancelled) return;
        setReports(list);
        setSelectedReportId((prev) => list.find((r) => r.report_id === prev || r.versions.some((v) => v.report_id === prev))?.report_id ?? list[0]?.report_id ?? null);
        setSelectedVersionId(null);
      })
      .catch((err: unknown) => {
        if (!cancelled) setListError(friendlyError(err));
      })
      .finally(() => {
        if (!cancelled) setListLoading(false);
      });
    return () => { cancelled = true; };
  }, [app.client, app.activeWorkspaceId, runId, detail?.analysis_revision_no, listReload]);

  const selectedSummary = reports.find((r) => r.report_id === selectedReportId) ?? null;
  const selectedVersion = selectedSummary?.versions.find((v) => v.report_id === selectedVersionId)
    ?? selectedSummary?.versions.find((v) => v.report_id === selectedSummary.report_id);
  const reportId = selectedVersion?.report_id;
  const reportRevision = selectedVersion?.report_revision_no;
  const oldVersion = selectedSummary != null && reportId !== selectedSummary.report_id;
  useEffect(() => {
    let cancelled = false;
    setReportDetail(null);
    setReportError(null);
    if (!app.client || !app.activeWorkspaceId || !reportId) {
      setReportLoading(false);
      return;
    }
    setReportLoading(true);
    void app.client.diagnosisShow(app.activeWorkspaceId, reportId, reportRevision)
      .then((report) => {
        if (cancelled) return;
        if (report.report_id !== reportId || report.revision_no !== reportRevision || report.workspace_id !== app.activeWorkspaceId || report.run_id !== selectedVersion?.run_id) {
          throw new Error('报告详情与所选企业、期间或报告不一致，请重新加载。');
        }
        setReportDetail(report);
      })
      .catch((err: unknown) => {
        if (!cancelled) setReportError(friendlyError(err));
      })
      .finally(() => {
        if (!cancelled) setReportLoading(false);
      });
    return () => { cancelled = true; };
  }, [app.client, app.activeWorkspaceId, selectedVersion?.run_id, reportId, reportRevision, detailReload]);

  // 切换选中项的当次渲染即隐藏旧正文，不等待 effect 清理。
  const selected = reportDetail?.report_id === reportId && reportDetail?.revision_no === reportRevision
    ? reportDetail : null;

  const activeMetric = detail?.metrics.find((m) => m.delta != null && m.target_value != null);
  const defaultProblem = useMemo(() => {
    if (!activeMetric) return '';
    // 默认问题表述：由指标与差异确定性拼接（呈现层组句，不做计算）。
    const unit = activeMetric.unit ?? '';
    return `${activeMetric.metric_label}本期 ${groupDecimal(activeMetric.actual_value ?? '')}${unit}，目标 ${groupDecimal(activeMetric.target_value ?? '')}${unit}，差异 ${groupDecimal(activeMetric.delta ?? '')}${unit}，需要分析差异原因`;
  }, [activeMetric]);

  const problemText = problem || defaultProblem;

  /** 显式按钮：生成诊断报告 → 预览。 */
  const requestGenerate = async () => {
    if (!app.client || !app.activeWorkspaceId || !runId || !detail || historical || sending || app.loading) return;
    setBusy(true);
    setNotice(null);
    try {
      const useEvidenceHelper = app.mode === 'tauri' && app.client.diagnosisEvidence != null;
      const evidenceMetrics = activeMetric ? [activeMetric] : (detail?.metrics ?? []);
      const evidenceIds = [...new Set(useEvidenceHelper
        ? await app.client.diagnosisEvidence!(app.activeWorkspaceId, runId, detail.analysis_revision_no, activeMetric?.metric_id)
        : evidenceMetrics.flatMap((metric) => [
            ...(metric.mom?.evidence_ids ?? []), ...(metric.yoy?.evidence_ids ?? []),
          ]))];
      if (!mounted.current) return;
      if (evidenceIds.length === 0) {
        setNotice(useEvidenceHelper
          ? '该版本未保留可用的来源证据，请使用同一数据目录重新运行本地分析后再生成诊断'
          : '本期间暂无可用来源证据，请先核对数据来源并重新运行本地分析。');
        return;
      }
      const preview = await app.client.diagnosisGeneratePreview({
        workspace_id: app.activeWorkspaceId,
        run_id: runId,
        framework,
        problem: problemText,
        focus_metric_id: activeMetric?.metric_id,
        // 核心返回的当前分析版本证据；不编造单元格定位。
        evidence_ids: evidenceIds,
      });
      generation.reset();
      setPending({ preview, kind: 'generate', label: preview.diagnosis_replacement
        ? '确认重新生成（发送 1 次请求）' : '确认生成诊断报告（发送 1 次请求）' });
    } catch (err) {
      setNotice(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  /** 显式按钮：继续分析 → 预览（含补充事实）。 */
  const requestContinue = async () => {
    if (!app.client || !app.activeWorkspaceId || !selected || oldVersion || continuationUnavailable || sending || app.loading) return;
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
      generation.reset();
      setPending({ preview, kind: 'continue', label: '确认继续分析（发送 1 次请求）' });
    } catch (err) {
      setNotice(friendlyError(err));
    } finally {
      setBusy(false);
    }
  };

  const confirmPending = async () => {
    if (!app.client || !pending || sending || app.loading) return;
    const client = app.client;
    const confirmed = pending;
    app.setContextLock('正在生成诊断；请等待完成或在生成弹窗停止后再切换企业、期间或页面。');
    const result = await generation.run(confirmed.preview.confirmation_id, () => confirmed.kind === 'generate'
      ? client.diagnosisGenerateConfirm(confirmed.preview.confirmation_id)
      : client.diagnosisContinueConfirm(confirmed.preview.confirmation_id));
    if (result.status === 'succeeded') {
      if (confirmed.kind === 'generate') {
        const receipt = result.value;
        if (receipt.report_id) setSelectedReportId(receipt.report_id);
        setNotice(receipt.report_id
          ? '诊断报告已生成并保存到当前企业档案。'
          : '本轮分析已保存，尚未形成报告。');
      } else {
        setSupplement('');
        setNotice('补充事实已随本轮分析发送并记录为用户陈述（未核实）。');
      }
      setListReload((value) => value + 1);
    }
  };

  const stopReport = () => {
    if (!selected || oldVersion) return;
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
      setNotice(friendlyError(err));
    }
  };

  const isStopped = selected ? stoppedLocal.has(selected.report_id) || !selected.can_continue || !selectedSummary?.can_continue : false;

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
            disabled={busy || sending || historical || app.loading}
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
            {listError ? (
              <div role="alert" className="panel-body">
                <p>诊断记录加载失败：{listError}</p>
                <button type="button" className="outline" onClick={() => setListReload((value) => value + 1)}>重新加载记录</button>
              </div>
            ) : listLoading ? (
              <p role="status" className="panel-note">正在读取诊断记录…</p>
            ) : reports.length === 0 ? (
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
                    setSelectedVersionId(null);
                    setSupplement('');
                  }}
                  data-testid={`diagnosis-item-${r.report_id.slice(0, 8)}`}
                >
                  <strong>
                    {r.problem_statement.slice(0, 18)}… · {r.framework.toUpperCase()}
                  </strong>
                  <small>
                    第 {r.version_no} 版 · 生成于 {r.created_at.slice(0, 10)} ·{' '}
                    {app.mode === 'tauri' ? '已保存' : r.can_continue ? '可继续' : '已停止'}
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
          {reportError ? (
            <div className="panel-body" role="alert">
              <p>诊断报告加载失败：{reportError}</p>
              <button type="button" className="outline" onClick={() => setDetailReload((value) => value + 1)}>重新加载报告</button>
            </div>
          ) : listLoading || reportLoading ? (
            <div className="panel-body" role="status">正在读取诊断报告…</div>
          ) : selected ? (
            <>
              <div className="diagnosis-report-head">
                <div>
                  <strong>{selected.scope.problem_statement}</strong>
                  <div className="diagnosis-report-meta">
                    {app.activeWorkspace?.name} · {selected.period.from ?? '—'} 至{' '}
                    {selected.period.to ?? '—'} · {selected.framework.toUpperCase()} ·{' '}
                    <select
                      className="select diagnosis-version-select"
                      aria-label="报告版本"
                      data-testid="diagnosis-version-select"
                      value={reportId}
                      disabled={busy || sending}
                      onChange={(event) => { setSelectedVersionId(event.target.value); setSupplement(''); }}
                    >
                      {[...(selectedSummary?.versions ?? [])].sort((a, b) => b.version_no - a.version_no).map((version) => (
                        <option key={version.report_id} value={version.report_id}>
                          第 {version.version_no} 版{version.report_id === selectedSummary?.report_id ? '（最新）' : '（历史）'} · {version.created_at.slice(0, 10)}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>
                {oldVersion ? (
                  <Tag tone="gold">历史版本 · 仅供查看</Tag>
                ) : continuationUnavailable ? (
                  <Tag tone="blue">报告已保存</Tag>
                ) : isStopped ? (
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
                  disabled={oldVersion || isStopped || continuationUnavailable}
                />
                <div className="diagnosis-actions">
                  <button
                    type="button"
                    className="primary"
                    onClick={() => void requestContinue()}
                    disabled={busy || sending || oldVersion || isStopped || continuationUnavailable || app.loading}
                    data-testid="continue-diagnosis-button"
                  >
                    继续分析
                  </button>
                  <button type="button" className="outline" onClick={stopReport} disabled={oldVersion || isStopped || continuationUnavailable}>
                    停止并保留报告
                  </button>
                  <button type="button" className="outline" onClick={() => void download('md')}>
                    下载 MD
                  </button>
                  <button type="button" className="outline" onClick={() => void download('json')}>
                    下载 JSON
                  </button>
                  <span className="diagnosis-status">
                    {oldVersion
                      ? '当前查看历史版本，仅供查看和下载；任务生成只可引用最新版。'
                      : continuationUnavailable
                      ? '当前版本暂不支持对已保存报告继续追问，已有内容可查看和下载。'
                      : isStopped
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

      {pending ? (
        <GenerationDialog
          preview={pending.preview}
          title={pending.kind === 'generate' ? '生成诊断报告' : '继续分析'}
          confirmLabel={pending.label}
          generation={generation}
          contextLoading={app.loading}
          onConfirm={() => void confirmPending()}
          onClose={() => { generation.reset(); setPending(null); }}
        />
      ) : null}
    </div>
  );
}
