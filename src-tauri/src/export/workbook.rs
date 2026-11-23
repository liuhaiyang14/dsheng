//! 三份业务工作簿（§5 布局逐格）：核算表/推移表/任务单。
//!
//! 业务 sheet 的标题、说明、表头、冻结窗格与备注行遵循 two_tables_v1；
//! 运行元数据集中放在末尾隐藏的 `_metadata` sheet。任务 sheet 保留永久
//! task_id/step_id 技术列（隐藏），多步骤按 task_id 展开。

use super::cells::{
    write_headers, write_opt_decimal, write_opt_display_decimal, write_text, DecimalTextLog,
    DisplayKind,
};
use super::{
    decimal_of, text_of, ExportData, ExportError, ExportResult, SHEET_ACCOUNTING, SHEET_CHART,
    SHEET_DIFF, SHEET_TASK_EVIDENCE, SHEET_TASK_PLAN, SHEET_TASK_REVIEW, SHEET_TREND,
    TEMPLATE_VERSION,
};
use rust_xlsxwriter::{Chart, ChartType, Format, FormatAlign, Workbook, Worksheet};
use serde_json::Value;

/// 工作簿种类（cli-surface export --workbook）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbookKind {
    Accounting,
    Trend,
    Task,
}

impl WorkbookKind {
    pub fn file_name(self) -> &'static str {
        match self {
            Self::Accounting => "核算表.xlsx",
            Self::Trend => "推移表.xlsx",
            Self::Task => "任务单.xlsx",
        }
    }
}

pub const ACCOUNTING_HEADERS: [&str; 12] = [
    "序号",
    "科目",
    "计算公式 / 取数来源",
    "本月MP",
    "年度计划·当月目标",
    "月度预定",
    "销售额占比(预定)",
    "月度实绩",
    "销售额占比(实绩)",
    "预实差异额",
    "实绩达成预定比",
    "实绩达成年度计划比",
];
pub const TREND_HEADERS: [&str; 14] = [
    "月份",
    "总销售额(元)",
    "采购成本(元)",
    "毛利额(元)",
    "毛利率",
    "经营费用(元)",
    "人工费(元)",
    "结算收益(元)",
    "利润率",
    "单位时间附加值(元/h)",
    "费用构成比(经营费用/销售)",
    "预定实绩差异额",
    "实绩达成预定比",
    "实绩达成年度计划比",
];
pub const DIFF_HEADERS: [&str; 6] = [
    "分析维度",
    "预定/目标",
    "实绩",
    "差异",
    "状态",
    "说明与对策方向",
];
pub const CHART_HEADERS: [&str; 7] = [
    "chart_id",
    "图表类型",
    "metric_key",
    "from",
    "to",
    "grain",
    "source_refs",
];

pub const TASK_REVIEW_HEADERS: [&str; 16] = [
    "类别",
    "任务名称",
    "指标",
    "单位",
    "目标值",
    "实际值",
    "行动方案",
    "责任人",
    "开始日期",
    "截止日期",
    "状态",
    "进度",
    "检视说明",
    "任务依据",
    "task_id",
    "step_id",
];
pub const TASK_PLAN_HEADERS: [&str; 15] = [
    "类别",
    "任务名称",
    "指标",
    "单位",
    "目标值",
    "行动方案",
    "责任人",
    "开始日期",
    "截止日期",
    "资源需求",
    "完成标准",
    "待确认项",
    "任务依据",
    "task_id",
    "step_id",
];
pub const EVIDENCE_HEADERS: [&str; 14] = [
    "依据ID",
    "指标",
    "来源期间",
    "目标期间",
    "来源文件标签",
    "工作表",
    "单元格/行",
    "比较类型",
    "基期",
    "基期值",
    "本期值",
    "绝对变化",
    "变化率",
    "待确认原因",
];

/// 数据起始行（0 基：第 7 行）。
pub const DATA_ROW: u32 = 4;
/// 表头行（0 基：第 6 行）。
pub const HEADER_ROW: u32 = 3;

/// 写元数据块（§5 精确位置；空元数据留空）。
pub fn write_metadata(ws: &mut Worksheet, data: &ExportData) -> ExportResult<()> {
    let m = &data.meta;
    let pairs: [(u32, u16, &str, String); 15] = [
        (
            0,
            0,
            "dsheng_template_version",
            TEMPLATE_VERSION.to_string(),
        ),
        (1, 0, "workspace_name", m.workspace_name.clone()),
        (1, 2, "currency", m.currency.clone().unwrap_or_default()),
        (1, 4, "tax_basis", m.tax_basis.clone().unwrap_or_default()),
        (2, 0, "source_from", m.source_from.clone()),
        (2, 2, "source_to", m.source_to.clone()),
        (2, 4, "grain", m.grain.clone()),
        (
            2,
            10,
            "target_from",
            m.target_from.clone().unwrap_or_default(),
        ),
        (2, 12, "target_to", m.target_to.clone().unwrap_or_default()),
        (3, 0, "timezone", m.policy.timezone_id.clone()),
        (3, 2, "week_start_day", m.policy.week_start_day.to_string()),
        (
            3,
            10,
            "month_start_day",
            m.policy.month_start_day.to_string(),
        ),
        (
            3,
            4,
            "fiscal_year_start_month",
            m.policy.fiscal_year_start_month.to_string(),
        ),
        (3, 6, "coverage_status", m.coverage_status.clone()),
        (
            3,
            8,
            "empty_days_are_zero",
            m.empty_days_are_zero.to_string(),
        ),
    ];
    for (row, col, key, value) in pairs {
        write_text(ws, row, col, key)?;
        if !value.is_empty() {
            write_text(ws, row, col + 1, &value)?;
        }
    }
    Ok(())
}

/// 空白模板元数据（键有值空；A1/B1 固定）。
pub fn write_blank_metadata(ws: &mut Worksheet) -> ExportResult<()> {
    let keys: [(u32, u16, &str); 15] = [
        (0, 0, "dsheng_template_version"),
        (1, 0, "workspace_name"),
        (1, 2, "currency"),
        (1, 4, "tax_basis"),
        (2, 0, "source_from"),
        (2, 2, "source_to"),
        (2, 4, "grain"),
        (2, 10, "target_from"),
        (2, 12, "target_to"),
        (3, 0, "timezone"),
        (3, 2, "week_start_day"),
        (3, 4, "fiscal_year_start_month"),
        (3, 6, "coverage_status"),
        (3, 8, "empty_days_are_zero"),
        (3, 10, "month_start_day"),
    ];
    for (row, col, key) in keys {
        write_text(ws, row, col, key)?;
    }
    write_text(ws, 0, 1, TEMPLATE_VERSION)?;
    Ok(())
}

