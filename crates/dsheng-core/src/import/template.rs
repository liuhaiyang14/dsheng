//! 模板识别与适配器锚点（data-period-import.md §4.3）。
//!
//! 锚点来自表头/合并结构，坐标仅描述真实布局形态，不是唯一匹配条件：
//! - accounting：sheet`核算表` + metric_label + actual/planned/annual_month_plan 之一；
//! - trend：sheet`推移表` + metric_label/科目锚点 + 至少一个可解析期间列；
//! - task：sheet 后缀`任务单检视`/`任务单计划`（允许当月/下月/本期/下期/日期
//!   前缀）+ 任务依据；表头须 task_action 或 metric_label 且 owner/due_date/
//!   planned/actual 之一；
//! - 不满足结构 → generic_table（用户确认，部分识别保留待映射）。

use super::headers::{
    find_header_blocks, map_header, parse_period_header, parse_period_row, strip_unit_suffix,
    HeaderBlock, PeriodRowLabel,
};
use crate::contracts::domain::DateRange;

/// sheet 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SheetRole {
    Accounting,
    Trend,
    TaskReview,
    TaskPlan,
    /// 任务依据块（多块重复表头逐块识别）。
    TaskEvidence,
    Chart,
    GenericTable,
}

impl SheetRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accounting => "accounting",
            Self::Trend => "trend",
            Self::TaskReview => "task_review",
            Self::TaskPlan => "task_plan",
            Self::TaskEvidence => "task_evidence",
            Self::Chart => "chart",
            Self::GenericTable => "generic_table",
        }
    }
}

/// 转置推移表布局（§4.3 2026-11-04 用户批准增量：期间为行、指标为列）。
#[derive(Debug, Clone, PartialEq)]
pub struct TransposedTrendLayout {
    /// 表头行（1 基；A 列 period 表头如 `月份` 所在行）。
    pub header_row: usize,
    /// 指标列：(0 基列下标, 单位后缀剥离后的指标名)。
    pub metric_columns: Vec<(usize, String)>,
    /// 期间数据行：(0 基行下标, 已解析期间)。
    pub period_rows: Vec<(usize, DateRange)>,
    /// `累计` 行：(0 基行下标, 明确起止)。独立范围观察（§2.5），
    /// 不与 period_rows 的月行重复相加。
    pub cumulative_rows: Vec<(usize, DateRange)>,
}

/// 识别结果（每 sheet 只一个角色；冲突需用户选）。
#[derive(Debug, Clone, PartialEq)]
pub struct SheetMatch {
    pub role: SheetRole,
    /// 无已知字段块时为 None（generic_table 由用户确认表头/范围）。
    pub header: Option<HeaderBlock>,
    /// 命中的锚点说明。
    pub anchors: Vec<String>,
    /// 未知列（保留进 unmapped，不丢列）。
    pub unmapped_columns: Vec<usize>,
    /// 转置推移表布局（与标准 trend 互斥；仅 trend 角色可能非 None）。
    pub transposed: Option<TransposedTrendLayout>,
}

/// 识别 sheet 角色（§4.3；无目录信息——转置锚点须指标目录，见
/// [`detect_sheet_role_with_names`]）。
pub fn detect_sheet_role(sheet_name: &str, header_rows: &[Vec<String>]) -> Option<SheetMatch> {
    detect_sheet_role_with_names(sheet_name, header_rows, &[])
}

