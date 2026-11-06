//! 模型输入合同 v2 的 typed 投影（task-input / diagnosis-input schema）。
//!
//! 字段以 `contracts/v1/task-input.schema.json` 与
//! `contracts/v1/diagnosis-input.schema.json` 为准（冻结合同，本文件只做
//! Rust 结构映射，不重新选择字段/枚举）。serde 序列化结果必须通过
//! `provider::schema::validate_task_input` / `validate_diagnosis_input`
//! （见 tests/wp_provider_assemble.rs）。
//!
//! 数值边界：模型 number 走 `serde_json::Number`（workspace 已启用
//! `arbitrary_precision`，词法保留、不经 f64）；规范十进制字符串字段
//! （manualInput.value 等）在构造侧用 contracts::decimal 校验。
//!
//! context_policy_version=2 的会话状态（session_state / continuation_input /
//! pending）由本模块承载 wire 结构；发送前跨字段硬校验见
//! generation::context（S6）。

use crate::contracts::domain::UnitSignature;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Number;

// ---------------------------------------------------------------------------
// 共享 $defs（两份输入合同的公共投影）
// ---------------------------------------------------------------------------

/// 工作区投影（$defs/workspace；cadence 含 custom）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceProjection {
    pub workspace_id: String,
    pub industry: Option<String>,
    pub cadence: CadenceProjection,
    pub timezone: String,
    pub metric_definitions: Vec<MetricDefinitionProjection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CadenceProjection {
    Day,
    Week,
    Month,
    Quarter,
    Custom,
}