fn ratio_text(numerator: Option<&str>, denominator: Option<&str>) -> Option<String> {
    use rust_decimal::Decimal;
    let n = Decimal::from_str_exact(numerator?).ok()?;
    let d = Decimal::from_str_exact(denominator?).ok()?;
    dsheng_core::analysis::engine::positive_div(n, d)
        .ok()
        .map(dsheng_core::contracts::decimal::to_canonical_string)
}

const ACCOUNTING_COLS: u16 = 12;
const TREND_COLS: u16 = 14;
const TASK_HEADER_ROW: u32 = 5;
const TASK_DATA_ROW: u32 = 6;

fn heading_text(data: &ExportData, kind: WorkbookKind) -> (String, String) {
    let workspace = if data.meta.workspace_name.trim().is_empty() {
        "工作区"
    } else {
        data.meta.workspace_name.trim()
    };
    let period = if data.meta.source_from.is_empty() || data.meta.source_to.is_empty() {
        data.meta.source_to.clone()
    } else if data.meta.source_from == data.meta.source_to {
        data.meta.source_to.clone()
    } else {
        format!("{} — {}", data.meta.source_from, data.meta.source_to)
    };
    let title = match kind {
        WorkbookKind::Accounting => format!("{workspace} — {period}整体核算表"),
        WorkbookKind::Trend => format!("{workspace} — 月度推移表（{period}）"),
        WorkbookKind::Task => format!("{workspace} — 任务单"),
    };
    let description = format!(
        "口径：{}｜来源期间：{}至{}｜粒度：{}｜覆盖：{}｜输出契约：{}",
        data.meta.tax_basis.as_deref().unwrap_or("未指定"),
        data.meta.source_from,
        data.meta.source_to,
        data.meta.grain,
        data.meta.coverage_status,
        data.meta.output_contract_id
    );
    (title, description)
}

/// 写与模板一致的标题、说明和冻结窗格。业务 sheet 不再占用元数据行，
/// 运行元数据集中写入最后的隐藏 `_metadata` 技术 sheet。
pub fn write_sheet_heading(
    ws: &mut Worksheet,
    data: &ExportData,
    kind: WorkbookKind,
    last_col: u16,
    freeze_col: u16,
) -> ExportResult<()> {
    let (title, description) = heading_text(data, kind);
    let title_format = Format::new()
        .set_bold()
        .set_font_size(14)
        .set_font_color("1F3864")
        .set_align(FormatAlign::Left)
        .set_text_wrap();
    let note_format = Format::new().set_text_wrap().set_align(FormatAlign::Left);
    ws.merge_range(0, 0, 0, last_col, &title, &title_format)
        .map_err(|e| ExportError(format!("合并标题失败: {e}")))?;
    ws.merge_range(1, 0, 1, last_col, &description, &note_format)
        .map_err(|e| ExportError(format!("合并说明失败: {e}")))?;
    ws.set_row_height(0, 24.0)
        .map_err(|e| ExportError(e.to_string()))?;
    ws.set_row_height(1, 36.0)
        .map_err(|e| ExportError(e.to_string()))?;
    ws.set_freeze_panes(4, freeze_col)
        .map_err(|e| ExportError(format!("冻结窗格失败: {e}")))?;
    Ok(())
}

fn style_headers(ws: &mut Worksheet, last_col: u16) -> ExportResult<()> {
    let format = Format::new()
        .set_bold()
        .set_font_color("FFFFFF")
        .set_background_color("1F3864")
        .set_align(FormatAlign::Center)
        .set_text_wrap();
    ws.set_range_format(HEADER_ROW, 0, HEADER_ROW, last_col, &format)
        .map_err(|e| ExportError(format!("设置表头格式失败: {e}")))?;
    ws.set_row_height(HEADER_ROW, 34.0)
        .map_err(|e| ExportError(e.to_string()))?;
    Ok(())
}

fn section_format() -> Format {
    Format::new()
        .set_bold()
        .set_background_color("EAF2F8")
        .set_align(FormatAlign::Left)
}

fn metric_key(v: &Value) -> String {
    text_of(&v["metric_key"])
}

fn key_matches(key: &str, aliases: &[&str]) -> bool {
    aliases.iter().any(|candidate| *candidate == key)
}

fn metric_section(metric: &Value) -> String {
    for field in ["section", "section_name", "group", "category"] {
        if let Some(value) = metric[field].as_str().filter(|s| !s.trim().is_empty()) {
            return value.to_string();
        }
    }
    match metric["semantic_type"].as_str() {
        Some("configured_derived") => "派生指标".into(),
        Some("ratio") => "效率与比率指标".into(),
        Some("stock") => "存量指标".into(),
        Some("flow") => "业务指标".into(),
        _ => "指标".into(),
    }
}

/// 将 profile 提供的指标放入模板预留的业务区。这里只使用稳定 metric_key/语义，
/// 不把示例企业的科目编码或金额写进生成逻辑。
fn metric_bucket(metric: &Value) -> usize {
    let key = metric_key(metric);
    if key_matches(
        &key,
        &[
            "order_booking",
            "sales_order_amount",
            "purchase_commitment",
            "purchase_inbound",
            "input_invoice",
            "orders_amount",
        ],
    ) {
        return 0;
    }
    if metric["revenue_semantics"].as_bool() == Some(true)
        || key_matches(&key, &["sales_net", "total_sales", "sales_amount"])
    {
        return 1;
    }
    if key_matches(
        &key,
        &[
            "operating_expense",
            "labor_cost",
            "expense_total",
            "expense_ratio",
        ],
    ) || key.starts_with("expense_")
    {
        return 2;
    }
    if key_matches(
        &key,
        &[
            "total_hours",
            "work_hours",
            "overtime_hours",
            "headcount",
            "labor_hours",
        ],
    ) {
        return 3;
    }
    if key_matches(
        &key,
        &[
            "value_added_per_hour",
            "value_added_hourly",
            "value_added_before_labor",
            "added_value",
            "value_added",
        ],
    ) {
        return 4;
    }
    if key_matches(
        &key,
        &[
            "pretax_profit",
            "settlement_income",
            "settlement_margin",
            "profit_margin",
            "operating_profit_after_labor",
            "operating_income_after_labor",
            "operating_profit",
            "labor_cost_per_hour",
            "inventory_amount",
            "inventory_balance",
        ],
    ) {
        return 5;
    }
    if key_matches(
        &key,
        &[
            "collections_amount",
            "payments_amount",
            "invoice_amount",
            "receivables",
            "payables",
        ],
    ) || key.starts_with("cash_")
    {
        return 6;
    }
    match metric_section(metric) {
        section if section.contains("收入") => 1,
        section if section.contains("费用") => 2,
        section if section.contains("时间") => 3,
        section if section.contains("单位") => 4,
        section if section.contains("现金") || section.contains("往来") => 6,
        _ => 2,
    }
}

