/**
 * API 合同层类型（手写，与 contracts/v1 JSON Schema 及
 * crates/dsheng-core/src/contracts/*.rs 字段逐一对应）。
 *
 * 硬约束：所有业务数值（金额/数量/工时/进度/差异）在 TS 层一律为
 * DecimalString（规范十进制字符串），绝不经 Number/parseFloat 参与计算或比较。
 */

/** 规范十进制字符串（无指数/分组/前导零/尾随零/负零；≤6 位小数）。 */
export type DecimalString = string;

/** 进度值：0–100，最多两位小数；auto 模式为 null。 */
export type ProgressString = DecimalString | null;

/** ISO 日期 YYYY-MM-DD。 */
export type IsoDate = string;

/** RFC3339 date-time（UTC）。 */
export type IsoDateTime = string;

// ---------------------------------------------------------------------------
// CLI JSON envelope（cli-surface.json response_contract）
// ---------------------------------------------------------------------------

export type CliErrorCode =
  // exit 2：参数/配置/确认/版本冲突
  | 'INVALID_ARGUMENT'
  | 'WORKSPACE_REQUIRED'
  | 'WORKSPACE_MISMATCH'
  | 'REVISION_CONFLICT'
  | 'PROVIDER_REQUIRED'
  | 'PROVIDER_KEY_MISSING'
  | 'PROVIDER_URL_INVALID'
  | 'PROVIDER_IN_USE'
  | 'CONFIRMATION_REQUIRED'
  | 'CONFIRMATION_STALE'
  | 'CONFIRMATION_CANCELLED'
  | 'REQUEST_TOO_LARGE'
  | 'USER_MESSAGE_TOO_LARGE'
  | 'CONTEXT_BUDGET_EXCEEDED'
  | 'INVALID_TIMEOUT_BUDGET'
  | 'INVALID_CONTEXT_BUDGET'
  | 'RUNTIME_MISSING'
  | 'INPUT_REVISION_CHANGED'
  | 'IDEMPOTENCY_CONFLICT'
  | 'STALE_DIAGNOSIS_REFERENCE'
  | 'CONTEXT_INVALID'
  | 'SCOPE_SELECTION_REQUIRED'
  | 'CLARIFICATION_RESTATEMENT_REQUIRED'
  | 'MUTATION_RESULT_DELETED'
  | 'DELETE_PREVIEW_STALE'
  | 'INPUT_CONTRACT_INVALID'
  // exit 3：数据/本地存储
  | 'DATA_VALIDATION_FAILED'
  | 'DATA_DIRECTORY_UNAVAILABLE'
  | 'LOCAL_SAVE_FAILED'
  | 'SOURCE_CHANGED'
  | 'DB_SCHEMA_UNSUPPORTED'
  | 'DB_MIGRATION_BUSY'
  | 'DB_MIGRATION_FAILED'
  | 'DELETE_DEPENDENCY_BLOCKED'
  | 'UNIT_MISMATCH'
  | 'STEP_STILL_BLOCKED'
  | 'DB_BACKUP_FAILED'
  | 'DB_INTEGRITY_FAILED'
  | 'DB_RESTORE_FAILED'
  | 'TASK_CATEGORY_METRIC_MISMATCH'
  | 'TASK_CATEGORY_UNRECOGNIZED'
  | 'DUPLICATE_CANDIDATE_KEY'
  | 'DUPLICATE_RECONCILE_TARGET'
  | 'STATE_PROGRESS_MISMATCH'
  | 'STEP_OUTSIDE_TASK_PERIOD'
  | 'AT_LEAST_ONE_STEP'
  // exit 4：Provider/发送后失败
  | 'NETWORK_ERROR'
  | 'TLS_ERROR'
  | 'REDIRECT_REJECTED'
  | 'CONNECT_TIMEOUT'
  | 'RESPONSE_TIMEOUT'
  | 'TOTAL_TIMEOUT'
  | 'AUTH_FAILED'
  | 'ACCESS_DENIED'
  | 'ENDPOINT_OR_MODEL_NOT_FOUND'
  | 'RATE_LIMITED'
  | 'QUOTA_EXHAUSTED'
  | 'PROVIDER_REJECTED_UNKNOWN'
  | 'PROVIDER_SERVER_ERROR'
  | 'CONTEXT_REJECTED'
  | 'RESPONSE_TOO_LARGE'
  | 'INCOMPLETE_OUTPUT'
  | 'PROTOCOL_ERROR'
  | 'INVALID_JSON'
  | 'SCHEMA_VALIDATION_FAILED'
  | 'SEMANTIC_VALIDATION_FAILED'
  | 'CANCELLED'
  | 'INTERRUPTED'
  // exit 5：导出
  | 'EXPORT_FAILED';

