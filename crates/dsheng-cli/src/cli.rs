//! clap 命令树：覆盖 contracts/v1/cli-surface.json 全部 54 条命令。
//!
//! 设计要点：
//! - `--json` 为全局 flag（stdout 单 envelope 的开关；进度/诊断走 stderr）。
//! - 未知 flag 由 clap 拒绝 → 映射 INVALID_ARGUMENT（exit 2）。
//! - 双门禁 flag（--dry-run / --confirmation-id + --confirm-send）互斥性由
//!   命令层再校验（clap group 不足以表达"必须成对出现"）。
//! - `--db` 显式数据库路径（默认 DSHENG_DB/平台数据目录；不进任何导出）。

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "dsheng",
    about = "dsheng 两表一单本地经营分析工具 CLI（合同版本 1）",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// 输出 JSON envelope（stdout 单对象；进度/诊断仅 stderr）
    #[arg(long, global = true)]
    pub json: bool,

    /// SQLite 数据库路径（默认 DSHENG_DB 或平台数据目录）
    #[arg(long, global = true)]
    pub db: Option<String>,

    /// 输出版本契约（版本/合同版本/平台）
    #[arg(long)]
    pub version: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    // ---- workspace / 周期策略（6） ----
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    Provider {
        #[command(subcommand)]
        action: ProviderAction,
    },
    Scan {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        dir: String,
    },
    Mapping {
        #[command(subcommand)]
        action: MappingAction,
    },
    ManualInput {
        #[command(subcommand)]
        action: ManualInputAction,
    },
    Metric {
        #[command(subcommand)]
        action: MetricAction,
    },
    SourceConflict {
        #[command(subcommand)]
        action: SourceConflictAction,
    },
    Analyze {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        dir: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        grain: AnalyzeGrain,
        #[arg(long)]
        policy: Option<String>,
        #[arg(long, default_value_t = false)]
        recalculate: bool,
        #[arg(long)]
        input_json: Option<String>,
    },
    Analysis {
        #[command(subcommand)]
        action: AnalysisAction,
    },
    Diagnosis {
        #[command(subcommand)]
        action: DiagnosisAction,
    },
    Generation {
        #[command(subcommand)]
        action: GenerationAction,
    },
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },
    History {
        #[command(subcommand)]
        action: HistoryAction,
    },
    Export(ExportArgs),
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum WorkspaceAction {
    List,
    Create {
        #[arg(long)]
        input_json: String,
    },
    Update {
        #[arg(long)]
        workspace: String,
        #[arg(long)]
        input_json: String,
    },
    Use {
        #[arg(long)]
        workspace: String,
    },
    Period {
        #[command(subcommand)]
        action: PeriodAction,
    },
}