fn accounting_identifier(metric: &Value, index: usize) -> String {
    ["sequence", "code", "row_key", "metric_id"]
        .iter()
        .find_map(|field| metric[*field].as_str().map(str::to_owned))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            let key = metric_key(metric);
            if key.is_empty() {
                format!("{}", index + 1)
            } else {
                key
            }
        })
}

fn revenue_metric<'a>(metrics: &'a [Value]) -> Option<&'a Value> {
    metrics.iter().find(|m| {
        m["revenue_semantics"].as_bool() == Some(true)
            || key_matches(
                &metric_key(m),
                &["sales_net", "total_sales", "sales_amount"],
            )
    })
}

fn metric_source_note(metric: &Value) -> String {
    let mut note = metric["source_note"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    if note.is_empty() {
        if let Some(formula) = metric["formula_text"].as_str() {
            note = formula.to_string();
        } else if !metric["formula"].is_null() {
            note = metric["formula"].to_string();
        }
    }
    if let Some(label) = metric["target_basis"]["label"].as_str() {
        if !note.is_empty() {
            note.push('；');
        }
        note.push_str(&format!("月度预定来源：{label}"));
        if metric["target_basis"]["is_reference"].as_bool() == Some(true) {
            note.push_str("（参考值，非已确认目标）");
        }
    }
    note
}

fn sorted_months(data: &ExportData) -> Vec<String> {
    let mut months = std::collections::BTreeSet::new();
    for series in &data.series {
        if let Some(buckets) = series["buckets"].as_array() {
            for bucket in buckets {
                let text = text_of(bucket);
                if let Some(month) = month_of(&text) {
                    months.insert(month);
                }
            }
        }
    }
    if months.is_empty() {
        if let Some(month) = month_of(&data.meta.source_to) {
            months.insert(month);
        }
    }
    months.into_iter().collect()
}

fn month_of(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    if bytes.len() >= 7
        && bytes[4] == b'-'
        && bytes[6].is_ascii_digit()
        && (bytes.len() == 7 || bytes[7] == b'-')
    {
        return Some(value[..7].to_string());
    }
    None
}

fn month_label(month: &str) -> String {
    let year = month.get(0..4).and_then(|v| v.parse::<u32>().ok());
    let number = month.get(5..7).and_then(|v| v.parse::<u32>().ok());
    match (year, number) {
        (Some(year), Some(number)) => format!("{year}年{number}月"),
        _ => month.to_string(),
    }
}

fn metric_semantic_type(series: &Value, key: &str) -> &'static str {
    match series["semantic_type"].as_str() {
        Some("flow") => "flow",
        Some("ratio") => "ratio",
        Some("stock") => "stock",
        _ if key_matches(
            key,
            &[
                "order_gross_margin",
                "settlement_margin",
                "expense_ratio",
                "plan_achievement_ratio",
                "annual_plan_achievement_ratio",
            ],
        ) =>
        {
            "ratio"
        }
        _ => "flow",
    }
}

fn series_month_values(
    series: &Value,
) -> (
    std::collections::BTreeMap<String, String>,
    std::collections::BTreeMap<String, String>,
) {
    let key = metric_key(series);
    let semantic = metric_semantic_type(series, &key);
    let buckets = series["buckets"].as_array().cloned().unwrap_or_default();
    let actual = series["actual"].as_array().cloned().unwrap_or_default();
    let plan = series["plan"].as_array().cloned().unwrap_or_default();
    let mut actual_map = std::collections::BTreeMap::<String, Vec<String>>::new();
    let mut plan_map = std::collections::BTreeMap::<String, Vec<String>>::new();
    for (i, bucket) in buckets.iter().enumerate() {
        let Some(month) = month_of(&text_of(bucket)) else {
            continue;
        };
        if let Some(value) = actual.get(i).and_then(decimal_of) {
            actual_map.entry(month.clone()).or_default().push(value);
        }
        if let Some(value) = plan.get(i).and_then(decimal_of) {
            plan_map.entry(month).or_default().push(value);
        }
    }
    fn collapse(values: Vec<String>, semantic: &str) -> Option<String> {
        if values.is_empty() {
            return None;
        }
        if semantic != "flow" {
            return values.into_iter().last();
        }
        let mut total = rust_decimal::Decimal::ZERO;
        for value in values {
            total += rust_decimal::Decimal::from_str_exact(&value).ok()?;
        }
        Some(dsheng_core::contracts::decimal::to_canonical_string(total))
    }
    let actual_map = actual_map
        .into_iter()
        .filter_map(|(k, v)| collapse(v, semantic).map(|v| (k, v)))
        .collect();
    let plan_map = plan_map
        .into_iter()
        .filter_map(|(k, v)| collapse(v, semantic).map(|v| (k, v)))
        .collect();
    (actual_map, plan_map)
}

fn month_metric_value(
    data: &ExportData,
    month: &str,
    aliases: &[&str],
) -> (Option<String>, Option<String>) {
    for series in &data.series {
        if key_matches(&metric_key(series), aliases) {
            let (actual, plan) = series_month_values(series);
            if actual.get(month).is_some() || plan.get(month).is_some() {
                return (actual.get(month).cloned(), plan.get(month).cloned());
            }
        }
    }
    // A run may expose only a metric row for the current month. Keep that value
    // without fabricating historical buckets or zeroes.
    if month_of(&data.meta.source_to).as_deref() == Some(month) {
        if let Some(metric) = data
            .metrics
            .iter()
            .find(|m| key_matches(&metric_key(m), aliases))
        {
            return (
                decimal_of(&metric["actual_value"]),
                decimal_of(&metric["target_value"]),
            );
        }
    }
    (None, None)
}

