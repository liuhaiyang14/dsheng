//! 任务候选本地校验（prompt-context-validation.md §2 任务目标规则、
//! 主计划 §8.3/§9.4；read-views.schema.json $defs/taskCandidateView 对齐
//! 主 Agent 2026-11-02 冻结字段）。
//!
//! 层次：C 方案提取（provider::extract）→ 任务输出 schema2（provider::schema）
//! → **本模块候选解析与本地语义校验** → 数据工作包落库
//! （步骤 status 一律重置 not_started 的合同行为由持久化层执行；
//! 本层不改动模型正文）。
//!
//! 校验项：
//! - 模型 task_key/step_id 仅候选内有效（重复键 DUPLICATE_CANDIDATE_KEY）；
//! - 类别 exact 映射（contracts::tasks::category_from_label；
//!   未知 → TASK_CATEGORY_UNRECOGNIZED 拒整份）；
//! - category/metric 关系（revenue/cost/efficiency 必须引用输入指标；
//!   other 可无）；
//! - evidence 引用必须在获准集合（本轮输入 source_records/manual_inputs/
//!   metrics/conflicts 的 evidence）；
//! - 模型 number → 规范十进制（≤6 位小数，无指数；不经 f64）；
//! - 输入侧目标候选结构（resolved↔冲突组 resolution_id 对应
//!   → INPUT_CONTRACT_INVALID）；
//! - 月度下限复核（仅完整月+monthly_revenue_floor+up：期望值 =
//!   max(合法显式、年度当月分解、去年同月)；未解决冲突不参与；
//!   显式低于下限时模型须输出上调后目标且 needs_review 非空，
//!   原 explicit 输入不被改写）。
//!
//! 纯逻辑层：不做 Provider 调用。

use crate::contracts::cli::CliErrorCode;
use crate::contracts::decimal::{model_number_to_decimal_string, parse_input_decimal};
use crate::contracts::provider::ExtractionOutcome;
use crate::contracts::tasks::{category_from_label, TaskCategory};
use crate::generation::input::{
    CandidateProjection, ConflictStatusProjection, MetricProjection, ResolutionStatusProjection,
    TaskInputProjection,
};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 候选校验错误（拒整份；不静默修正模型数字）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("candidate error: {code:?} {detail}")]
pub struct CandidateError {
    pub code: CliErrorCode,
    pub detail: String,
}

fn error(code: CliErrorCode, detail: impl Into<String>) -> CandidateError {
    CandidateError {
        code,
        detail: detail.into(),
    }
}

