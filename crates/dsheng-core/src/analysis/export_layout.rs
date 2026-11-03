//! 导出单元格类型决策（data-period-import.md §5，纯逻辑；写文件由 CLI/导出工作包执行）。
//!
//! - 文本以 `=`、`+`、`-`、`@` 开头时 XLSX 写文本类型，CSV 以单引号安全转义；
//! - 真正数值（包括负数）保留数值类型，不套用字符串防护；
//! - XLSX 超过 15 位有效数写精确 decimal 文本并标 `number_storage=decimal_text`。

use rust_decimal::Decimal;

/// XLSX 单元格写出类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XlsxCellKind {
    Text,
    Number,
    /// 精确 decimal 文本（超过 15 位有效数）。
    DecimalText,
}

impl XlsxCellKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::DecimalText => "text",
        }
    }

    /// 超精度时的存储标记。
    pub fn number_storage(self) -> Option<&'static str> {
        match self {
            Self::DecimalText => Some("decimal_text"),
            _ => None,
        }
    }
}

/// 文本开头是否触发公式防护字符。
fn starts_with_formula_guard(text: &str) -> bool {
    matches!(text.chars().next(), Some('=' | '+' | '-' | '@'))
}

/// 判定 XLSX 写出类型。
///
/// `numeric_value`：业务数值（含负数）→ number/decimal_text；
/// 文本（含公式样式字符串）→ text。
pub fn xlsx_export_kind(value_text: &str, numeric: bool) -> XlsxCellKind {
    if numeric {
        let digits = significant_digits(value_text);
        if digits > 15 {
            XlsxCellKind::DecimalText
        } else {
            XlsxCellKind::Number
        }
    } else {
        XlsxCellKind::Text
    }
}

/// CSV 安全转义：以 =、+、-、@ 开头的文本加前导单引号。
pub fn csv_export_text(text: &str) -> String {
    if starts_with_formula_guard(text) {
        format!("'{text}")
    } else {
        text.to_string()
    }
}

fn significant_digits(value_text: &str) -> u32 {
    let digits: String = value_text
        .trim_start_matches('-')
        .chars()
        .filter(|c| c.is_ascii_digit())
        .collect();
    let trimmed = digits.trim_start_matches('0');
    trimmed.len() as u32
}

/// 数值文本本身是否以防护字符开头（用于文本误写防护说明）。
pub fn text_needs_guard(text: &str) -> bool {
    starts_with_formula_guard(text)
}

/// 规范数值文本辅助（供导出器复用；不改变值）。
pub fn canonical_or_raw(value: Decimal) -> String {
    crate::contracts::decimal::to_canonical_string(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        assert_eq!(xlsx_export_kind("=SUM(A1:A3)", false), XlsxCellKind::Text);
        assert_eq!(xlsx_export_kind("-12", true), XlsxCellKind::Number);
        assert_eq!(
            xlsx_export_kind("1234567890123456", true),
            XlsxCellKind::DecimalText
        );
        assert_eq!(
            xlsx_export_kind("1234567890123456", true).number_storage(),
            Some("decimal_text")
        );
        assert_eq!(
            xlsx_export_kind("123456789012345", true),
            XlsxCellKind::Number
        );
    }

    #[test]
    fn csv_guard() {
        assert_eq!(csv_export_text("=SUM(A1:A3)"), "'=SUM(A1:A3)");
        assert_eq!(csv_export_text("普通文本"), "普通文本");
    }
}