fn trend_values_for_month(
    data: &ExportData,
    month: &str,
    aliases: &[&[&str]; 13],
) -> Vec<Option<String>> {
    let mut values = Vec::with_capacity(13);
    let mut plans = Vec::with_capacity(13);
    for keys in aliases {
        let (actual, plan) = month_metric_value(data, month, keys);
        values.push(actual);
        plans.push(plan);
    }
    if values[11].is_none() {
        values[11] = match (values[0].as_deref(), plans[0].as_deref()) {
            (Some(actual), Some(plan)) => ratio_text(Some(actual), Some(plan)),
            _ => None,
        };
    }
    if values[10].is_none() {
        values[10] = match (values[0].as_deref(), plans[0].as_deref()) {
            (Some(actual), Some(plan)) => {
                let a = rust_decimal::Decimal::from_str_exact(actual).ok();
                let p = rust_decimal::Decimal::from_str_exact(plan).ok();
                a.zip(p)
                    .map(|(a, p)| dsheng_core::contracts::decimal::to_canonical_string(a - p))
            }
            _ => None,
        };
    }
    if values[12].is_none() && month_of(&data.meta.source_to).as_deref() == Some(month) {
        if let Some(sales) = revenue_metric(&data.metrics) {
            values[12] = ratio_text(
                values[0].as_deref(),
                decimal_of(&sales["annual_month_plan"]).as_deref(),
            );
        }
    }
    values
}

fn write_trend_value(
    ws: &mut Worksheet,
    row: u32,
    index: usize,
    value: Option<&str>,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    let kind = if [3, 7, 9, 11, 12].contains(&index) {
        DisplayKind::Percent
    } else {
        DisplayKind::Money
    };
    write_opt_display_decimal(ws, row, (index + 1) as u16, SHEET_TREND, value, kind, log)
}

