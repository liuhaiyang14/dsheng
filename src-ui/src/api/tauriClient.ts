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
  TaskEditInput,
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
    return this.invoke('workspace_list');
  }
  workspaceCreate(input: WorkspaceCreateInput): Promise<WorkspaceView> {
    return this.invoke('workspace_create', { input });
  }
  workspaceUpdate(input: WorkspaceUpdateInput): Promise<WorkspaceView> {
    return this.invoke('workspace_update', { input });
  }
  workspaceUse(input: WorkspaceUseInput): Promise<void> {
    return this.invoke('workspace_use', { input });
  }

  // ---- period（dsheng workspace period list/set） ----
  periodList(workspaceId: string): Promise<PeriodPolicyView[]> {
    return this.invoke('period_list', { workspaceId });
  }
  periodSet(input: PeriodPolicySetInput): Promise<PeriodPolicyView> {
    return this.invoke('period_set', { input });
  }

  // ---- provider（保存不发请求；无连接测试命令） ----
  providerList(workspaceId: string): Promise<ProviderView[]> {
    return this.invoke('provider_list', { workspaceId });
  }
  providerSet(
    input: ProviderUpsertInput,
    apiKey?: string,
  ): Promise<ProviderView> {
    // Key 单独通道（对应 --api-key-stdin），绝不进 input JSON。
    return this.invoke('provider_set', { input, apiKey: apiKey ?? null });
  }
  providerUse(input: ProviderUseInput): Promise<void> {
    return this.invoke('provider_use', { input });
  }
  providerClearKey(input: ProviderClearKeyInput): Promise<ProviderView> {
    return this.invoke('provider_clear_key', { input });
  }
  providerDelete(input: ProviderDeleteInput): Promise<void> {
    return this.invoke('provider_delete', { input });
  }

  // ---- 数据源（dsheng scan / mapping show） ----
  scan(workspaceId: string, directoryLabel: string): Promise<ScanSummaryView> {
    return this.invoke('scan', { workspaceId, directoryLabel });
  }
  fieldMappings(workspaceId: string, runId: string): Promise<FieldMappingRow[]> {
    return this.invoke('mapping_show', { workspaceId, runId });
  }

  // ---- 经营分析（dsheng analyze；本地计算，无 Provider） ----
  analyze(input: AnalyzeInput): Promise<AnalysisRunView> {
    return this.invoke('analyze', { input });
  }

  // ---- 诊断（dsheng diagnosis generate/continue/list/show/export） ----
  diagnosisGeneratePreview(
    request: DiagnosisGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('diagnosis_generate_preview', { request });
  }
  diagnosisGenerateConfirm(confirmationId: string): Promise<DiagnosisReport> {
    return this.invoke('diagnosis_generate_confirm', { confirmationId });
  }
  diagnosisContinuePreview(
    request: DiagnosisContinueRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('diagnosis_continue_preview', { request });
  }
  diagnosisContinueConfirm(confirmationId: string): Promise<DiagnosisReport> {
    return this.invoke('diagnosis_continue_confirm', { confirmationId });
  }
  diagnosisList(
    workspaceId: string,
    runId: string,
  ): Promise<DiagnosisReport[]> {
    return this.invoke('diagnosis_list', { workspaceId, runId });
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

  // ---- 任务生成（dsheng task generate / task reconcile） ----
  taskGeneratePreview(
    request: TaskGenerateRequest,
  ): Promise<import('./types').ConfirmationPreview> {
    return this.invoke('task_generate_preview', { request });
  }
  taskGenerateConfirm(confirmationId: string): Promise<TaskCandidateView> {
    return this.invoke('task_generate_confirm', { confirmationId });
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
    return this.invoke('task_list', { workspaceId, runId: runId ?? null });
  }
  taskShow(workspaceId: string, taskId: string): Promise<TaskView> {
    return this.invoke('task_show', { workspaceId, taskId });
  }
  taskUpdate(input: TaskEditInput): Promise<TaskView> {
    return this.invoke('task_update', { input });
  }
  taskTimeline(query: TaskTimelineQuery): Promise<TimelineEntryView[]> {
    return this.invoke('task_timeline', { query });
  }

  // ---- 历史（dsheng history list/show） ----
  historyList(workspaceId: string): Promise<HistoryEntryView[]> {
    return this.invoke('history_list', { workspaceId });
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
