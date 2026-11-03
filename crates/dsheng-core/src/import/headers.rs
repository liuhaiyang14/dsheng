//! 表头规范化、重复列检测与分块识别（data-period-import.md §4.1 / DC I02、I09）。
//!
//! 仅表头匹配做：首尾/换行空白清理、全角 ASCII 转半角、连续空格合一；
//! 其他原文保持。完整 §4.2 exact 别名表与模板适配器在导入器工作阶段补齐。

/// 表头文本规范化（仅表头；数据单元格原文不动）。
pub fn normalize_header(raw: &str) -> String {
    let half: String = raw
        .chars()
        .map(|c| {
            // 全角 ASCII → 半角；全角空格 U+3000 → 普通空格
            if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            } else if c == '\u{3000}' {
                ' '
            } else {
                c
            }
        })
        .collect();
    // 连续空白合一后首尾清理
    let mut out = String::with_capacity(half.len());
    let mut pending_space = false;
    for c in half.chars() {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// 列字母（0 → A、25 → Z、26 → AA …）。
pub fn column_letter(index: usize) -> String {
    let mut n = index;
    let mut out = Vec::new();
    loop {
        out.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    out.reverse();
    String::from_utf8(out).expect("列字母")
}

/// 重复列检测（DC I02）：规范化后同名的列要求手动映射，不互相覆盖。
///
/// 空表头不参与重复判定（空列本就进 unmapped）。返回重复列的下标（列序）；
/// 调用方用 [`column_letter`] 或实际列布局换算列字母。
pub fn duplicate_columns(headers: &[&str]) -> Vec<usize> {
    let mut seen = std::collections::HashMap::new();
    let mut dup = vec![false; headers.len()];
    for (i, h) in headers.iter().enumerate() {
        let key = normalize_header(h);
        if key.is_empty() {
            continue;
        }
        match seen.get(&key) {
            Some(&first) => {
                dup[first] = true;
                dup[i] = true;
            }
            None => {
                seen.insert(key, i);
            }
        }
    }
    (0..headers.len()).filter(|i| dup[*i]).collect()
}

/// 工作表布局：表头行与整行合并标题/说明段行号（1 基）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockLayout {
    /// 各块表头行号（升序）。
    pub header_rows: Vec<usize>,
    /// 整行合并标题/说明段行号（不是交易行，§4.3）。
    pub title_rows: Vec<usize>,
    /// 总行数。
    pub total_rows: usize,
}

/// 分块计划（DC I09）：每块独立识别；标题行不作数据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPlan {
    pub blocks: usize,
    /// 各块数据行区间（1 基闭区间起、止）。
    pub data_rows: Vec<(usize, usize)>,
    /// 标题行是否被并入数据（按 §4.1 永远 false）。
    pub title_rows_as_data: bool,
}

/// 按 header_rows 分块：每块数据从表头行下一行到下一表头行（或表尾），
/// 排除标题/说明段行（§4.3：重复表头逐块识别，标题不当明细）。
pub fn plan_blocks(layout: &BlockLayout) -> BlockPlan {
    let mut data_rows = Vec::new();
    let mut headers = layout.header_rows.clone();
    headers.sort_unstable();
    let bounds: Vec<Option<usize>> = headers
        .iter()
        .enumerate()
        .map(|(i, _)| headers.get(i + 1).copied())
        .collect();
    for (idx, header_row) in headers.iter().enumerate() {
        let end_bound = bounds[idx].unwrap_or(layout.total_rows + 1);
        let mut rows: Vec<usize> = (*header_row + 1..end_bound)
            .filter(|r| !layout.title_rows.contains(r))
            .collect();
        if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
            data_rows.push((*first, *last));
        }
        rows.clear();
    }
    BlockPlan {
        blocks: headers.len(),
        data_rows,
        title_rows_as_data: false,
    }
}

/// 跨业务范围合并求和是否允许（§4.3：不同块业务范围不相同时不合并）。
pub fn cross_scope_sum_allowed(scopes: &[&str]) -> bool {
    scopes.windows(2).all(|w| w[0] == w[1])
}

/// (col,row) 0 基 → A1 坐标。
pub fn a1((col, row): (u32, u32)) -> String {
    format!("{}{}", column_letter(col as usize), row + 1)
}