/// 核算表 sheet。
fn accounting_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_ACCOUNTING)
        .map_err(|e| ExportError(e.to_string()))?;
    write_sheet_heading(ws, data, WorkbookKind::Accounting, ACCOUNTING_COLS - 1, 0)?;
    write_headers(ws, HEADER_ROW, &ACCOUNTING_HEADERS)?;
    style_headers(ws, ACCOUNTING_COLS - 1)?;
    for (col, width) in [
        (0, 10.0),
        (1, 24.0),
        (2, 42.0),
        (3, 16.0),
        (4, 16.0),
        (5, 16.0),
        (6, 16.0),
        (7, 16.0),
        (8, 16.0),
        (9, 16.0),
        (10, 16.0),
        (11, 18.0),
    ] {
        ws.set_column_width(col, width)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    let revenue = revenue_metric(&data.metrics);
    let revenue_target = revenue.and_then(|m| decimal_of(&m["target_value"]));
    let revenue_actual = revenue.and_then(|m| decimal_of(&m["actual_value"]));
    // 模板固定分组行（A5/A10/A13/A22/A26/A29/A35），科目按
    // 注册指标语义归入分组；显式 section/group 字段优先参与分类。
    let section_rows = [4u32, 9, 12, 21, 25, 28, 34];
    let section_names = [
        "【先行指标（未来经营信号）】",
        "【收入部分】",
        "【费用部分（根据企业特点自行添加）】",
        "【时间部分】",
        "【单位时间附加值部分】",
        "【核算表栏外部分】",
        "【表外参考（现金与往来）】",
    ];
    for (row, name) in section_rows.iter().zip(section_names) {
        ws.merge_range(*row, 0, *row, ACCOUNTING_COLS - 1, name, &section_format())
            .map_err(|e| ExportError(format!("合并核算表分组行失败: {e}")))?;
    }
    let mut next_rows = [5u32, 10, 13, 22, 26, 29, 35];
    let mut overflow: Vec<(usize, &Value)> = Vec::new();
    for (index, metric) in data.metrics.iter().enumerate() {
        let bucket = metric_bucket(metric);
        let row = next_rows[bucket];
        next_rows[bucket] = next_rows[bucket].saturating_add(1);
        if row > 40 {
            overflow.push((index, metric));
            continue;
        }
        let kind = DisplayKind::for_metric(metric);
        write_text(ws, row, 0, &accounting_identifier(metric, index))?;
        write_text(ws, row, 1, &text_of(&metric["metric_label"]))?;
        let source_note = metric_source_note(metric);
        write_text(ws, row, 2, &source_note)?;
        write_text(ws, row, 3, &text_of(&metric["mp_label"]))?;
        let annual = decimal_of(&metric["annual_month_plan"]);
        let target = decimal_of(&metric["target_value"]);
        let actual = decimal_of(&metric["actual_value"]);
        write_opt_display_decimal(ws, row, 4, SHEET_ACCOUNTING, annual.as_deref(), kind, log)?;
        write_opt_display_decimal(ws, row, 5, SHEET_ACCOUNTING, target.as_deref(), kind, log)?;
        write_opt_display_decimal(
            ws,
            row,
            6,
            SHEET_ACCOUNTING,
            ratio_text(target.as_deref(), revenue_target.as_deref()).as_deref(),
            DisplayKind::Percent,
            log,
        )?;
        write_opt_display_decimal(ws, row, 7, SHEET_ACCOUNTING, actual.as_deref(), kind, log)?;
        write_opt_display_decimal(
            ws,
            row,
            8,
            SHEET_ACCOUNTING,
            ratio_text(actual.as_deref(), revenue_actual.as_deref()).as_deref(),
            DisplayKind::Percent,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            9,
            SHEET_ACCOUNTING,
            decimal_of(&metric["delta"]).as_deref(),
            kind.absolute_change(),
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            10,
            SHEET_ACCOUNTING,
            ratio_text(actual.as_deref(), target.as_deref()).as_deref(),
            DisplayKind::Percent,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            11,
            SHEET_ACCOUNTING,
            ratio_text(actual.as_deref(), annual.as_deref()).as_deref(),
            DisplayKind::Percent,
            log,
        )?;
    }
    let notes = [
        "备注1：空单元格表示来源缺失或待补，不按 0 参与计算。",
        "备注2：附加值（扣人工前）与经营收益（扣人工后）按企业 profile 分别保留。",
        "备注3：目标、公式和来源写入对应列；未确认目标不会被当作已确认计划。",
        "备注4：比例和差异仅在分子、分母均有值且分母为正时计算。",
        "备注5：重新导入或补录会保留旧 revision，修订版由用户主动生成。",
        "备注6：本表输出契约固定为 two_tables_v1；输入文件仍按 source profile 适配。",
        "备注7：企业类型、字段名和费用分类由版本化 profile 决定，生成层不硬编码企业数据。",
    ];
    for (offset, note) in notes.iter().enumerate() {
        let row = 41 + offset as u32;
        ws.merge_range(
            row,
            0,
            row,
            ACCOUNTING_COLS - 1,
            note,
            &Format::new().set_text_wrap(),
        )
        .map_err(|e| ExportError(format!("合并核算表备注失败: {e}")))?;
        ws.set_row_height(row, 30.0)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    if !overflow.is_empty() {
        let section_row = 48u32;
        ws.merge_range(
            section_row,
            0,
            section_row,
            ACCOUNTING_COLS - 1,
            "【profile 扩展指标】",
            &section_format(),
        )
        .map_err(|e| ExportError(format!("合并扩展指标分组行失败: {e}")))?;
        for (offset, (index, metric)) in overflow.into_iter().enumerate() {
            let row = section_row + 1 + offset as u32;
            let kind = DisplayKind::for_metric(metric);
            write_text(ws, row, 0, &accounting_identifier(metric, index))?;
            write_text(ws, row, 1, &text_of(&metric["metric_label"]))?;
            write_text(ws, row, 2, &metric_source_note(metric))?;
            write_text(ws, row, 3, &text_of(&metric["mp_label"]))?;
            let annual = decimal_of(&metric["annual_month_plan"]);
            let target = decimal_of(&metric["target_value"]);
            let actual = decimal_of(&metric["actual_value"]);
            write_opt_display_decimal(ws, row, 4, SHEET_ACCOUNTING, annual.as_deref(), kind, log)?;
            write_opt_display_decimal(ws, row, 5, SHEET_ACCOUNTING, target.as_deref(), kind, log)?;
            write_opt_display_decimal(ws, row, 7, SHEET_ACCOUNTING, actual.as_deref(), kind, log)?;
            write_opt_display_decimal(
                ws,
                row,
                9,
                SHEET_ACCOUNTING,
                decimal_of(&metric["delta"]).as_deref(),
                kind.absolute_change(),
                log,
            )?;
        }
    }
    Ok(())
}

/// 将 metricSeriesView 转成模板固定的“每行月份、14 列”推移表。
fn trend_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_TREND)
        .map_err(|e| ExportError(e.to_string()))?;
    write_sheet_heading(ws, data, WorkbookKind::Trend, TREND_COLS - 1, 1)?;
    write_headers(ws, HEADER_ROW, &TREND_HEADERS)?;
    style_headers(ws, TREND_COLS - 1)?;
    for (col, width) in [
        (0, 14.0),
        (1, 16.0),
        (2, 16.0),
        (3, 16.0),
        (4, 13.0),
        (5, 16.0),
        (6, 14.0),
        (7, 16.0),
        (8, 13.0),
        (9, 18.0),
        (10, 17.0),
        (11, 18.0),
        (12, 18.0),
        (13, 20.0),
    ] {
        ws.set_column_width(col, width)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    let months: Vec<String> = sorted_months(data).into_iter().take(16).collect();
    let aliases: [&[&str]; 13] = [
        &["sales_net", "total_sales"],
        &["order_cost", "purchase_cost"],
        &["order_gross_profit", "gross_profit"],
        &["order_gross_margin", "gross_margin"],
        &["operating_expense"],
        &["labor_cost"],
        &[
            "settlement_income",
            "operating_income",
            "operating_profit_after_labor",
            "operating_income_after_labor",
            "operating_profit",
        ],
        &["settlement_margin", "profit_margin"],
        &[
            "value_added_per_hour",
            "value_added_hourly",
            "value_added_before_labor",
        ],
        &["expense_ratio"],
        &["plan_delta"],
        &["plan_achievement_ratio"],
        &["annual_plan_achievement_ratio"],
    ];
    let mut monthly: Vec<Vec<Option<String>>> = Vec::new();
    for slot in 0..16u32 {
        let row = DATA_ROW + slot;
        if let Some(month) = months.get(slot as usize) {
            write_text(ws, row, 0, month)?;
            let values = trend_values_for_month(data, month, &aliases);
            for (index, value) in values.iter().enumerate() {
                write_trend_value(ws, row, index, value.as_deref(), log)?;
            }
            monthly.push(values);
        } else {
            monthly.push(vec![None; 13]);
        }
    }
    // 模板的累计行固定为第 21 行；仅对已有月份聚合，不将缺失月份当作 0。
    let cumulative_row = 20u32;
    let cumulative_label = match (months.first(), months.last()) {
        (Some(first), Some(last)) if first == last => format!("{}累计", month_label(first)),
        (Some(first), Some(last)) if first.get(0..4) == last.get(0..4) => {
            let year = first.get(0..4).unwrap_or_default();
            let start = first.get(5..7).unwrap_or_default();
            let end = last.get(5..7).unwrap_or_default();
            format!(
                "{year}年{}-{}月累计",
                start.parse::<u32>().unwrap_or(0),
                end.parse::<u32>().unwrap_or(0)
            )
        }
        (Some(first), Some(last)) => format!("{}—{}累计", month_label(first), month_label(last)),
        _ => "累计".to_string(),
    };
    write_text(ws, cumulative_row, 0, &cumulative_label)?;
    let mut cumulative = vec![None; 13];
    for index in 0..13 {
        let present: Vec<&str> = monthly
            .iter()
            .filter_map(|values| values.get(index).and_then(|v| v.as_deref()))
            .collect();
        if present.is_empty() {
            continue;
        }
        cumulative[index] = if [0, 1, 2, 5, 6, 10].contains(&index) {
            let mut total = rust_decimal::Decimal::ZERO;
            for value in present {
                let Ok(value) = rust_decimal::Decimal::from_str_exact(value) else {
                    total = rust_decimal::Decimal::ZERO;
                    break;
                };
                total += value;
            }
            Some(dsheng_core::contracts::decimal::to_canonical_string(total))
        } else {
            present.last().map(|value| (*value).to_string())
        };
    }
    // 累计率用累计分子/分母重算，避免把月度比率相加。
    if cumulative[0].is_some() && cumulative[2].is_some() {
        cumulative[3] = ratio_text(cumulative[2].as_deref(), cumulative[0].as_deref());
    }
    if cumulative[0].is_some() && cumulative[5].is_some() {
        cumulative[9] = ratio_text(cumulative[5].as_deref(), cumulative[0].as_deref());
    }
    if cumulative[0].is_some() && cumulative[6].is_some() {
        cumulative[7] = ratio_text(cumulative[6].as_deref(), cumulative[0].as_deref());
    }
    for (index, value) in cumulative.iter().enumerate() {
        write_trend_value(ws, cumulative_row, index, value.as_deref(), log)?;
    }
    let notes = [
        "备注1：月份按 ISO 日期归并并升序排列，月度数据按行展示。",
        "备注2：缺失值留空，不以 0 代替；累计只汇总已有月份。",
        "备注3：费用、人工和结算收益是否可追溯由 source profile 的覆盖范围决定。",
        "备注4：附加值与经营收益沿用同一 revision 的核算口径。",
        "备注5：预定、差异和达成率仅在目标及实绩输入充分时计算。",
    ];
    for (offset, note) in notes.iter().enumerate() {
        let row = 22 + offset as u32;
        ws.merge_range(
            row,
            0,
            row,
            TREND_COLS - 1,
            note,
            &Format::new().set_text_wrap(),
        )
        .map_err(|e| ExportError(format!("合并推移表备注失败: {e}")))?;
        ws.set_row_height(row, 30.0)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    Ok(())
}

/// 差异分析保持模板六列，详细比较证据仍保留在 `_metadata` 与导出 JSON 中。
fn diff_sheet(ws: &mut Worksheet, data: &ExportData, log: &mut DecimalTextLog) -> ExportResult<()> {
    ws.set_name(SHEET_DIFF)
        .map_err(|e| ExportError(e.to_string()))?;
    write_sheet_heading(
        ws,
        data,
        WorkbookKind::Accounting,
        DIFF_HEADERS.len() as u16 - 1,
        0,
    )?;
    write_headers(ws, HEADER_ROW, &DIFF_HEADERS)?;
    style_headers(ws, DIFF_HEADERS.len() as u16 - 1)?;
    for (col, width) in [
        (0, 24.0),
        (1, 18.0),
        (2, 18.0),
        (3, 18.0),
        (4, 12.0),
        (5, 64.0),
    ] {
        ws.set_column_width(col, width)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    for (i, metric) in data.metrics.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        let kind = DisplayKind::for_metric(metric);
        write_text(ws, row, 0, &text_of(&metric["metric_label"]))?;
        write_opt_display_decimal(
            ws,
            row,
            1,
            SHEET_DIFF,
            decimal_of(&metric["target_value"]).as_deref(),
            kind,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            2,
            SHEET_DIFF,
            decimal_of(&metric["actual_value"]).as_deref(),
            kind,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            3,
            SHEET_DIFF,
            decimal_of(&metric["delta"]).as_deref(),
            kind.absolute_change(),
            log,
        )?;
        write_text(ws, row, 4, &text_of(&metric["direction"]))?;
        let mut note = metric_source_note(metric);
        for key in ["yoy", "mom"] {
            if let Some(reason) = metric[key]["reason"].as_str() {
                if !note.is_empty() {
                    note.push('；');
                }
                note.push_str(&format!("{key}:{reason}"));
            }
        }
        write_text(ws, row, 5, &note)?;
    }
    Ok(())
}

/// 图表 sheet：仅标题、副标题和原生折线图对象，不输出配置表。
fn chart_sheet(ws: &mut Worksheet, data: &ExportData) -> ExportResult<()> {
    ws.set_name(SHEET_CHART)
        .map_err(|e| ExportError(e.to_string()))?;
    let (title, _) = heading_text(data, WorkbookKind::Trend);
    let title = format!("{title} — 图表");
    let subtitle = "四张折线：总销售额趋势 / 结算收益趋势 / 利润率趋势 / 单位时间附加值趋势；数据源同「推移表」sheet";
    let title_format = Format::new()
        .set_bold()
        .set_font_size(14)
        .set_font_color("1F3864");
    ws.merge_range(0, 0, 0, 7, &title, &title_format)
        .map_err(|e| ExportError(format!("合并图表标题失败: {e}")))?;
    ws.merge_range(1, 0, 1, 7, subtitle, &Format::new().set_text_wrap())
        .map_err(|e| ExportError(format!("合并图表副标题失败: {e}")))?;
    ws.set_row_height(0, 24.0)
        .map_err(|e| ExportError(e.to_string()))?;
    ws.set_row_height(1, 32.0)
        .map_err(|e| ExportError(e.to_string()))?;
    // 图表固定引用模板的 16 个月槽位（A5:A20），空槽位不会被当作 0。
    let end = DATA_ROW + 15;
    {
        for (position, (column, name)) in [
            (1u16, "总销售额(元)"),
            (7u16, "结算收益(元)"),
            (8u16, "利润率"),
            (9u16, "单位时间附加值(元/h)"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut chart = Chart::new(ChartType::Line);
            chart.title().set_name(name);
            chart.x_axis().set_name("月份");
            chart.y_axis().set_name(name);
            chart
                .add_series()
                .set_name(name)
                .set_categories((SHEET_TREND, DATA_ROW, 0, end, 0))
                .set_values((SHEET_TREND, DATA_ROW, column, end, column));
            let row = if position < 2 { 3 } else { 21 };
            let col = if position % 2 == 0 { 1 } else { 10 };
            ws.insert_chart(row, col, &chart)
                .map_err(|e| ExportError(format!("插入图表失败: {e}")))?;
        }
    }
    Ok(())
}

fn task_in_review(task: &Value, meta: &super::ExportMeta) -> bool {
    match task["target_period"]["to"].as_str() {
        Some(to) => to <= meta.source_to.as_str(),
        None => true,
    }
}

fn step_rows(task: &Value) -> Vec<Value> {
    let steps = task["steps"].as_array().cloned().unwrap_or_default();
    if steps.is_empty() {
        vec![Value::Null]
    } else {
        steps
    }
}

/// 任务单检视 sheet（多步骤展开；字段重复；空日期保留空）。
fn task_review_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_TASK_REVIEW)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, TASK_HEADER_ROW, &TASK_REVIEW_HEADERS)?;
    let mut row = TASK_DATA_ROW;
    for task in data.tasks.iter().filter(|t| task_in_review(t, &data.meta)) {
        let kind = DisplayKind::for_metric(task);
        for step in step_rows(task) {
            write_text(ws, row, 0, category_label(&task["category"]))?;
            write_text(ws, row, 1, &text_of(&task["title"]))?;
            write_text(ws, row, 2, &text_of(&task["metric_label"]))?;
            write_text(ws, row, 3, &text_of(&task["unit"]))?;
            write_opt_display_decimal(
                ws,
                row,
                4,
                SHEET_TASK_REVIEW,
                decimal_of(&task["target_value"]).as_deref(),
                kind,
                log,
            )?;
            write_opt_display_decimal(
                ws,
                row,
                5,
                SHEET_TASK_REVIEW,
                decimal_of(&task["actual_value"]).as_deref(),
                kind,
                log,
            )?;
            write_text(ws, row, 6, &text_of(&step["action"]))?;
            let owner = if step["owner"].is_null() {
                &task["owner"]
            } else {
                &step["owner"]
            };
            write_text(ws, row, 7, &text_of(owner))?;
            let start = if step["start_date"].is_null() {
                &task["planned_start"]
            } else {
                &step["start_date"]
            };
            let due = if step["due_date"].is_null() {
                &task["due_date"]
            } else {
                &step["due_date"]
            };
            write_text(ws, row, 8, &text_of(start))?;
            write_text(ws, row, 9, &text_of(due))?;
            let status = if step.is_null() {
                &task["status"]
            } else {
                &step["status"]
            };
            write_text(ws, row, 10, &text_of(status))?;
            write_opt_decimal(
                ws,
                row,
                11,
                SHEET_TASK_REVIEW,
                decimal_of(&task["progress"]).as_deref(),
                log,
            )?;
            write_text(ws, row, 12, &text_of(&task["review_note"]))?;
            write_text(ws, row, 13, &evidence_ids(task))?;
            write_text(ws, row, 14, &text_of(&task["task_id"]))?;
            write_text(ws, row, 15, &text_of(&step["step_id"]))?;
            row += 1;
        }
    }
    hide_tech_columns(ws, 14)?;
    Ok(())
}

