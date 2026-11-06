//! 三份业务工作簿（§5 布局逐格）：核算表/推移表/任务单。
//!
//! 统一元数据：A1 dsheng_template_version/B1 1；A2 workspace_name/B2、C2 currency/D2、
//! E2 tax_basis/F2；A3 source_from/B3、C3 source_to/D3、E3 grain/F3、
//! K3 target_from/L3、M3 target_to/N3；A4 timezone/B4、C4 week_start_day/D4、
//! E4 fiscal_year_start_month/F4、G4 coverage_status/H4、I4 empty_days_are_zero/J4；
//! 第 5 行空；第 6 行列名；第 7 行起数据。任务 sheet 名固定（完整期间放第 3 行）；
//! 多步骤按永久 task_id 分组展开行；task_id/step_id 技术列（隐藏）。

use super::cells::{write_decimal, write_headers, write_opt_decimal, write_text, DecimalTextLog};
use super::{
    decimal_of, text_of, ExportData, ExportError, ExportResult, SHEET_ACCOUNTING, SHEET_CHART,
    SHEET_DIFF, SHEET_TASK_EVIDENCE, SHEET_TASK_PLAN, SHEET_TASK_REVIEW, SHEET_TREND,
    TEMPLATE_VERSION,
};
use rust_xlsxwriter::{Workbook, Worksheet};
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

pub const ACCOUNTING_HEADERS: [&str; 10] = [
    "科目",
    "单位",
    "计算公式/取数来源",
    "本月MP",
    "年度计划·当月目标",
    "月度预定",
    "月度实绩",
    "预实差异额",
    "预定比",
    "年度计划比",
];
pub const DIFF_HEADERS: [&str; 14] = [
    "科目",
    "单位",
    "本期目标",
    "本期实际",
    "绝对差异",
    "差异方向",
    "同比基期",
    "同比绝对差",
    "同比变化率",
    "环比基期",
    "环比绝对差",
    "环比变化率",
    "缺失/不可比原因",
    "证据",
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
pub const DATA_ROW: u32 = 6;
/// 表头行（0 基：第 6 行）。
pub const HEADER_ROW: u32 = 5;

/// 写元数据块（§5 精确位置；空元数据留空）。
pub fn write_metadata(ws: &mut Worksheet, data: &ExportData) -> ExportResult<()> {
    let m = &data.meta;
    let pairs: [(u32, u16, &str, String); 14] = [
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
    let keys: [(u32, u16, &str); 14] = [
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

fn comparison_cells(c: &Value) -> (String, Option<String>, Option<String>, Option<String>) {
    if c.is_null() {
        return (String::new(), None, None, None);
    }
    let base = format!("{}/{}", text_of(&c["base_from"]), text_of(&c["base_to"]));
    (
        base,
        decimal_of(&c["absolute_change"]),
        decimal_of(&c["change_rate"]),
        c["reason"].as_str().map(str::to_string),
    )
}

/// 核算表 sheet。
fn accounting_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_ACCOUNTING)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, HEADER_ROW, &ACCOUNTING_HEADERS)?;
    for (i, m) in data.metrics.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        write_text(ws, row, 0, &text_of(&m["metric_label"]))?;
        write_text(ws, row, 1, &text_of(&m["unit"]))?;
        write_text(ws, row, 2, &text_of(&m["source_note"]))?;
        write_text(ws, row, 3, &text_of(&m["mp_label"]))?;
        write_opt_decimal(
            ws,
            row,
            4,
            SHEET_ACCOUNTING,
            decimal_of(&m["annual_month_plan"]).as_deref(),
            log,
        )?;
        let target = decimal_of(&m["target_value"]);
        let actual = decimal_of(&m["actual_value"]);
        write_opt_decimal(ws, row, 5, SHEET_ACCOUNTING, target.as_deref(), log)?;
        write_opt_decimal(ws, row, 6, SHEET_ACCOUNTING, actual.as_deref(), log)?;
        write_opt_decimal(
            ws,
            row,
            7,
            SHEET_ACCOUNTING,
            decimal_of(&m["delta"]).as_deref(),
            log,
        )?;
        // 派生值写已计算字面量（不写公式）
        let plan_ratio = ratio_text(actual.as_deref(), target.as_deref());
        write_opt_decimal(ws, row, 8, SHEET_ACCOUNTING, plan_ratio.as_deref(), log)?;
        let annual_ratio = ratio_text(
            actual.as_deref(),
            decimal_of(&m["annual_month_plan"]).as_deref(),
        );
        write_opt_decimal(ws, row, 9, SHEET_ACCOUNTING, annual_ratio.as_deref(), log)?;
    }
    Ok(())
}