/// 指标定义投影（$defs/metricDefinition，全部 16 字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricDefinitionProjection {
    pub metric_id: uuid::Uuid,
    pub metric_key: String,
    pub version: u32,
    pub semantic_hash: String,
    pub label: String,
    pub semantic_type: SemanticTypeProjection,
    pub unit_signature: UnitSignature,
    pub direction: DirectionProjection,
    pub target_constraint_scope: TargetConstraintScopeProjection,
    pub revenue_semantics: bool,
    pub aggregation: AggregationProjection,
    pub formula: Option<FormulaAst>,
    pub required_inputs: Vec<String>,
    pub denominator_policy: DenominatorPolicyProjection,
    pub comparison_policy: ComparisonPolicyProjection,
    pub effective_from: NaiveDate,
    pub scope_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticTypeProjection {
    Flow,
    Stock,
    Ratio,
    ConfiguredDerived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DirectionProjection {
    Up,
    Down,
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetConstraintScopeProjection {
    MonthlyRevenueFloor,
    ExplicitTargetOnly,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregationProjection {
    SumDisjoint,
    ConfiguredRecompute,
    RecomputeAfterAggregation,
    RatioOfAggregatedInputs,
    OpeningBoundary,
    ClosingBalanceOrRollforward,
    ClosingBoundary,
    SumItemClosingValues,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenominatorPolicyProjection {
    PositiveOnly,
}

/// 公式 AST（$defs/formulaAst：ref / const / 二元运算）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum FormulaAst {
    Ref { metric_key: String },
    Const { value: String },
    Add { args: [Box<FormulaAst>; 2] },
    Sub { args: [Box<FormulaAst>; 2] },
    Mul { args: [Box<FormulaAst>; 2] },
    Div { args: [Box<FormulaAst>; 2] },
}

/// 比较口径投影（$defs/comparison_policy，两字段均为 const 值）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonPolicyProjection {
    pub positive_base: PositiveBaseProjection,
    pub zero_or_negative_base: ZeroOrNegativeBaseProjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PositiveBaseProjection {
    #[serde(rename = "relative_change")]
    RelativeChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZeroOrNegativeBaseProjection {
    #[serde(rename = "absolute_only")]
    AbsoluteOnly,
}

/// 数值候选（$defs/candidate；resolved↔resolution_id 对应关系由 schema allOf 保证）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateProjection {
    pub value: Number,
    pub unit: String,
    pub evidence_ids: Vec<String>,
    pub comparable: bool,
    pub conflict_status: ConflictStatusProjection,
    pub resolution_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStatusProjection {
    None,
    Unresolved,
    Resolved,
}

/// 指标投影（$defs/metric）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetricProjection {
    pub metric_id: uuid::Uuid,
    pub label: String,
    pub unit: String,
    pub direction: DirectionProjection,
    pub target_constraint_scope: TargetConstraintScopeProjection,
    pub explicit_target: Option<CandidateProjection>,
    pub annual_allocated_target: Option<CandidateProjection>,
    pub prior_year_actual: Option<CandidateProjection>,
    pub target: Option<Number>,
    pub actual: Option<Number>,
    pub delta: Option<Number>,
    pub mom: Option<ComparisonProjection>,
    pub yoy: Option<ComparisonProjection>,
    pub evidence_ids: Vec<String>,
}

/// 同比/环比投影（$defs/comparison）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonProjection {
    pub comparison_type: ComparisonTypeProjection,
    pub label: String,
    pub base_from: NaiveDate,
    pub base_to: NaiveDate,
    pub base_value: Option<Number>,
    pub absolute_change: Option<Number>,
    pub change_rate: Option<Number>,
    pub availability_status: AvailabilityStatusProjection,
    pub reason: Option<ComparisonReasonProjection>,
    pub base_refs: Vec<BaseRefProjection>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonTypeProjection {
    Mom,
    Yoy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatusProjection {
    Available,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonReasonProjection {
    ZeroBase,
    NegativeBase,
    SourceConflict,
    ConfigurationRequired,
    UnsupportedFormula,
    PrecisionUnverified,
    InvalidInput,
    DefinitionMismatch,
    UnitMismatch,
    ScopeMismatch,
    CalendarMismatch,
    CoverageIncomplete,
    MissingCurrent,
    MissingBase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BaseRefProjection {
    pub run_id: uuid::Uuid,
    pub revision_no: u32,
}

/// 来源清单条目（$defs/sourceManifest）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceManifestEntry {
    pub source_id: uuid::Uuid,
    pub file_label: String,
    pub sheet: Option<String>,
    pub selected_range: String,
    pub content_hash: String,
    pub selection_hash: String,
}

/// 来源记录（$defs/sourceRecord；evidence_id 形如 cell:<uuid>:<sheet>:<A1>）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRecordProjection {
    pub evidence_id: String,
    pub source_id: uuid::Uuid,
    pub file_label: String,
    pub sheet: Option<String>,
    pub cell_or_row: String,
    pub values: SourceCellValues,
}

/// 单元格值投影（$defs/sourceCellValues）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCellValues {
    pub cell_type: CellTypeProjection,
    pub raw_text: String,
    pub typed_value: Option<TypedCellValue>,
    pub formula: Option<String>,
    pub value_state: ValueStateProjection,
    pub number_format: Option<String>,
    pub merged_range: Option<String>,
    pub mapping: Option<SourceCellMapping>,
}

/// typed_value：string | boolean | null（schema 限定）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TypedCellValue {
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellTypeProjection {
    Text,
    Number,
    Formula,
    Boolean,
    Error,
    Blank,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueStateProjection {
    Valid,
    Missing,
    PendingInput,
    Invalid,
    Conflict,
    ConfigurationRequired,
    UnsupportedFormula,
    PrecisionUnverified,
}

/// 单元格映射投影（$defs/sourceCellMapping）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCellMapping {
    pub metric_id: uuid::Uuid,
    pub field_key: String,
    pub measure_role: MeasureRoleProjection,
    pub period: String,
    pub scope_key: String,
    pub unit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureRoleProjection {
    Actual,
    Plan,
    AnnualMonthPlan,
}

/// 手工输入投影（$defs/manualInput；value 为规范十进制字符串或 null）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManualInputProjection {
    pub input_id: uuid::Uuid,
    pub metric_id: uuid::Uuid,
    pub value: Option<String>,
    pub unit: Option<String>,
    pub confirmed: bool,
    pub source_note: String,
    pub evidence_ids: Vec<String>,
    pub input_revision: u32,
    pub measure_role: MeasureRoleProjection,
    /// `YYYY-MM-DD/YYYY-MM-DD` 或 null。
    pub period: Option<String>,
    pub scope_key: String,
    pub raw_text: String,
}

/// 缺失/异常提示（$defs/anomaly）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnomalyProjection {
    pub code: String,
    pub metric_id: Option<String>,
    pub message: String,
    pub evidence_ids: Vec<String>,
}

/// 来源冲突投影（$defs/conflict）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConflictProjection {
    pub conflict_id: uuid::Uuid,
    pub metric_id: uuid::Uuid,
    pub measure_role: MeasureRoleProjection,
    pub period: String,
    pub scope_key: String,
    pub unit: String,
    pub candidate_values: Vec<ConflictCandidateValue>,
    pub resolution_status: ResolutionStatusProjection,
    pub resolution_id: Option<uuid::Uuid>,
    pub selected_evidence_id: Option<String>,
    pub resolution_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConflictCandidateValue {
    pub value: Number,
    pub evidence_id: String,
    pub source_label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatusProjection {
    Unresolved,
    Resolved,
}

/// 报告事实/假设引用（$defs/reportFact；optional_diagnosis_reference 与
/// 诊断续问共用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReportFactProjection {
    pub item_id: String,
    pub statement: String,
    pub fact_status: FactStatusProjection,
    pub evidence_ids: Vec<String>,
    pub user_quote: Option<String>,
    pub confidence: ConfidenceProjection,
    pub reasoning_summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactStatusProjection {
    Fact,
    UserReport,
    Hypothesis,
    Unknown,
}

/// 已采纳事实（$defs/acceptedFact；fact_status 固定 user_report）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedFactProjection {
    pub fact_id: String,
    pub source_turn_id: String,
    pub source_turn_no: u32,
    pub user_quote: String,
    pub normalized_statement: String,
    pub open_question_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub fact_status: UserReportOnly,
    pub confidence: ConfidenceProjection,
}

/// fact_status const "user_report" 的标记类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserReportOnly {
    #[serde(rename = "user_report")]
    UserReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceProjection {
    High,
    Medium,
    Low,
    Unknown,
}

/// 澄清问题（$defs/question）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClarificationQuestionProjection {
    pub question_id: String,
    pub question: String,
    pub why_needed: String,
    pub evidence_ids: Vec<String>,
}

/// pending_clarification（$defs/pending：最近一个未解决 needs_clarification turn）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingClarificationProjection {
    pub turn_id: String,
    pub user_message: String,
    pub clarification_questions: Vec<ClarificationQuestionProjection>,
    pub redirect_message: Option<String>,
    pub stop_reason: Option<String>,
}