/// 任务单计划 sheet。
fn task_plan_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_TASK_PLAN)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, TASK_HEADER_ROW, &TASK_PLAN_HEADERS)?;
    let mut row = TASK_DATA_ROW;
    for task in data.tasks.iter().filter(|t| !task_in_review(t, &data.meta)) {
        let kind = DisplayKind::for_metric(task);
        for step in step_rows(task) {
            write_text(ws, row, 0, category_label(&task["category"]))?;
            write_text(ws, row, 1, &text_of(&task["title"]))?;
            write_text(ws, row, 2, &text_of(&task["metric_label"]))?;
            write_text(ws, row, 3, &text_of(&task["unit"]))?;
            write_opt_display_decimal(
                ws,
                row,
                4,
                SHEET_TASK_PLAN,
                decimal_of(&task["target_value"]).as_deref(),
                kind,
                log,
            )?;
            write_text(ws, row, 5, &text_of(&step["action"]))?;
            let owner = if step["owner"].is_null() {
                &task["owner"]
            } else {
                &step["owner"]
            };
            write_text(ws, row, 6, &text_of(owner))?;
            let start = if step["start_date"].is_null() {
                &task["planned_start"]
            } else {
                &step["start_date"]
            };
            let due = if step["due_date"].is_null() {
                &task["due_date"]
            } else {
                &step["due_date"]
            };
            write_text(ws, row, 7, &text_of(start))?;
            write_text(ws, row, 8, &text_of(due))?;
            write_text(ws, row, 9, &text_of(&task["resource_need"]))?;
            write_text(ws, row, 10, &text_of(&task["success_criteria"]))?;
            let pending: Vec<String> = task["smart_check"]["checks"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter(|c| c["status"].as_str() == Some("needs_confirmation"))
                .map(|c| format!("{}:{}", text_of(&c["dimension"]), text_of(&c["note"])))
                .collect();
            write_text(ws, row, 11, &pending.join("；"))?;
            write_text(ws, row, 12, &evidence_ids(task))?;
            write_text(ws, row, 13, &text_of(&task["task_id"]))?;
            write_text(ws, row, 14, &text_of(&step["step_id"]))?;
            row += 1;
        }
    }
    hide_tech_columns(ws, 13)?;
    Ok(())
}