export interface CliErrorBody {
  code: CliErrorCode;
  /** 安全中文文本；不含 Key/URL 凭据/完整请求或失败响应。 */
  message: string;
  /** nullable JSON-pointer 或 flag 名。 */
  field: string | null;
  /** 脱敏白名单元数据。 */
  details: unknown;
}

export interface CliOkEnvelope<T> {
  schema_version: '1';
  ok: true;
  data: T;
}

export interface CliErrorEnvelope {
  schema_version: '1';
  ok: false;
  error: CliErrorBody;
}

export type CliEnvelope<T> = CliOkEnvelope<T> | CliErrorEnvelope;

/** 客户端抛出的核心错误（保留 envelope 正文，供 UI 展示 message/code）。 */
export class DshengCliError extends Error {
  readonly code: CliErrorCode;
  readonly field: string | null;
  readonly details: unknown;

  constructor(body: CliErrorBody) {
    super(body.message);
    this.name = 'DshengCliError';
    this.code = body.code;
    this.field = body.field;
    this.details = body.details;
  }
}

// ---------------------------------------------------------------------------
// settings.schema.json：公开视图与输入
// ---------------------------------------------------------------------------

export interface Timeouts {
  connect_seconds: number;
  response_seconds: number;
  total_seconds: number;
}

export interface Budget {
  context_window_tokens: number;
  output_reserve_tokens: number;
}

export type Cadence = 'day' | 'week' | 'month' | 'quarter';

export interface PeriodPolicy {
  cadence: Cadence;
  week_start_day: number; // 1..7
  fiscal_year_start_month: number; // 1..12
  timezone_id: string;
  effective_from: IsoDate;
}

/** settings.schema.json#/$defs/workspaceView */
export interface WorkspaceView {
  workspace_id: string;
  name: string;
  config_revision: number;
  active_provider_id: string | null;
  current_period_policy_id: string;
  created_at: IsoDateTime;
  updated_at: IsoDateTime;
}

/** settings.schema.json#/$defs/providerView（Key 永不出现，仅 key_present）。 */
export interface ProviderView {
  provider_id: string;
  workspace_id: string;
  revision: number;
  config: ProviderPublic;
  key_present: boolean;
  created_at: IsoDateTime;
  updated_at: IsoDateTime;
}

export interface ProviderPublic {
  name: string;
  protocol: 'chat_completions';
  base_url: string;
  model: string;
  stream: false;
  timeouts: Timeouts;
  budget: Budget;
}

/** settings.schema.json#/$defs/periodPolicyView */
export interface PeriodPolicyView {
  period_policy_id: string;
  workspace_id: string;
  version: number;
  policy: PeriodPolicy;
  effective_to: IsoDate | null;
  created_at: IsoDateTime;
}

// -- 设置操作输入（settings.schema.json oneOf） ------------------------------

export interface WorkspaceCreateInput {
  schema_version: '1';
  operation: 'workspace_create';
  name: string;
  initial_period_policy: PeriodPolicy;
}

export interface WorkspaceUpdateInput {
  schema_version: '1';
  operation: 'workspace_update';
  workspace_id: string;
  expected_revision: number;
  patch: { name: string };
}

export interface WorkspaceUseInput {
  schema_version: '1';
  operation: 'workspace_use';
  workspace_id: string;
}

export interface PeriodPolicySetInput {
  schema_version: '1';
  operation: 'period_policy_set';
  workspace_id: string;
  expected_revision: number;
  policy: PeriodPolicy;
}

