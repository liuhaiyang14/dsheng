/**
 * TauriClient：经 window.__TAURI__.core.invoke 调用本地 Rust 核心。
 *
 * 运行时默认实现。Tauri command 名与参数名（snake_case）即本文件给出的
 * 约定面——主 Agent 集成时按此把 dsheng-core 服务暴露为 Tauri commands；
 * 每组方法注释标明对应 CLI 命令（contracts/v1/cli-surface.json）。
 *
 * envelope 约定：command 返回 CliEnvelope JSON（schema_version/ok/data|error）。
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
import { unwrapEnvelope } from './client';
import type {
  AnalysisRunView,
  CliEnvelope,
  DiagnosisReport,
  DiagnosisReportSummaryView,
  DiagnosisGenerationReceipt,
  FieldMappingRow,
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
  TaskCandidateRecoveryView,
  TaskEditInput,
  TaskCreateInput,
  TaskReconcileInput,
  TaskView,
  TimelineEntryView,
  WorkspaceCreateInput,
  WorkspaceUpdateInput,
  WorkspaceUseInput,
  WorkspaceView,
} from './types';

export type { ExportResult };

interface TauriInvoke {
  (cmd: string, args?: Record<string, unknown>): Promise<unknown>;
}

interface TauriCore {
  invoke: TauriInvoke;
}

interface TauriNamespace {
  core: TauriCore;
}

declare global {
  interface Window {
    __TAURI__?: TauriNamespace;
    __TAURI_INTERNALS__?: unknown;
  }
}

export function tauriAvailable(): boolean {
  return typeof window !== 'undefined' && window.__TAURI__ != null;
}

export class TauriClient implements DshengClient {
  readonly kind = 'tauri' as const;

  private invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    const tauri = window.__TAURI__;
    if (!tauri) {
      throw new Error('Tauri 运行时不可用（未连接核心）');
    }
    return tauri.core.invoke(cmd, args).then((raw) => {
      const envelope = raw as CliEnvelope<T>;
      if (
        envelope == null ||
        typeof envelope !== 'object' ||
        envelope.schema_version !== '1' ||
        typeof envelope.ok !== 'boolean'
      ) {
        throw new Error(`核心返回缺少标准 envelope：${cmd}`);
      }
      return unwrapEnvelope<T>(envelope);
    });
  }

  // ---- workspace（dsheng workspace list/create/update/use） ----
  workspaceList(): Promise<WorkspaceView[]> {
    return this.invoke<{ workspaces: WorkspaceView[] }>('workspace_list').then(
      (d) => d.workspaces,
    );
  }
  workspaceCreate(input: WorkspaceCreateInput): Promise<WorkspaceView> {
    // CLI/core 的 workspace_create 返回 { workspace, initial_period_policy }；
    // UI 客户端对外只暴露 WorkspaceView，避免创建后读取 view.workspace_id
    // 得到 undefined 并在后续 history_list 中丢失 workspaceId。
    return this.invoke<{ workspace: WorkspaceView }>('workspace_create', { input }).then(
      (d) => d.workspace,
    );
  }
  workspaceUpdate(input: WorkspaceUpdateInput): Promise<WorkspaceView> {
    return this.invoke<{ workspace: WorkspaceView }>('workspace_update', { input }).then(
      (d) => d.workspace,
    );
  }
  workspaceUse(input: WorkspaceUseInput): Promise<void> {
    return this.invoke('workspace_use', { input });
  }

  // ---- period（dsheng workspace period list/set） ----
  periodList(workspaceId: string): Promise<PeriodPolicyView[]> {
    return this.invoke<{ policies: PeriodPolicyView[] }>('period_list', { workspaceId }).then(
      (d) => d.policies,
    );
  }
  periodSet(input: PeriodPolicySetInput): Promise<PeriodPolicyView> {
    return this.invoke<{ policy: PeriodPolicyView }>('period_set', { input }).then(
      (d) => d.policy,
    );
  }

  // ---- provider（保存不发请求；无连接测试命令） ----
  providerList(workspaceId: string): Promise<ProviderView[]> {
    return this.invoke<{ providers: ProviderView[] }>('provider_list', { workspaceId }).then(
      (d) => d.providers,
    );
  }
  providerSet(
    input: ProviderUpsertInput,
    apiKey?: string,
  ): Promise<ProviderView> {
    // Key 单独通道（对应 --api-key-stdin），绝不进 input JSON。
    return this.invoke<{ provider: ProviderView }>('provider_set', {
      input,
      apiKey: apiKey ?? null,
    }).then((d) => d.provider);
  }
  providerUse(input: ProviderUseInput): Promise<void> {
    return this.invoke('provider_use', { input });
  }
  providerClearKey(input: ProviderClearKeyInput): Promise<ProviderView> {
    return this.invoke<{ provider: ProviderView }>('provider_clear_key', { input }).then(
      (d) => d.provider,
    );
  }
  providerDelete(input: ProviderDeleteInput): Promise<void> {
    return this.invoke('provider_delete', { input });
  }

  // ---- 数据源（dsheng scan / mapping show） ----
  scan(workspaceId: string, directoryLabel: string, sourceProfile?: string, generic = false): Promise<ScanSummaryView> {
    return this.invoke('scan', {
      workspaceId,
      directoryLabel,
      sourceProfile: sourceProfile ?? null,
      generic,
    });
  }
  templateDownload(
    outPath: string,
    packageName: 'source' | 'output' = 'source',
    profile?: string,
    contract?: string,
  ): Promise<ExportResult> {
    return this.invoke('template_download', {
      outPath,
      package: packageName,
      profile: profile ?? null,
      contract: contract ?? null,
    });
  }
  exportTables(input: {
    workspaceId: string;
    runId: string;
    revision?: number;
    kind: 'accounting' | 'trend' | 'bundle';
    outPath: string;
  }): Promise<ExportResult> {
    return this.invoke<ExportResult>('export_tables', {
      workspaceId: input.workspaceId,
      runId: input.runId,
      revision: input.revision ?? null,
      kind: input.kind,
      outPath: input.outPath,
    });
  }
  metricRegisterDefaults(workspaceId: string): Promise<{ registered: number; skipped_already_present: string[]; failed: unknown[] }> {
    return this.invoke('metric_register_defaults', { workspaceId });
  }
  fieldMappings(workspaceId: string, runId: string): Promise<FieldMappingRow[]> {
    return this
      .invoke<{ mappings: FieldMappingRow[] }>('mapping_show', { workspaceId, runId })
      .then((d) => d.mappings);
  }
  sourceMappingSet(workspaceId: string, directoryLabel: string, profileHash: string, mappings: unknown[] = []): Promise<{ confirmed: boolean }> {
    return this.invoke<{ confirmed: boolean }>('source_mapping_set', {
      workspaceId,
      directoryLabel,
      profileHash,
      mappings,
    });
  }

  // ---- 经营分析（dsheng analyze；本地计算，无 Provider） ----
  analyze(input: AnalyzeInput): Promise<AnalysisRunView> {
    return this.invoke('analyze', { input });
  }

  // ---- 诊断（dsheng diagnosis generate/continue/list/show/export） ----
  diagnosisEvidence(
    workspaceId: string,
    runId: string,
    analysisRevisionNo: number,
    metricId?: string,
  ): Promise<string[]> {
    return this.invoke('diagnosis_evidence', {
      workspaceId, runId, revisionNo: analysisRevisionNo, metricId: metricId ?? null,
    });
  }
  diagnosisGeneratePreview(
    request: DiagnosisGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('diagnosis_generate_preview', { request });
  }
  diagnosisGenerateConfirm(confirmationId: string): Promise<DiagnosisGenerationReceipt> {
    return this.invoke('diagnosis_generate_confirm', { confirmationId });
  }
  diagnosisContinuePreview(
    request: DiagnosisContinueRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('diagnosis_continue_preview', { request });
  }
  diagnosisContinueConfirm(confirmationId: string): Promise<DiagnosisGenerationReceipt> {
    return this.invoke('diagnosis_continue_confirm', { confirmationId });
  }
  diagnosisList(
    workspaceId: string,
    runId: string,
  ): Promise<DiagnosisReportSummaryView[]> {
    return this
      .invoke<{ run_id: string; reports: DiagnosisReportSummaryView[] }>('diagnosis_list', {
        workspaceId,
        runId,
      })
      .then((d) => d.reports);
  }
  diagnosisShow(
    workspaceId: string,
    reportId: string,
    revision?: number,
  ): Promise<DiagnosisReport> {
    return this.invoke('diagnosis_show', {
      workspaceId,
      reportId,
      revision: revision ?? null,
    });
  }
  diagnosisExport(
    workspaceId: string,
    reportId: string,
    format: 'md' | 'json',
    outPath: string,
  ): Promise<ExportResult> {
    return this.invoke('diagnosis_export', {
      workspaceId,
      reportId,
      format,
      outPath,
    });
  }

  // ---- 生成（取消；流式进度经 generation://progress 事件订阅） ----
  generationCancel(
    workspaceId: string,
    attemptId: string,
  ): Promise<import('./types').GenerationCancelResult> {
    return this.invoke('generation_cancel', { workspaceId, attemptId });
  }
  generationCancelByConfirmation(
    workspaceId: string,
    confirmationId: string,
  ): Promise<Omit<import('./types').GenerationCancelResult, 'attempt_id'> & { attempt_id: string | null }> {
    return this.invoke('generation_cancel_by_confirmation', { workspaceId, confirmationId });
  }

  // ---- 任务生成（dsheng task generate / task reconcile） ----
  taskGeneratePreview(
    request: TaskGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('task_generate_preview', { request });
  }
  taskGenerateConfirm(confirmationId: string): Promise<TaskCandidateView> {
    return this.invoke('task_generate_confirm', { confirmationId });
  }
  taskCandidateLatest(workspaceId: string, runId: string, revisionNo: number): Promise<TaskCandidateRecoveryView | null> {
    return this.invoke('task_candidate_latest', { workspaceId, runId, revisionNo });
  }
  taskReconcileDryRun(
    input: TaskReconcileInput,
  ): Promise<TaskReconcileDryRunResult> {
    return this.invoke('task_reconcile_dry_run', { input });
  }
  taskReconcileApply(
    input: TaskReconcileInput,
    applyToken: string,
  ): Promise<TaskView[]> {
    return this.invoke('task_reconcile_apply', { input, applyToken });
  }

  // ---- 任务读写（dsheng task list/show/update、task timeline） ----
  taskList(workspaceId: string, runId?: string): Promise<TaskView[]> {
    return this.invoke<{ tasks: TaskView[] }>('task_list', {
      workspaceId,
      runId: runId ?? null,
    }).then((d) => d.tasks);
  }
  taskShow(workspaceId: string, taskId: string): Promise<TaskView> {
    return this.invoke('task_show', { workspaceId, taskId });
  }
  taskCreate(input: TaskCreateInput): Promise<TaskView> {
    return this.invoke('task_create', { input });
  }
  taskUpdate(input: TaskEditInput): Promise<import('./client').TaskUpdateSummary> {
    return this.invoke('task_update', { input });
  }
  taskTimeline(query: TaskTimelineQuery): Promise<TimelineEntryView[]> {
    return this
      .invoke<{ entries: TimelineEntryView[] }>('task_timeline', { query })
      .then((d) => d.entries);
  }

  // ---- 历史（dsheng history list/show） ----
  historyList(workspaceId: string): Promise<HistoryEntryView[]> {
    return this.invoke<{ runs: HistoryEntryView[] }>('history_list', { workspaceId }).then(
      (d) => d.runs,
    );
  }
  historyShow(
    workspaceId: string,
    runId: string,
    revision?: number,
  ): Promise<HistoryDetailView> {
    return this.invoke('history_show', {
      workspaceId,
      runId,
      revision: revision ?? null,
    });
  }
}

/** 编译期检查：TauriClient 完整实现 DshengClient。 */
const _typeCheck: DshengClient = new TauriClient();
void _typeCheck;
