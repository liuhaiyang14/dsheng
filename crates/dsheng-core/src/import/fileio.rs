//! 工作簿/CSV 读取（data-period-import.md §4.1）。
//!
//! - 仅 xlsx/xls/csv；PPTX 等标 unsupported，不外发。
//! - CSV 接受 UTF-8/BOM；解码失败要求用户显式选择 GB18030（`csv_gb18030_required`），
//!   不猜乱码。
//! - XLSX 数值优先读取 XML 原始字面量（[`super::xlsx_literal`]，与读取库 float 交叉
//!   核验一致才采用）；只有 float 且不能恢复原始字面量时标 `precision_unverified`
//!   （§1.1：禁止以 f64 中转精确金额；整数为精确值）。
//! - 不执行宏、外链或未知公式；公式原文保留，缓存值仅人工对照。
//! - 行列按绝对坐标展开：`rows[r][c]` 恒为 Excel 第 r+1 行、第 c+1 列
//!   （首个已用单元格不在 A1 时也不截断末行/末列）。

use super::xlsx_literal::{read_numeric_literals, SheetLiterals};
use calamine::{Data, Range, Reader, Xls, Xlsx};
use std::io::Read;
use std::path::Path;

/// 读取错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("unsupported_file_kind")]
    UnsupportedFileKind,
    #[error("workbook_read_failed: {0}")]
    WorkbookReadFailed(String),
    #[error("csv_gb18030_required")]
    CsvGb18030Required,
    #[error("csv_read_failed: {0}")]
    CsvReadFailed(String),
}

/// 单元格值（词法保留；数值经规范十进制字符串化或标 precision_unverified）。
#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    /// 精确整数。
    Int(i64),
    /// XLSX 数值单元格的 XML 原始字面量（已与读取库 float 交叉核验一致；§1.1）。
    /// 精确十进制换算见 [`super::value::parse_numeric_literal`]。
    Number(String),
    /// 浮点词法（calamine 仅给 f64）：原文审计用，值状态 precision_unverified。
    FloatUnverified(String),
    Bool(bool),
    /// 公式原文与缓存（缓存不作为 valid 值）。
    Formula {
        source: String,
        cached: Option<String>,
    },
    Error(String),
}

impl CellValue {
    /// 原文文本（raw_text 语义）。
    pub fn raw_text(&self) -> String {
        match self {
            Self::Empty => String::new(),
            Self::Text(t) | Self::Number(t) | Self::FloatUnverified(t) => t.clone(),
            Self::Int(i) => i.to_string(),
            Self::Bool(b) => b.to_string(),
            Self::Formula { source, .. } => source.clone(),
            Self::Error(e) => e.clone(),
        }
    }

    /// 值状态映射（§1.1；公式缓存不写入业务值）。
    ///
    /// `Number` 超输入精度（>6 位小数 / >28 位有效数字）为 invalid
    /// （decimal_precision_exceeded / decimal_overflow），不截断。
    pub fn value_state(&self) -> crate::contracts::domain::ValueState {
        use crate::contracts::domain::ValueState;
        match self {
            Self::Empty | Self::Text(_) | Self::Bool(_) | Self::Error(_) => ValueState::Missing,
            Self::Int(_) => ValueState::Valid,
            Self::Number(lit) => match super::value::parse_numeric_literal(lit) {
                Ok(_) => ValueState::Valid,
                Err(_) => ValueState::Invalid,
            },
            Self::FloatUnverified(_) => ValueState::PrecisionUnverified,
            Self::Formula { .. } => ValueState::UnsupportedFormula,
        }
    }

    /// 精确值（Int 与可换算的 XML 数值字面量）；float 不经 f64 转业务值。
    pub fn exact_decimal(&self) -> Option<rust_decimal::Decimal> {
        match self {
            Self::Int(i) => Some(rust_decimal::Decimal::from(*i)),
            Self::Number(lit) => super::value::parse_numeric_literal(lit).ok(),
            _ => None,
        }
    }
}