export type KeyAction = 'keep' | 'replace';

export interface ProviderUpsertInput {
  schema_version: '1';
  operation: 'provider_upsert';
  workspace_id: string;
  provider_id: string | null;
  expected_revision: number | null;
  config: ProviderPublic;
  key_action: KeyAction;
}

export interface ProviderUseInput {
  schema_version: '1';
  operation: 'provider_use';
  workspace_id: string;
  provider_id: string | null;
  expected_revision: number;
}

export interface ProviderClearKeyInput {
  schema_version: '1';
  operation: 'provider_clear_key';
  workspace_id: string;
  provider_id: string;
  expected_revision: number;
  confirm_local: true;
}

export interface ProviderDeleteInput {
  schema_version: '1';
  operation: 'provider_delete';
  workspace_id: string;
  provider_id: string;
  expected_revision: number;
  confirm_local: true;
}

// ---------------------------------------------------------------------------
// settings.schema.json#/$defs/confirmationPreview（发送预览）
// ---------------------------------------------------------------------------

export type GenerationWorkflow = 'task' | 'diagnosis';

export type GenerationOperationName =
  | 'task_generate'
  | 'diagnosis_generate'
  | 'diagnosis_continue'
  | 'generation_retry';

export interface ConfirmationSourceSummary {
  source_id: string;
  file_label: string;
  sheet: string | null;
  selected_range: string;
  row_count: number;
  content_hash: string;
}

export interface ConfirmationPreview {
  confirmation_id: string;
  expires_at: IsoDateTime;
  workspace_id: string;
  workflow: GenerationWorkflow;
  operation: GenerationOperationName;
  provider: {
    provider_id: string;
    revision: number;
    name: string;
    base_url: string;
    model: string;
  };
  run_id: string;
  analysis_revision_no: number;
  analysis_input_fingerprint: string;
  report_id: string | null;
  report_revision_no: number | null;
  input_manifest_hash: string;
  prompt_version: string;
  schema_version: string;
  context_policy_version: string | null;
  source_scope_summary: ConfirmationSourceSummary[];
  request_bytes: number;
  conservative_input_tokens: number;
  budget: Budget;
  timeouts: Timeouts;
  request_count_maximum: 1;
  key_present: true;
}

// ---------------------------------------------------------------------------
// task-edit.schema.json：任务编辑输入
// ---------------------------------------------------------------------------

export type TaskCategory = 'revenue' | 'cost' | 'efficiency' | 'other';

/** 类别 → 中文标签（task-lifecycle §4.2 固定 exact 映射）。 */
export const TASK_CATEGORY_LABELS: Record<TaskCategory, string> = {
  revenue: '销售最大化',
  cost: '费用最小化',
  efficiency: '时间最短化',
  other: '其他事项',
};

export type TaskStatus = 'not_started' | 'in_progress' | 'completed' | 'blocked';

export const TASK_STATUS_LABELS: Record<TaskStatus, string> = {
  not_started: '待开始',
  in_progress: '进行中',
  completed: '已完成',
  blocked: '有阻塞',
};

export type TaskLifecycle = 'active' | 'superseded';
export type ProgressMode = 'auto' | 'manual';
export type SmartStatus = 'generated' | 'stale';
export type CycleRole = 'review' | 'plan';

export const CYCLE_ROLE_LABELS: Record<CycleRole, string> = {
  review: '本期检视',
  plan: '下期计划',
};

export interface DateRange {
  from: IsoDate;
  to: IsoDate;
}

/**
 * 内容 patch。注意：字段缺省（不修改）与显式 null（清空）在合同上是两个值，
 * TS 用 `?: T | null` 表达。
 */
export interface TaskContentPatch {
  title?: string;
  summary?: string;
  category?: TaskCategory;
  metric_id?: string | null;
  target_value?: DecimalString | null;
  planned_value?: DecimalString | null;
  actual_value?: DecimalString | null;
  current_value?: DecimalString | null;
  unit?: string | null;
  mp_label?: string | null;
  mp_unit?: string | null;
  owner?: string | null;
  planned_start?: IsoDate | null;
  due_date?: IsoDate | null;
  resource_need?: string | null;
  success_criteria?: string | null;
  note?: string;
  review_note?: string;
  target_period?: DateRange;
}

