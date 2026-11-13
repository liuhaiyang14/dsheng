//! 导出实现（CLI/导出工作包所有权：src-tauri/src/export/**）。
//!
//! 合同：data-period-import.md §5（三工作簿/会议 10 页/模板包/公式注入防护）。
//! 桌面与 CLI 复用同一导出逻辑：本目录为**纯库**（只依赖 dsheng-core 只读
//! 合同类型、rust_xlsxwriter、zip、png、serde），所有文件以 `super::` 互引，
//! 既可由桌面壳 `pub mod export;` 挂载，也可由 CLI 经 `#[path]` 挂载同一源文件。
//!
//! 输入：[`ExportData`]（read-views 形状的 JSON 视图＋导出元数据）。输出：
//! - 业务包 ZIP（三工作簿＋结构化 JSON）；
//! - 单工作簿（核算/推移/任务）；
//! - 会议版 10 页 md/json；
//! - 图表 png/csv/xlsx；
//! - source profile 输入模板包与固定 two_tables_v1 两表输出模板包；生成结果不回灌为输入。
//!
//! 不变量：文本以 `=`、`+`、`-`、`@` 开头写文本类型（CSV 单引号转义）；
//! 超过 15 位有效数写 decimal 文本并标 `number_storage=decimal_text`；
//! API Key 永不进导出；元数据不含本机绝对路径。

pub mod bundle;
pub mod cells;
pub mod chart;
pub mod meeting;
pub mod template;
pub mod workbook;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 导出错误（映射 CLI EXPORT_FAILED）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportError(pub String);

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExportError {}

pub type ExportResult<T> = Result<T, ExportError>;

/// 导出元数据（§5：workspace/run/revision/as_of、policy、指标 hash、
/// source manifests、基期 run/revision、过滤器；不写本机绝对路径/Key）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportMeta {
    /// 所有业务结果固定使用 two_tables_v1；source profile 只影响输入适配。
    pub output_contract_id: String,
    pub workspace_id: String,
    pub workspace_name: String,
    pub run_id: String,
    pub revision_no: u32,
    pub as_of: String,
    pub analysis_input_fingerprint: String,
    pub policy: PolicyMeta,
    pub currency: Option<String>,
    pub tax_basis: Option<String>,
    pub source_from: String,
    pub source_to: String,
    pub target_from: Option<String>,
    pub target_to: Option<String>,
    pub grain: String,
    pub coverage_status: String,
    pub empty_days_are_zero: bool,
    /// metric_key → semantic_hash。
    pub metric_hashes: Vec<MetricHashMeta>,
    pub source_manifests: Vec<SourceManifestMeta>,
    pub base_refs: Vec<BaseRefMeta>,
    pub filters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyMeta {
    pub version: String,
    pub cadence: String,
    pub timezone_id: String,
    pub week_start_day: u32,
    #[serde(default = "default_month_start_day")]
    pub month_start_day: u32,
    pub fiscal_year_start_month: u32,
}

fn default_month_start_day() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricHashMeta {
    pub metric_key: String,
    pub version: u32,
    pub semantic_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceManifestMeta {
    pub source_id: String,
    pub file_label: String,
    pub sheet: Option<String>,
    pub selected_range: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaseRefMeta {
    pub run_id: String,
    pub revision_no: u32,
}

/// 导出输入：read-views 形状的 JSON 视图（metricRowView / metricSeriesView /
/// taskView）＋证据行＋可选诊断报告。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportData {
    pub meta: ExportMeta,
    /// metricRowView[]。
    pub metrics: Vec<Value>,
    /// metricSeriesView[]。
    pub series: Vec<Value>,
    /// taskView[]（本期检视＝source_period 内；下期计划＝target_period 内）。
    pub tasks: Vec<Value>,
    /// 任务依据行：evidence_id/metric_label/source_period/target_period/
    /// file_label/sheet/cell_or_row/comparison_type/base_period/base_value/
    /// current_value/absolute_change/change_rate/pending_reason。
    pub evidence: Vec<Value>,
    /// 诊断报告快照（可选；第 4/10 页引用）。
    pub diagnosis: Option<Value>,
}

/// 模板版本常量（§5：第 1 行 template_version=1）。
pub const TEMPLATE_VERSION: &str = "1";

/// 任务 sheet 固定名（§5；完整期间放第 3 行）。
pub const SHEET_TASK_REVIEW: &str = "任务单检视";
pub const SHEET_TASK_PLAN: &str = "任务单计划";
pub const SHEET_TASK_EVIDENCE: &str = "任务依据";
pub const SHEET_ACCOUNTING: &str = "核算表";
pub const SHEET_TREND: &str = "推移表";
pub const SHEET_DIFF: &str = "差异分析";
pub const SHEET_CHART: &str = "图表";

/// 字符串辅助：JSON 值 → 展示文本（null → 空）。
pub(crate) fn text_of(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

/// decimal 字符串（read-views decimalString）或 null → Option<String>。
pub(crate) fn decimal_of(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
