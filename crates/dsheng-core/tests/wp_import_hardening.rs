//! 导入器真实文件硬化（核心侧）：XLSX 数值 XML 原始字面量与绝对坐标。
//!
//! 夹具为测试内构造的最小 xlsx（stored zip，calamine 可读），模仿真实核算表
//! 结构：第 1–2 行整行合并标题/说明、第 4 行表头、内联字符串科目列、
//! C 列公式说明文本、带样式的空数值格、显式 0、负数与两位小数金额。
//! 数值全部为合成值；真实业务文件绝不入库。

use dsheng_core::contracts::domain::ValueState;
use dsheng_core::import::fileio::{read_workbook, CellValue};
use rust_decimal::Decimal;
use std::path::{Path, PathBuf};

// ---- 最小 xlsx 写出器（stored zip；仅测试夹具用） ----

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        crc = table[((crc ^ *b as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc ^ 0xFFFF_FFFF
}

fn stored_zip(parts: &[(&str, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, content) in parts {
        let data = content.as_bytes();
        let crc = crc32(data);
        let offset = out.len() as u32;
        let name_bytes = name.as_bytes();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // stored
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0x21u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes());
        central.extend_from_slice(&0x21u16.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 12]); // extra/comment/disk/attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }
    let cd_offset = out.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&(central.len() as u32).to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// 单元格 XML 片段（真实文件形态：内联串、带样式数值、带样式空格、公式）。
enum C {
    T(&'static str, &'static str),
    N(&'static str, &'static str),
    Blank(&'static str),
    F(&'static str, &'static str, &'static str),
}

fn cell_xml(c: &C) -> String {
    match c {
        C::T(r, t) => format!(r#"<c r="{r}" s="2" t="inlineStr"><is><t>{t}</t></is></c>"#),
        C::N(r, v) => format!(r#"<c r="{r}" s="1" t="n"><v>{v}</v></c>"#),
        C::Blank(r) => format!(r#"<c r="{r}" s="1" t="n"></c>"#),
        C::F(r, f, v) => format!(r#"<c r="{r}" s="1"><f>{f}</f><v>{v}</v></c>"#),
    }
}

fn sheet_xml(rows: &[(u32, Vec<C>)], merges: &[&str]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, cells) in rows {
        s.push_str(&format!(r#"<row r="{r}">"#));
        for c in cells {
            s.push_str(&cell_xml(c));
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if !merges.is_empty() {
        s.push_str(&format!(r#"<mergeCells count="{}">"#, merges.len()));
        for m in merges {
            s.push_str(&format!(r#"<mergeCell ref="{m}"/>"#));
        }
        s.push_str("</mergeCells>");
    }
    s.push_str("</worksheet>");
    s
}

fn write_workbook(path: &Path, sheets: &[(&str, String)]) {
    let mut overrides = String::new();
    let mut entries = String::new();
    let mut rels = String::new();
    let mut parts: Vec<(String, String)> = Vec::new();
    for (i, (name, xml)) in sheets.iter().enumerate() {
        let n = i + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        entries.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
        parts.push((format!("xl/worksheets/sheet{n}.xml"), xml.clone()));
    }
    let styles = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font/></fonts><fills count="1"><fill/></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="4" applyNumberFormat="1"/><xf numFmtId="0"/></cellXfs></styleSheet>"#;
    let mut all: Vec<(&str, String)> = vec![
        (
            "[Content_Types].xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>{overrides}</Types>"#
            ),
        ),
        (
            "_rels/.rels",
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.to_string(),
        ),
        (
            "xl/workbook.xml",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{entries}</sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels",
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}<Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        ("xl/styles.xml", styles.to_string()),
    ];
    for (name, xml) in &parts {
        all.push((name.as_str(), xml.clone()));
    }
    std::fs::write(path, stored_zip(&all)).expect("写出合成 xlsx");
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dsheng-import-hard-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 合成核算表（行号与真实布局形态一致，数值为合成值）。
fn accounting_rows() -> Vec<(u32, Vec<C>)> {
    use C::*;
    vec![
        (
            1,
            vec![T("A1", "合成公司 — 2026年10月整体核算表（合成夹具）")],
        ),
        (2, vec![T("A2", "模板二（费用项下含人工费）｜合成说明行")]),
        (
            4,
            vec![
                T("A4", "序号"),
                T("B4", "科目"),
                T("C4", "计算公式 / 取数来源"),
                T("D4", "本月MP"),
                T("E4", "年度计划·当月目标"),
                T("F4", "月度预定"),
                T("G4", "销售额占比(预定)"),
                T("H4", "月度实绩"),
                T("I4", "销售额占比(实绩)"),
                T("J4", "预实差异额"),
                T("K4", "实绩达成预定比"),
                T("L4", "实绩达成年度计划比"),
            ],
        ),
        (10, vec![T("A10", "【收入部分】")]),
        (
            12,
            vec![
                T("A12", "A5"),
                T("B12", "总销售额"),
                T("C12", "A5=A1+A2+A3+A4"),
                T("D12", "—"),
                N("E12", "360000"),
                N("F12", "200000"),
                N("G12", "1"),
                N("H12", "123456.78"),
                N("I12", "1"),
                N("J12", "-76543.22"),
                N("K12", "0.6172839"),
                F("L12", "H12/E12", "0.3429355"),
            ],
        ),
        (
            20,
            vec![
                T("A20", "B20"),
                T("B20", "人工费（不可作为削减对象）"),
                T("C20", "直接取数·工资条"),
                Blank("E20"),
                N("F20", "20000"),
                N("H20", "18000"),
            ],
        ),
        (
            21,
            vec![
                T("A21", "C"),
                T("B21", "结算收益（附加价值）"),
                T("C21", "C=A5-B1（=税前利润）"),
                N("E21", "40000"),
                N("F21", "30000"),
                N("H21", "-4321.37"),
            ],
        ),
        (
            36,
            vec![
                T("A36", "P1"),
                T("B36", "当月收款"),
                T("C36", "直接取数·收款单"),
                Blank("E36"),
                N("F36", "50000"),
                T("G36", "—"),
                N("H36", "0"),
            ],
        ),
        (
            38,
            vec![
                T("A38", "P3"),
                T("B38", "当月销项开票"),
                T("C38", "直接取数·销项发票"),
                Blank("E38"),
                Blank("F38"),
                T("G38", "—"),
                Blank("H38"),
            ],
        ),
    ]
}

#[test]
fn xlsx_numeric_literals_exact_and_blank_cells_empty() {
    let dir = temp_dir();
    let path = dir.join("合成核算表.xlsx");
    write_workbook(
        &path,
        &[(
            "核算表",
            sheet_xml(&accounting_rows(), &["A1:L1", "A2:L2", "A10:L10"]),
        )],
    );
    let sheets = read_workbook(&path).expect("读取合成核算表");
    let rows = &sheets[0].rows;
    let at = |a1_row: usize, col: usize| rows[a1_row - 1][col].clone();
    // 两位小数金额：取 XML 原始字面量（不再因 f64 降级为 precision_unverified）
    assert_eq!(at(12, 7), CellValue::Number("123456.78".into()));
    assert_eq!(at(12, 7).value_state(), ValueState::Valid);
    assert_eq!(
        at(12, 7).exact_decimal(),
        Some(Decimal::from_str_exact("123456.78").unwrap())
    );
    assert_eq!(at(21, 7), CellValue::Number("-4321.37".into()), "负数原样");
    assert_eq!(at(12, 9), CellValue::Number("-76543.22".into()));
    // 整数字面量为精确整数；显式 0 保留为 0
    assert_eq!(at(20, 7), CellValue::Int(18000));
    assert_eq!(at(36, 7), CellValue::Int(0));
    // 带样式的空数值格（无 <v>）仍是空格
    assert_eq!(at(38, 5), CellValue::Empty);
    assert_eq!(at(38, 7), CellValue::Empty);
    assert_eq!(at(20, 4), CellValue::Empty);
    // 超输入精度的字面量：原文保留，invalid（decimal_precision_exceeded），不截断
    assert_eq!(at(12, 10), CellValue::Number("0.6172839".into()));
    assert_eq!(at(12, 10).value_state(), ValueState::Invalid);
    assert_eq!(at(12, 10).exact_decimal(), None);
    // 公式格不采缓存字面量（仍 precision_unverified，缓存仅人工对照）
    assert!(matches!(at(12, 11), CellValue::FloatUnverified(_)));
    // C 列公式说明只是文本
    assert_eq!(at(12, 2), CellValue::Text("A5=A1+A2+A3+A4".into()));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn sheet_not_starting_at_a1_keeps_absolute_coordinates() {
    use C::*;
    let dir = temp_dir();
    let path = dir.join("偏移.xlsx");
    // 首个已用单元格为 B2、末格 D5：rows[r][c] 必须对应绝对坐标且不截断末行末列
    let rows = vec![
        (2, vec![T("B2", "科目"), T("C2", "月度实绩")]),
        (3, vec![T("B3", "当月收款"), N("C3", "12.5")]),
        (
            5,
            vec![T("B5", "当月付款（实付）"), N("C5", "7.25"), N("D5", "3")],
        ),
    ];
    write_workbook(&path, &[("核算表", sheet_xml(&rows, &[]))]);
    let sheets = read_workbook(&path).expect("读取");
    let grid = &sheets[0].rows;
    assert_eq!(grid.len(), 5, "行 1–5 全部展开（含末行）");
    assert!(
        grid.iter().all(|r| r.len() == 4),
        "列 A–D 全部展开（含末列）"
    );
    assert_eq!(grid[1][1], CellValue::Text("科目".into()));
    assert_eq!(grid[2][2], CellValue::Number("12.5".into()));
    assert_eq!(grid[4][2], CellValue::Number("7.25".into()));
    assert_eq!(grid[4][3], CellValue::Int(3));
    assert_eq!(grid[0][0], CellValue::Empty);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn blank_value_cells_are_missing_not_zero() {
    // F1：带样式的空数值格 → Empty/missing，绝不读成 0；显式 0 才是 valid 0
    let dir = temp_dir();
    let path = dir.join("合成核算表.xlsx");
    write_workbook(&path, &[("核算表", sheet_xml(&accounting_rows(), &[]))]);
    let sheets = read_workbook(&path).expect("读取");
    let rows = &sheets[0].rows;
    for (row, col) in [(38usize, 4usize), (38, 5), (38, 7), (36, 4), (20, 4)] {
        let cell = &rows[row - 1][col];
        assert_eq!(cell, &CellValue::Empty, "R{row}C{col}");
        assert_eq!(cell.value_state(), ValueState::Missing);
        assert_eq!(cell.exact_decimal(), None, "空格不产生 0");
    }
    let zero = &rows[35][7];
    assert_eq!(zero.value_state(), ValueState::Valid);
    assert_eq!(zero.exact_decimal(), Some(Decimal::ZERO));
    // `—` 占位是文本（非数值、非 0）
    assert_eq!(rows[37][6], CellValue::Text("—".into()));
    assert_eq!(rows[37][6].exact_decimal(), None);
    std::fs::remove_dir_all(&dir).ok();
}
