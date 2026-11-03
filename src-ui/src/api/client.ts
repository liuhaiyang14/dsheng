/**
 * DshengClient：UI 与本地核心（Rust）之间的唯一通道。
 *
 * 方法面对应 contracts/v1/cli-surface.json 中 53 条 CLI 命令的读写子集：
 * workspace/provider/period/scan/analyze/diagnosis/task/history 系列。
 * 主 Agent 集成时按本接口把 TauriClient 的 command 名接线到 Tauri command。
 *
 * 语义约定：
 * - 生成类（diagnosis generate/continue、task generate）严格两段式：
 *   preview* 只做本地预览（无网络）；confirm* 才携带 confirmation_id 真正执行。
 * - 一切业务规则（进度计算、SMART、确认门禁）以核心返回为准；
 *   失败抛 DshengCliError（保留 error envelope 的 code/message 供 UI 展示）。
 */

import { DshengCliError } from './types';
import type {
  AnalysisRunView,
  Cadence,
  CliEnvelope,
  ConfirmationPreview,
  DiagnosisFramework,
  DiagnosisReport,
  HistoryDetailView,
  HistoryEntryView,
  IsoDate,
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
  TaskStatus,
} from './types';

export interface AnalyzeInput {
  workspace_id: string;
  directory_label: string;
  from: IsoDate;
  to: IsoDate;
  grain: Cadence | 'auto';
  /** 历史回算：显式选择周期策略版本（可选）。 */
  policy_id?: string;
  /** 历史回算：按指定口径重算（可选）。 */
  recalculate?: boolean;
}

export interface DiagnosisGenerateRequest {
  workspace_id: string;
  run_id: string;
  framework: DiagnosisFramework;
  problem: string;
  focus_metric_id?: string;
  provider_id?: string;
  /** CLI 合同必填：≥1 条当前 run 来源证据 ID。 */
  evidence_ids: string[];
}

export interface DiagnosisContinueRequest {
  workspace_id: string;
  session_id: string;
  supplement_fact: string;
  provider_id?: string;
}

export interface TaskGenerateRequest {
  workspace_id: string;
  run_id: string;
  diagnosis_report_id?: string;
  provider_id?: string;
}

export interface TaskTimelineQuery {
  workspace_id: string;
  from: IsoDate;
  to: IsoDate;
  grain: Cadence;
}

export interface TaskReconcileDryRunResult {
  diff: unknown;
  apply_token: string;
  expires_at: string;
}

/** 诊断报告导出结果（核心 CLI 契约形状 export_written）。 */
export interface ExportResult {
  written: boolean;
  out: string;
  bytes: number;
}

/** task update 返回摘要（核心 CLI 契约形状；重读用 taskShow）。 */
export interface TaskUpdateSummary {
  task_id: string;
  task_revision_no: number;
  status: TaskStatus;
  progress: string;
  replayed: boolean;
}

export interface DshengClient {
  readonly kind: 'tauri' | 'fixture';

  // ---- workspace 系列 ----
  workspaceList(): Promise<WorkspaceView[]>;
  workspaceCreate(input: WorkspaceCreateInput): Promise<WorkspaceView>;
  workspaceUpdate(input: WorkspaceUpdateInput): Promise<WorkspaceView>;
  workspaceUse(input: WorkspaceUseInput): Promise<void>;

  // ---- period 系列 ----
  periodList(workspaceId: string): Promise<PeriodPolicyView[]>;
  periodSet(input: PeriodPolicySetInput): Promise<PeriodPolicyView>;

  // ---- provider 系列（保存不发请求；无连接测试） ----
  providerList(workspaceId: string): Promise<ProviderView[]>;
  /**
   * API Key 只经单独通道传入（对应 CLI --api-key-stdin），
   * 绝不进入 JSON 输入；key_action='replace' 时必须提供 apiKey。
   */
  providerSet(
    input: ProviderUpsertInput,
    apiKey?: string,
  ): Promise<ProviderView>;
  providerUse(input: ProviderUseInput): Promise<void>;
  providerClearKey(input: ProviderClearKeyInput): Promise<ProviderView>;
  providerDelete(input: ProviderDeleteInput): Promise<void>;

  // ---- 数据源 ----
  scan(workspaceId: string, directoryLabel: string): Promise<ScanSummaryView>;
  /** 空白导入模板包（三工作簿＋字段说明）落盘到指定路径。 */
  templateDownload(outPath: string): Promise<ExportResult>;
  fieldMappings(workspaceId: string, runId: string): Promise<
    { tool_field: string; source_label: string; recognized: string; usage: string }[]
  >;

  // ---- 经营分析（本地计算，无 Provider） ----
  analyze(input: AnalyzeInput): Promise<AnalysisRunView>;

  // ---- 诊断（显式按钮 → preview → confirm；绝不自动触发） ----
  diagnosisGeneratePreview(
    request: DiagnosisGenerateRequest,
  ): Promise<ConfirmationPreview>;
  diagnosisGenerateConfirm(
    confirmationId: string,
  ): Promise<DiagnosisReport>;
  diagnosisContinuePreview(
    request: DiagnosisContinueRequest,
  ): Promise<ConfirmationPreview>;
  diagnosisContinueConfirm(
    confirmationId: string,
  ): Promise<DiagnosisReport>;
  diagnosisList(workspaceId: string, runId: string): Promise<DiagnosisReport[]>;
  diagnosisShow(
    workspaceId: string,
    reportId: string,
    revision?: number,
  ): Promise<DiagnosisReport>;
  /** 下载 md/json 占位：调核心导出，UI 不自行拼模型正文。 */
  diagnosisExport(
    workspaceId: string,
    reportId: string,
    format: 'md' | 'json',
    outPath: string,
  ): Promise<ExportResult>;

  // ---- 任务生成（显式按钮 → preview → confirm） ----
  taskGeneratePreview(request: TaskGenerateRequest): Promise<ConfirmationPreview>;
  taskGenerateConfirm(confirmationId: string): Promise<TaskCandidateView>;
  taskReconcileDryRun(
    input: TaskReconcileInput,
  ): Promise<TaskReconcileDryRunResult>;
  taskReconcileApply(
    input: TaskReconcileInput,
    applyToken: string,
  ): Promise<TaskView[]>;

  // ---- 任务读写 ----
  taskList(workspaceId: string, runId?: string): Promise<TaskView[]>;
  taskShow(workspaceId: string, taskId: string): Promise<TaskView>;
  taskUpdate(input: TaskEditInput): Promise<TaskUpdateSummary>;
  taskTimeline(query: TaskTimelineQuery): Promise<TimelineEntryView[]>;

  // ---- 历史查询 ----
  historyList(workspaceId: string): Promise<HistoryEntryView[]>;
  historyShow(
    workspaceId: string,
    runId: string,
    revision?: number,
  ): Promise<HistoryDetailView>;
}

/** 解析核心 envelope：ok=false 抛 DshengCliError，ok=true 返回 data。 */
export function unwrapEnvelope<T>(envelope: CliEnvelope<T>): T {
  if (envelope.ok) {
    return envelope.data;
  }
  throw new DshengCliError(envelope.error);
}