// ---------------------------------------------------------------------------
// 候选 typed 结构（schema2 输出对象的解析结果；正文保持模型原样）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct TaskCandidateSet {
    pub summary: String,
    pub tasks: Vec<TaskCandidate>,
    pub envelope_locally_reconstructed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskCandidate {
    pub task_key: String,
    pub title: String,
    pub summary: String,
    pub category: TaskCategory,
    /// 模型原始类别标签（exact 映射的输入；保留以供审计）。
    pub category_label: String,
    pub metric_id: Option<uuid::Uuid>,
    pub source_period: Option<(NaiveDate, NaiveDate)>,
    pub target_period: Option<(NaiveDate, NaiveDate)>,
    /// 规范十进制字符串或 None（模型 null）。
    pub baseline_value: Option<String>,
    pub baseline_unit: Option<String>,
    pub target_value: Option<String>,
    pub unit: Option<String>,
    pub owner: Option<String>,
    pub planned_start: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub steps: Vec<StepCandidate>,
    pub resource_need: Option<String>,
    pub success_criteria: Option<String>,
    pub evidence_ids: Vec<String>,
    pub assumptions: Vec<AssumptionCandidate>,
    pub needs_review: Vec<String>,
    /// SMART 中的全部 evidence 引用（引用域校验用）。
    pub smart_evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepCandidate {
    pub step_id: String,
    pub action: String,
    /// 模型输出状态原样保留（落库时由数据包重置 not_started）。
    pub status_label: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssumptionCandidate {
    pub statement: String,
    pub evidence_ids: Vec<String>,
}

// ---------------------------------------------------------------------------
// 提取结果 → 候选解析
// ---------------------------------------------------------------------------

/// 解析 C 方案提取结果为候选集。仅 `contract_matched` / `wrapped_bare_task`
/// 两阶段有对象（后者必须带 envelope_locally_reconstructed 标记）。
pub fn parse_task_output(
    extraction: &ExtractionOutcome,
) -> Result<TaskCandidateSet, CandidateError> {
    use crate::contracts::provider::ExtractionStage;
    let object = match extraction.stage {
        ExtractionStage::ContractMatched | ExtractionStage::RootShapeButInvalid => {
            extraction.object.as_ref()
        }
        ExtractionStage::WrappedBareTask => extraction.object.as_ref(),
        ExtractionStage::AmbiguousBareTasks => {
            return Err(error(
                CliErrorCode::InvalidJson,
                "多个裸任务候选，拒绝（不得猜测包装）",
            ))
        }
        ExtractionStage::NoContractCandidate => {
            return Err(error(
                CliErrorCode::InvalidJson,
                "响应中无通过任务输出合同的对象",
            ))
        }
        ExtractionStage::NoJsonObject => {
            return Err(error(CliErrorCode::InvalidJson, "响应中无可解码 JSON 对象"))
        }
    };
    let root = object.ok_or_else(|| error(CliErrorCode::InvalidJson, "提取阶段无对象"))?;
    // 类别 exact 映射先行：未知 Provider 类别整份拒绝（TASK_CATEGORY_UNRECOGNIZED），
    // 不落入通用 schema 失败码（生命周期合同 §9.4）
    for task in root["tasks"].as_array().cloned().unwrap_or_default() {
        let label = task["category"].as_str().unwrap_or_default();
        if category_from_label(label).is_none() {
            return Err(error(
                CliErrorCode::TaskCategoryUnrecognized,
                format!("未知类别标签 {label:?}（exact 映射失败，拒整份）"),
            ));
        }
    }
    // 双保险：提取后再过一次任务输出 schema2
    crate::provider::schema::validate_task_output(root)
        .map_err(|errors| error(CliErrorCode::SchemaValidationFailed, errors.join("; ")))?;

    let tasks_value = root["tasks"].as_array().cloned().unwrap_or_default();
    let mut tasks = Vec::with_capacity(tasks_value.len());
    for (idx, task) in tasks_value.iter().enumerate() {
        tasks.push(parse_task(task).map_err(|mut e| {
            e.detail = format!("tasks[{idx}]: {}", e.detail);
            e
        })?);
    }
    Ok(TaskCandidateSet {
        summary: root["summary"].as_str().unwrap_or_default().to_string(),
        tasks,
        envelope_locally_reconstructed: extraction.envelope_locally_reconstructed,
    })
}

fn parse_task(task: &serde_json::Value) -> Result<TaskCandidate, CandidateError> {
    let category_label = task["category"].as_str().unwrap_or_default().to_string();
    let category = category_from_label(&category_label).ok_or_else(|| {
        error(
            CliErrorCode::TaskCategoryUnrecognized,
            format!("未知类别标签 {category_label:?}（exact 映射失败，拒整份）"),
        )
    })?;
    let metric_id = match task["metric_id"].as_str() {
        Some(raw) => Some(
            raw.parse::<uuid::Uuid>()
                .map_err(|_| error(CliErrorCode::SchemaValidationFailed, "metric_id 非法 uuid"))?,
        ),
        None => None,
    };
    let mut steps = Vec::new();
    for step in task["steps"].as_array().cloned().unwrap_or_default() {
        steps.push(StepCandidate {
            step_id: step["step_id"].as_str().unwrap_or_default().to_string(),
            action: step["action"].as_str().unwrap_or_default().to_string(),
            status_label: step["status"].as_str().unwrap_or_default().to_string(),
            evidence_ids: string_array(&step["evidence_ids"]),
        });
    }
    let mut assumptions = Vec::new();
    for a in task["assumptions"].as_array().cloned().unwrap_or_default() {
        assumptions.push(AssumptionCandidate {
            statement: a["statement"].as_str().unwrap_or_default().to_string(),
            evidence_ids: string_array(&a["evidence_ids"]),
        });
    }
    // SMART 各准则 evidence 引用
    let mut smart_evidence_ids = Vec::new();
    let criteria = &task["smart_check"]["overall_goal"]["criteria"];
    for key in ["S", "M", "A", "R", "T"] {
        smart_evidence_ids.extend(string_array(&criteria[key]["evidence_ids"]));
    }
    if let Some(subgoals) = task["smart_check"]["subgoals"].as_array() {
        for sub in subgoals {
            for key in ["S", "M", "A", "R", "T"] {
                smart_evidence_ids.extend(string_array(&sub["criteria"][key]["evidence_ids"]));
            }
        }
    }
    Ok(TaskCandidate {
        task_key: task["task_key"].as_str().unwrap_or_default().to_string(),
        title: task["title"].as_str().unwrap_or_default().to_string(),
        summary: task["summary"].as_str().unwrap_or_default().to_string(),
        category,
        category_label,
        metric_id,
        source_period: parse_period(&task["source_period"]),
        target_period: parse_period(&task["target_period"]),
        baseline_value: parse_model_number(&task["baseline"]["value"], "baseline.value")?,
        baseline_unit: task["baseline"]["unit"].as_str().map(str::to_string),
        target_value: parse_model_number(&task["target_value"], "target_value")?,
        unit: task["unit"].as_str().map(str::to_string),
        owner: task["owner"].as_str().map(str::to_string),
        planned_start: parse_date(&task["planned_start"]),
        due_date: parse_date(&task["due_date"]),
        steps,
        resource_need: task["resource_need"].as_str().map(str::to_string),
        success_criteria: task["success_criteria"].as_str().map(str::to_string),
        evidence_ids: string_array(&task["evidence_ids"]),
        assumptions,
        needs_review: string_array(&task["needs_review"]),
        smart_evidence_ids,
    })
}

fn string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_period(value: &serde_json::Value) -> Option<(NaiveDate, NaiveDate)> {
    let from = parse_date(&value["from"])?;
    let to = parse_date(&value["to"])?;
    Some((from, to))
}

fn parse_date(value: &serde_json::Value) -> Option<NaiveDate> {
    value
        .as_str()
        .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
}

/// 模型 number 边界：词法 → 规范十进制（≤6 位小数、无指数、28 位有效）。
fn parse_model_number(
    value: &serde_json::Value,
    field: &str,
) -> Result<Option<String>, CandidateError> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(n) => {
            let lexeme = model_number_to_decimal_string(n).ok_or_else(|| {
                error(
                    CliErrorCode::SemanticValidationFailed,
                    format!("{field} 数值词法非规范十进制（含指数或越界）"),
                )
            })?;
            let decimal = parse_input_decimal(&lexeme).map_err(|e| {
                error(
                    CliErrorCode::SemanticValidationFailed,
                    format!("{field} 数值不满足 decimal 合同: {e}"),
                )
            })?;
            Ok(Some(crate::contracts::decimal::to_canonical_string(
                decimal,
            )))
        }
        _ => Err(error(
            CliErrorCode::SemanticValidationFailed,
            format!("{field} 必须是 number 或 null"),
        )),
    }
}