/// 推移表 sheet（期间列按序列桶；累计仅 flow 且全桶非空）。
fn trend_sheet(
    ws: &mut Worksheet,
    data: &ExportData,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    ws.set_name(SHEET_TREND)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    // 桶并集（升序）
    let mut buckets: Vec<String> = Vec::new();
    for s in &data.series {
        for b in s["buckets"].as_array().cloned().unwrap_or_default() {
            let label = text_of(&b);
            if !buckets.contains(&label) {
                buckets.push(label);
            }
        }
    }
    buckets.sort();
    let mut headers: Vec<String> = vec!["科目".into(), "单位".into(), "取数来源".into()];
    headers.extend(buckets.iter().cloned());
    headers.push("累计".into());
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    write_headers(ws, HEADER_ROW, &header_refs)?;
    for (i, s) in data.series.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        write_text(ws, row, 0, &text_of(&s["metric_label"]))?;
        write_text(ws, row, 1, &text_of(&s["unit"]))?;
        write_text(ws, row, 2, &text_of(&s["source_note"]))?;
        let s_buckets: Vec<String> = s["buckets"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(text_of)
            .collect();
        let actual: Vec<Value> = s["actual"].as_array().cloned().unwrap_or_default();
        let mut sum = Some(rust_decimal::Decimal::ZERO);
        for (bi, b) in buckets.iter().enumerate() {
            let col = 3 + bi as u16;
            match s_buckets.iter().position(|x| x == b) {
                Some(idx) => {
                    let v = decimal_of(actual.get(idx).unwrap_or(&Value::Null));
                    match &v {
                        Some(text) => {
                            if let (Some(acc), Ok(d)) =
                                (sum, rust_decimal::Decimal::from_str_exact(text))
                            {
                                sum = Some(acc + d);
                            } else {
                                sum = None;
                            }
                        }
                        None => sum = None,
                    }
                    write_opt_decimal(ws, row, col, SHEET_TREND, v.as_deref(), log)?;
                }
                None => sum = None,
            }
        }
        let cumulative_col = 3 + buckets.len() as u16;
        let is_flow = s["semantic_type"]
            .as_str()
            .map(|t| t == "flow")
            .unwrap_or(false);
        match (is_flow, sum) {
            (true, Some(total)) if !buckets.is_empty() => write_decimal(
                ws,
                row,
                cumulative_col,
                SHEET_TREND,
                &dsheng_core::contracts::decimal::to_canonical_string(total),
                log,
            )?,
            _ => write_text(ws, row, cumulative_col, "")?,
        }
    }
    // 注明：非 flow 无累计语义
    let note_row = DATA_ROW + data.series.len() as u32 + 1;
    write_text(
        ws,
        note_row,
        0,
        "注：累计仅对 flow 指标且全桶有值时计算；ratio/stock 无累计语义留空；缺桶不造值。",
    )?;
    Ok(())
}