/// §4.2 exact 别名表（规范化后精确匹配；除此之外手动映射）。
/// 元素：(规范字段, 别名列表, 是否核心字段)。
pub const FIELD_ALIASES: &[(&str, &[&str], bool)] = &[
    (
        "metric_label",
        &["科目", "项目", "指标", "指标目标", "MP名称", "MP指标"],
        true,
    ),
    (
        "source_note",
        &[
            "计算公式/取数来源",
            "计算公式",
            "取数来源",
            "数据来源",
            "来源说明",
        ],
        false,
    ),
    ("mp_label", &["本月MP", "MP", "重点指标"], false),
    ("unit", &["单位", "计量单位"], false),
    (
        "annual_month_plan",
        &[
            "年度计划·当月目标",
            "年度当月目标",
            "年度计划当月目标",
            "年度月度分解目标",
        ],
        true,
    ),
    (
        "planned",
        &[
            "月度预定",
            "本月预定",
            "本期预定",
            "期间预定",
            "预定",
            "计划值",
            "目标值",
            "本期目标",
            "目标",
        ],
        true,
    ),
    (
        "actual",
        &[
            "月度实绩",
            "本月实绩",
            "本期实绩",
            "期间实绩",
            "实绩",
            "实际值",
            "本期实际",
        ],
        true,
    ),
    (
        "delta",
        &["预实差异", "预实差异额", "差异额", "绝对差异"],
        false,
    ),
    (
        "plan_ratio",
        &["预定比", "预定达成率", "计划达成率", "达成率"],
        false,
    ),
    (
        "annual_plan_ratio",
        &["年度计划比", "年度计划达成率"],
        false,
    ),
    (
        "sales_share",
        &["预定销售额占比", "实绩销售额占比", "销售额占比"],
        false,
    ),
    (
        "period",
        &["期间", "日期", "业务日期", "交易日期", "月份", "季度"],
        true,
    ),
    (
        "document_id",
        &["单据号", "订单号", "发票号", "业务单号"],
        false,
    ),
    ("line_id", &["行号", "明细行号", "行ID"], false),
    (
        "scope",
        &["业务范围", "部门", "货品", "仓库", "客户", "对象"],
        false,
    ),
    ("task_title", &["任务名称", "任务", "事项"], true),
    (
        "task_action",
        &["行动方案", "行动步骤", "具体行动", "怎么做"],
        true,
    ),
    ("owner", &["责任人", "责任岗位", "负责人", "责任"], false),
    ("planned_start", &["开始日期", "计划开始"], false),
    ("due_date", &["时间节点", "截止日期", "计划完成日期"], false),
    ("task_status", &["状态", "完成状态"], false),
    ("progress", &["进度", "完成率"], false),
    ("evidence", &["任务依据", "依据", "证据", "来源"], false),
    ("category", &["方向", "类别", "经营方向"], false),
    ("review_note", &["复盘说明", "检视说明"], false),
    ("next_plan", &["后续计划"], false),
];

/// exact 别名映射（先规范化：全角→半角、连续空格合一、首尾清理）。
/// 未列别名返回 None（进 unmapped/手动映射，不猜）。
pub fn map_header(raw: &str) -> Option<&'static str> {
    let norm = normalize_header(raw);
    for (field, aliases, _) in FIELD_ALIASES {
        if aliases.contains(&norm.as_str()) || *field == norm {
            return Some(field);
        }
    }
    None
}

/// 表头块（§4.1：首 50 行、连续 1–3 行、≥2 已知字段、取命中必需字段最多者）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderBlock {
    /// 表头起始行（1 基）。
    pub row: usize,
    pub row_count: usize,
    /// (列下标 0 基, 规范字段)。
    pub columns: Vec<(usize, &'static str)>,
    /// 命中核心字段数。
    pub core_score: usize,
}

fn row_texts(sheet_rows: &[Vec<String>], row_idx: usize) -> Vec<String> {
    sheet_rows.get(row_idx).cloned().unwrap_or_default()
}

/// 搜索表头块；并列最高分时返回全部候选（同分多个需用户选，§4.1）。
pub fn find_header_blocks(sheet_rows: &[Vec<String>]) -> Vec<HeaderBlock> {
    let mut best: Vec<HeaderBlock> = Vec::new();
    let scan = sheet_rows.len().min(50);
    for start in 0..scan {
        for span in 1..=3usize {
            if start + span > scan {
                break;
            }
            // 连续 1–3 行：每列取首个非空表头（合并表头左上标签投影）
            let width = (start..start + span)
                .map(|r| row_texts(sheet_rows, r).len())
                .max()
                .unwrap_or(0);
            let mut columns = Vec::new();
            for col in 0..width {
                for r in start..start + span {
                    let text = sheet_rows
                        .get(r)
                        .and_then(|row| row.get(col))
                        .map(String::as_str)
                        .unwrap_or("");
                    if text.trim().is_empty() {
                        continue;
                    }
                    if let Some(field) = map_header(text) {
                        columns.push((col, field));
                    }
                    break;
                }
            }
            if columns.len() < 2 {
                continue;
            }
            let core_score = columns
                .iter()
                .filter(|(_, f)| {
                    FIELD_ALIASES
                        .iter()
                        .any(|(field, _, core)| field == f && *core)
                })
                .count();
            let block = HeaderBlock {
                row: start + 1,
                row_count: span,
                columns,
                core_score,
            };
            match best.first() {
                Some(b) if b.core_score > block.core_score => {}
                Some(b) if b.core_score == block.core_score => {
                    // 同分：列映射相同的视为同一块（取更小行数），否则并列待用户选
                    let same_columns = b.columns == block.columns;
                    if same_columns {
                        if block.row_count < b.row_count {
                            best.clear();
                            best.push(block);
                        }
                    } else {
                        best.push(block);
                    }
                }
                _ => {
                    best.clear();
                    best.push(block);
                }
            }
        }
    }
    best
}

