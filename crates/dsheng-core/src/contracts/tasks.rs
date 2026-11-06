//! 任务编辑/候选合并/状态类型（task-edit.schema.json、task-reconcile.schema.json、
//! task-lifecycle.md §1/§4/§5）。

use crate::contracts::domain::DateRange;
use serde::{Deserialize, Serialize};

/// 任务类别规范码（UI 显示：销售最大化/费用最小化/时间最短化/其他事项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    Revenue,
    Cost,
    Efficiency,
    Other,
}

/// 类别 exact 标签映射（lifecycle §4.2；仅首尾空白清理，不做模糊匹配）。
pub fn category_from_label(raw: &str) -> Option<TaskCategory> {
    match raw.trim() {
        "revenue" | "收入最大" | "收入改善" | "销售最大化" => {
            Some(TaskCategory::Revenue)
        }
        "cost" | "费用最小" | "费用控制" | "费用最小化" => Some(TaskCategory::Cost),
        "efficiency" | "时间最短" | "时间/效率改善" | "时间最短化" => {
            Some(TaskCategory::Efficiency)
        }
        "other" | "其他事项" => Some(TaskCategory::Other),
        _ => None,
    }
}

impl TaskCategory {
    pub fn display_label(self) -> &'static str {
        match self {
            TaskCategory::Revenue => "销售最大化",
            TaskCategory::Cost => "费用最小化",
            TaskCategory::Efficiency => "时间最短化",
            TaskCategory::Other => "其他事项",
        }
    }
}

/// 执行状态（与记录生命周期分开）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    NotStarted,
    InProgress,
    Completed,
    Blocked,
}

/// 记录生命周期。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycle {
    Active,
    Superseded,
}

/// 进度模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressMode {
    Auto,
    Manual,
}

/// SMART 校核状态（generated=生成时评价；stale=内容已改待复核）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmartStatus {
    Generated,
    Stale,
}

/// 跨期角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleRole {
    Review,
    Plan,
}

/// 操作者来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorSource {
    Ui,
    Cli,
    Import,
    Generation,
}

/// 人工创建任务：期间由已保存分析及所选角色决定，不接受模型候选身份。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskCreateInput {
    pub schema_version: String,
    pub workspace_id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub analysis_revision_no: u32,
    pub input_fingerprint: String,
    pub mutation_id: uuid::Uuid,
    pub cycle_role: CycleRole,
    pub source_note: Option<String>,
    pub content: TaskContentPatch,
    pub steps: Vec<StepContent>,
}

/// task-edit.schema.json 顶层输入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskEditInput {
    pub schema_version: String,
    pub workspace_id: uuid::Uuid,
    pub task_id: uuid::Uuid,
    pub expected_task_revision: u32,
    pub mutation_id: uuid::Uuid,
    pub operation: TaskEditOperation,
}

/// task-edit operation oneOf。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskEditOperation {
    Content {
        #[serde(default, deserialize_with = "deserialize_boxed_patch")]
        patch: Option<Box<TaskContentPatch>>,
        step_operations: Option<Vec<StepOperation>>,
        /// patch 含数值字段时必填。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_note: Option<String>,
    },
    Progress {
        progress_mode: ProgressMode,
        /// auto 模式必须 null（服务计算）。
        progress: Option<String>,
        status: TaskStatus,
        /// manual 模式必填非空；auto 必须 null。
        adjustment_reason: Option<String>,
        /// status=blocked 必填非空；否则 null。
        blocked_reason: Option<String>,
    },
    PeriodLink {
        action: PeriodLinkAction,
        run_id: uuid::Uuid,
        analysis_revision_no: u32,
        cycle_role: CycleRole,
        source_period: DateRange,
        target_period: DateRange,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodLinkAction {
    Attach,
    Update,
    Detach,
}

/// 内容 patch（全字段可选，minProperties 1 在 JSON 层校验）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct TaskContentPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<TaskCategory>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric_id: Option<uuid::Uuid>,
    /// 规范十进制字符串或 null。
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub target_value: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub planned_value: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub actual_value: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub current_value: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub unit: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub mp_label: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub mp_unit: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub owner: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub planned_start: Option<Option<chrono::NaiveDate>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub due_date: Option<Option<chrono::NaiveDate>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub resource_need: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub success_criteria: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub note: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_period: Option<DateRange>,
}

impl TaskContentPatch {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// 数值变更需要 source_note（lifecycle §4.2）。
    pub fn touches_numbers(&self) -> bool {
        self.target_value.is_some()
            || self.planned_value.is_some()
            || self.actual_value.is_some()
            || self.current_value.is_some()
    }
}

