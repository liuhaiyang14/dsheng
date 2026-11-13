//! 扫描 / 读取 / 字段映射 / 模板识别（数据与 SQLite 工作包所有权）。
//!
//! 合同：docs/specs/data-period-import.md §2（去重/覆盖/冲突）、§4（模板识别与
//! 字段映射）。去重/冲突域算法复用 [`crate::analysis::observations`]；
//! 本模块负责文件读取（calamine/csv）、值状态机、表头映射与模板锚点。

pub mod fileio;
pub mod headers;
pub mod scan;
pub mod template;
pub mod value;
pub mod xlsx_literal;

pub use fileio::{
    decode_utf8, parse_csv_text, read_csv, read_workbook, CellValue, ImportError, SheetData,
};
pub use headers::{
    a1, column_letter, cross_scope_sum_allowed, duplicate_columns, find_header_blocks, map_header,
    normalize_header, parse_period_header, parse_period_row, plan_blocks, strip_unit_suffix,
    BlockLayout, BlockPlan, HeaderBlock, PeriodRowLabel, FIELD_ALIASES,
};
pub use scan::{
    chart_series_from_resolved_cells, source_period, CHART_SOURCE_UNAVAILABLE,
    PERIOD_CONFIRMATION_REQUIRED,
};
pub use template::{
    detect_sheet_role, detect_sheet_role_with_names, is_generated_output_sheet_name, is_task_sheet,
    period_with_year_context, SheetMatch, SheetRole, TransposedTrendLayout,
};
pub use value::{
    classify_cell, expand_merged_anchor, normalize_money_to_yuan, parse_amount_text,
    parse_percent_text, parse_progress_cell, CellClass, MoneyUnit,
};