/// 一个 sheet 的矩形数据（含合并范围 A1 串）。
#[derive(Debug, Clone, PartialEq)]
pub struct SheetData {
    pub name: String,
    pub rows: Vec<Vec<CellValue>>,
    /// 合并单元格范围（如 "B7:B9"；值只保留左上锚点，见 §4.1）。
    pub merged_ranges: Vec<String>,
}

/// 读取 xlsx/xls：全部 sheet；kind unsupported 的扩展名直接拒绝。
pub fn read_workbook(path: &Path) -> Result<Vec<SheetData>, ImportError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("xlsx") => read_xlsx(path),
        Some("xls") => read_xls(path),
        _ => Err(ImportError::UnsupportedFileKind),
    }
}

/// 按绝对坐标（A1=(0,0)）展开到已用区域右下角；`literals` 为该 sheet 的
/// XML 数值字面量（xls 或包结构不可解析时为 None）。
fn convert_range(range: &Range<Data>, literals: Option<&SheetLiterals>) -> Vec<Vec<CellValue>> {
    let Some((end_row, end_col)) = range.end() else {
        return Vec::new();
    };
    let mut rows = Vec::with_capacity(end_row as usize + 1);
    for r in 0..=end_row {
        let mut row = Vec::with_capacity(end_col as usize + 1);
        for c in 0..=end_col {
            let literal = literals.and_then(|m| m.get(&(r, c))).map(String::as_str);
            row.push(convert_cell(range.get_value((r, c)), literal));
        }
        rows.push(row);
    }
    rows
}

fn convert_cell(d: Option<&Data>, literal: Option<&str>) -> CellValue {
    match d {
        None | Some(Data::Empty) => CellValue::Empty,
        Some(Data::String(s)) => CellValue::Text(s.clone()),
        Some(Data::Int(i)) => CellValue::Int(*i),
        Some(Data::Float(f)) => number_cell(*f, literal),
        Some(Data::Bool(b)) => CellValue::Bool(*b),
        Some(Data::DateTime(dt)) => CellValue::Text(dt.to_string()),
        Some(Data::Error(e)) => CellValue::Error(format!("{e}")),
        Some(Data::DateTimeIso(s)) => CellValue::Text(s.to_string()),
        Some(Data::DurationIso(s)) => CellValue::Text(s.to_string()),
    }
}

/// 数值单元格（§1.1）：XML 原始字面量存在且与读取库 f64 一致时采用字面量
/// （纯整数 → `Int`，其余 → `Number(原文)`）；否则 `FloatUnverified`。
fn number_cell(value: f64, literal: Option<&str>) -> CellValue {
    if let Some(lit) = literal {
        if lit.parse::<f64>().ok() == Some(value) {
            if let Ok(i) = lit.parse::<i64>() {
                return CellValue::Int(i);
            }
            return CellValue::Number(lit.to_string());
        }
    }
    CellValue::FloatUnverified(format!("{value}"))
}

fn read_xlsx(path: &Path) -> Result<Vec<SheetData>, ImportError> {
    let file =
        std::fs::File::open(path).map_err(|e| ImportError::WorkbookReadFailed(e.to_string()))?;
    let mut book = Xlsx::new(file).map_err(|e| ImportError::WorkbookReadFailed(e.to_string()))?;
    // §1.1 数值优先 XML 原始字面量；包结构不可解析 → 无字面量，回退 precision_unverified
    let literals = read_numeric_literals(path).unwrap_or_default();
    let sheet_names = book.sheet_names().to_vec();
    let mut out = Vec::new();
    for name in sheet_names {
        let Ok(range) = book.worksheet_range(&name) else {
            continue;
        };
        let merged = book
            .merge_cells_by_sheet_name(&name)
            .unwrap_or_default()
            .iter()
            .map(|r| {
                // Dimensions：start/end 为 (row, col) 公开字段
                let (sr, sc) = r.start;
                let (er, ec) = r.end;
                format!(
                    "{}:{}",
                    crate::import::headers::a1((sc, sr)),
                    crate::import::headers::a1((ec, er))
                )
            })
            .collect();
        let rows = convert_range(&range, literals.get(&name));
        out.push(SheetData {
            name,
            rows,
            merged_ranges: merged,
        });
    }
    Ok(out)
}