/// 识别 sheet 角色（§4.3），带工作区指标名集合（metric_key/label/label_aliases）
/// 供转置锚点做后缀剥离后的精确匹配。
pub fn detect_sheet_role_with_names(
    sheet_name: &str,
    header_rows: &[Vec<String>],
    metric_names: &[String],
) -> Option<SheetMatch> {
    let blocks = find_header_blocks(header_rows);
    let best = blocks.first().cloned();
    let fields: Vec<&'static str> = best
        .as_ref()
        .map(|b| b.columns.iter().map(|(_, f)| *f).collect())
        .unwrap_or_default();
    let has = |f: &str| fields.contains(&map_header_alias(f));
    let name = sheet_name.trim();
    let mut anchors = Vec::new();
    let mut transposed = None;

    let role = if name == "核算表"
        && has("metric_label")
        && (has("actual") || has("planned") || has("annual_month_plan"))
    {
        anchors.push("sheet=核算表".into());
        anchors.push("metric_label+数值列".into());
        SheetRole::Accounting
    } else if name == "推移表" {
        // 先试标准 trend（科目行×期间列），不命中再试转置（期间为行×指标列）；
        // 两者互斥，命中即出结构化观察。
        let standard_hits = (has("metric_label") || column_a_is_account(header_rows))
            && count_period_columns(header_rows, best.as_ref()) >= 1;
        if standard_hits {
            anchors.push("sheet=推移表".into());
            anchors.push(format!(
                "period_columns={}",
                count_period_columns(header_rows, best.as_ref())
            ));
            SheetRole::Trend
        } else if let Some(layout) = detect_transposed_trend(header_rows, metric_names) {
            anchors.push("sheet=推移表".into());
            anchors.push(format!(
                "transposed_metric_columns={}",
                layout.metric_columns.len()
            ));
            anchors.push(format!(
                "transposed_period_rows={}",
                layout.period_rows.len()
            ));
            transposed = Some(layout);
            SheetRole::Trend
        } else {
            SheetRole::GenericTable
        }
    } else if is_task_sheet(name) {
        if has("evidence") {
            anchors.push("sheet后缀含任务单+任务依据".into());
            SheetRole::TaskEvidence
        } else if (has("task_action") || has("metric_label"))
            && (has("owner") || has("due_date") || has("planned") || has("actual"))
        {
            anchors.push("sheet后缀含任务单".into());
            if name.ends_with("任务单检视") {
                SheetRole::TaskReview
            } else {
                SheetRole::TaskPlan
            }
        } else {
            SheetRole::GenericTable
        }
    } else if name == "图表" {
        SheetRole::Chart
    } else {
        SheetRole::GenericTable
    };

    let mapped_cols: std::collections::HashSet<usize> = best
        .as_ref()
        .map(|b| b.columns.iter().map(|(c, _)| *c).collect())
        .unwrap_or_default();
    let width = header_rows.iter().map(Vec::len).max().unwrap_or(0);
    let unmapped = (0..width).filter(|c| !mapped_cols.contains(c)).collect();
    Some(SheetMatch {
        role,
        header: best,
        anchors,
        unmapped_columns: unmapped,
        transposed,
    })
}