/// 步骤操作 oneOf。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepOperation {
    Add {
        client_step_key: String,
        content: StepContent,
        before_step_id: Option<uuid::Uuid>,
    },
    Update {
        step_id: uuid::Uuid,
        patch: StepPatch,
    },
    Delete {
        step_id: uuid::Uuid,
    },
    Move {
        step_id: uuid::Uuid,
        before_step_id: Option<uuid::Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepContent {
    pub action: String,
    pub owner: Option<String>,
    pub start_date: Option<chrono::NaiveDate>,
    pub due_date: Option<chrono::NaiveDate>,
    pub done_criteria: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct StepPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub owner: Option<Option<String>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_date: Option<Option<chrono::NaiveDate>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub due_date: Option<Option<chrono::NaiveDate>>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub done_criteria: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskStatus>,
    #[serde(
        default,
        deserialize_with = "deserialize_nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub blocked_reason: Option<Option<String>>,
}

/// task-reconcile.schema.json 顶层输入。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskReconcileInput {
    pub schema_version: String,
    pub workspace_id: uuid::Uuid,
    pub run_id: uuid::Uuid,
    pub analysis_revision_no: u32,
    pub input_fingerprint: String,
    pub candidate_generation_id: uuid::Uuid,
    pub candidate_revision_no: u32,
    pub expected_tasks: Vec<ExpectedTaskRevision>,
    pub mutation_id: uuid::Uuid,
    pub mappings: Vec<CandidateMapping>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedTaskRevision {
    pub task_id: uuid::Uuid,
    pub task_revision_no: u32,
}

/// 候选映射 oneOf（new/ignore/replace/update）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum CandidateMapping {
    New {
        candidate_task_key: String,
    },
    Ignore {
        candidate_task_key: String,
    },
    Replace {
        candidate_task_key: String,
        existing_task_id: uuid::Uuid,
        reason: String,
    },
    Update {
        candidate_task_key: String,
        existing_task_id: uuid::Uuid,
        take_fields: Vec<TaskTakeField>,
        step_mappings: Vec<StepMapping>,
        remove_existing_step_ids: Vec<uuid::Uuid>,
        final_step_order: Vec<FinalStepOrderEntry>,
    },
}

/// update 的替换白名单字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskTakeField {
    Title,
    Summary,
    Category,
    MetricId,
    Baseline,
    TargetPeriod,
    TargetValue,
    Unit,
    Owner,
    PlannedStart,
    DueDate,
    ResourceNeed,
    SuccessCriteria,
    EvidenceIds,
    Assumptions,
    NeedsReview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepTakeField {
    Action,
    Owner,
    StartDate,
    DueDate,
    DoneCriteria,
    EvidenceIds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepMapping {
    New {
        candidate_step_id: String,
    },
    Update {
        candidate_step_id: String,
        existing_step_id: uuid::Uuid,
        take_fields: Vec<StepTakeField>,
    },
    Ignore {
        candidate_step_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FinalStepOrderEntry {
    Existing { step_id: uuid::Uuid },
    NewCandidate { candidate_step_id: String },
}

/// 缺省字段保留 None，显式 null 反序列化为 Some(None)，供部分更新执行清空。
fn deserialize_nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// serde 辅助：透明装箱，不改变 wire 格式。
fn deserialize_boxed_patch<'de, D>(
    deserializer: D,
) -> Result<Option<Box<TaskContentPatch>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<TaskContentPatch> = Option::deserialize(deserializer)?;
    Ok(opt.map(Box::new))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_exact_mapping() {
        assert_eq!(
            category_from_label("销售最大化"),
            Some(TaskCategory::Revenue)
        );
        assert_eq!(
            category_from_label(" 费用最小化 "),
            Some(TaskCategory::Cost)
        );
        assert_eq!(
            category_from_label("时间最短化"),
            Some(TaskCategory::Efficiency)
        );
        assert_eq!(category_from_label("其他事项"), Some(TaskCategory::Other));
        assert_eq!(category_from_label("收入"), None, "未知类别不默认 other");
        assert_eq!(category_from_label("销售 最大"), None, "不做模糊匹配");
    }

    #[test]
    fn parse_task_edit_json() {
        let json = r#"{
            "schema_version": "1",
            "workspace_id": "00000000-0000-4000-8000-000000000001",
            "task_id": "00000000-0000-4000-8000-000000000002",
            "expected_task_revision": 3,
            "mutation_id": "00000000-0000-4000-8000-000000000003",
            "operation": {"kind": "content", "patch": {"title": "复核供应计划"}}
        }"#;
        let input: TaskEditInput = serde_json::from_str(json).expect("SC-01 样例应可解析");
        assert!(matches!(input.operation, TaskEditOperation::Content { .. }));
    }

    #[test]
    fn reject_unknown_fields() {
        let json = r#"{
            "schema_version": "1",
            "workspace_id": "00000000-0000-4000-8000-000000000001",
            "task_id": "00000000-0000-4000-8000-000000000002",
            "expected_task_revision": 3,
            "mutation_id": "00000000-0000-4000-8000-000000000003",
            "operation": {"kind": "progress", "progress_mode": "manual",
                          "progress": "50", "status": "in_progress",
                          "adjustment_reason": "两步并一步", "blocked_reason": null,
                          "unexpected": true}
        }"#;
        assert!(serde_json::from_str::<TaskEditInput>(json).is_err());
    }
}