/// 任务依据 sheet。
fn evidence_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_TASK_EVIDENCE)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, TASK_HEADER_ROW, &EVIDENCE_HEADERS)?;
    for (i, e) in data.evidence.iter().enumerate() {
        let row = TASK_DATA_ROW + i as u32;
        let kind = DisplayKind::for_metric(e);
        write_text(ws, row, 0, &text_of(&e["evidence_id"]))?;
        write_text(ws, row, 1, &text_of(&e["metric_label"]))?;
        write_text(ws, row, 2, &text_of(&e["source_period"]))?;
        write_text(ws, row, 3, &text_of(&e["target_period"]))?;
        write_text(ws, row, 4, &text_of(&e["file_label"]))?;
        write_text(ws, row, 5, &text_of(&e["sheet"]))?;
        write_text(ws, row, 6, &text_of(&e["cell_or_row"]))?;
        write_text(ws, row, 7, &text_of(&e["comparison_type"]))?;
        write_text(ws, row, 8, &text_of(&e["base_period"]))?;
        write_opt_display_decimal(
            ws,
            row,
            9,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["base_value"]).as_deref(),
            kind,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            10,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["current_value"]).as_deref(),
            kind,
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            11,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["absolute_change"]).as_deref(),
            kind.absolute_change(),
            log,
        )?;
        write_opt_display_decimal(
            ws,
            row,
            12,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["change_rate"]).as_deref(),
            DisplayKind::Percent,
            log,
        )?;
        write_text(ws, row, 13, &text_of(&e["pending_reason"]))?;
    }
    Ok(())
}

fn hide_tech_columns(ws: &mut Worksheet, first_col: u16) -> ExportResult<()> {
    for col in first_col..first_col + 2 {
        ws.set_column_hidden(col)
            .map_err(|e| ExportError(e.to_string()))?;
    }
    Ok(())
}

fn evidence_ids(task: &Value) -> String {
    task["evidence"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|e| text_of(&e["evidence_id"]))
        .collect::<Vec<_>>()
        .join("；")
}