export interface StepContent {
  action: string;
  owner: string | null;
  start_date: IsoDate | null;
  due_date: IsoDate | null;
  done_criteria: string | null;
}

export interface StepPatch {
  action?: string;
  owner?: string | null;
  start_date?: IsoDate | null;
  due_date?: IsoDate | null;
  done_criteria?: string | null;
  status?: TaskStatus;
  blocked_reason?: string | null;
}

export type StepOperation =
  | {
      op: 'add';
      client_step_key: string;
      content: StepContent;
      before_step_id: string | null;
    }
  | { op: 'update'; step_id: string; patch: StepPatch }
  | { op: 'delete'; step_id: string }
  | { op: 'move'; step_id: string; before_step_id: string | null };

export type TaskEditOperation =
  | {
      kind: 'content';
      patch?: TaskContentPatch;
      step_operations?: StepOperation[];
      /** patch 含数值字段时必填（task-lifecycle §4.2）。 */
      source_note?: string;
    }
  | {
      kind: 'progress';
      progress_mode: ProgressMode;
      /** auto 模式必须 null（服务计算）。 */
      progress: ProgressString;
      status: TaskStatus;
      /** manual 模式必填非空；auto 必须 null。 */
      adjustment_reason: string | null;
      /** status=blocked 必填非空；否则 null。 */
      blocked_reason: string | null;
    }
  | {
      kind: 'period_link';
      action: 'attach' | 'update' | 'detach';
      run_id: string;
      analysis_revision_no: number;
      cycle_role: CycleRole;
      source_period: DateRange;
      target_period: DateRange;
    };

/** task-edit.schema.json 顶层输入。 */
export interface TaskEditInput {
  schema_version: '1';
  workspace_id: string;
  task_id: string;
  expected_task_revision: number;
  mutation_id: string;
  operation: TaskEditOperation;
}

// ---------------------------------------------------------------------------
// task-reconcile.schema.json：候选合并输入
// ---------------------------------------------------------------------------

export type TaskTakeField =
  | 'title'
  | 'summary'
  | 'category'
  | 'metric_id'
  | 'baseline'
  | 'target_period'
  | 'target_value'
  | 'unit'
  | 'owner'
  | 'planned_start'
  | 'due_date'
  | 'resource_need'
  | 'success_criteria'
  | 'evidence_ids'
  | 'assumptions'
  | 'needs_review';

export type StepTakeField =
  | 'action'
  | 'owner'
  | 'start_date'
  | 'due_date'
  | 'done_criteria'
  | 'evidence_ids';

export type StepMapping =
  | { action: 'new'; candidate_step_id: string }
  | {
      action: 'update';
      candidate_step_id: string;
      existing_step_id: string;
      take_fields: StepTakeField[];
    }
  | { action: 'ignore'; candidate_step_id: string };

export type FinalStepOrderEntry =
  | { kind: 'existing'; step_id: string }
  | { kind: 'new_candidate'; candidate_step_id: string };

export type CandidateMapping =
  | { action: 'new'; candidate_task_key: string }
  | { action: 'ignore'; candidate_task_key: string }
  | {
      action: 'replace';
      candidate_task_key: string;
      existing_task_id: string;
      reason: string;
    }
  | {
      action: 'update';
      candidate_task_key: string;
      existing_task_id: string;
      take_fields: TaskTakeField[];
      step_mappings: StepMapping[];
      remove_existing_step_ids: string[];
      final_step_order: FinalStepOrderEntry[];
    };

export interface ExpectedTaskRevision {
  task_id: string;
  task_revision_no: number;
}

export interface TaskReconcileInput {
  schema_version: '1';
  workspace_id: string;
  run_id: string;
  analysis_revision_no: number;
  input_fingerprint: string;
  candidate_generation_id: string;
  candidate_revision_no: number;
  expected_tasks: ExpectedTaskRevision[];
  mutation_id: string;
  mappings: CandidateMapping[];
}

