//! 模板识别与适配器锚点（data-period-import.md §4.3）。
//!
//! 锚点来自表头/合并结构，坐标仅描述真实布局形态，不是唯一匹配条件：
//! - accounting：sheet`核算表` + metric_label + actual/planned/annual_month_plan 之一；
//! - trend：sheet`推移表` + metric_label/科目锚点 + 至少一个可解析期间列；
//! - task：sheet 后缀`任务单检视`/`任务单计划`（允许当月/下月/本期/下期/日期
//!   前缀）+ 任务依据；表头须 task_action 或 metric_label 且 owner/due_date/
//!   planned/actual 之一；
//! - 不满足结构 → generic_table（用户确认，部分识别保留待映射）。

use super::headers::{find_header_blocks, map_header, parse_period_header, HeaderBlock};

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
}

/// 识别 sheet 角色（§4.3）。
pub fn detect_sheet_role(sheet_name: &str, header_rows: &[Vec<String>]) -> Option<SheetMatch> {
    let blocks = find_header_blocks(header_rows);
    let best = blocks.first().cloned();
    let fields: Vec<&'static str> = best
        .as_ref()
        .map(|b| b.columns.iter().map(|(_, f)| *f).collect())
        .unwrap_or_default();
    let has = |f: &str| fields.contains(&map_header_alias(f));
    let name = sheet_name.trim();
    let mut anchors = Vec::new();

    let role = if name == "核算表"
        && has("metric_label")
        && (has("actual") || has("planned") || has("annual_month_plan"))
    {
        anchors.push("sheet=核算表".into());
        anchors.push("metric_label+数值列".into());
        SheetRole::Accounting
    } else if name == "推移表" && (has("metric_label") || column_a_is_account(header_rows)) {
        let period_cols = count_period_columns(header_rows, best.as_ref());
        if period_cols >= 1 {
            anchors.push("sheet=推移表".into());
            anchors.push(format!("period_columns={period_cols}"));
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
    let mut count = 0;
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for col in 0..width {
        // 表头块之外或未映射的列按期间列尝试
        if block.is_some_and(|b| b.columns.iter().any(|(c, _)| *c == col)) {
            continue;
        }
        let hit = rows.iter().take(10).any(|r| {
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