// ---------------------------------------------------------------------------
// 本地语义校验 + taskCandidateView（read-views.schema.json 冻结字段）
// ---------------------------------------------------------------------------

/// 候选身份（视图层字段；由调用方从生成记录取得）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateIdentity {
    pub candidate_generation_id: uuid::Uuid,
    pub candidate_revision_no: u32,
    pub run_id: String,
    pub analysis_revision_no: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateState {
    Candidate,
    Applied,
    Discarded,
}

/// read-views.schema.json $defs/taskCandidateView（additionalProperties=false）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCandidateView {
    pub candidate_generation_id: uuid::Uuid,
    pub candidate_revision_no: u32,
    pub run_id: String,
    pub analysis_revision_no: u32,
    pub state: CandidateState,
    /// C 方案本地包装标记：wrapped_bare_task 必须为 true。
    pub envelope_locally_reconstructed: bool,
    pub tasks: Vec<TaskCandidateViewEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCandidateViewEntry {
    pub task_key: String,
    pub title: String,
    pub summary: String,
    pub category: TaskCategory,
    /// 规范十进制字符串或 null。
    pub target_value: Option<String>,
    pub owner: Option<String>,
    pub steps: Vec<TaskCandidateViewStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCandidateViewStep {
    /// 模型局部 step_id（永久身份由应用管理）。
    pub step_id: String,
    pub action: String,
}

/// 获准 evidence 集合（本轮输入投影）。
pub fn approved_evidence_ids(input: &TaskInputProjection) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for record in &input.source_records {
        ids.insert(record.evidence_id.clone());
    }
    for manual in &input.manual_inputs {
        for id in &manual.evidence_ids {
            ids.insert(id.clone());
        }
    }
    for metric in &input.metrics {
        for id in &metric.evidence_ids {
            ids.insert(id.clone());
        }
        for candidate in [
            &metric.explicit_target,
            &metric.annual_allocated_target,
            &metric.prior_year_actual,
        ]
        .into_iter()
        .flatten()
        {
            for id in &candidate.evidence_ids {
                ids.insert(id.clone());
            }
        }
    }
    for conflict in &input.source_conflicts {
        for value in &conflict.candidate_values {
            ids.insert(value.evidence_id.clone());
        }
    }
    ids
}