// ---------------------------------------------------------------------------
// 诊断报告（crates/dsheng-core/prompts/v1/schema/diagnosis-report.schema.json v3）
// ---------------------------------------------------------------------------

export type DiagnosisFramework = '5why' | '5w' | '5w2h';

export const DIAGNOSIS_FRAMEWORK_OPTIONS: {
  value: DiagnosisFramework;
  label: string;
  purpose: string;
}[] = [
  { value: '5why', label: '5WHY · 根因追查', purpose: '逐层追问因果关系，追查根本原因' },
  { value: '5w', label: '5W · 问题界定', purpose: '界定问题的 What/Why/Where/When/Who 背景' },
  {
    value: '5w2h',
    label: '5W2H · 问题与行动要素',
    purpose: '结构化问题与行动要素（含 How / How much）',
  },
];

export type FactStatus = 'fact' | 'user_report' | 'hypothesis' | 'unknown';

export const FACT_STATUS_LABELS: Record<FactStatus, string> = {
  fact: '事实',
  user_report: '用户陈述',
  hypothesis: '假设',
  unknown: '未知',
};

export type Confidence = 'high' | 'medium' | 'low' | 'unknown';

export const CONFIDENCE_LABELS: Record<Confidence, string> = {
  high: '高',
  medium: '中',
  low: '低',
  unknown: '未知',
};

export type FindingStatus = 'candidate' | 'needs_confirmation' | 'resolved';

export interface Finding5Why {
  finding_id: string;
  level: number; // 1..5
  question: string;
  answer: string;
  fact_status: FactStatus;
  evidence_ids: string[];
  reasoning_summary: string;
  confidence: Confidence;
  validation_question: string | null;
  status: FindingStatus;
}

export interface Finding5W {
  finding_id: string;
  dimension: 'What' | 'Why' | 'Where' | 'When' | 'Who';
  question: string;
  answer: string;
  fact_status: FactStatus;
  evidence_ids: string[];
  confidence: Confidence;
  missing_information: string[];
  clarification_questions: string[];
  status: FindingStatus;
}

export interface Finding5W2H {
  finding_id: string;
  dimension: 'What' | 'Why' | 'Who' | 'When' | 'Where' | 'How' | 'How much';
  question: string;
  answer: string;
  kind: 'observed' | 'proposal';
  fact_status: FactStatus;
  evidence_ids: string[];
  confidence: Confidence;
  missing_information: string[];
  clarification_questions: string[];
  status: FindingStatus;
}

export type DiagnosisFinding = Finding5Why | Finding5W | Finding5W2H;

export interface FactOrHypothesis {
  item_id: string;
  statement: string;
  fact_status: FactStatus;
  evidence_ids: string[];
  user_quote: string | null;
  confidence: Confidence;
  reasoning_summary: string;
}

export interface EvidenceRef {
  evidence_id: string;
  source_id: string;
  source_label: string;
  locator: string;
}

export interface OpenQuestion {
  question_id: string;
  question: string;
  status: 'open' | 'answered' | 'resolved';
  answer_summary: string | null;
}

/** warning 的解决要求（S5 展示重点）。 */
export type ResolutionRequirement = 'source_record' | 'user_confirmation';

export const RESOLUTION_REQUIREMENT_LABELS: Record<ResolutionRequirement, string> = {
  source_record: '需补充来源记录',
  user_confirmation: '需用户确认',
};

export interface DiagnosisWarning {
  warning_id: string;
  message: string;
  status: 'active' | 'resolved';
  resolution_requirement: ResolutionRequirement;
  evidence_ids: string[];
  fact_ids: string[];
  resolution_note: string | null;
}

export interface DiagnosisSource {
  source_id: string;
  file_label: string;
  sheet: string | null;
  selected_range: string;
  content_hash: string;
  selection_hash: string;
}