/// 推移表期间列解析（§4.3）：YYYY-MM、YYYY年M月、YYYY-MM-DD；
/// “1月” 须已有明确年元数据（year_context）。
pub fn parse_period_header(
    text: &str,
    year_context: Option<i32>,
) -> Option<crate::contracts::domain::DateRange> {
    use crate::contracts::domain::DateRange;
    let t = normalize_header(text);
    // 年月闭区间构造（自然月）
    fn month_range(year: i32, month: u32) -> Option<DateRange> {
        let from = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
        let to = {
            let (ny, nm) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            chrono::NaiveDate::from_ymd_opt(ny, nm, 1)? - chrono::Duration::days(1)
        };
        DateRange::new(from, to).ok()
    }
    // YYYY- 前缀：YYYY-MM-DD（单日）或 YYYY-MM（自然月）
    let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let bytes = t.as_bytes();
    if bytes.len() >= 7 && bytes[..4].iter().all(u8::is_ascii_digit) && bytes[4] == b'-' {
        let year: i32 = t[..4].parse().ok()?;
        let rest = &t[5..];
        if rest.len() == 5 && rest.as_bytes()[2] == b'-' && digits(&rest[..2]) && digits(&rest[3..])
        {
            // YYYY-MM-DD
            let month: u32 = rest[..2].parse().ok()?;
            let day: u32 = rest[3..].parse().ok()?;
            let from = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            return DateRange::new(from, from).ok();
        }
        if rest.len() == 2 && digits(rest) {
            // YYYY-MM
            let month: u32 = rest.parse().ok()?;
            return month_range(year, month);
        }
        return None;
    }
    // YYYY年M月（年必须显式）
    if let Some(stripped) = t.strip_suffix('月') {
        if let Some((year_s, month_s)) = stripped.split_once('年') {
            if !year_s.is_empty() && !month_s.is_empty() {
                let year: i32 = year_s.parse().ok()?;
                let month: u32 = month_s.parse().ok()?;
                return month_range(year, month);
            }
        }
        // “1月”：须明确年元数据
        if !stripped.contains('年') && !stripped.is_empty() {
            let year = year_context?;
            let month: u32 = stripped.parse().ok()?;
            return month_range(year, month);
        }
    }
    None
}

#[cfg(test)]
mod period_tests {
    use super::parse_period_header;
    use chrono::NaiveDate;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn contract_period_formats() {
        // YYYY-MM：自然月闭区间
        let m = parse_period_header("2026-10", None).unwrap();
        assert_eq!(m.from, d("2026-10-01"));
        assert_eq!(m.to, d("2026-10-31"));
        // YYYY年M月
        let cm = parse_period_header("2025年7月", None).unwrap();
        assert_eq!(cm.from, d("2025-07-01"));
        assert_eq!(cm.to, d("2025-07-31"));
        // YYYY-MM-DD：单日（§4.3 接受；此前实现拒绝）
        let day = parse_period_header("2026-10-31", None).unwrap();
        assert_eq!(day.from, d("2026-10-31"));
        assert_eq!(day.to, d("2026-10-31"));
        // “1月”须明确年元数据
        assert!(parse_period_header("1月", None).is_none());
        let with_year = parse_period_header("1月", Some(2027)).unwrap();
        assert_eq!(with_year.from, d("2027-01-01"));
        assert_eq!(with_year.to, d("2027-01-31"));
        // 非期间文本
        assert!(parse_period_header("月份", None).is_none());
        assert!(parse_period_header("2026年1-10月累计", None).is_none());
        assert!(parse_period_header("累计", None).is_none());
        assert!(parse_period_header("26-10", None).is_none());
    }
}
