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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_normalization() {
        assert_eq!(normalize_header("  科目　"), "科目");
        assert_eq!(normalize_header("计算公式／取数来源"), "计算公式/取数来源");
        assert_eq!(normalize_header("月度　  实绩"), "月度 实绩");
        // 数据原文保持（仅表头清理）
        assert_eq!(normalize_header("１２３"), "123");
    }

    #[test]
    fn duplicate_columns_detected() {
        let cols = duplicate_columns(&["金额", "金额", "实绩"]);
        assert_eq!(cols, vec![0, 1]);
        assert_eq!(
            cols.iter().map(|i| column_letter(*i)).collect::<Vec<_>>(),
            vec!["A", "B"]
        );
        let none = duplicate_columns(&["金额", "实绩"]);
        assert!(none.is_empty());
    }

    #[test]
    fn blocks_planned() {
        let plan = plan_blocks(&BlockLayout {
            header_rows: vec![5, 11, 47],
            title_rows: vec![1, 2],
            total_rows: 50,
        });
        assert_eq!(plan.blocks, 3);
        assert!(!plan.title_rows_as_data);
        assert_eq!(plan.data_rows[0], (6, 10));
        assert_eq!(plan.data_rows[2], (48, 50));
    }

    #[test]
    fn cross_scope() {
        assert!(cross_scope_sum_allowed(&["A", "A"]));
        assert!(!cross_scope_sum_allowed(&["A", "B", "C"]));
    }

    #[test]
    fn column_letters() {
        assert_eq!(column_letter(0), "A");
        assert_eq!(column_letter(2), "C");
        assert_eq!(column_letter(26), "AA");
    }
}