export interface DiagnosisReport {
  schema_version: '3';
  report_id: string;
  generation_session_id: string;
  workspace_id: string;
  run_id: string;
  framework: DiagnosisFramework;
  scope: { problem_statement: string; focus_metric_ids: string[] };
  period: { from: IsoDate | null; to: IsoDate | null; period_policy_version: string };
  revision_no: number;
  summary: string;
  source_manifest: DiagnosisSource[];
  findings: DiagnosisFinding[];
  facts_and_hypotheses: FactOrHypothesis[];
  evidence_refs: EvidenceRef[];
  clarification_questions: OpenQuestion[];
  warnings: DiagnosisWarning[];
  stop_reason: string | null;
  can_continue: boolean;
  created_at: IsoDateTime;
  updated_at: IsoDateTime;
  analysis_revision_no: number;
  analysis_input_fingerprint: string;
}

// ---------------------------------------------------------------------------
// 读侧视图（run/分析/任务/时间线/历史）。
// 说明：cli-surface.json 仅对 workspace/provider/period/confirmationPreview
// 固定了输出契约；下列 read view 按 task-lifecycle.md §7 表字段与计划 §6/§9
// 组装，字段名保持 snake_case 以便与未来核心输出对齐（合同缺口见完成报告）。
// ---------------------------------------------------------------------------

export interface ScanFileInfo {
  file_id: string;
  file_label: string;
  row_count: number;
  sheet_count: number;
  mapping_status: 'mapped' | 'pending_review';
  note: string | null;
}

export interface ScanSummaryView {
  workspace_id: string;
  directory_label: string;
  scanned_at: IsoDateTime;
  files: ScanFileInfo[];
  total_rows: number;
  coverage: { from: IsoDate; to: IsoDate; granularity: Cadence };
  pending_inputs: string[];
}

export interface FieldMappingView {
  tool_field: string;
  source_label: string;
  recognized: string;
  usage: string;
}

/** mapping show 读视图行（数据源屏字段映射表）。 */
export type FieldMappingRow = FieldMappingView;

export type ComparisonAvailability = 'available' | 'partial' | 'unavailable';

/** 比较对象（read-views comparisonView：§1.4 全字段；change_rate 为 ratio 口径）。 */
export interface ComparisonView {
  comparison_type: 'mom' | 'yoy';
  label: string;
  base_from: IsoDate;
  base_to: IsoDate;
  base_value: DecimalString | null;
  absolute_change: DecimalString | null;
  /** ratio 口径：0.153 = 15.3%，展示层自行 ×100（字符串移位）。 */
  change_rate: DecimalString | null;
  availability_status: ComparisonAvailability;
  reason: string | null;
  base_refs: { run_id: string; revision_no: number }[];
  evidence_ids: string[];
}

/** 指标行：目标/实绩/差异等数值全部为 DecimalString。 */
export interface MetricRowView {
  metric_id: string;
  metric_key: string;
  metric_label: string;
  unit: string;
  direction: 'up' | 'down' | 'neutral';
  target_value: DecimalString | null;
  actual_value: DecimalString | null;
  /** actual - target；缺任一操作数为 null。 */
  delta: DecimalString | null;
  mom: ComparisonView | null;
  yoy: ComparisonView | null;
  /**
   * 展示层本地分类（read-views 合同外字段，核心不返回）：
   * 由 UI 依据 delta/mom/yoy/availability 自行推导；Fixture 演示数据会预填。
   */
  status_tag?: 'focus' | 'watch' | 'ok' | 'incomplete';
}

export interface AnalysisRunView {
  workspace_id: string;
  run_id: string;
  period: DateRange;
  current_revision_no: number;
  analysis_input_fingerprint: string;
  grain: Cadence;
  metrics: MetricRowView[];
  /** 每指标桶序列；缺桶为 null（断线不插值）。值为 DecimalString。 */
  series: MetricSeriesView[];
}

export interface MetricSeriesView {
  metric_key: string;
  metric_label: string;
  unit: string;
  buckets: string[];
  /** 与 buckets 对齐；缺桶 null。 */
  actual: (DecimalString | null)[];
  /** 计划/目标线；无则 null。 */
  plan: (DecimalString | null)[];
  /** 逐桶差异（核心计算返回）；无则 null。 */
  delta: (DecimalString | null)[] | null;
  /** 构成分析用的分类构成（如回款构成）。 */
  composition: { name: string; value: DecimalString }[] | null;
}