/// 输入侧目标候选结构校验：conflict_status=resolved 的候选其 resolution_id
/// 必须对应同指标的已解决冲突组；none/unresolved 必须为 null。
pub fn validate_input_metric_candidates(input: &TaskInputProjection) -> Result<(), CandidateError> {
    for metric in &input.metrics {
        for (name, candidate) in [
            ("explicit_target", &metric.explicit_target),
            ("annual_allocated_target", &metric.annual_allocated_target),
            ("prior_year_actual", &metric.prior_year_actual),
        ] {
            if let Some(c) = candidate {
                check_candidate_resolution(c, &metric.metric_id.to_string(), name, input)?;
            }
        }
    }
    Ok(())
}

fn check_candidate_resolution(
    c: &CandidateProjection,
    metric_id: &str,
    name: &str,
    input: &TaskInputProjection,
) -> Result<(), CandidateError> {
    match c.conflict_status {
        ConflictStatusProjection::Resolved => {
            let id = c.resolution_id.as_deref().ok_or_else(|| {
                error(
                    CliErrorCode::InputContractInvalid,
                    format!("{name}: resolved 候选缺 resolution_id"),
                )
            })?;
            let matched = input.source_conflicts.iter().any(|conflict| {
                conflict.resolution_status == ResolutionStatusProjection::Resolved
                    && conflict.metric_id.to_string() == metric_id
                    && conflict
                        .resolution_id
                        .map(|rid| rid.to_string() == id)
                        .unwrap_or(false)
            });
            if !matched {
                return Err(error(
                    CliErrorCode::InputContractInvalid,
                    format!("{name}: resolution_id 不对应任何已解决冲突组"),
                ));
            }
        }
        ConflictStatusProjection::None | ConflictStatusProjection::Unresolved => {
            if c.resolution_id.is_some() {
                return Err(error(
                    CliErrorCode::InputContractInvalid,
                    format!("{name}: 非 resolved 候选 resolution_id 必须为 null"),
                ));
            }
        }
    }
    Ok(())
}