fn category_label(v: &Value) -> &'static str {
    match v.as_str() {
        Some("revenue") => "销售最大化",
        Some("cost") => "费用最小化",
        Some("efficiency") => "时间最短化",
        _ => "其他事项",
    }
}

/// 原值追溯只扩展元数据，不改 two_tables_v1 业务表列与固定单元格位置。
pub fn write_precision_metadata(
    book: &mut Workbook,
    data: &ExportData,
    log: &DecimalTextLog,
) -> ExportResult<()> {
    // 固定两位/百分点标签比原始数字长；仅按数值展示内容扩列，避免 Excel
    // 默认窄列显示 ####，也避免 autofit 把长来源说明扩成不可读的整屏宽列。
    let mut widths: std::collections::BTreeMap<(&str, u16), usize> = Default::default();
    for cell in &log.display_cells {
        let col = cell
            .cell
            .bytes()
            .take_while(u8::is_ascii_alphabetic)
            .fold(0u16, |n, c| n * 26 + u16::from(c - b'A' + 1))
            - 1;
        let width = cell
            .display_value
            .chars()
            .map(|c| if c.is_ascii() { 1 } else { 2 })
            .sum::<usize>()
            + 2;
        widths
            .entry((&cell.sheet, col))
            .and_modify(|w| *w = (*w).max(width))
            .or_insert(width.max(14));
    }
    for ((sheet, col), width) in widths {
        book.worksheet_from_name(sheet)
            .and_then(|ws| ws.set_column_width(col, width as f64))
            .map_err(|e| ExportError(format!("设置展示数值列宽失败: {e}")))?;
    }
    let ws = book.add_worksheet();
    ws.set_name("_metadata")
        .map_err(|e| ExportError(e.to_string()))?;
    ws.set_hidden(true);
    write_text(ws, 0, 0, "dsheng_template_version")?;
    write_text(ws, 0, 1, TEMPLATE_VERSION)?;
    write_text(ws, 1, 0, "output_contract_id")?;
    write_text(ws, 1, 1, &data.meta.output_contract_id)?;
    write_text(ws, 2, 0, "display_precision")?;
    write_text(
        ws,
        2,
        1,
        "金额两位小数；百分数/百分点一位小数；half_even；raw_value 为未经展示舍入的原值",
    )?;
    let metadata_pairs = [
        ("workspace_id", data.meta.workspace_id.clone()),
        ("workspace_name", data.meta.workspace_name.clone()),
        ("run_id", data.meta.run_id.clone()),
        ("revision_no", data.meta.revision_no.to_string()),
        ("as_of", data.meta.as_of.clone()),
        (
            "analysis_input_fingerprint",
            data.meta.analysis_input_fingerprint.clone(),
        ),
        ("source_from", data.meta.source_from.clone()),
        ("source_to", data.meta.source_to.clone()),
        (
            "target_from",
            data.meta.target_from.clone().unwrap_or_default(),
        ),
        ("target_to", data.meta.target_to.clone().unwrap_or_default()),
        ("grain", data.meta.grain.clone()),
        ("coverage_status", data.meta.coverage_status.clone()),
        ("currency", data.meta.currency.clone().unwrap_or_default()),
        ("tax_basis", data.meta.tax_basis.clone().unwrap_or_default()),
        (
            "policy",
            serde_json::to_string(&data.meta.policy).unwrap_or_default(),
        ),
        (
            "metric_hashes",
            serde_json::to_string(&data.meta.metric_hashes).unwrap_or_default(),
        ),
        (
            "source_manifests",
            serde_json::to_string(&data.meta.source_manifests).unwrap_or_default(),
        ),
        (
            "base_refs",
            serde_json::to_string(&data.meta.base_refs).unwrap_or_default(),
        ),
        (
            "filters",
            serde_json::to_string(&data.meta.filters).unwrap_or_default(),
        ),
    ];
    let mut metadata_row = 3u32;
    for (key, value) in metadata_pairs {
        write_text(ws, metadata_row, 0, key)?;
        write_text(ws, metadata_row, 1, &value)?;
        metadata_row += 1;
    }
    write_headers(
        ws,
        metadata_row + 1,
        &[
            "record_type",
            "sheet",
            "cell_or_metric_key",
            "raw_value",
            "display_value",
            "number_format",
            "number_storage",
        ],
    )?;
    let mut row = metadata_row + 2;
    for cell in &log.display_cells {
        for (col, text) in [
            "display_decimal",
            cell.sheet.as_str(),
            cell.cell.as_str(),
            cell.raw_value.as_str(),
            cell.display_value.as_str(),
            cell.number_format.as_str(),
            cell.number_storage.as_str(),
        ]
        .iter()
        .enumerate()
        {
            write_text(ws, row, col as u16, text)?;
        }
        row += 1;
    }
    for metric in &data.metrics {
        if !metric["target_basis"].is_null() {
            write_text(ws, row, 0, "target_basis")?;
            write_text(ws, row, 2, &text_of(&metric["metric_key"]))?;
            write_text(ws, row, 3, &metric["target_basis"].to_string())?;
            row += 1;
        }
    }
    Ok(())
}

/// 构建单工作簿字节（含 decimal_text 记录）。
pub fn build_workbook(
    kind: WorkbookKind,
    data: &ExportData,
) -> ExportResult<(Vec<u8>, DecimalTextLog)> {
    let mut book = Workbook::new();
    let mut log = DecimalTextLog::default();
    match kind {
        WorkbookKind::Accounting => {
            accounting_sheet(book.add_worksheet(), data, &mut log)?;
            trend_sheet(book.add_worksheet(), data, &mut log)?;
            diff_sheet(book.add_worksheet(), data, &mut log)?;
        }
        WorkbookKind::Trend => {
            trend_sheet(book.add_worksheet(), data, &mut log)?;
            chart_sheet(book.add_worksheet(), data)?;
        }
        WorkbookKind::Task => {
            task_review_sheet(book.add_worksheet(), data, &mut log)?;
            task_plan_sheet(book.add_worksheet(), data, &mut log)?;
            evidence_sheet(book.add_worksheet(), data, &mut log)?;
        }
    }
    write_precision_metadata(&mut book, data, &log)?;
    let bytes = book
        .save_to_buffer()
        .map_err(|e| ExportError(format!("工作簿序列化失败: {e}")))?;
    Ok((bytes, log))
}