/// 续问输入（$defs/continuation；restate 必须绑定 pending turn）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContinuationInputProjection {
    pub mode: ContinuationMode,
    pub answer_to_turn_id: Option<String>,
    pub selected_open_question_ids: Vec<String>,
    pub user_message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContinuationMode {
    Append,
    Restate,
}

/// 诊断未解决问题投影（session_state.unresolved_questions；status 固定 open）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnresolvedQuestionProjection {
    pub question_id: String,
    pub question: String,
    pub status: OpenOnly,
    pub answer_summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpenOnly {
    #[serde(rename = "open")]
    Open,
}

// ---------------------------------------------------------------------------
// 任务输入（dsheng-task-input-v2）
// ---------------------------------------------------------------------------

/// 任务生成模型输入（顶层）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInputProjection {
    pub request: TaskRequestProjection,
    pub workspace: WorkspaceProjection,
    pub metrics: Vec<MetricProjection>,
    pub source_manifest: Vec<SourceManifestEntry>,
    pub source_records: Vec<SourceRecordProjection>,
    pub manual_inputs: Vec<ManualInputProjection>,
    pub missing_or_anomalous_inputs: Vec<AnomalyProjection>,
    pub source_conflicts: Vec<SourceConflictProjection>,
    pub period: TaskPeriodProjection,
    pub business_review: BusinessReviewProjection,
    pub task_skill_context: TaskSkillContextProjection,
    pub optional_diagnosis_reference: Option<OptionalDiagnosisReference>,
    pub user_context: Option<String>,
    pub output_contract: TaskOutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRequestProjection {
    pub input_schema_version: InputSchemaVersion,
    pub run_id: String,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
    pub prompt_version: TaskPromptVersion,
    pub workflow: TaskWorkflowMarker,
    pub schema_version: TaskSchemaVersion,
}