/// 候选集本地语义校验并生成 taskCandidateView。
///
/// 拒绝路径：类别未知/重复键/引用越界/数值非规范/月度下限不符 → 拒整份，
/// 不静默修正。
pub fn validate_candidates(
    set: &TaskCandidateSet,
    input: &TaskInputProjection,
    identity: &CandidateIdentity,
) -> Result<TaskCandidateView, CandidateError> {
    // 输入侧结构先校验（resolved↔冲突组）
    validate_input_metric_candidates(input)?;

    let evidence_domain = approved_evidence_ids(input);
    let metric_domain: BTreeSet<String> = input
        .metrics
        .iter()
        .map(|m| m.metric_id.to_string())
        .collect();

    // task_key 候选内唯一
    let mut task_keys = BTreeSet::new();
    for task in &set.tasks {
        if !task_keys.insert(task.task_key.clone()) {
            return Err(error(
                CliErrorCode::DuplicateCandidateKey,
                format!("重复 task_key {}", task.task_key),
            ));
        }
        // 目标期间绑定本轮获准窗口，不能通过改成别月或部分月绕过月度下限。
        if let Some(period) = task.target_period {
            if period != (input.period.target_from, input.period.target_to) {
                return Err(error(
                    CliErrorCode::SemanticValidationFailed,
                    format!("任务 {} 的目标期间与本轮批准窗口不一致", task.task_key),
                ));
            }
        }
        // category/metric 关系
        if task.category != TaskCategory::Other {
            let metric_id = task.metric_id.ok_or_else(|| {
                error(
                    CliErrorCode::TaskCategoryMetricMismatch,
                    format!(
                        "任务 {}（{}）必须关联指标",
                        task.task_key, task.category_label
                    ),
                )
            })?;
            if !metric_domain.contains(&metric_id.to_string()) {
                return Err(error(
                    CliErrorCode::TaskCategoryMetricMismatch,
                    format!(
                        "任务 {} 引用的 metric {} 不在本轮输入指标集合",
                        task.task_key, metric_id
                    ),
                ));
            }
        }
        // step_id 任务内唯一
        let mut step_ids = BTreeSet::new();
        for step in &task.steps {
            if !step_ids.insert(step.step_id.clone()) {
                return Err(error(
                    CliErrorCode::DuplicateCandidateKey,
                    format!("任务 {} 内重复 step_id {}", task.task_key, step.step_id),
                ));
            }
        }
        // evidence 引用域
        let mut referenced = task.evidence_ids.clone();
        referenced.extend(task.smart_evidence_ids.iter().cloned());
        for step in &task.steps {
            referenced.extend(step.evidence_ids.iter().cloned());
        }
        for assumption in &task.assumptions {
            referenced.extend(assumption.evidence_ids.iter().cloned());
        }
        for id in &referenced {
            if !evidence_domain.contains(id) {
                return Err(error(
                    CliErrorCode::SemanticValidationFailed,
                    format!(
                        "任务 {} 引用 evidence {id} 超出获准集合（跨 session/未知 ID）",
                        task.task_key
                    ),
                ));
            }
        }
        // 月度下限复核
        check_monthly_floor(task, input)?;
    }

    Ok(TaskCandidateView {
        candidate_generation_id: identity.candidate_generation_id,
        candidate_revision_no: identity.candidate_revision_no,
        run_id: identity.run_id.clone(),
        analysis_revision_no: identity.analysis_revision_no,
        state: CandidateState::Candidate,
        envelope_locally_reconstructed: set.envelope_locally_reconstructed,
        tasks: set
            .tasks
            .iter()
            .map(|t| TaskCandidateViewEntry {
                task_key: t.task_key.clone(),
                title: t.title.clone(),
                summary: t.summary.clone(),
                category: t.category,
                target_value: t.target_value.clone(),
                owner: t.owner.clone(),
                steps: t
                    .steps
                    .iter()
                    .map(|s| TaskCandidateViewStep {
                        step_id: s.step_id.clone(),
                        action: s.action.clone(),
                    })
                    .collect(),
            })
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// 月度下限复核（§2）
// ---------------------------------------------------------------------------

/// 目标窗口是否完整月周期（包含自定义月起始日及短月末日钳制）。
pub fn is_full_month(period: Option<(NaiveDate, NaiveDate)>) -> bool {
    period.is_some_and(|(from, to)| {
        (1..=31).any(|day| {
            let expected = crate::analysis::periods::monthly_period_span(from, day);
            expected.from == from && expected.to == to
        })
    })
}

/// 候选数值是否可参与下限计算（可比且无未解决冲突）。
fn eligible_for_floor(c: &CandidateProjection) -> bool {
    c.comparable && c.conflict_status != ConflictStatusProjection::Unresolved
}

fn candidate_value(c: &CandidateProjection) -> Option<rust_decimal::Decimal> {
    model_number_to_decimal_string(&c.value).and_then(|s| parse_input_decimal(&s).ok())
}

/// 月度下限复核：模型 target_value 必须等于期望值
/// max(合法显式、年度当月分解、去年同月)；未解决冲突不参与；
/// 显式低于下限时输出上调值且 needs_review 非空（"候选上调待采纳"），
/// 原 explicit 输入不被改写；非月度/费用/耗时/none 不套下限，
/// 仅显式确认目标可填值。
pub fn check_monthly_floor(
    task: &TaskCandidate,
    input: &TaskInputProjection,
) -> Result<(), CandidateError> {
    let Some(metric_id) = task.metric_id else {
        if task.category != TaskCategory::Other {
            return Ok(()); // 关系错误在 validate_candidates 已拒
        }
        // other 类不套月度下限；目标只能显式（此处无指标即无显式目标）
        return check_explicit_only(task, None);
    };
    let metric = input
        .metrics
        .iter()
        .find(|m| m.metric_id == metric_id)
        .expect("引用域已校验");

    use crate::generation::input::TargetConstraintScopeProjection as Scope;
    match metric.target_constraint_scope {
        Scope::MonthlyRevenueFloor => {
            let approved_month = input.period.target_grain
                == crate::generation::input::CadenceProjection::Month
                && task.target_period == Some((input.period.target_from, input.period.target_to))
                && is_full_month(task.target_period);
            if !approved_month {
                // 非获准完整月周期不套月度下限 → 仅显式目标
                return check_explicit_only(task, Some(metric));
            }
            let mut candidates: Vec<(&str, rust_decimal::Decimal)> = Vec::new();
            if let Some(explicit) = &metric.explicit_target {
                if eligible_for_floor(explicit) {
                    if let Some(v) = candidate_value(explicit) {
                        candidates.push(("explicit", v));
                    }
                }
            }
            if let Some(annual) = &metric.annual_allocated_target {
                if eligible_for_floor(annual) {
                    if let Some(v) = candidate_value(annual) {
                        candidates.push(("annual", v));
                    }
                }
            }
            if let Some(prior) = &metric.prior_year_actual {
                if eligible_for_floor(prior) {
                    if let Some(v) = candidate_value(prior) {
                        candidates.push(("prior", v));
                    }
                }
            }
            let expected = candidates
                .iter()
                .map(|(_, v)| *v)
                .max()
                .map(crate::contracts::decimal::to_canonical_string);
            match expected {
                Some(expected) => {
                    let actual = task.target_value.as_deref().ok_or_else(|| {
                        error(
                            CliErrorCode::SemanticValidationFailed,
                            format!(
                                "任务 {}：存在可用下限/合法显式目标（{expected}），模型不得留空",
                                task.task_key
                            ),
                        )
                    })?;
                    if actual != expected {
                        return Err(error(
                            CliErrorCode::SemanticValidationFailed,
                            format!(
                                "任务 {}：月度下限复核失败（期望 {expected}，模型 {actual}；未解决冲突不参与）",
                                task.task_key
                            ),
                        ));
                    }
                    // 显式低于下限 → 候选上调需 needs_review（原输入不改写）
                    let explicit_value = metric
                        .explicit_target
                        .as_ref()
                        .filter(|c| eligible_for_floor(c))
                        .and_then(candidate_value);
                    if let Some(explicit) = explicit_value {
                        let floor = candidates
                            .iter()
                            .filter(|(name, _)| *name != "explicit")
                            .map(|(_, v)| *v)
                            .max();
                        if floor.is_some_and(|f| f > explicit) && task.needs_review.is_empty() {
                            return Err(error(
                                CliErrorCode::SemanticValidationFailed,
                                format!(
                                    "任务 {}：候选上调待采纳（显式 {explicit} 低于下限），needs_review 不得为空",
                                    task.task_key
                                ),
                            ));
                        }
                    }
                    Ok(())
                }
                None => {
                    // 无可用下限且无合法显式 → 目标必须 null
                    if task.target_value.is_some() {
                        return Err(error(
                            CliErrorCode::SemanticValidationFailed,
                            format!(
                                "任务 {}：无合法目标依据（候选不可比或未解决冲突），目标必须为 null",
                                task.task_key
                            ),
                        ));
                    }
                    Ok(())
                }
            }
        }
        Scope::ExplicitTargetOnly | Scope::None => check_explicit_only(task, Some(metric)),
    }
}

/// 非月度下限路径：仅明确确认且可比无冲突的目标可填值，无则 null；
/// 不用往年/年度数字推低或推高，不自行 min/max。
fn check_explicit_only(
    task: &TaskCandidate,
    metric: Option<&MetricProjection>,
) -> Result<(), CandidateError> {
    let explicit = metric
        .and_then(|m| m.explicit_target.as_ref())
        .filter(|c| eligible_for_floor(c))
        .and_then(candidate_value)
        .map(crate::contracts::decimal::to_canonical_string);
    match (explicit, task.target_value.as_deref()) {
        (Some(expected), Some(actual)) => {
            if actual != expected {
                return Err(error(
                    CliErrorCode::SemanticValidationFailed,
                    format!(
                        "任务 {}：仅可使用明确目标 {expected}，模型输出 {actual}（不得推算）",
                        task.task_key
                    ),
                ));
            }
            Ok(())
        }
        (Some(_), None) => Err(error(
            CliErrorCode::SemanticValidationFailed,
            format!("任务 {}：存在明确确认目标，模型不得留空", task.task_key),
        )),
        (None, Some(actual)) => Err(error(
            CliErrorCode::SemanticValidationFailed,
            format!(
                "任务 {}：无明确目标依据（不可比/冲突/缺省），目标必须为 null（模型输出 {actual}）",
                task.task_key
            ),
        )),
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试（候选解析与本地校验，非 Provider 调用）。
    use super::*;

    #[test]
    fn full_month_detection() {
        assert!(is_full_month(Some((
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        ))));
        assert!(is_full_month(Some((
            NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            NaiveDate::from_ymd_opt(2024, 2, 29).unwrap(),
        ))));
        assert!(!is_full_month(Some((
            NaiveDate::from_ymd_opt(2026, 10, 2).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 31).unwrap(),
        ))));
        assert!(!is_full_month(Some((
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 30).unwrap(),
        ))));
        assert!(!is_full_month(Some((
            NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 30).unwrap(),
        ))));
        assert!(!is_full_month(None));
    }
}