/// 转置推移表锚点（§4.3 增量）：
/// - 首 50 行内找 A 列 period 表头（`月份` 等 §4.2 period 别名）行；
/// - 其余列该行须全部为指标名（[`strip_unit_suffix`] 后按 `metric_names`
///   精确匹配）；任一列（含空表头但有数据的列）无法解析为指标名 → 整表回退
///   generic_table（返回 None）；
/// - 表头行之下：A 列可解析为期间/累计的行是数据行（标题/说明行跳过），
///   须至少一个期间行；`累计` 行须明确起止，单列为独立范围观察。
/// - 无指标目录（`metric_names` 为空）不猜 → None。
fn detect_transposed_trend(
    rows: &[Vec<String>],
    metric_names: &[String],
) -> Option<TransposedTrendLayout> {
    if metric_names.is_empty() {
        return None;
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    if width < 2 {
        return None;
    }
    let scan_rows = rows.len().min(50);
    let header_row_idx = (0..scan_rows).find(|&r| {
        rows.get(r)
            .and_then(|row| row.first())
            .map(|t| map_header(t) == Some("period"))
            .unwrap_or(false)
    })?;
    // 指标列：表头行的其余列全部须精确命中（§4.3 整表回退规则）
    let header = rows.get(header_row_idx)?;
    let mut metric_columns = Vec::new();
    for col in 1..width {
        let text = header
            .get(col)
            .map(String::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            // 空表头但有数据的列同样无法解析为指标名 → 回退
            let has_data = rows
                .iter()
                .skip(header_row_idx + 1)
                .any(|r| r.get(col).map(|c| !c.trim().is_empty()).unwrap_or(false));
            if has_data {
                return None;
            }
            continue;
        }
        let stripped = strip_unit_suffix(&text);
        if metric_names.iter().any(|n| n == stripped) {
            metric_columns.push((col, stripped.to_string()));
        } else {
            return None;
        }
    }
    if metric_columns.is_empty() {
        return None;
    }
    // 数据行：A 列期间/累计（其余行跳过）；至少一个期间行
    let mut period_rows = Vec::new();
    let mut cumulative_rows = Vec::new();
    for (idx, row) in rows.iter().enumerate().skip(header_row_idx + 1) {
        let text = row
            .first()
            .map(String::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if text.is_empty() {
            continue;
        }
        match parse_period_row(&text, None) {
            Some(PeriodRowLabel::Period(range)) => period_rows.push((idx, range)),
            Some(PeriodRowLabel::Cumulative(range)) => cumulative_rows.push((idx, range)),
            None => {}
        }
    }
    if period_rows.is_empty() {
        return None;
    }
    Some(TransposedTrendLayout {
        header_row: header_row_idx + 1,
        metric_columns,
        period_rows,
        cumulative_rows,
    })
}

fn map_header_alias(f: &str) -> &'static str {
    // 规范字段名直查（fields 集合本身即规范名）
    map_header(f).unwrap_or_default()
}

/// 任务单 sheet 名：后缀 任务单检视/任务单计划，允许当月/下月/本期/下期/日期前缀。
pub fn is_task_sheet(name: &str) -> bool {
    (name.ends_with("任务单检视") || name.ends_with("任务单计划"))
        && name.chars().count() > "任务单检视".chars().count()
}

fn column_a_is_account(rows: &[Vec<String>]) -> bool {
    // A 列科目锚点（§4.3 trend：metric_label 或 A 列科目）
    rows.iter().take(10).any(|r| {
        r.first()
            .map(|s| map_header(s) == Some("metric_label"))
            .unwrap_or(false)
    })
}

fn count_period_columns(rows: &[Vec<String>], block: Option<&HeaderBlock>) -> usize {
    // §4.1/§4.3：期间列只在表头块行内识别（数据行里的日期文本不是期间列锚点）
    let Some(block) = block else {
        return 0;
    };
    let header_rows: Vec<&Vec<String>> = (block.row - 1..block.row - 1 + block.row_count)
        .filter_map(|r| rows.get(r))
        .collect();
    if header_rows.is_empty() {
        return 0;
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut count = 0;
    for col in 0..width {
        if block.columns.iter().any(|(c, _)| *c == col) {
            continue;
        }
        let hit = header_rows.iter().any(|r| {
            r.get(col)
                .map(|t| parse_period_header(t, None).is_some())
                .unwrap_or(false)
        });
        if hit {
            count += 1;
        }
    }
    count
}

/// 年元数据下推（§4.3：“1月”须已有明确年元数据）。
pub fn period_with_year_context(
    text: &str,
    year: i32,
) -> Option<crate::contracts::domain::DateRange> {
    parse_period_header(text, Some(year))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(v: Vec<Vec<&str>>) -> Vec<Vec<String>> {
        v.into_iter()
            .map(|r| r.into_iter().map(str::to_string).collect())
            .collect()
    }

    #[test]
    fn accounting_anchor() {
        let m = detect_sheet_role(
            "核算表",
            &rows(vec![
                vec!["", "科目", "计算公式/取数来源", "月度预定", "月度实绩"],
                vec!["", "不含税销售额", "来源A", "100", "120"],
            ]),
        )
        .unwrap();
        assert_eq!(m.role, SheetRole::Accounting);
        assert!(!m.anchors.is_empty());
    }

    #[test]
    fn trend_requires_period_column() {
        let with_period = detect_sheet_role(
            "推移表",
            &rows(vec![vec!["科目", "单位", "2026-08", "2026-09", "累计"]]),
        )
        .unwrap();
        assert_eq!(with_period.role, SheetRole::Trend);
        let without =
            detect_sheet_role("推移表", &rows(vec![vec!["科目", "单位", "备注"]])).unwrap();
        assert_eq!(without.role, SheetRole::GenericTable);
    }

    #[test]
    fn task_sheet_suffix() {
        let review = detect_sheet_role(
            "2026年10月任务单检视",
            &rows(vec![vec![
                "类别",
                "指标目标",
                "行动方案",
                "责任",
                "时间节点",
                "状态",
                "复盘说明",
            ]]),
        )
        .unwrap();
        assert_eq!(review.role, SheetRole::TaskReview);
        let plan = detect_sheet_role(
            "下期任务单计划",
            &rows(vec![vec![
                "类别",
                "指标目标",
                "行动方案",
                "责任人",
                "截止日期",
                "任务依据",
            ]]),
        )
        .unwrap();
        assert_eq!(plan.role, SheetRole::TaskEvidence); // 有任务依据列 → 依据块
        assert!(is_task_sheet("当月任务单检视"));
        assert!(!is_task_sheet("核算表"));
    }

    #[test]
    fn generic_fallback() {
        let m = detect_sheet_role("明细", &rows(vec![vec!["甲", "乙"]])).unwrap();
        assert_eq!(m.role, SheetRole::GenericTable);
    }
}

#[cfg(test)]
mod hardening_tests {
    use super::*;
    use crate::import::SheetRole;

    fn rows(v: Vec<Vec<&str>>) -> Vec<Vec<String>> {
        v.into_iter()
            .map(|r| r.into_iter().map(str::to_string).collect())
            .collect()
    }

    #[test]
    fn trend_period_anchor_only_from_header_rows() {
        // 表头行含期间列 → trend
        let hit = detect_sheet_role(
            "推移表",
            &rows(vec![
                vec!["科目", "单位", "2026-09", "2026-10"],
                vec!["不含税销售额", "元", "100", "200"],
            ]),
        )
        .unwrap();
        assert_eq!(hit.role, SheetRole::Trend);
        // 期间文本只出现在数据行（非表头）→ 不是期间列锚点 → generic_table
        let data_only = detect_sheet_role(
            "推移表",
            &rows(vec![
                vec!["科目", "单位", "备注"],
                vec!["2025-07", "2025-08", "数据起点"],
                vec!["不含税销售额", "元", "100"],
            ]),
        )
        .unwrap();
        assert_eq!(data_only.role, SheetRole::GenericTable);
        // YYYY-MM-DD 表头期间列（§4.3 接受格式；表头块须≥2 已知字段）
        let day_header = detect_sheet_role(
            "推移表",
            &rows(vec![
                vec!["科目", "单位", "2026-10-31"],
                vec!["甲", "元", "1"],
            ]),
        )
        .unwrap();
        assert_eq!(day_header.role, SheetRole::Trend);
        // 真实企业形态（转置）：A 列是期间值、B 列起是指标名。无指标目录时
        // 不猜（§4.3 增量：列头须按目录 label/别名精确匹配）→ generic_table；
        // 带目录时命中转置锚点（见 transposed_tests）。
        let transposed = detect_sheet_role(
            "推移表",
            &rows(vec![
                vec!["月份", "总销售额(元)", "结算收益(元)"],
                vec!["2025-07", "100", "20"],
                vec!["2026-10", "300", "-5"],
            ]),
        )
        .unwrap();
        assert_eq!(transposed.role, SheetRole::GenericTable);
    }
}

#[cfg(test)]
mod transposed_tests {
    use super::*;
    use crate::contracts::domain::DateRange;
    use chrono::NaiveDate;

    fn rows(v: Vec<Vec<&str>>) -> Vec<Vec<String>> {
        v.into_iter()
            .map(|r| r.into_iter().map(str::to_string).collect())
            .collect()
    }

    fn names() -> Vec<String> {
        ["总销售额", "结算收益", "单位时间附加值", "人工费"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn range(from: &str, to: &str) -> DateRange {
        DateRange::new(d(from), d(to)).unwrap()
    }

    /// 真实文件同构：首 2 行标题、第 4 行 A=月份 + B..D 指标名（带/不带后缀）、
    /// 数据行 A 列 YYYY-MM / YYYY年M月、含累计行与说明行。
    fn transposed_rows() -> Vec<Vec<String>> {
        rows(vec![
            vec!["合成公司 — 月度推移表（合成夹具）"],
            vec!["口径：合成说明行，不是数据"],
            vec![""],
            vec!["月份", "总销售额(元)", "结算收益", "单位时间附加值(元/h)"],
            vec!["2025-07", "1", "2", "3"],
            vec!["2025年8月", "4", "5", "6"],
            vec!["2026-10", "7", "8", "9"],
            vec!["2026年1-10月累计", "10", "11", "12"],
            vec!["注1：说明段，不是数据行"],
        ])
    }

    #[test]
    fn transposed_anchor_hits_trend_with_catalog() {
        let m = detect_sheet_role_with_names("推移表", &transposed_rows(), &names()).unwrap();
        assert_eq!(m.role, SheetRole::Trend);
        let layout = m.transposed.expect("转置布局");
        assert_eq!(layout.header_row, 4);
        assert_eq!(
            layout.metric_columns,
            vec![
                (1, "总销售额".to_string()),
                (2, "结算收益".to_string()),
                (3, "单位时间附加值".to_string()),
            ]
        );
        assert_eq!(
            layout.period_rows,
            vec![
                (4, range("2025-07-01", "2025-07-31")),
                (5, range("2025-08-01", "2025-08-31")),
                (6, range("2026-10-01", "2026-10-31")),
            ]
        );
        // 累计行单独成范围观察；说明行跳过
        assert_eq!(
            layout.cumulative_rows,
            vec![(7, range("2026-01-01", "2026-10-31"))]
        );
    }

    #[test]
    fn non_metric_column_falls_back_to_generic() {
        let mut with_remark = transposed_rows();
        with_remark[3].push("备注".to_string());
        with_remark[4].push("文本".to_string());
        let m = detect_sheet_role_with_names("推移表", &with_remark, &names()).unwrap();
        assert_eq!(
            m.role,
            SheetRole::GenericTable,
            "任一列不是指标名 → 整表回退"
        );
        // 空表头但有数据的列同样回退
        let mut blank_header = transposed_rows();
        blank_header[3].push(String::new());
        blank_header[4].push("99".to_string());
        let m = detect_sheet_role_with_names("推移表", &blank_header, &names()).unwrap();
        assert_eq!(m.role, SheetRole::GenericTable);
    }

    #[test]
    fn transposed_requires_period_row_and_period_header() {
        let names = names();
        // 无期间数据行（只有累计）→ 不命中（期间为行是锚点）
        let only_cumulative = rows(vec![
            vec!["月份", "总销售额(元)"],
            vec!["2026年1-10月累计", "10"],
        ]);
        let m = detect_sheet_role_with_names("推移表", &only_cumulative, &names).unwrap();
        assert_eq!(m.role, SheetRole::GenericTable);
        // A 列表头不是 period 别名 → 不命中
        let bad_header = rows(vec![vec!["科目", "总销售额"], vec!["2026-10", "1"]]);
        let m = detect_sheet_role_with_names("推移表", &bad_header, &names).unwrap();
        assert_eq!(m.role, SheetRole::GenericTable);
        // 指标名剥不出目录匹配（后缀剥离后无命中）→ 回退
        let unknown_metric = rows(vec![vec!["月份", "毛利率"], vec!["2026-10", "1"]]);
        let m = detect_sheet_role_with_names("推移表", &unknown_metric, &names).unwrap();
        assert_eq!(m.role, SheetRole::GenericTable);
    }

    #[test]
    fn standard_trend_takes_priority_over_transposed() {
        // 科目行×期间列的标准形态即使列头也能按目录解释，仍走标准锚点（互斥）
        let standard = rows(vec![
            vec!["科目", "单位", "2026-09", "2026-10"],
            vec!["总销售额", "元", "1", "2"],
        ]);
        let m = detect_sheet_role_with_names("推移表", &standard, &names()).unwrap();
        assert_eq!(m.role, SheetRole::Trend);
        assert!(m.transposed.is_none(), "标准命中时不试转置");
        assert!(m.header.is_some());
    }
}
