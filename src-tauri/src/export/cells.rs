//! 单元格写出策略（§5）：公式注入防护、decimal 文本、CSV 转义。
//!
//! 类型决策复用 dsheng-core `analysis::export_layout`（纯逻辑）；本文件只做
//! rust_xlsxwriter 写入与 CSV 行拼装。数值 ≤15 位有效数时写数值类型——
//! f64 仅作为 XLSX 数值格的载体（XLSX 数值本身即 IEEE double），写前校验
//! 最短往返字面量与规范 decimal 字符串逐���相等，否则降级为 decimal 文本，
//! 保证单元格字面量与业务值一致、不经 f64 算术。

use super::ExportResult;
use dsheng_core::analysis::export_layout::{csv_export_text, xlsx_export_kind, XlsxCellKind};
use rust_xlsxwriter::{Format, Worksheet};

/// Excel 单元格文本上限（UTF-16 code units）。长证据串必须保留为可下载文本，
/// 但不能让一个来源单元格超过 XLSX 的硬限制而使整份核算表导出失败。
const XLSX_MAX_TEXT_UNITS: usize = 32_767;
const XLSX_TRUNCATION_MARKER: &str = "…[已截断]";

/// 一次写入的记录：>15 位有效数的单元格（number_storage=decimal_text）。
#[derive(Debug, Default, Clone)]
pub struct DecimalTextLog {
    pub cells: Vec<DecimalTextCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecimalTextCell {
    pub sheet: String,
    pub cell: String,
    pub value: String,
}

/// A1 坐标。
pub fn a1(row: u32, col: u16) -> String {
    let mut col_text = String::new();
    let mut c = col as u32 + 1;
    while c > 0 {
        let rem = (c - 1) % 26;
        col_text.insert(0, (b'A' + rem as u8) as char);
        c = (c - 1) / 26;
    }
    format!("{col_text}{}", row + 1)
}

/// 写文本格（含防护：=+-@ 开头仍写文本类型——rust_xlsxwriter write_string 本就
/// 不会解释为公式；显式用 string 类型即为防护）。
pub fn write_text(ws: &mut Worksheet, row: u32, col: u16, text: &str) -> ExportResult<()> {
    let safe = truncate_xlsx_text(text);
    ws.write_string(row, col, &safe)
        .map(|_| ())
        .map_err(|e| super::ExportError(format!("写文本失败 {}: {e}", a1(row, col))))
}

/// 将任意业务文本压到 XLSX 单元格上限内；按 UTF-16 计数，避免 emoji 等
/// 非 BMP 字符造成“看起来未超长、实际写入失败”的边界错误。
pub fn truncate_xlsx_text(text: &str) -> String {
    if text.encode_utf16().count() <= XLSX_MAX_TEXT_UNITS {
        return text.to_owned();
    }
    let marker_units = XLSX_TRUNCATION_MARKER.encode_utf16().count();
    let limit = XLSX_MAX_TEXT_UNITS.saturating_sub(marker_units);
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let units = ch.len_utf16();
        if used + units > limit {
            break;
        }
        out.push(ch);
        used += units;
    }
    out.push_str(XLSX_TRUNCATION_MARKER);
    out
}

/// 写业务数值格：规范 decimal 字符串 → 数值或 decimal 文本（>15 位有效数）。
pub fn write_decimal(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    sheet: &str,
    canonical: &str,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    match xlsx_export_kind(canonical, true) {
        XlsxCellKind::Number => {
            // 载体校验：最短往返字面量须与规范字符串逐字一致，否则降级为文本
            let parsed: Result<f64, _> = canonical.parse::<f64>();
            match parsed {
                Ok(f) if shortest_literal(f) == canonical => ws
                    .write_number(row, col, f)
                    .map(|_| ())
                    .map_err(|e| super::ExportError(format!("写数值失败 {}: {e}", a1(row, col)))),
                _ => {
                    log.cells.push(DecimalTextCell {
                        sheet: sheet.to_string(),
                        cell: a1(row, col),
                        value: canonical.to_string(),
                    });
                    write_text(ws, row, col, canonical)
                }
            }
        }
        XlsxCellKind::DecimalText | XlsxCellKind::Text => {
            log.cells.push(DecimalTextCell {
                sheet: sheet.to_string(),
                cell: a1(row, col),
                value: canonical.to_string(),
            });
            write_text(ws, row, col, canonical)
        }
    }
}

/// 可空 decimal：None 写空（空值保留空，不写 0）。
pub fn write_opt_decimal(
    ws: &mut Worksheet,
    row: u32,
    col: u16,
    sheet: &str,
    value: Option<&str>,
    log: &mut DecimalTextLog,
) -> ExportResult<()> {
    match value {
        Some(v) if !v.is_empty() => write_decimal(ws, row, col, sheet, v, log),
        _ => Ok(()),
    }
}

/// f64 最短往返字面量（与 XLSX XML 写出一致的形式；整数去 `.0`）。
fn shortest_literal(f: f64) -> String {
    if f.fract() == 0.0 && f.abs() < 1e15 {
        format!("{}", f as i64)
    } else {
        let mut s = format!("{f}");
        if s.contains('e') || s.contains('E') {
            return s;
        }
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        s
    }
}

/// 表头粗体格式。
pub fn header_format() -> Format {
    Format::new().set_bold()
}

/// 写一行表头。
pub fn write_headers(ws: &mut Worksheet, row: u32, headers: &[&str]) -> ExportResult<()> {
    let fmt = header_format();
    for (i, h) in headers.iter().enumerate() {
        ws.write_string_with_format(row, i as u16, *h, &fmt)
            .map_err(|e| super::ExportError(format!("写表头失败: {e}")))?;
    }
    Ok(())
}

/// CSV 行拼装（RFC4180 引号 + 公式防护单引号）。
pub fn csv_row(fields: &[String]) -> String {
    fields
        .iter()
        .map(|f| {
            let guarded = csv_export_text(f);
            if guarded.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", guarded.replace('"', "\"\""))
            } else {
                guarded
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a1_coordinates() {
        assert_eq!(a1(0, 0), "A1");
        assert_eq!(a1(5, 25), "Z6");
        assert_eq!(a1(2, 26), "AA3");
    }

    #[test]
    fn shortest_literal_roundtrip() {
        assert_eq!(shortest_literal(11500.25), "11500.25");
        assert_eq!(shortest_literal(12500.0), "12500");
        assert_eq!(shortest_literal(-0.5), "-0.5");
    }

    #[test]
    fn csv_guard_and_quote() {
        assert_eq!(
            csv_row(&["=SUM(A1)".into(), "a,b".into()]),
            "'=SUM(A1),\"a,b\""
        );
    }

    #[test]
    fn xlsx_text_is_bounded_in_utf16_units() {
        let text = "😀".repeat(20_000);
        let safe = truncate_xlsx_text(&text);
        assert!(safe.encode_utf16().count() <= XLSX_MAX_TEXT_UNITS);
        assert!(safe.ends_with(XLSX_TRUNCATION_MARKER));
        assert_eq!(truncate_xlsx_text("短文本"), "短文本");
    }
}
