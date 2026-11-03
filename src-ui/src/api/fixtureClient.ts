/**
 * 【仅开发/测试演示数据层：虚构演示数据，非 Provider 响应、非真实结果】
 *
 * FixtureClient 在内存中模拟本地 Rust 核心的读写行为（版本/隔离/进度规则/
 * 确认门禁），供页面开发与 vitest 使用。它：
 *   - 不发起任何网络请求，也不模拟 Provider 响应；
 *   - 所有返回值均为虚构演示数据（fixtureData.ts）；
 *   - 仅为方便测试而尽量贴合同语义（workspace 隔离、revision 冲突、
 *     auto 进度计算、99.99 上限、block 优先、显式确认），不代表真实实现。
 *
 * 运行时默认使用 TauriClient；FixtureClient 只在显式选择时启用
 * （见 createRuntimeClient：无 Tauri 环境时 UI 显示“未连接核心”空状态，
 * 绝不静默回退到本类）。
 */

import type {
  AnalyzeInput,
  DiagnosisContinueRequest,
  DiagnosisGenerateRequest,
  DshengClient,
  ExportResult,
  TaskGenerateRequest,
  TaskReconcileDryRunResult,
  TaskTimelineQuery,
} from './client';
import type {
  AnalysisRunView,
  Cadence,
  DiagnosisReport,
  HistoryDetailView,
  HistoryEntryView,
  PeriodPolicySetInput,
  PeriodPolicyView,
  ProviderClearKeyInput,
  ProviderDeleteInput,
  ProviderUpsertInput,
  ProviderUseInput,
  ProviderView,
  ScanSummaryView,
  TaskCandidateView,
  TaskEditInput,
  TaskEditOperation,
  TaskReconcileInput,
  TaskStatus,
  TaskStepView,
  TaskView,
  TimelineEntryView,
  WorkspaceCreateInput,
  WorkspaceUpdateInput,
  WorkspaceUseInput,
  WorkspaceView,
} from './types';
import { DshengCliError } from './types';
import {
  DEMO_NOW,
  RUN_2026_10,
  WS_DEMO_ID,
  WS_EMPTY_ID,
  fixtureAnalysisRun,
  fixtureDiagnosisReports,
  fixtureFieldMappings,
  fixtureHistory,
  fixturePeriodPolicies,
  fixtureProviders,
  fixtureScan,
  fixtureTasks,
  fixtureWorkspaces,
} from './fixtureData';

const PROGRESS_PATTERN = /^(?:(?:0|[1-9][0-9]?)(?:\.[0-9]?[1-9])?|100)$/;
const DECIMAL_INPUT_PATTERN = /^-?(?:0|[1-9][0-9]*)(?:\.[0-9]{0,5}[1-9])?$/;

let uuidCounter = 0;
function nextUuid(): string {
  uuidCounter += 1;
  return `00000000-0000-4000-8000-${String(uuidCounter).padStart(12, '0')}`;
}

/** auto 进度：completed/total×100 四舍五入两位；未全完成不因舍入到 100。
 * （计数为离散整数，用整数算术；结果为规范字符串。） */
function computeAutoProgress(completedCount: number, totalCount: number): string {
  if (totalCount <= 0) return '0';
  if (completedCount >= totalCount) return '100';
  const scaled = Math.round((completedCount * 10000) / totalCount);
  const whole = Math.floor(scaled / 100);
  const frac = scaled % 100;
  let text: string;
  if (frac === 0) {
    text = `${whole}`;
  } else if (frac % 10 === 0) {
    text = `${whole}.${frac / 10}`;
  } else {
    text = `${whole}.${String(frac).padStart(2, '0')}`;
  }
  return text === '100' ? '99.99' : text;
}

/** auto 状态优先序（task-lifecycle §5.1）。 */
function computeAutoStatus(task: {
  blocked_reason: string | null;
  steps: { status: TaskStatus }[];
}): TaskStatus {
  const anyBlockedStep = task.steps.some((s) => s.status === 'blocked');
  if ((task.blocked_reason ?? '') !== '' || anyBlockedStep) return 'blocked';
  if (task.steps.length > 0 && task.steps.every((s) => s.status === 'completed')) {
    return 'completed';
  }
  if (task.steps.some((s) => s.status === 'completed' || s.status === 'in_progress')) {
    return 'in_progress';
  }
  return 'not_started';
}

export class FixtureClient implements DshengClient {
  readonly kind = 'fixture' as const;

  private workspaces: WorkspaceView[];
  private policies: PeriodPolicyView[];
  private providers: ProviderView[];
  /** 仅记 key 是否存在与最近一次传入长度；绝不把 Key 放进任何返回值。 */
  private providerKeys = new Map<string, { present: boolean }>();
  private scans: Record<string, ScanSummaryView>;
  private tasks: TaskView[];
  private reports: DiagnosisReport[];
  private history: typeof fixtureHistory;
  private pendingConfirmations = new Map<
    string,
    { preview: import('./types').ConfirmationPreview; payload: unknown }
  >();
  private candidates = new Map<string, TaskCandidateView>();

  constructor() {
    this.workspaces = structuredClone(fixtureWorkspaces);
    this.policies = structuredClone(fixturePeriodPolicies);
    this.providers = structuredClone(fixtureProviders);
    this.scans = structuredClone(fixtureScan);
    this.tasks = structuredClone(fixtureTasks);
    this.reports = structuredClone(fixtureDiagnosisReports);
    this.history = structuredClone(fixtureHistory);
    this.providerKeys.set(fixtureProviders[0].provider_id, { present: true });
  }

  // ---- 内部校验辅助 ----