/// const "2"（输入合同版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputSchemaVersion {
    #[serde(rename = "2")]
    V2,
}

/// const "2.5"（任务 prompt 版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskPromptVersion {
    #[serde(rename = "2.5")]
    V25,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskWorkflowMarker {
    #[serde(rename = "task")]
    Task,
}

/// const "2"（任务输出 schema 版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskSchemaVersion {
    #[serde(rename = "2")]
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskPeriodProjection {
    pub source_from: NaiveDate,
    pub source_to: NaiveDate,
    pub target_from: NaiveDate,
    pub target_to: NaiveDate,
    pub target_grain: CadenceProjection,
    pub period_policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BusinessReviewProjection {
    pub summary: String,
    pub prior_task_outcomes: Vec<PriorTaskOutcome>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PriorTaskOutcome {
    pub task_id: String,
    pub title: String,
    pub status: TaskOutcomeStatus,
    pub progress: Number,
    pub outcome_note: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcomeStatus {
    NotStarted,
    InProgress,
    Completed,
    Blocked,
}

/// task_skill_context：只有规则版本与结构元数据，不含 rules 正文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskSkillContextProjection {
    pub rules_version: TaskPromptVersion,
    pub required_fields: RequiredFieldsConst,
    pub category_options: CategoryOptionsConst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RequiredFieldsConst;

impl Serialize for RequiredFieldsConst {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        const FIELDS: [&str; 20] = [
            "task_key",
            "title",
            "summary",
            "category",
            "metric_id",
            "source_period",
            "target_period",
            "baseline",
            "target_value",
            "unit",
            "owner",
            "planned_start",
            "due_date",
            "steps",
            "resource_need",
            "success_criteria",
            "evidence_ids",
            "assumptions",
            "needs_review",
            "smart_check",
        ];
        s.collect_seq(FIELDS)
    }
}

impl<'de> Deserialize<'de> for RequiredFieldsConst {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        const FIELDS: [&str; 20] = [
            "task_key",
            "title",
            "summary",
            "category",
            "metric_id",
            "source_period",
            "target_period",
            "baseline",
            "target_value",
            "unit",
            "owner",
            "planned_start",
            "due_date",
            "steps",
            "resource_need",
            "success_criteria",
            "evidence_ids",
            "assumptions",
            "needs_review",
            "smart_check",
        ];
        let expected = serde_json::to_value(FIELDS).map_err(serde::de::Error::custom)?;
        if v == expected {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("required_fields 不等于冻结常量"))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CategoryOptionsConst;

impl Serialize for CategoryOptionsConst {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(["revenue", "cost", "efficiency", "other"])
    }
}

impl<'de> Deserialize<'de> for CategoryOptionsConst {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        let expected = serde_json::json!(["revenue", "cost", "efficiency", "other"]);
        if v == expected {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("category_options 不等于冻结常量"))
        }
    }
}

/// 可选诊断引用（§3：同 run/revision/指纹一致才可引用；无引用必须 null）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionalDiagnosisReference {
    pub report_id: String,
    pub report_revision_no: u32,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
    pub framework: FrameworkProjection,
    pub summary: String,
    pub facts: Vec<ReportFactProjection>,
    pub hypotheses: Vec<ReportFactProjection>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkProjection {
    #[serde(rename = "5why")]
    Why5,
    #[serde(rename = "5w")]
    W5,
    #[serde(rename = "5w2h")]
    W5h2,
}

