//! 扫描 / 读取 / 字段映射 / 模板识别（数据与 SQLite 工作包所有权）。
//!
//! 合同：docs/specs/data-period-import.md §2（去重/覆盖/冲突）、§4（模板识别与
//! 字段映射）。去重/冲突域算法复用 [`crate::analysis::observations`]；
//! 本模块负责文件读取（calamine/csv）、值状态机、表头映射与模板锚点。

pub mod headers;
pub mod scan;
pub mod value;

pub use headers::{
    column_letter, cross_scope_sum_allowed, duplicate_columns, normalize_header, plan_blocks,
    BlockLayout, BlockPlan,
};
pub use scan::{
    chart_series_from_resolved_cells, source_period, CHART_SOURCE_UNAVAILABLE,
    PERIOD_CONFIRMATION_REQUIRED,
};
pub use value::{
    classify_cell, expand_merged_anchor, normalize_money_to_yuan, parse_amount_text,
    parse_percent_text, parse_progress_cell, CellClass, MoneyUnit,
};