fn read_xls(path: &Path) -> Result<Vec<SheetData>, ImportError> {
    let file =
        std::fs::File::open(path).map_err(|e| ImportError::WorkbookReadFailed(e.to_string()))?;
    let mut book = Xls::new(file).map_err(|e| ImportError::WorkbookReadFailed(e.to_string()))?;
    let sheet_names = book.sheet_names().to_vec();
    let mut out = Vec::new();
    for name in sheet_names {
        let Ok(range) = book.worksheet_range(&name) else {
            continue;
        };
        out.push(SheetData {
            name,
            rows: convert_range(&range, None),
            merged_ranges: Vec::new(),
        });
    }
    Ok(out)
}

/// CSV 读取：UTF-8（含 BOM）；失败 → [`ImportError::CsvGb18030Required`]
/// （要求用户显式选择 GB18030，不猜乱码）。RFC4180 逻辑记录，行号 1 基。
pub fn read_csv(path: &Path) -> Result<SheetData, ImportError> {
    let mut raw = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut f| f.read_to_end(&mut raw))
        .map_err(|e| ImportError::CsvReadFailed(e.to_string()))?;
    let text = decode_utf8(&raw).ok_or(ImportError::CsvGb18030Required)?;
    parse_csv_text(&text)
}

/// 解码 UTF-8（剥离 BOM）。
pub fn decode_utf8(bytes: &[u8]) -> Option<String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).ok()
}

/// 解析 CSV 文本为矩形表（缺失尾列补 Empty；行按最长行对齐）。
pub fn parse_csv_text(text: &str) -> Result<SheetData, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rows = Vec::new();
    for record in reader.records() {
        let record = record.map_err(|e| ImportError::CsvReadFailed(e.to_string()))?;
        rows.push(
            record
                .iter()
                .map(|c| {
                    if c.is_empty() {
                        CellValue::Empty
                    } else {
                        CellValue::Text(c.to_string())
                    }
                })
                .collect(),
        );
    }
    let width = rows.iter().map(Vec::len).max().unwrap_or(0);
    for row in &mut rows {
        row.resize(width, CellValue::Empty);
    }
    Ok(SheetData {
        name: "csv".into(),
        rows,
        merged_ranges: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_utf8_and_bom() {
        let sheet = parse_csv_text("科目,实绩\n不含税销售额,100").unwrap();
        assert_eq!(sheet.rows.len(), 2);
        assert_eq!(sheet.rows[0][0], CellValue::Text("科目".into()));
        assert_eq!(sheet.rows[1][1], CellValue::Text("100".into()));
        let with_bom = "\u{FEFF}科目,实绩\n";
        assert_eq!(
            parse_csv_text(with_bom).unwrap().rows[0][0],
            CellValue::Text("科目".into())
        );
    }

    #[test]
    fn csv_invalid_utf8_requires_gb18030_choice() {
        // GB18030 编码的 “科目” 在 UTF-8 下非法
        let gb = [0xBF, 0xC6, 0xC4, 0xBF];
        assert!(decode_utf8(&gb).is_none());
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.csv");
        std::fs::write(&p, gb).unwrap();
        assert_eq!(read_csv(&p), Err(ImportError::CsvGb18030Required));
    }

    #[test]
    fn unsupported_kind_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("deck.pptx");
        std::fs::write(&p, b"xx").unwrap();
        assert_eq!(read_workbook(&p), Err(ImportError::UnsupportedFileKind));
    }
}