  private requireWorkspace(workspaceId: string): WorkspaceView {
    const ws = this.workspaces.find((w) => w.workspace_id === workspaceId);
    if (!ws) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '企业档案不存在或不属于当前选择',
        field: 'workspace_id',
        details: null,
      });
    }
    return ws;
  }

  private hasData(workspaceId: string): boolean {
    return workspaceId === WS_DEMO_ID;
  }

  // ---- workspace ----

  async workspaceList(): Promise<WorkspaceView[]> {
    return structuredClone(this.workspaces);
  }

  async workspaceCreate(input: WorkspaceCreateInput): Promise<WorkspaceView> {
    const now = DEMO_NOW;
    const policyId = nextUuid();
    const ws: WorkspaceView = {
      workspace_id: nextUuid(),
      name: input.name,
      config_revision: 1,
      active_provider_id: null,
      current_period_policy_id: policyId,
      created_at: now,
      updated_at: now,
    };
    this.workspaces.push(ws);
    this.policies.push({
      period_policy_id: policyId,
      workspace_id: ws.workspace_id,
      version: 1,
      policy: structuredClone(input.initial_period_policy),
      effective_to: null,
      created_at: now,
    });
    this.scans[ws.workspace_id] = {
      workspace_id: ws.workspace_id,
      directory_label: '',
      scanned_at: now,
      files: [],
      total_rows: 0,
      coverage: { from: '2026-10-01', to: '2026-10-01', granularity: 'month' },
      pending_inputs: [],
    };
    return structuredClone(ws);
  }

  async workspaceUpdate(input: WorkspaceUpdateInput): Promise<WorkspaceView> {
    const ws = this.requireWorkspace(input.workspace_id);
    if (ws.config_revision !== input.expected_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: '企业档案版本已变化，请刷新后重试',
        field: 'expected_revision',
        details: { current: ws.config_revision },
      });
    }
    ws.name = input.patch.name;
    ws.config_revision += 1;
    ws.updated_at = DEMO_NOW;
    return structuredClone(ws);
  }

  async workspaceUse(_input: WorkspaceUseInput): Promise<void> {
    this.requireWorkspace(_input.workspace_id);
  }

  // ---- period ----

  async periodList(workspaceId: string): Promise<PeriodPolicyView[]> {
    this.requireWorkspace(workspaceId);
    return structuredClone(
      this.policies.filter((p) => p.workspace_id === workspaceId),
    );
  }

  async periodSet(input: PeriodPolicySetInput): Promise<PeriodPolicyView> {
    const ws = this.requireWorkspace(input.workspace_id);
    if (ws.config_revision !== input.expected_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: '企业档案版本已变化，请刷新后重试',
        field: 'expected_revision',
        details: { current: ws.config_revision },
      });
    }
    const existing = this.policies.filter((p) => p.workspace_id === ws.workspace_id);
    const previous = existing.find((p) => p.effective_to === null);
    if (previous) {
      previous.effective_to = input.policy.effective_from;
    }
    const view: PeriodPolicyView = {
      period_policy_id: nextUuid(),
      workspace_id: ws.workspace_id,
      version: existing.length + 1,
      policy: structuredClone(input.policy),
      effective_to: null,
      created_at: DEMO_NOW,
    };
    this.policies.push(view);
    ws.current_period_policy_id = view.period_policy_id;
    ws.config_revision += 1;
    ws.updated_at = DEMO_NOW;
    return structuredClone(view);
  }

  // ---- provider（保存不发请求；无连接测试方法） ----

  async providerList(workspaceId: string): Promise<ProviderView[]> {
    this.requireWorkspace(workspaceId);
    return structuredClone(
      this.providers.filter((p) => p.workspace_id === workspaceId),
    );
  }

  async providerSet(
    input: ProviderUpsertInput,
    apiKey?: string,
  ): Promise<ProviderView> {
    this.requireWorkspace(input.workspace_id);
    if (input.key_action === 'replace' && (apiKey ?? '').length === 0) {
      throw new DshengCliError({
        code: 'PROVIDER_KEY_MISSING',
        message: '替换 Key 需要单独提供访问凭证（不写入 JSON）',
        field: 'api_key',
        details: null,
      });
    }
    if (input.provider_id == null) {
      if (input.expected_revision != null) {
        throw new DshengCliError({
          code: 'INVALID_ARGUMENT',
          message: '新建配置 expected_revision 必须为 null',
          field: 'expected_revision',
          details: null,
        });
      }
      const provider: ProviderView = {
        provider_id: nextUuid(),
        workspace_id: input.workspace_id,
        revision: 1,
        config: structuredClone(input.config),
        key_present: input.key_action === 'replace',
        created_at: DEMO_NOW,
        updated_at: DEMO_NOW,
      };
      this.providers.push(provider);
      this.providerKeys.set(provider.provider_id, {
        present: provider.key_present,
      });
      return structuredClone(provider);
    }
    const provider = this.providers.find(
      (p) => p.provider_id === input.provider_id,
    );
    if (!provider) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: 'Provider 不存在或不属于当前企业档案',
        field: 'provider_id',
        details: null,
      });
    }
    if (provider.revision !== input.expected_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: 'Provider 版本已变化，请刷新后重试',
        field: 'expected_revision',
        details: { current: provider.revision },
      });
    }
    provider.config = structuredClone(input.config);
    if (input.key_action === 'replace') {
      this.providerKeys.set(provider.provider_id, { present: true });
    }
    provider.key_present =
      this.providerKeys.get(provider.provider_id)?.present ?? false;
    provider.revision += 1;
    provider.updated_at = DEMO_NOW;
    return structuredClone(provider);
  }

  async providerUse(input: ProviderUseInput): Promise<void> {
    const ws = this.requireWorkspace(input.workspace_id);
    if (ws.config_revision !== input.expected_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: '企业档案版本已变化，请刷新后重试',
        field: 'expected_revision',
        details: { current: ws.config_revision },
      });
    }
    if (input.provider_id != null) {
      const provider = this.providers.find(
        (p) => p.provider_id === input.provider_id,
      );
      if (!provider || provider.workspace_id !== input.workspace_id) {
        throw new DshengCliError({
          code: 'WORKSPACE_MISMATCH',
          message: 'Provider 不存在或不属于当前企业档案',
          field: 'provider_id',
          details: null,
        });
      }
    }
    ws.active_provider_id = input.provider_id;
    ws.config_revision += 1;
    ws.updated_at = DEMO_NOW;
  }

  async providerClearKey(input: ProviderClearKeyInput): Promise<ProviderView> {
    const provider = this.takeProvider(input);
    this.providerKeys.set(provider.provider_id, { present: false });
    provider.key_present = false;
    provider.revision += 1;
    provider.updated_at = DEMO_NOW;
    return structuredClone(provider);
  }

  async providerDelete(input: ProviderDeleteInput): Promise<void> {
    const provider = this.takeProvider(input);
    this.providers = this.providers.filter(
      (p) => p.provider_id !== provider.provider_id,
    );
    this.providerKeys.delete(provider.provider_id);
    for (const ws of this.workspaces) {
      if (ws.active_provider_id === provider.provider_id) {
        ws.active_provider_id = null;
        ws.config_revision += 1;
        ws.updated_at = DEMO_NOW;
      }
    }
  }

  private takeProvider(input: {
    workspace_id: string;
    provider_id: string;
    expected_revision: number;
  }): ProviderView {
    this.requireWorkspace(input.workspace_id);
    const provider = this.providers.find(
      (p) => p.provider_id === input.provider_id,
    );
    if (!provider || provider.workspace_id !== input.workspace_id) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: 'Provider 不存在或不属于当前企业档案',
        field: 'provider_id',
        details: null,
      });
    }
    if (provider.revision !== input.expected_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: 'Provider 版本已变化，请刷新后重试',
        field: 'expected_revision',
        details: { current: provider.revision },
      });
    }
    return provider;
  }

  // ---- 数据源 ----

  async scan(workspaceId: string, directoryLabel: string): Promise<ScanSummaryView> {
    this.requireWorkspace(workspaceId);
    if (!this.hasData(workspaceId)) {
      const empty = this.scans[workspaceId] ?? this.scans[WS_EMPTY_ID];
      return structuredClone({ ...empty, workspace_id: workspaceId, directory_label: directoryLabel ?? '' });
    }
    return structuredClone(this.scans[workspaceId]);
  }

  async fieldMappings(workspaceId: string, _runId: string) {
    this.requireWorkspace(workspaceId);
    return structuredClone(fixtureFieldMappings[workspaceId] ?? []);
  }

  // ---- 经营分析 ----

  async analyze(input: AnalyzeInput): Promise<AnalysisRunView> {
    this.requireWorkspace(input.workspace_id);
    if (!this.hasData(input.workspace_id)) {
      throw new DshengCliError({
        code: 'DATA_VALIDATION_FAILED',
        message: '当前企业档案尚无已扫描数据，请先完成目录选择与字段映射',
        field: 'workspace_id',
        details: null,
      });
    }
    const run = structuredClone(fixtureAnalysisRun);
    if (input.grain !== 'auto' && input.grain !== 'day') {
      run.grain = input.grain as Cadence;
    }
    return run;
  }

  // ---- 诊断（显式 preview → confirm；不自动触发） ----

  async diagnosisGeneratePreview(
    request: DiagnosisGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    this.requireWorkspace(request.workspace_id);
    if (!this.hasData(request.workspace_id)) {
      throw new DshengCliError({
        code: 'DATA_VALIDATION_FAILED',
        message: '请先为当前企业档案导入数据并完成字段映射',
        field: 'workspace_id',
        details: null,
      });
    }
    if ((request.problem ?? '').trim().length === 0) {
      throw new DshengCliError({
        code: 'INVALID_ARGUMENT',
        message: '生成诊断报告必须提供问题表述',
        field: 'problem',
        details: null,
      });
    }
    const preview = this.buildPreview('diagnosis', 'diagnosis_generate', request);
    this.pendingConfirmations.set(preview.confirmation_id, {
      preview,
      payload: request,
    });
    return structuredClone(preview);
  }

  async diagnosisGenerateConfirm(
    confirmationId: string,
  ): Promise<DiagnosisReport> {
    const pending = this.consumeConfirmation(confirmationId, 'diagnosis_generate');
    const request = pending.payload as DiagnosisGenerateRequest;
    const template =
      fixtureDiagnosisReports.find((r) => r.framework === request.framework) ??
      fixtureDiagnosisReports[0];
    const report = structuredClone(template);
    report.report_id = nextUuid();
    report.generation_session_id = nextUuid();
    report.workspace_id = request.workspace_id;
    report.run_id = request.run_id;
    report.framework = request.framework;
    report.scope = {
      problem_statement: request.problem,
      focus_metric_ids: request.focus_metric_id ? [request.focus_metric_id] : [],
    };
    report.revision_no = 1;
    report.created_at = DEMO_NOW;
    report.updated_at = DEMO_NOW;
    report.analysis_revision_no = fixtureAnalysisRun.current_revision_no;
    report.analysis_input_fingerprint = fixtureAnalysisRun.analysis_input_fingerprint;
    this.reports.unshift(report);
    return structuredClone(report);
  }

  async diagnosisContinuePreview(
    request: DiagnosisContinueRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    this.requireWorkspace(request.workspace_id);
    const report = this.reports.find(
      (r) => r.generation_session_id === request.session_id,
    );
    if (!report) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '诊断会话不存在或不属于当前企业档案',
        field: 'session_id',
        details: null,
      });
    }
    if (!report.can_continue) {
      throw new DshengCliError({
        code: 'CONTEXT_INVALID',
        message: '本报告已停止，现有内容仍可查看与下载',
        field: 'session_id',
        details: null,
      });
    }
    if ((request.supplement_fact ?? '').trim().length === 0) {
      throw new DshengCliError({
        code: 'INVALID_ARGUMENT',
        message: '请先补充一条现场事实或核实结果',
        field: 'supplement_fact',
        details: null,
      });
    }
    const preview = this.buildPreview('diagnosis', 'diagnosis_continue', request);
    this.pendingConfirmations.set(preview.confirmation_id, {
      preview,
      payload: request,
    });
    return structuredClone(preview);
  }

  async diagnosisContinueConfirm(
    confirmationId: string,
  ): Promise<DiagnosisReport> {
    const pending = this.consumeConfirmation(confirmationId, 'diagnosis_continue');
    const request = pending.payload as DiagnosisContinueRequest;
    const report = this.reports.find(
      (r) => r.generation_session_id === request.session_id,
    )!;
    report.revision_no += 1;
    report.updated_at = DEMO_NOW;
    report.facts_and_hypotheses.push({
      item_id: `fh-${nextUuid()}`,
      statement: request.supplement_fact,
      fact_status: 'user_report',
      evidence_ids: [],
      user_quote: request.supplement_fact,
      confidence: 'unknown',
      reasoning_summary: '用户补充的未核实陈述，仅记录于本报告。',
    });
    return structuredClone(report);
  }

  async diagnosisList(
    workspaceId: string,
    runId: string,
  ): Promise<DiagnosisReport[]> {
    this.requireWorkspace(workspaceId);
    return structuredClone(
      this.reports.filter(
        (r) => r.workspace_id === workspaceId && r.run_id === runId,
      ),
    );
  }

  async diagnosisShow(
    workspaceId: string,
    reportId: string,
    revision?: number,
  ): Promise<DiagnosisReport> {
    this.requireWorkspace(workspaceId);
    const report = this.reports.find(
      (r) => r.workspace_id === workspaceId && r.report_id === reportId,
    );
    if (!report) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '报告不存在或不属于当前企业档案',
        field: 'report_id',
        details: null,
      });
    }
    if (revision != null && revision !== report.revision_no) {
      throw new DshengCliError({
        code: 'INPUT_REVISION_CHANGED',
        message: '请求的报告版本与当前版本不一致',
        field: 'revision',
        details: { current: report.revision_no },
      });
    }
    return structuredClone(report);
  }

  async diagnosisExport(
    workspaceId: string,
    reportId: string,
    format: 'md' | 'json',
    outPath: string,
  ): Promise<ExportResult> {
    await this.diagnosisShow(workspaceId, reportId);
    return { out_path: outPath, format };
  }

  // ---- 任务生成（显式 preview → confirm） ----

  async taskGeneratePreview(
    request: TaskGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    this.requireWorkspace(request.workspace_id);
    if (!this.hasData(request.workspace_id)) {
      throw new DshengCliError({
        code: 'DATA_VALIDATION_FAILED',
        message: '请先为当前企业档案导入数据并完成字段映射',
        field: 'workspace_id',
        details: null,
      });
    }
    if (request.diagnosis_report_id) {
      const report = this.reports.find(
        (r) => r.report_id === request.diagnosis_report_id,
      );
      if (!report || report.run_id !== request.run_id) {
        throw new DshengCliError({
          code: 'STALE_DIAGNOSIS_REFERENCE',
          message: '引用的诊断报告不属于当前分析期间，请重选或取消引用',
          field: 'diagnosis_report_id',
          details: null,
        });
      }
    }
    const preview = this.buildPreview('task', 'task_generate', request);
    this.pendingConfirmations.set(preview.confirmation_id, {
      preview,
      payload: request,
    });
    return structuredClone(preview);
  }

  async taskGenerateConfirm(confirmationId: string): Promise<TaskCandidateView> {
    this.consumeConfirmation(confirmationId, 'task_generate');
    const candidate: TaskCandidateView = {
      candidate_generation_id: nextUuid(),
      candidate_revision_no: 1,
      run_id: RUN_2026_10,
      analysis_revision_no: fixtureAnalysisRun.current_revision_no,
      state: 'candidate',
      envelope_locally_reconstructed: false, // 演示层：C 方案兜底标记（真实核心在裸任务包装时为 true）
      tasks: [
        {
          task_key: 'ct-1',
          title: '建立高折扣订单预警阈值',
          summary: '销售最大化 · 折扣阈值与成本版本联动校验',
          category: 'revenue',
          target_value: '210000',
          owner: '经营管理岗',
          steps: [
            { step_id: 'cs-1', action: '定义高折扣阈值' },
            { step_id: 'cs-2', action: '配置联动校验规则' },
            { step_id: 'cs-3', action: '试运行并复盘' },
          ],
        },
        {
          task_key: 'ct-2',
          title: '规范工时记录流程',
          summary: '时间最短化 · 缺失工时的记录与复核流程',
          category: 'efficiency',
          target_value: '450',
          owner: '生产计划岗',
          steps: [
            { step_id: 'cs-4', action: '梳理工时记录流程' },
            { step_id: 'cs-5', action: '明确记录与复核责任' },
          ],
        },
      ],
    };
    this.candidates.set(candidate.candidate_generation_id, candidate);
    return structuredClone(candidate);
  }

  async taskReconcileDryRun(
    input: TaskReconcileInput,
  ): Promise<TaskReconcileDryRunResult> {
    this.requireWorkspace(input.workspace_id);
    if (!this.candidates.has(input.candidate_generation_id)) {
      throw new DshengCliError({
        code: 'INPUT_CONTRACT_INVALID',
        message: '候选不存在或已被应用',
        field: 'candidate_generation_id',
        details: null,
      });
    }
    for (const expected of input.expected_tasks) {
      const task = this.tasks.find((t) => t.task_id === expected.task_id);
      if (!task) {
        throw new DshengCliError({
          code: 'WORKSPACE_MISMATCH',
          message: 'expected_tasks 引用了不存在的任务',
          field: 'expected_tasks',
          details: null,
        });
      }
      if (task.task_revision_no !== expected.task_revision_no) {
        throw new DshengCliError({
          code: 'REVISION_CONFLICT',
          message: '任务版本已变化，请重新预览',
          field: 'expected_tasks',
          details: { task_id: task.task_id, current: task.task_revision_no },
        });
      }
    }
    return {
      diff: { mappings: input.mappings },
      apply_token: `demo-token-${nextUuid()}`,
      expires_at: DEMO_NOW,
    };
  }

  async taskReconcileApply(
    input: TaskReconcileInput,
    _applyToken: string,
  ): Promise<TaskView[]> {
    const candidate = this.candidates.get(input.candidate_generation_id);
    if (!candidate) {
      throw new DshengCliError({
        code: 'INPUT_CONTRACT_INVALID',
        message: '候选不存在或已被应用',
        field: 'candidate_generation_id',
        details: null,
      });
    }
    const created: TaskView[] = [];
    for (const mapping of input.mappings) {
      if (mapping.action !== 'new') continue;
      const candidateTask = candidate.tasks.find(
        (t) => t.task_key === mapping.candidate_task_key,
      );
      if (!candidateTask) {
        throw new DshengCliError({
          code: 'DUPLICATE_CANDIDATE_KEY',
          message: '候选任务键不存在',
          field: 'mappings',
          details: null,
        });
      }
      const task = this.buildTaskFromCandidate(input.workspace_id, candidateTask);
      this.tasks.unshift(task);
      created.push(task);
    }
    candidate.state = 'applied';
    return structuredClone(created);
  }

  private buildTaskFromCandidate(
    workspaceId: string,
    candidateTask: TaskCandidateView['tasks'][number],
  ): TaskView {
    const steps: TaskStepView[] = candidateTask.steps.map((s, i) => ({
      step_id: nextUuid(),
      step_order: i + 1,
      action: s.action,
      owner: candidateTask.owner,
      start_date: null,
      due_date: null,
      done_criteria: null,
      status: 'not_started',
      blocked_reason: null,
      completed_at: null,
    }));
    const metric = fixtureAnalysisRun.metrics.find(
      (m) => m.metric_label === candidateTask.summary.split(' · ')[0],
    );
    return {
      workspace_id: workspaceId,
      task_id: nextUuid(),
      task_revision_no: 1,
      title: candidateTask.title,
      summary: candidateTask.summary,
      category: candidateTask.category,
      metric_id: metric?.metric_id ?? null,
      metric_label: metric?.metric_label ?? null,
      target_value: candidateTask.target_value,
      planned_value: null,
      actual_value: null,
      current_value: null,
      unit: metric?.unit ?? null,
      mp_label: null,
      mp_unit: null,
      owner: candidateTask.owner,
      planned_start: null,
      due_date: null,
      resource_need: null,
      success_criteria: null,
      note: null,
      review_note: null,
      target_period: null,
      lifecycle: 'active',
      status: 'not_started',
      progress: '0',
      progress_mode: 'auto',
      adjustment_reason: null,
      blocked_reason: null,
      completed_at: null,
      smart_check: {
        smart_status: 'generated',
        assessed_task_revision: 1,
        checks: [
          { dimension: 'S', pass: true, note: '生成时评价：目标指向差异指标' },
          { dimension: 'M', pass: true, note: '生成时评价：以目标数值衡量' },
          { dimension: 'A', pass: true, note: '生成时评价：按已有依据评估' },
          { dimension: 'R', pass: true, note: '生成时评价：对应经营重点' },
          { dimension: 'T', pass: false, note: '生成时未给出期限，保留待确认' },
        ],
      },
      steps,
      evidence: [],
      events: [
        {
          event_id: nextUuid(),
          task_id: '',
          task_revision_no: 1,
          event_seq: Date.parse(DEMO_NOW),
          kind: 'task_created',
          reason: `任务生成（候选 ${candidateTask.task_key} 应用）`,
          actor_source: 'generation',
          occurred_at: DEMO_NOW,
        },
      ],
      origin_run_id: RUN_2026_10,
      overdue: false,
    };
  }

  // ---- 任务读写 ----

  async taskList(workspaceId: string, runId?: string): Promise<TaskView[]> {
    this.requireWorkspace(workspaceId);
    return structuredClone(
      this.tasks.filter(
        (t) =>
          t.workspace_id === workspaceId &&
          (runId == null || t.origin_run_id === runId),
      ),
    );
  }

  async taskShow(workspaceId: string, taskId: string): Promise<TaskView> {
    this.requireWorkspace(workspaceId);
    const task = this.tasks.find(
      (t) => t.workspace_id === workspaceId && t.task_id === taskId,
    );
    if (!task) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '任务不存在或不属于当前企业档案',
        field: 'task_id',
        details: null,
      });
    }
    return structuredClone(task);
  }

  async taskUpdate(input: TaskEditInput): Promise<TaskView> {
    this.requireWorkspace(input.workspace_id);
    const task = this.tasks.find(
      (t) =>
        t.workspace_id === input.workspace_id && t.task_id === input.task_id,
    );
    if (!task) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '任务不存在或不属于当前企业档案',
        field: 'task_id',
        details: null,
      });
    }
    if (task.task_revision_no !== input.expected_task_revision) {
      throw new DshengCliError({
        code: 'REVISION_CONFLICT',
        message: '任务已被其他修改更新，请刷新后重试',
        field: 'expected_task_revision',
        details: { current: task.task_revision_no },
      });
    }
    this.applyTaskEdit(task, input.operation);
    task.task_revision_no += 1;
    task.events.unshift({
      event_id: nextUuid(),
      task_id: task.task_id,
      task_revision_no: task.task_revision_no,
      event_seq: task.events.length + 1,
      kind: input.operation.kind === 'progress' ? 'progress_event' : 'content_edit',
      reason:
        input.operation.kind === 'progress'
          ? input.operation.adjustment_reason ?? '进度更新'
          : '内容修改',
      actor_source: 'ui',
      occurred_at: DEMO_NOW,
    });
    return structuredClone(task);
  }

  private applyTaskEdit(task: TaskView, operation: TaskEditOperation): void {
    if (operation.kind === 'content') {
      const patch = operation.patch;
      const numericTouched =
        patch &&
        (patch.target_value !== undefined ||
          patch.planned_value !== undefined ||
          patch.actual_value !== undefined ||
          patch.current_value !== undefined);
      if (numericTouched) {
        for (const [key, value] of Object.entries({
          target_value: patch?.target_value,
          planned_value: patch?.planned_value,
          actual_value: patch?.actual_value,
          current_value: patch?.current_value,
        })) {
          if (value !== undefined && value !== null && !DECIMAL_INPUT_PATTERN.test(value)) {
            throw new DshengCliError({
              code: 'DATA_VALIDATION_FAILED',
              message: `${key} 必须是规范十进制字符串（≤6 位小数，无分组/指数）`,
              field: `patch/${key}`,
              details: null,
            });
          }
        }
        if ((operation.source_note ?? '').trim().length === 0) {
          throw new DshengCliError({
            code: 'DATA_VALIDATION_FAILED',
            message: '修改目标/预定/实绩/当前数值必须注明人工调整依据（source_note）',
            field: 'source_note',
            details: null,
          });
        }
      }
      let structureChanged = false;
      if (patch) {
        const assigns: [string, unknown][] = [
          ['title', patch.title],
          ['summary', patch.summary],
          ['category', patch.category],
          ['metric_id', patch.metric_id],
          ['target_value', patch.target_value],
          ['planned_value', patch.planned_value],
          ['actual_value', patch.actual_value],
          ['current_value', patch.current_value],
          ['unit', patch.unit],
          ['mp_label', patch.mp_label],
          ['mp_unit', patch.mp_unit],
          ['owner', patch.owner],
          ['planned_start', patch.planned_start],
          ['due_date', patch.due_date],
          ['resource_need', patch.resource_need],
          ['success_criteria', patch.success_criteria],
          ['note', patch.note],
          ['review_note', patch.review_note],
          ['target_period', patch.target_period],
        ];
        for (const [key, value] of assigns) {
          if (value !== undefined) {
            (task as unknown as Record<string, unknown>)[key] = value;
          }
        }
        if (assigns.some(([key]) => (patch as Record<string, unknown>)[key] !== undefined)) {
          task.smart_check = {
            ...task.smart_check,
            smart_status: 'stale',
          };
        }
      }
      if (operation.step_operations?.length) {
        structureChanged = this.applyStepOperations(task, operation.step_operations);
      }
      if (structureChanged) {
        this.recomputeAuto(task);
      }
      return;
    }
    if (operation.kind === 'progress') {
      this.applyProgressEdit(task, operation);
      return;
    }
    // period_link：演示层仅记录事件，不展开 link 存储。
  }

  private applyStepOperations(task: TaskView, ops: import('./types').StepOperation[]): boolean {
    let structural = false;
    for (const op of ops) {
      if (op.op === 'add') {
        const beforeIndex = task.steps.findIndex((s) => s.step_id === op.before_step_id);
        const step: TaskStepView = {
          step_id: nextUuid(),
          step_order: 0,
          action: op.content.action,
          owner: op.content.owner,
          start_date: op.content.start_date,
          due_date: op.content.due_date,
          done_criteria: op.content.done_criteria,
          status: 'not_started',
          blocked_reason: null,
          completed_at: null,
        };
        if (beforeIndex >= 0) {
          task.steps.splice(beforeIndex, 0, step);
        } else {
          task.steps.push(step);
        }
        structural = true;
      } else if (op.op === 'update') {
        const step = task.steps.find((s) => s.step_id === op.step_id);
        if (!step) {
          throw new DshengCliError({
            code: 'WORKSPACE_MISMATCH',
            message: '步骤不存在',
            field: 'step_id',
            details: null,
          });
        }
        const patch = op.patch;
        if (patch.action !== undefined) step.action = patch.action;
        if (patch.owner !== undefined) step.owner = patch.owner;
        if (patch.start_date !== undefined) step.start_date = patch.start_date;
        if (patch.due_date !== undefined) step.due_date = patch.due_date;
        if (patch.done_criteria !== undefined) step.done_criteria = patch.done_criteria;
        if (patch.status !== undefined) {
          if (patch.status === 'blocked' && (patch.blocked_reason ?? '') === '') {
            throw new DshengCliError({
              code: 'STATE_PROGRESS_MISMATCH',
              message: '步骤阻塞必须填写非空理由',
              field: 'patch/blocked_reason',
              details: null,
            });
          }
          if (
            patch.status === 'completed' &&
            step.status !== 'completed'
          ) {
            step.completed_at = DEMO_NOW;
          }
          if (patch.status !== 'completed') step.completed_at = null;
          step.status = patch.status;
          if (patch.blocked_reason !== undefined) step.blocked_reason = patch.blocked_reason;
          structural = true;
        }
      } else if (op.op === 'delete') {
        const index = task.steps.findIndex((s) => s.step_id === op.step_id);
        if (index >= 0) {
          if (task.steps.length <= 1) {
            throw new DshengCliError({
              code: 'AT_LEAST_ONE_STEP',
              message: '任务必须保留至少一条行动步骤',
              field: 'step_operations',
              details: null,
            });
          }
          task.steps.splice(index, 1);
          structural = true;
        }
      } else if (op.op === 'move') {
        const index = task.steps.findIndex((s) => s.step_id === op.step_id);
        if (index >= 0) {
          const [step] = task.steps.splice(index, 1);
          const beforeIndex = task.steps.findIndex(
            (s) => s.step_id === op.before_step_id,
          );
          if (beforeIndex >= 0) {
            task.steps.splice(beforeIndex, 0, step);
          } else {
            task.steps.push(step);
          }
        }
      }
    }
    if (task.steps.length === 0) {
      throw new DshengCliError({
        code: 'AT_LEAST_ONE_STEP',
        message: '任务必须保留至少一条行动步骤',
        field: 'step_operations',
        details: null,
      });
    }
    task.steps.forEach((s, i) => {
      s.step_order = i + 1;
    });
    return structural;
  }

  private applyProgressEdit(
    task: TaskView,
    op: Extract<TaskEditOperation, { kind: 'progress' }>,
  ): void {
    if (op.progress_mode === 'auto') {
      if (op.progress !== null || op.adjustment_reason !== null) {
        throw new DshengCliError({
          code: 'STATE_PROGRESS_MISMATCH',
          message: 'auto 模式 progress 与 adjustment_reason 必须为 null，由核心计算',
          field: 'progress',
          details: null,
        });
      }
      const computedStatus = computeAutoStatus(task);
      if (op.status !== computedStatus && op.status !== 'blocked') {
        throw new DshengCliError({
          code: 'STATE_PROGRESS_MISMATCH',
          message: `提交的状态与自动计算结果不一致（计算为 ${computedStatus}）`,
          field: 'status',
          details: { computed: computedStatus },
        });
      }
      if (op.status === 'blocked' && (op.blocked_reason ?? '') === '') {
        throw new DshengCliError({
          code: 'STATE_PROGRESS_MISMATCH',
          message: '阻塞必须填写非空理由',
          field: 'blocked_reason',
          details: null,
        });
      }
      if (
        computedStatus !== 'blocked' &&
        op.status !== 'blocked' &&
        op.blocked_reason !== null
      ) {
        throw new DshengCliError({
          code: 'STATE_PROGRESS_MISMATCH',
          message: '非阻塞状态 blocked_reason 必须为 null',
          field: 'blocked_reason',
          details: null,
        });
      }
      this.recomputeAuto(task, op.blocked_reason);
      return;
    }
    // manual
    if ((op.adjustment_reason ?? '').trim().length === 0) {
      throw new DshengCliError({
        code: 'DATA_VALIDATION_FAILED',
        message: '手工调整进度必须填写调整原因',
        field: 'adjustment_reason',
        details: null,
      });
    }
    if (op.progress == null || !PROGRESS_PATTERN.test(op.progress)) {
      throw new DshengCliError({
        code: 'DATA_VALIDATION_FAILED',
        message: '手工进度必须是 0–100 的规范十进制字符串（最多两位小数）',
        field: 'progress',
        details: null,
      });
    }
    if (op.status === 'not_started' && op.progress !== '0') {
      throw new DshengCliError({
        code: 'STATE_PROGRESS_MISMATCH',
        message: '待开始任务的完成比例应为 0',
        field: 'progress',
        details: null,
      });
    }
    if (op.status === 'completed' && op.progress !== '100') {
      throw new DshengCliError({
        code: 'STATE_PROGRESS_MISMATCH',
        message: '任务进度达到 100% 后才能标记为已完成',
        field: 'progress',
        details: null,
      });
    }
    if (op.status === 'blocked' && (op.blocked_reason ?? '') === '') {
      throw new DshengCliError({
        code: 'STATE_PROGRESS_MISMATCH',
        message: '阻塞必须填写非空理由',
        field: 'blocked_reason',
        details: null,
      });
    }
    if (op.status !== 'blocked' && op.blocked_reason !== null) {
      throw new DshengCliError({
        code: 'STATE_PROGRESS_MISMATCH',
        message: '非阻塞状态 blocked_reason 必须为 null',
        field: 'blocked_reason',
        details: null,
      });
    }
    this.transitionStatus(task, op.status, op.blocked_reason);
    task.progress = op.progress;
    task.progress_mode = 'manual';
    task.adjustment_reason = op.adjustment_reason;
  }

  /** 步骤结构/状态变化后重算 auto 进度与状态；manual 被重置（§5.2）。 */
  private recomputeAuto(task: TaskView, explicitBlockReason?: string | null): void {
    const wasManual = task.progress_mode === 'manual';
    const completed = task.steps.filter((s) => s.status === 'completed').length;
    task.progress = computeAutoProgress(completed, task.steps.length);
    const blockedReason =
      explicitBlockReason !== undefined ? explicitBlockReason : task.blocked_reason;
    task.blocked_reason = blockedReason;
    const status = computeAutoStatus({ blocked_reason: blockedReason, steps: task.steps });
    this.transitionStatus(task, status);
    task.progress_mode = 'auto';
    task.adjustment_reason = null;
    if (status !== 'blocked') task.blocked_reason = null;
    if (wasManual) {
      task.events.unshift({
        event_id: nextUuid(),
        task_id: task.task_id,
        task_revision_no: task.task_revision_no + 1,
        event_seq: task.events.length + 2,
        kind: 'progress_override_reset',
        reason: '步骤状态/结构变化，切回自动进度',
        actor_source: 'ui',
        occurred_at: DEMO_NOW,
      });
    }
  }

  private transitionStatus(
    task: TaskView,
    status: TaskStatus,
    blockedReason?: string | null,
  ): void {
    if (task.status === 'completed' && status !== 'completed') {
      task.events.unshift({
        event_id: nextUuid(),
        task_id: task.task_id,
        task_revision_no: task.task_revision_no + 1,
        event_seq: task.events.length + 2,
        kind: 'reopened',
        reason: '任务从已完成重开',
        actor_source: 'ui',
        occurred_at: DEMO_NOW,
      });
      task.completed_at = null;
    }
    if (status === 'completed' && task.status !== 'completed') {
      task.completed_at = DEMO_NOW;
    }
    task.status = status;
    if (blockedReason !== undefined) {
      task.blocked_reason = blockedReason;
    }
  }

  async taskTimeline(query: TaskTimelineQuery): Promise<TimelineEntryView[]> {
    this.requireWorkspace(query.workspace_id);
    const entries: TimelineEntryView[] = [];
    for (const task of this.tasks.filter(
      (t) => t.workspace_id === query.workspace_id,
    )) {
      if (task.planned_start) {
        entries.push({
          task_id: task.task_id,
          task_title: task.title,
          kind: 'planned_start',
          date: task.planned_start,
          occurred_at: null,
          detail: '任务计划开始',
        });
      }
      if (task.due_date) {
        entries.push({
          task_id: task.task_id,
          task_title: task.title,
          kind: 'due',
          date: task.due_date,
          occurred_at: null,
          detail: '任务计划截止',
        });
      }
      if (task.completed_at) {
        entries.push({
          task_id: task.task_id,
          task_title: task.title,
          kind: 'completed',
          date: task.completed_at.slice(0, 10),
          occurred_at: task.completed_at,
          detail: '任务标记为完成',
        });
      }
      for (const event of task.events) {
        entries.push({
          task_id: task.task_id,
          task_title: task.title,
          kind: 'progress_event',
          date: event.occurred_at.slice(0, 10),
          occurred_at: event.occurred_at,
          detail: event.reason ?? event.kind,
        });
      }
      if (
        !task.planned_start &&
        !task.due_date &&
        !task.completed_at &&
        task.events.length === 0
      ) {
        entries.push({
          task_id: task.task_id,
          task_title: task.title,
          kind: 'progress_event',
          date: null,
          occurred_at: null,
          detail: '暂无计划日期或进度事件',
        });
      }
    }
    return entries;
  }

  // ---- 历史 ----

  async historyList(workspaceId: string): Promise<HistoryEntryView[]> {
    this.requireWorkspace(workspaceId);
    return structuredClone(
      this.history.filter((h) => h.workspace_id === workspaceId),
    );
  }

  async historyShow(
    workspaceId: string,
    runId: string,
    revision?: number,
  ): Promise<HistoryDetailView> {
    this.requireWorkspace(workspaceId);
    const entry = this.history.find(
      (h) => h.workspace_id === workspaceId && h.run_id === runId,
    );
    if (!entry) {
      throw new DshengCliError({
        code: 'WORKSPACE_MISMATCH',
        message: '期间不存在或不属于当前企业档案',
        field: 'run_id',
        details: null,
      });
    }
    const tasks = this.tasks
      .filter((t) => t.workspace_id === workspaceId)
      .map((t) => ({
        task_id: t.task_id,
        title: t.title,
        status: t.status,
        progress: t.progress,
        task_revision_no: t.task_revision_no,
      }));
    return {
      run_id: runId,
      analysis_revision_no: revision ?? entry.current_revision_no,
      task_snapshot_no: entry.task_snapshot_no,
      captured_at: entry.updated_at,
      metrics: structuredClone(fixtureAnalysisRun.metrics),
      series: structuredClone(fixtureAnalysisRun.series),
      grain: entry.cadence,
      period: entry.period,
      tasks,
    };
  }

  // ---- 预览构建/消费 ----

  private buildPreview(
    workflow: 'task' | 'diagnosis',
    operation: import('./types').GenerationOperationName,
    request: { workspace_id: string },
  ): import('./types').ConfirmationPreview {
    const ws = this.requireWorkspace(request.workspace_id);
    const provider = this.providers.find(
      (p) => p.provider_id === ws.active_provider_id,
    );
    if (!provider) {
      throw new DshengCliError({
        code: 'PROVIDER_REQUIRED',
        message: '当前企业档案没有已确认的默认 Provider，请先在 Provider 设置中设为默认',
        field: 'provider_id',
        details: null,
      });
    }
    if (!provider.key_present) {
      throw new DshengCliError({
        code: 'PROVIDER_KEY_MISSING',
        message: '默认 Provider 缺少 API Key',
        field: 'api_key',
        details: null,
      });
    }
    return {
      confirmation_id: nextUuid(),
      expires_at: '2026-10-29T09:40:00Z',
      workspace_id: ws.workspace_id,
      workflow,
      operation,
      provider: {
        provider_id: provider.provider_id,
        revision: provider.revision,
        name: provider.config.name,
        base_url: provider.config.base_url,
        model: provider.config.model,
      },
      run_id: RUN_2026_10,
      analysis_revision_no: fixtureAnalysisRun.current_revision_no,
      analysis_input_fingerprint: fixtureAnalysisRun.analysis_input_fingerprint,
      report_id: null,
      report_revision_no: null,
      input_manifest_hash: 'f1e2d3c4b5a6978877665544332211ff00112233445566778899aabbccddeeff',
      prompt_version: 'diagnosis-v1-demo',
      schema_version: '3',
      context_policy_version: null,
      source_scope_summary: fixtureAnalysisRun.metrics.slice(0, 2).map((_m, i) => ({
        source_id: `s-${i + 1}`,
        file_label: i === 0 ? '示例核算表_202610.xlsx' : '示例推移表_202610.xlsx',
        sheet: i === 0 ? '核算表' : '日度推移',
        selected_range: i === 0 ? 'F2:F826' : 'A2:H1373',
        row_count: i === 0 ? 824 : 1372,
        content_hash: 'aabbccdd'.repeat(8),
      })),
      request_bytes: 48_512,
      conservative_input_tokens: 12_128,
      budget: provider.config.budget,
      timeouts: provider.config.timeouts,
      request_count_maximum: 1,
      key_present: true,
    };
  }

  private consumeConfirmation(
    confirmationId: string,
    expectedOperation: string,
  ): { preview: import('./types').ConfirmationPreview; payload: unknown } {
    const pending = this.pendingConfirmations.get(confirmationId);
    if (!pending || pending.preview.operation !== expectedOperation) {
      throw new DshengCliError({
        code: 'CONFIRMATION_STALE',
        message: '确认不存在、已消费或与操作不匹配，请重新预览',
        field: 'confirmation_id',
        details: null,
      });
    }
    this.pendingConfirmations.delete(confirmationId);
    return pending;
  }
}

/** 编译期检查：FixtureClient 完整实现 DshengClient。 */
const _typeCheck: DshengClient = new FixtureClient();
void _typeCheck;
