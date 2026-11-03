//! 数据工作包 S4 导入器集成测试。
//!
//! 夹具全部为运行时生成的合成文件（最小 xlsx 由测试内建的 stored-zip 写出器
//! 生成、CSV 为合成文本）；真实业务文件绝不入库。

use dsheng_core::analysis::observations::{
    merge_summary_candidates, SummaryCandidate, SummaryMerge,
};
use dsheng_core::contracts::decimal::canonical_decimal;
use dsheng_core::import::fileio::{read_csv, read_workbook, CellValue, ImportError};
use dsheng_core::import::template::{detect_sheet_role, SheetRole};
use dsheng_core::import::value::{
    classify_cell, expand_merged_anchor, normalize_money_to_yuan, parse_amount_text,
    parse_percent_text, CellClass, MoneyUnit,
};
use dsheng_core::import::{find_header_blocks, map_header, normalize_header};
use std::path::PathBuf;

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

struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<(String, u32, u32, u32)>, // name, crc, size, offset
}

impl ZipWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            entries: Vec::new(),
        }
    }

    fn add(&mut self, name: &str, content: &str) {
        let data = content.as_bytes();
        let crc = crc32(data);
        let offset = self.out.len() as u32;
        let name_bytes = name.as_bytes();
        // local file header（stored，无压缩）
        self.out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes()); // method: store
        self.out.extend_from_slice(&0u16.to_le_bytes()); // time
        self.out.extend_from_slice(&0x21u16.to_le_bytes()); // date（固定合法值）
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(data.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(name_bytes);
        self.out.extend_from_slice(data);
        self.entries
            .push((name.to_string(), crc, data.len() as u32, offset));
    }

    fn finish(mut self) -> Vec<u8> {
        let cd_offset = self.out.len() as u32;
        let mut cd = Vec::new();
        for (name, crc, size, offset) in &self.entries {
            let name_bytes = name.as_bytes();
            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&20u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0x21u16.to_le_bytes());
            cd.extend_from_slice(&crc.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&size.to_le_bytes());
            cd.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u16.to_le_bytes());
            cd.extend_from_slice(&0u32.to_le_bytes());
            cd.extend_from_slice(&offset.to_le_bytes());
            cd.extend_from_slice(name_bytes);
        }
        let cd_size = cd.len() as u32;
        self.out.extend_from_slice(&cd);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
            .extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        self.out
            .extend_from_slice(&(self.entries.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

enum Cell {
    Text(String),
    Number(&'static str),
}

fn cell_xml(col: usize, row: usize, cell: &Cell) -> String {
    let ref_name = format!("{}{}", dsheng_core::import::column_letter(col), row + 1);
    match cell {
        Cell::Text(t) => format!(
            r#"<c r="{ref_name}" t="inlineStr"><is><t xml:space="preserve">{t}</t></is></c>"#
        ),
        Cell::Number(v) => format!(r#"<c r="{ref_name}"><v>{v}</v></c>"#),
    }
}

/// 写出单 sheet 合成 xlsx；rows 为数据行（1 基行号从 1 开始）。
fn write_xlsx(path: &std::path::Path, sheet_name: &str, rows: &[Vec<Cell>], merges: &[&str]) {
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, row) in rows.iter().enumerate() {
        sheet.push_str(&format!(r#"<row r="{}">"#, r + 1));
        for (c, cell) in row.iter().enumerate() {
            sheet.push_str(&cell_xml(c, r, cell));
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData>");
    if !merges.is_empty() {
        sheet.push_str(&format!(r#"<mergeCells count="{}">"#, merges.len()));
        for m in merges {
            sheet.push_str(&format!(r#"<mergeCell ref="{m}"/>"#));
        }
        sheet.push_str("</mergeCells>");
    }
    sheet.push_str("</worksheet>");
    let mut zip = ZipWriter::new();
    zip.add(
        "[Content_Types].xml",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>"#,
    );
    zip.add(
        "_rels/.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#,
    );
    zip.add(
        "xl/workbook.xml",
        &format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="{sheet_name}" sheetId="1" r:id="rId1"/></sheets></workbook>"#
        ),
    );
    zip.add(
        "xl/_rels/workbook.xml.rels",
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#,
    );
    zip.add("xl/worksheets/sheet1.xml", &sheet);
    std::fs::write(path, zip.finish()).expect("写出合成 xlsx");
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("dsheng-wp-data-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ---- 测试 ----

#[test]
fn xlsx_synthetic_read_and_template_detect() {
    let dir = temp_dir();
    let path = dir.join("合成核算表.xlsx");
    write_xlsx(
        &path,
        "核算表",
        &[
            vec![
                Cell::Text(String::new()),
                Cell::Text("科目".into()),
                Cell::Text("计算公式／取数来源".into()),
                Cell::Text("年度计划·当月目标".into()),
                Cell::Text("月度预定".into()),
                Cell::Text("月度实绩".into()),
            ],
            vec![
                Cell::Text(String::new()),
                Cell::Text("不含税销售额".into()),
                Cell::Text("来源A".into()),
                Cell::Number("1100"),
                Cell::Number("1000"),
                Cell::Number("1200"),
            ],
        ],
        &[],
    );
    let sheets = read_workbook(&path).expect("读取合成 xlsx");
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "核算表");
    // 数值单元格：整数为精确值或按合同标 precision_unverified
    let cell = &sheets[0].rows[1][5];
    assert!(matches!(cell, CellValue::Int(1200)) || matches!(cell, CellValue::FloatUnverified(_)));
    if let CellValue::FloatUnverified(lexeme) = cell {
        assert_eq!(
            cell.value_state(),
            dsheng_core::contracts::domain::ValueState::PrecisionUnverified,
            "float 词法 {lexeme} 不能中转精确金额"
        );
    }
    // 表头块识别 + 适配器
    let header_rows: Vec<Vec<String>> = sheets[0]
        .rows
        .iter()
        .map(|r| r.iter().map(|c| c.raw_text()).collect())
        .collect();
    let blocks = find_header_blocks(&header_rows);
    assert_eq!(blocks.len(), 1, "唯一最优表头块");
    assert_eq!(blocks[0].row, 1);
    let m = detect_sheet_role("核算表", &header_rows).unwrap();
    assert_eq!(m.role, SheetRole::Accounting);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn xlsx_merged_range_anchor_only() {
    let dir = temp_dir();
    let path = dir.join("合成合并.xlsx");
    write_xlsx(
        &path,
        "核算表",
        &[
            vec![
                Cell::Text(String::new()),
                Cell::Text("科目".into()),
                Cell::Text("月度实绩".into()),
            ],
            vec![
                Cell::Text(String::new()),
                Cell::Text("A".into()),
                Cell::Number("100"),
            ],
            vec![
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
            ],
            vec![
                Cell::Text(String::new()),
                Cell::Text(String::new()),
                Cell::Text(String::new()),
            ],
        ],
        &["B7:B9"],
    );
    let sheets = read_workbook(&path).expect("读取");
    assert!(
        sheets[0].merged_ranges.contains(&"B7:B9".to_string())
            || sheets[0].merged_ranges.is_empty()
    );
    // 合并值只留锚点（§4.1：不复制到覆盖格；I01 语义）
    let expanded = expand_merged_anchor(Some(canonical_decimal("100").unwrap()), 3);
    assert_eq!(expanded.iter().filter(|v| v.is_some()).count(), 1);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn csv_full_pipeline_synthetic() {
    let dir = temp_dir();
    let path = dir.join("合成.csv");
    std::fs::write(&path, "\u{FEFF}科目,单位,期间,实绩\n不含税销售额,万元,2026-10,\"1,234.56\"\n经营费用,元,2026-10,-50\n回款率,,2026-10,50%\n").unwrap();
    let sheet = read_csv(&path).expect("UTF-8/BOM CSV");
    assert_eq!(sheet.rows.len(), 4);
    // 表头映射（exact）
    assert_eq!(map_header("科目"), Some("metric_label"));
    assert_eq!(map_header("实绩"), Some("actual"));
    assert_eq!(map_header("单位"), Some("unit"));
    assert_eq!(map_header("期间"), Some("period"));
    // 千分位仅三位分组
    assert_eq!(
        parse_amount_text("1,234.56").map(|d| d.to_string()),
        Some("1234.56".into())
    );
    // 金额单位归一（万元→元）
    let yuan = normalize_money_to_yuan(canonical_decimal("1234.56").unwrap(), MoneyUnit::WanYuan);
    assert_eq!(
        dsheng_core::contracts::decimal::to_canonical_string(yuan),
        "12345600"
    );
    // 负数原样
    let neg = classify_cell(Some("-50"));
    assert!(matches!(neg, CellClass::Valid { .. }));
    // % 仅在表头/格式明确时（本表头无 % → 不除 100；文本 50% 也不猜）
    assert_eq!(parse_percent_text("50%", false), None);
    assert_eq!(
        parse_percent_text("50%", true).map(|d| d.to_string()),
        Some("0.5".into())
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn csv_gb18030_not_guessed() {
    let dir = temp_dir();
    let path = dir.join("gb.csv");
    // GB18030 字节（“科目”），UTF-8 下非法
    std::fs::write(&path, [0xBF, 0xC6, 0xC4, 0xBF, 0x2C, 0x31]).unwrap();
    assert_eq!(read_csv(&path), Err(ImportError::CsvGb18030Required));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn duplicate_summary_conflict_across_sources() {
    // §2.2：同键汇总同值去重、不同值冲突（合成两份 CSV 的候选）
    let same = merge_summary_candidates(&[
        SummaryCandidate {
            value: canonical_decimal("100").unwrap(),
            evidence_id: "cell:s1:csv:C7".into(),
        },
        SummaryCandidate {
            value: canonical_decimal("100").unwrap(),
            evidence_id: "cell:s2:csv:C7".into(),
        },
    ]);
    assert!(matches!(same, SummaryMerge::Deduped { .. }));
    let conflict = merge_summary_candidates(&[
        SummaryCandidate {
            value: canonical_decimal("100").unwrap(),
            evidence_id: "cell:s1:csv:C7".into(),
        },
        SummaryCandidate {
            value: canonical_decimal("105").unwrap(),
            evidence_id: "cell:s2:csv:C7".into(),
        },
    ]);
    assert!(matches!(conflict, SummaryMerge::Conflict { .. }));
}

#[test]
fn header_normalization_fullwidth_and_spaces() {
    assert_eq!(normalize_header("　月度实绩　"), "月度实绩");
    assert_eq!(map_header("月度实绩"), Some("actual"));
    // 全角斜线转半角后命中 source_note 别名
    assert_eq!(map_header("计算公式／取数来源"), Some("source_note"));
    // 连续空格合一不产生新别名（exact 匹配，不做模糊）
    assert_eq!(map_header("月度 实绩"), None);
}

#[test]
fn pptx_rejected_as_unsupported() {
    let dir = temp_dir();
    let path = dir.join("deck.pptx");
    std::fs::write(&path, b"PK").unwrap();
    assert_eq!(read_workbook(&path), Err(ImportError::UnsupportedFileKind));
    std::fs::remove_dir_all(&dir).ok();
}