impl From<crate::contracts::provider::Framework> for FrameworkProjection {
    fn from(f: crate::contracts::provider::Framework) -> Self {
        match f {
            crate::contracts::provider::Framework::Why5 => Self::Why5,
            crate::contracts::provider::Framework::W5 => Self::W5,
            crate::contracts::provider::Framework::W5h2 => Self::W5h2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskOutputContract {
    pub schema: TaskOutputSchemaId,
    pub schema_version: TaskSchemaVersion,
}

/// const "task-output-v2"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TaskOutputSchemaId {
    #[serde(rename = "task-output-v2")]
    TaskOutputV2,
}

// ---------------------------------------------------------------------------
// 诊断输入（dsheng-diagnosis-input-v2）
// ---------------------------------------------------------------------------

/// 诊断（首轮/续问）模型输入（顶层）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisInputProjection {
    pub request: DiagnosisRequestProjection,
    pub workspace: WorkspaceProjection,
    pub metrics: Vec<MetricProjection>,
    pub source_manifest: Vec<SourceManifestEntry>,
    pub source_records: Vec<SourceRecordProjection>,
    pub manual_inputs: Vec<ManualInputProjection>,
    pub missing_or_anomalous_inputs: Vec<AnomalyProjection>,
    pub source_conflicts: Vec<SourceConflictProjection>,
    pub period: DiagnosisPeriodProjection,
    pub scope: ScopeProjection,
    pub user_message: String,
    pub continuation_input: Option<ContinuationInputProjection>,
    pub session_state: SessionStateProjection,
    pub output_contract: DiagnosisOutputContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisRequestProjection {
    pub input_schema_version: InputSchemaVersion,
    pub run_id: String,
    pub analysis_revision_no: u32,
    pub analysis_input_fingerprint: String,
    pub prompt_version: DiagnosisPromptVersion,
    pub workflow: DiagnosisWorkflowMarker,
    pub report_id: Option<String>,
    pub generation_session_id: String,
    pub round_no: u32,
    pub framework: FrameworkProjection,
    pub schema_version: DiagnosisSchemaVersion,
    pub user_confirmed_continue: bool,
    pub context_policy_version: ContextPolicyVersion,
}

/// const "2.1"（诊断 prompt 版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosisPromptVersion {
    #[serde(rename = "2.1")]
    V21,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosisWorkflowMarker {
    #[serde(rename = "diagnosis")]
    Diagnosis,
}

/// const "3"（诊断 turn/report schema 版本）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosisSchemaVersion {
    #[serde(rename = "3")]
    V3,
}

/// const "2"（context_policy_version）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ContextPolicyVersion {
    #[serde(rename = "2")]
    V2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisPeriodProjection {
    pub source_from: NaiveDate,
    pub source_to: NaiveDate,
    pub period_policy_version: String,
}

/// 诊断范围（每份报告锁定一个主问题）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeProjection {
    pub problem_statement: String,
    pub focus_metric_ids: Vec<String>,
    pub framework: FrameworkProjection,
    pub open_question_ids: Vec<String>,
}

/// session_state（context_policy_version=2：无 prior_relevant_turn_summaries）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionStateProjection {
    pub accepted_facts: Vec<AcceptedFactProjection>,
    /// 完整 schema3 报告对象（续问携带完整当前报告）；首轮 null。
    pub prior_report_summary: Option<serde_json::Value>,
    pub unresolved_questions: Vec<UnresolvedQuestionProjection>,
    pub pending_clarification: Option<PendingClarificationProjection>,
}

/// output_contract.schema 取所选 turn schema $id。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosisOutputContract {
    pub schema: DiagnosisTurnSchemaId,
    pub schema_version: DiagnosisSchemaVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosisTurnSchemaId {
    #[serde(rename = "diagnosis-turn-5why-v3")]
    Why5,
    #[serde(rename = "diagnosis-turn-5w-v3")]
    W5,
    #[serde(rename = "diagnosis-turn-5w2h-v3")]
    W5h2,
}

impl From<crate::contracts::provider::Framework> for DiagnosisTurnSchemaId {
    fn from(f: crate::contracts::provider::Framework) -> Self {
        match f {
            crate::contracts::provider::Framework::Why5 => Self::Why5,
            crate::contracts::provider::Framework::W5 => Self::W5,
            crate::contracts::provider::Framework::W5h2 => Self::W5h2,
        }
    }
}