export interface SmartCheckView {
  smart_status: SmartStatus;
  assessed_task_revision: number;
  checks: { dimension: 'S' | 'M' | 'A' | 'R' | 'T'; pass: boolean; note: string }[];
}

export interface TaskStepView {
  step_id: string;
  step_order: number;
  action: string;
  owner: string | null;
  start_date: IsoDate | null;
  due_date: IsoDate | null;
  done_criteria: string | null;
  status: TaskStatus;
  blocked_reason: string | null;
  completed_at: IsoDateTime | null;
}

export interface TaskEvidenceView {
  evidence_id: string;
  source_label: string;
  locator: string;
  analysis_revision_no: number;
}

export interface TaskEventView {
  event_id: string;
  task_id: string;
  task_revision_no: number;
  event_seq: number;
  kind: string;
  reason: string | null;
  actor_source: 'ui' | 'cli' | 'import' | 'generation';
  occurred_at: IsoDateTime;
}

/** 任务当前态读视图（task show 输出；数值全 DecimalString）。 */
export interface TaskView {
  workspace_id: string;
  task_id: string;
  task_revision_no: number;
  title: string;
  summary: string;
  category: TaskCategory;
  metric_id: string | null;
  metric_label: string | null;
  target_value: DecimalString | null;
  planned_value: DecimalString | null;
  actual_value: DecimalString | null;
  current_value: DecimalString | null;
  unit: string | null;
  mp_label: string | null;
  mp_unit: string | null;
  owner: string | null;
  planned_start: IsoDate | null;
  due_date: IsoDate | null;
  resource_need: string | null;
  success_criteria: string | null;
  note: string | null;
  review_note: string | null;
  target_period: DateRange | null;
  lifecycle: TaskLifecycle;
  status: TaskStatus;
  /** auto=核心按步骤计数计算；manual=人工值。均为 DecimalString。 */
  progress: DecimalString;
  progress_mode: ProgressMode;
  adjustment_reason: string | null;
  blocked_reason: string | null;
  completed_at: IsoDateTime | null;
  smart_check: SmartCheckView;
  steps: TaskStepView[];
  evidence: TaskEvidenceView[];
  events: TaskEventView[];
  origin_run_id: string | null;
  overdue: boolean;
}

export interface TaskCandidateView {
  candidate_generation_id: string;
  candidate_revision_no: number;
  run_id: string;
  analysis_revision_no: number;
  state: 'candidate' | 'applied' | 'discarded';
  /** C 方案兜底包装时为 true（根信封由本地重建）。 */
  envelope_locally_reconstructed: boolean;
  /** 候选任务（模型 task_key 仅在候选内唯一）。 */
  tasks: {
    task_key: string;
    title: string;
    summary: string;
    category: TaskCategory;
    target_value: DecimalString | null;
    owner: string | null;
    steps: { step_id: string; action: string }[];
  }[];
}

export interface TimelineEntryView {
  task_id: string;
  task_title: string;
  kind: 'planned_start' | 'due' | 'completed' | 'progress_event';
  /** null=未设置日期（不参与排序，显示“未设置”）。 */
  date: IsoDate | null;
  occurred_at: IsoDateTime | null;
  detail: string;
}

export interface HistoryEntryView {
  run_id: string;
  workspace_id: string;
  period: DateRange;
  period_key: string;
  cadence: Cadence;
  policy_version: number;
  current_revision_no: number;
  updated_at: IsoDateTime;
  summary: { metric_count: number; task_count: number; chart_count: number };
  task_snapshot_no: number;
}

export interface HistoryDetailView {
  run_id: string;
  analysis_revision_no: number;
  task_snapshot_no: number;
  captured_at: IsoDateTime;
  metrics: MetricRowView[];
  /** 图表序列（缺桶 null）；只读快照。 */
  series: MetricSeriesView[];
  grain: Cadence;
  period: DateRange;
  tasks: {
    task_id: string;
    title: string;
    status: TaskStatus;
    progress: DecimalString;
    task_revision_no: number;
  }[];
}
