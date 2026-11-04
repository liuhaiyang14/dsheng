//! 转置推移表锚点（data-period-import.md §4.3 2026-11-04 用户批准增量）。
//!
//! 合成 CSV 同构夹具（真实布局形态：首列 period 表头、其余列首行指标名带/不
//! 带单位后缀、数据行 A 列 YYYY-MM／YYYY年M月、`累计` 行、说明行）验证：
//! - 标准 trend（科目行×期间列）不命中时转置锚点命中，两者互斥；
//! - 布局坐标与读取网格（rows[r][c] 绝对坐标）精确对齐；
//! - 任一列无法解析为指标名（含空表头但有数据）→ 整表回退 generic_table；
//! - 无指标目录不猜（generic_table）。
//!
//! 数值全部为合成值；真实业务文件不入库。

use dsheng_core::import::fileio::{parse_csv_text, CellValue};
use dsheng_core::import::template::{
    detect_sheet_role, detect_sheet_role_with_names, SheetRole, TransposedTrendLayout,
};

fn names() -> Vec<String> {
    ["总销售额", "结算收益", "单位时间附加值", "人工费"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// 读取 CSV 文本并改名为 `推移表`（CSV sheet 名固定 csv，锚点按 sheet 名识别）。
fn trend_sheet(csv: &str) -> dsheng_core::import::fileio::SheetData {
    let mut sheet = parse_csv_text(csv).expect("csv");
    sheet.name = "推移表".into();
    sheet
}

fn header_rows(sheet: &dsheng_core::import::fileio::SheetData) -> Vec<Vec<String>> {
    sheet
        .rows
        .iter()
        .map(|row| row.iter().map(|c| c.raw_text()).collect())
        .collect()
}

fn detect(
    sheet: &dsheng_core::import::fileio::SheetData,
    metric_names: &[String],
) -> dsheng_core::import::template::SheetMatch {
    detect_sheet_role_with_names(&sheet.name, &header_rows(sheet), metric_names)
        .expect("识别结果总存在")
}

/// 真实文件同构（合成值）：第 1–2 行标题/说明、第 4 行 A=月份 + B..E 指标名
/// （带/不带后缀）、数据行 YYYY-MM／YYYY年M月、`2026年1-10月累计` 行、注行。
fn transposed_csv() -> String {
    [
        "合成公司 — 月度推移表（合成夹具）",
        "口径：合成说明行",
        "月份,总销售额(元),结算收益,单位时间附加值(元/h)",
        "2025-07,100,20.5,1.25",
        "2025年8月,101,21.5,2.25",
        "2026-10,300,-5,0",
        "2026年1-10月累计,900,60,",
        "注1：说明段，不是数据行",
    ]
    .join("\n")
}

fn layout_of(sheet: &dsheng_core::import::fileio::SheetData) -> TransposedTrendLayout {
    let m = detect(sheet, &names());
    assert_eq!(m.role, SheetRole::Trend);
    m.transposed.expect("转置布局")
}

#[test]
fn transposed_anchor_layout_aligns_with_read_grid() {
    let sheet = trend_sheet(&transposed_csv());
    let layout = layout_of(&sheet);
    assert_eq!(layout.header_row, 3);
    assert_eq!(
        layout
            .metric_columns
            .iter()
            .map(|(c, n)| (*c, n.as_str()))
            .collect::<Vec<_>>(),
        vec![(1, "总销售额"), (2, "结算收益"), (3, "单位时间附加值")],
        "单位后缀剥离后按目录精确匹配"
    );
    assert_eq!(layout.period_rows.len(), 3, "YYYY-MM 与 YYYY年M月 期间行");
    assert_eq!(layout.cumulative_rows.len(), 1, "累计行单独成范围观察");
    // 布局坐标 ↔ rows[r][c] 绝对坐标对齐（0 基行下标）
    let (oct_row, oct) = layout.period_rows[2];
    assert_eq!(
        oct.from,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
    );
    assert_eq!(
        oct.to,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
    );
    assert_eq!(
        sheet.rows[oct_row][1],
        CellValue::Text("300".into()),
        "2026-10 × 总销售额 数据格"
    );
    let (cum_row, cum) = layout.cumulative_rows[0];
    assert_eq!(
        cum.from,
        chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
    );
    assert_eq!(
        cum.to,
        chrono::NaiveDate::from_ymd_opt(2026, 10, 31).unwrap()
    );
    assert_eq!(
        sheet.rows[cum_row][3],
        CellValue::Empty,
        "累计行空格=missing"
    );
}

#[test]
fn any_non_metric_column_falls_back_whole_sheet() {
    // 非指标名列（“备注”）→ 整表 generic_table
    let with_remark = trend_sheet(&["月份,总销售额(元),备注", "2026-10,300,文本"].join("\n"));
    assert_eq!(detect(&with_remark, &names()).role, SheetRole::GenericTable);
    // 空表头但有数据的列同样无法解析为指标名 → 回退
    let blank_header = trend_sheet(&["月份,总销售额(元),", "2026-10,300,99"].join("\n"));
    assert_eq!(
        detect(&blank_header, &names()).role,
        SheetRole::GenericTable
    );
    // 后缀剥离后仍不在目录（“毛利率”）→ 回退
    let unknown = trend_sheet(&["月份,毛利率", "2026-10,0.5"].join("\n"));
    assert_eq!(detect(&unknown, &names()).role, SheetRole::GenericTable);
}

#[test]
fn transposed_anchor_requires_catalog_and_period_rows() {
    let sheet = trend_sheet(&transposed_csv());
    // 无指标目录（detect_sheet_role 不带目录）→ 不猜，generic_table
    let m = detect_sheet_role(&sheet.name, &header_rows(&sheet)).unwrap();
    assert_eq!(m.role, SheetRole::GenericTable);
    assert!(m.transposed.is_none());
    // 只有累计行、无期间数据行 → 不命中（期间为行是锚点）
    let only_cum = trend_sheet(&["月份,总销售额(元)", "2026年1-10月累计,900"].join("\n"));
    assert_eq!(detect(&only_cum, &names()).role, SheetRole::GenericTable);
    // A 列表头不是 period 别名 → 不命中
    let bad_header = trend_sheet(&["科目,总销售额", "2026-10,1"].join("\n"));
    assert_eq!(detect(&bad_header, &names()).role, SheetRole::GenericTable);
}

#[test]
fn standard_trend_anchor_takes_priority_mutually_exclusive() {
    // 标准形态（科目行×期间列）命中时不再试转置
    let standard = trend_sheet(&["科目,单位,2026-09,2026-10", "总销售额,元,1,2"].join("\n"));
    let m = detect(&standard, &names());
    assert_eq!(m.role, SheetRole::Trend);
    assert!(m.transposed.is_none(), "两者互斥");
    assert!(m.header.is_some(), "标准锚点保留表头块");
}