#[derive(Subcommand, Debug)]
pub enum PeriodAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Set {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        input_json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProviderAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Set {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        input_json: String,
        #[arg(long, default_value_t = false)]
        api_key_stdin: bool,
    },
    Use {
        #[arg(long)]
        workspace: Option<String>,
        /// provider UUID 或 "none"（清除默认）
        #[arg(long)]
        provider: String,
        #[arg(long)]
        expected_revision: u32,
    },
    ClearKey {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long, default_value_t = false)]
        confirm_local: bool,
    },
    Delete {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        provider: String,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long, default_value_t = false)]
        confirm_local: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum MappingAction {
    Show {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
    },
    Set {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        input_json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum ManualInputAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        period_key: String,
    },
    Set {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        period_key: String,
        #[arg(long)]
        input_json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum MetricAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Set {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        input_json: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SourceConflictAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
    },
    Resolve {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        input_json: String,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalyzeGrain {
    Auto,
    Day,
    Week,
    Month,
    Quarter,
}

#[derive(Subcommand, Debug)]
pub enum AnalysisAction {
    Show {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        revision: Option<u32>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameworkArg {
    #[value(name = "5why")]
    Why5,
    #[value(name = "5w")]
    W5,
    #[value(name = "5w2h")]
    W5h2,
}

#[derive(Subcommand, Debug)]
pub enum DiagnosisAction {
    Generate {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        framework: FrameworkArg,
        #[arg(long)]
        problem: String,
        #[arg(long)]
        focus_metric: Option<String>,
        #[arg(long = "evidence", num_args = 1..)]
        evidence: Vec<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        confirmation_id: Option<String>,
        #[arg(long, default_value_t = false)]
        confirm_send: bool,
    },
    Continue {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        session: String,
        #[arg(long, default_value_t = false)]
        answers_stdin: bool,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        confirmation_id: Option<String>,
        #[arg(long, default_value_t = false)]
        confirm_send: bool,
    },
    Revise {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        session: String,
        #[arg(long)]
        fact: String,
        #[arg(long, default_value_t = false)]
        replacement_stdin: bool,
    },
    Pending {
        #[command(subcommand)]
        action: DiagnosisPendingAction,
    },
    List {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
    },
    Show {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        report: String,
        #[arg(long)]
        revision: Option<u32>,
    },
    Export {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        report: String,
        #[arg(long)]
        revision: Option<u32>,
        #[arg(long)]
        format: ExportDocFormat,
        #[arg(long)]
        out: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum DiagnosisPendingAction {
    Cancel {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        session: String,
        #[arg(long)]
        turn: String,
        #[arg(long)]
        expected_revision: u32,
        #[arg(long, default_value_t = false)]
        confirm_local: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum GenerationAction {
    Retry {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        attempt: String,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        confirmation_id: Option<String>,
        #[arg(long, default_value_t = false)]
        confirm_send: bool,
    },
    Status {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        attempt: String,
    },
    Cancel {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        attempt: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum TaskAction {
    /// 人工新增任务，仅写本地数据，不调用模型。
    Create {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        input_json: String,
    },
    Generate {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        diagnosis_report: Option<String>,
        #[arg(long)]
        provider: Option<String>,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        confirmation_id: Option<String>,
        #[arg(long, default_value_t = false)]
        confirm_send: bool,
    },
    List {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: Option<String>,
    },
    Show {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        task: String,
    },
    Update {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        task: String,
        #[arg(long)]
        patch_json: Option<String>,
        // CLI sugar（aliases 注记：规范化到同一 task-edit 合同，仍需 expected revision）
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        progress: Option<String>,
        #[arg(long)]
        progress_mode: Option<String>,
        #[arg(long)]
        adjustment_reason: Option<String>,
        #[arg(long)]
        blocked_reason: Option<String>,
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        expected_revision: Option<u32>,
        #[arg(long)]
        mutation_id: Option<String>,
    },
    Step {
        #[command(subcommand)]
        action: StepAction,
    },
    Reconcile {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        input_json: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long, default_value_t = false)]
        apply: bool,
    },
    Timeline {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        grain: TimelineGrain,
    },
}

#[derive(Subcommand, Debug)]
pub enum StepAction {
    Update {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        step: String,
        #[arg(long)]
        patch_json: Option<String>,
        #[arg(long)]
        expected_revision: Option<u32>,
        #[arg(long)]
        mutation_id: Option<String>,
    },
    Add {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        task: String,
        #[arg(long)]
        input_json: String,
    },
    Delete {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        step: String,
        #[arg(long)]
        expected_revision: u32,
    },
    Move {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        step: String,
        /// 目标位置之前的步骤 ID；"end" 表示末尾
        #[arg(long)]
        before: String,
        #[arg(long)]
        expected_revision: u32,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineGrain {
    Day,
    Week,
    Month,
    Quarter,
}

#[derive(Subcommand, Debug)]
pub enum HistoryAction {
    List {
        #[arg(long)]
        workspace: Option<String>,
    },
    Show {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        revision: Option<u32>,
    },
    Revisions {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
    },
    Compare {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long)]
        revision: Option<u32>,
        #[arg(long = "type")]
        compare_type: CompareType,
    },
    Delete {
        #[arg(long)]
        workspace: Option<String>,
        #[arg(long)]
        run: String,
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        #[arg(long)]
        confirm_delete: Option<String>,
    },
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompareType {
    Mom,
    Yoy,
}

/// export：--bundle / --workbook / --meeting / --chart 四选一（cli-surface 语法）。
#[derive(clap::Args, Debug)]
pub struct ExportArgs {
    #[arg(long)]
    pub workspace: Option<String>,
    #[arg(long)]
    pub run: String,
    #[arg(long)]
    pub revision: Option<u32>,
    #[arg(long)]
    pub bundle: Option<String>,
    #[arg(long)]
    pub workbook: Option<WorkbookKind>,
    #[arg(long, default_value_t = false)]
    pub meeting: bool,
    #[arg(long)]
    pub chart: Option<String>,
    #[arg(long)]
    pub format: Option<ExportFormat>,
    #[arg(long)]
    pub out: String,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbookKind {
    Accounting,
    Trend,
    Task,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
    Csv,
    Xlsx,
    Md,
    Json,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportDocFormat {
    Md,
    Json,
    Pdf,
}

#[derive(Subcommand, Debug)]
pub enum TemplateAction {
    Download {
        #[arg(long)]
        package: String,
        #[arg(long)]
        out: String,
    },
}