/// 差异分析 sheet。
fn diff_sheet(ws: &mut Worksheet, data: &ExportData, log: &mut DecimalTextLog) -> ExportResult<()> {
    ws.set_name(SHEET_DIFF)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, HEADER_ROW, &DIFF_HEADERS)?;
    for (i, m) in data.metrics.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        write_text(ws, row, 0, &text_of(&m["metric_label"]))?;
        write_text(ws, row, 1, &text_of(&m["unit"]))?;
        write_opt_decimal(
            ws,
            row,
            2,
            SHEET_DIFF,
            decimal_of(&m["target_value"]).as_deref(),
            log,
        )?;
        write_opt_decimal(
            ws,
            row,
            3,
            SHEET_DIFF,
            decimal_of(&m["actual_value"]).as_deref(),
            log,
        )?;
        write_opt_decimal(
            ws,
            row,
            4,
            SHEET_DIFF,
            decimal_of(&m["delta"]).as_deref(),
            log,
        )?;
        write_text(ws, row, 5, &text_of(&m["direction"]))?;
        let (yoy_base, yoy_abs, yoy_rate, yoy_reason) = comparison_cells(&m["yoy"]);
        let (mom_base, mom_abs, mom_rate, mom_reason) = comparison_cells(&m["mom"]);
        write_text(ws, row, 6, &yoy_base)?;
        write_opt_decimal(ws, row, 7, SHEET_DIFF, yoy_abs.as_deref(), log)?;
        write_opt_decimal(ws, row, 8, SHEET_DIFF, yoy_rate.as_deref(), log)?;
        write_text(ws, row, 9, &mom_base)?;
        write_opt_decimal(ws, row, 10, SHEET_DIFF, mom_abs.as_deref(), log)?;
        write_opt_decimal(ws, row, 11, SHEET_DIFF, mom_rate.as_deref(), log)?;
        let reasons: Vec<String> = [
            yoy_reason.map(|r| format!("yoy:{r}")),
            mom_reason.map(|r| format!("mom:{r}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        write_text(ws, row, 12, &reasons.join("；"))?;
        let evidence: Vec<String> = ["mom", "yoy"]
            .iter()
            .flat_map(|k| {
                m[k]["evidence_ids"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|e| text_of(&e))
            })
            .collect();
        write_text(ws, row, 13, &evidence.join("；"))?;
    }
    Ok(())
}

/// 图表配置 sheet（空配置不造趋势；导出写序列来源）。
fn chart_sheet(ws: &mut Worksheet, data: &ExportData) -> ExportResult<()> {
    ws.set_name(SHEET_CHART)
        .map_err(|e| ExportError(e.to_string()))?;
    write_metadata(ws, data)?;
    write_headers(ws, HEADER_ROW, &CHART_HEADERS)?;
    for (i, s) in data.series.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        let key = text_of(&s["metric_key"]);
        write_text(ws, row, 0, &format!("trend:{key}"))?;
        write_text(ws, row, 1, "line")?;
        write_text(ws, row, 2, &key)?;
        write_text(ws, row, 3, &data.meta.source_from)?;
        write_text(ws, row, 4, &data.meta.source_to)?;
        write_text(ws, row, 5, &data.meta.grain)?;
        let refs: Vec<String> = data
            .meta
            .source_manifests
            .iter()
            .map(|m| m.file_label.clone())
            .collect();
        write_text(ws, row, 6, &refs.join("；"))?;
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
    write_headers(ws, HEADER_ROW, &TASK_REVIEW_HEADERS)?;
    let mut row = DATA_ROW;
    for task in data.tasks.iter().filter(|t| task_in_review(t, &data.meta)) {
        for step in step_rows(task) {
            write_text(ws, row, 0, category_label(&task["category"]))?;
            write_text(ws, row, 1, &text_of(&task["title"]))?;
            write_text(ws, row, 2, &text_of(&task["metric_label"]))?;
            write_text(ws, row, 3, &text_of(&task["unit"]))?;
            write_opt_decimal(
                ws,
                row,
                4,
                SHEET_TASK_REVIEW,
                decimal_of(&task["target_value"]).as_deref(),
                log,
            )?;
            write_opt_decimal(
                ws,
                row,
                5,
                SHEET_TASK_REVIEW,
                decimal_of(&task["actual_value"]).as_deref(),
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
    write_headers(ws, HEADER_ROW, &TASK_PLAN_HEADERS)?;
    let mut row = DATA_ROW;
    for task in data.tasks.iter().filter(|t| !task_in_review(t, &data.meta)) {
        for step in step_rows(task) {
            write_text(ws, row, 0, category_label(&task["category"]))?;
            write_text(ws, row, 1, &text_of(&task["title"]))?;
            write_text(ws, row, 2, &text_of(&task["metric_label"]))?;
            write_text(ws, row, 3, &text_of(&task["unit"]))?;
            write_opt_decimal(
                ws,
                row,
                4,
                SHEET_TASK_PLAN,
                decimal_of(&task["target_value"]).as_deref(),
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
    write_headers(ws, HEADER_ROW, &EVIDENCE_HEADERS)?;
    for (i, e) in data.evidence.iter().enumerate() {
        let row = DATA_ROW + i as u32;
        write_text(ws, row, 0, &text_of(&e["evidence_id"]))?;
        write_text(ws, row, 1, &text_of(&e["metric_label"]))?;
        write_text(ws, row, 2, &text_of(&e["source_period"]))?;
        write_text(ws, row, 3, &text_of(&e["target_period"]))?;
        write_text(ws, row, 4, &text_of(&e["file_label"]))?;
        write_text(ws, row, 5, &text_of(&e["sheet"]))?;
        write_text(ws, row, 6, &text_of(&e["cell_or_row"]))?;
        write_text(ws, row, 7, &text_of(&e["comparison_type"]))?;
        write_text(ws, row, 8, &text_of(&e["base_period"]))?;
        write_opt_decimal(
            ws,
            row,
            9,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["base_value"]).as_deref(),
            log,
        )?;
        write_opt_decimal(
            ws,
            row,
            10,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["current_value"]).as_deref(),
            log,
        )?;
        write_opt_decimal(
            ws,
            row,
            11,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["absolute_change"]).as_deref(),
            log,
        )?;
        write_opt_decimal(
            ws,
            row,
            12,
            SHEET_TASK_EVIDENCE,
            decimal_of(&e["change_rate"]).as_deref(),
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
    let bytes = book
        .save_to_buffer()
        .map_err(|e| ExportError(format!("工作簿序列化失败: {e}")))?;
    Ok((bytes, log))
}
