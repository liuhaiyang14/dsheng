//! 单元格值状态机与数值文本规范化（data-period-import.md §1.1 / §4.2 / DC I01、I04）。
//!
//! - 空白 → missing；`待补/待确认` → pending_input；`NA/N/A/-` 保留原文本记
//!   missing_token，不自动改为零；0 与负数原样 valid。
//! - 千分位仅接受三位分组，不猜逗号小数点；% 仅在明确声明时换算；
//!   金额单位元/千元/万元按 1/1000/10000 归一到元。

use crate::contracts::decimal::{
    check_significant_digits, parse_input_decimal, DecimalPolicyError, INPUT_MAX_FRACTION_DIGITS,
};
use crate::contracts::domain::ValueState;
use regex::Regex;
use rust_decimal::Decimal;
use std::sync::OnceLock;

/// 单元格值分类结果。
#[derive(Debug, Clone, PartialEq)]
pub enum CellClass {
    /// 空白；missing_token 为 `NA/N/A/-` 的原文本（保留）。
    Missing {
        missing_token: Option<String>,
    },
    PendingInput,
    Valid {
        value: Decimal,
    },
    InvalidText,
}

impl CellClass {
    pub fn value_state(&self) -> ValueState {
        match self {
            Self::Missing { .. } => ValueState::Missing,
            Self::PendingInput => ValueState::PendingInput,
            Self::Valid { .. } => ValueState::Valid,
            Self::InvalidText => ValueState::Invalid,
        }
    }
}

/// 值状态机（§1.1）。
pub fn classify_cell(raw: Option<&str>) -> CellClass {
    let Some(text) = raw else {
        return CellClass::Missing {
            missing_token: None,
        };
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return CellClass::Missing {
            missing_token: None,
        };
    }
    if trimmed == "待补" || trimmed == "待确认" {
        return CellClass::PendingInput;
    }
    if trimmed == "NA" || trimmed == "N/A" || trimmed == "-" {
        // 保留原文本记 missing_token，不改零
        return CellClass::Missing {
            missing_token: Some(text.to_string()),
        };
    }
    match parse_amount_text(trimmed) {
        Some(value) => CellClass::Valid { value },
        None => CellClass::InvalidText,
    }
}

/// 数值文本解析：规范十进制或三位分组千分位（§1.1）。
///
/// 千分位仅接受 `1,234.56` 形态；`1,23` / `12,34.5` 等逗号小数点猜测一律拒绝。
/// 原始文本允许多余尾零（`100.50`），值按规范形式校验（≤6 位小数、28 位有效数字）；
/// 前导零（`007`）、指数、正号不接受（不猜编码/科学计数文本）。
pub fn parse_amount_text(text: &str) -> Option<Decimal> {
    if let Ok(d) = parse_input_decimal(text) {
        return Some(d);
    }
    static PLAIN: OnceLock<Regex> = OnceLock::new();
    static THOUSANDS: OnceLock<Regex> = OnceLock::new();
    let plain =
        PLAIN.get_or_init(|| Regex::new(r"^-?(0|[1-9][0-9]*)\.[0-9]+$").expect("静态尾零小数正则"));
    let re = THOUSANDS.get_or_init(|| {
        Regex::new(
            r"^-?[1-9][0-9]{0,2}(,[0-9]{3})+(\.[0-9]{1,6})?$|^-?[1-9][0-9]{0,2}(,[0-9]{3})+$",
        )
        .expect("静态千分位正则")
    });
    if !plain.is_match(text) && !re.is_match(text) {
        return None;
    }
    let stripped = text.replace(',', "");
    let d = Decimal::from_str_exact(&stripped).ok()?;
    let d = d.normalize();
    if d.scale() > INPUT_MAX_FRACTION_DIGITS {
        return None;
    }
    check_significant_digits(&d).ok()?;
    Some(if d.is_zero() { Decimal::ZERO } else { d })
}

/// XLSX 数值单元格 XML 原始字面量 → Decimal（§1.1：优先读取原始字面量，不经 f64）。
///
/// 接受 XML Schema 数值词法 `-?digits[.digits][(e|E)[+-]digits]`（含尾零/指数，
/// 按精确十进制换算）；规范化后按输入精度校验：超过 6 位小数
/// → `PrecisionExceeded`，超过 28 位有效数字 → `Overflow`，不静默截断。
pub fn parse_numeric_literal(literal: &str) -> Result<Decimal, DecimalPolicyError> {
    static LITERAL: OnceLock<Regex> = OnceLock::new();
    let re = LITERAL.get_or_init(|| {
        Regex::new(r"^-?[0-9]+(\.[0-9]+)?([eE][+-]?[0-9]+)?$").expect("静态数值字面量正则")
    });
    let text = literal.trim();
    if !re.is_match(text) {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    let (mantissa, exponent) = match text.find(['e', 'E']) {
        Some(i) => (
            &text[..i],
            text[i + 1..]
                .parse::<i32>()
                .map_err(|_| DecimalPolicyError::Overflow)?,
        ),
        None => (text, 0),
    };
    let mut d = Decimal::from_str_exact(mantissa).map_err(|_| DecimalPolicyError::Overflow)?;
    if exponent < 0 {
        let scale = d.scale() + exponent.unsigned_abs();
        if scale > 28 {
            return Err(DecimalPolicyError::PrecisionExceeded {
                max: INPUT_MAX_FRACTION_DIGITS,
            });
        }
        d.set_scale(scale)
            .map_err(|_| DecimalPolicyError::Overflow)?;
    } else if exponent > 0 {
        let up = exponent.unsigned_abs();
        if up <= d.scale() {
            let scale = d.scale() - up;
            d.set_scale(scale)
                .map_err(|_| DecimalPolicyError::Overflow)?;
        } else {
            let times = up - d.scale();
            d.set_scale(0).map_err(|_| DecimalPolicyError::Overflow)?;
            for _ in 0..times {
                d = d
                    .checked_mul(Decimal::TEN)
                    .ok_or(DecimalPolicyError::Overflow)?;
            }
        }
    }
    let d = d.normalize();
    if d.scale() > INPUT_MAX_FRACTION_DIGITS {
        return Err(DecimalPolicyError::PrecisionExceeded {
            max: INPUT_MAX_FRACTION_DIGITS,
        });
    }
    check_significant_digits(&d)?;
    Ok(if d.is_zero() { Decimal::ZERO } else { d })
}

/// % 文本换算：仅在明确声明（表头/格式含 %）时把 `50%` 解析为底值 0.5。
///
/// 普通数字 0.5 不擅自乘 100（§1.4：原表 percent 底值 0.5 的业务值即 0.5）。
pub fn parse_percent_text(text: &str, percent_declared: bool) -> Option<Decimal> {
    if !percent_declared {
        return None;
    }
    let t = text.trim().strip_suffix('%')?;
    let d = parse_amount_text(t)?;
    let ratio = d / Decimal::from(100);
    Some(if ratio.is_zero() {
        Decimal::ZERO
    } else {
        ratio.normalize()
    })
}

/// 金额单位（§1.1：元/千元/万元归一到元）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyUnit {
    Yuan,
    ThousandYuan,
    WanYuan,
}

impl MoneyUnit {
    /// 解析明确单位文本；未知单位返回 None（不猜）。
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "元" => Some(Self::Yuan),
            "千元" => Some(Self::ThousandYuan),
            "万元" => Some(Self::WanYuan),
            _ => None,
        }
    }

    pub fn factor(self) -> Decimal {
        match self {
            Self::Yuan => Decimal::ONE,
            Self::ThousandYuan => Decimal::from(1000),
            Self::WanYuan => Decimal::from(10000),
        }
    }
}

/// 金额归一到元。
pub fn normalize_money_to_yuan(value: Decimal, unit: MoneyUnit) -> Decimal {
    value * unit.factor()
}

/// 任务进度单元格解析（§4.2 progress）。
///
/// 文本 `50%` → 50；Excel 百分数格式底值 `0.5` → 50；
/// 普通数字 `0.5` 仍是 0.5；最多 2 位小数、0–100。
pub fn parse_progress_cell(raw: &str, percent_format: bool) -> Option<Decimal> {
    let t = raw.trim();
    if let Some(without_pct) = t.strip_suffix('%') {
        // 文本 "50%" 即 50（百分比文本本身就是 0–100 刻度）
        return finish_progress(parse_amount_text(without_pct)?);
    }
    let v = parse_amount_text(t)?;
    if percent_format {
        // Excel 百分数格式底值 0.5 表示 50%
        return finish_progress(v * Decimal::from(100));
    }
    finish_progress(v)
}

fn finish_progress(v: Decimal) -> Option<Decimal> {
    let v = v.normalize();
    if v.scale() > 2 || v < Decimal::ZERO || v > Decimal::from(100) {
        return None;
    }
    Some(if v.is_zero() { Decimal::ZERO } else { v })
}

/// 合并单元格值展开：只左上锚点保留值，覆盖格为 None，不 forward-fill（§4.1）。
pub fn expand_merged_anchor<T: Clone>(anchor: Option<T>, span: usize) -> Vec<Option<T>> {
    let mut out = vec![None; span];
    if let Some(v) = anchor {
        out[0] = Some(v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_states() {
        assert_eq!(classify_cell(None).value_state(), ValueState::Missing);
        assert_eq!(classify_cell(Some("")).value_state(), ValueState::Missing);
        assert_eq!(classify_cell(Some("  ")).value_state(), ValueState::Missing);
        assert_eq!(
            classify_cell(Some("待补")).value_state(),
            ValueState::PendingInput
        );
        assert_eq!(
            classify_cell(Some("待确认")).value_state(),
            ValueState::PendingInput
        );
        let na = classify_cell(Some("NA"));
        assert_eq!(na.value_state(), ValueState::Missing);
        assert!(matches!(na, CellClass::Missing { missing_token: Some(t) } if t == "NA"));
        assert_eq!(classify_cell(Some("-")).value_state(), ValueState::Missing);
        let zero = classify_cell(Some("0"));
        assert_eq!(zero.value_state(), ValueState::Valid);
        let neg = classify_cell(Some("-2"));
        assert!(matches!(neg, CellClass::Valid { .. }));
    }

    #[test]
    fn thousands_grouping() {
        assert_eq!(
            parse_amount_text("1,234.56"),
            Some(Decimal::from_str_exact("1234.56").unwrap())
        );
        assert_eq!(parse_amount_text("12,34.5"), None, "逗号小数点不猜");
        assert_eq!(parse_amount_text("1,234"), Some(Decimal::from(1234)));
        assert_eq!(parse_amount_text("1,2345"), None);
    }

    #[test]
    fn plain_text_trailing_zeros_allowed() {
        // 原始文本允许多余尾零，值按规范形式校验
        assert_eq!(
            parse_amount_text("100.50").map(|d| d.to_string()),
            Some("100.5".into())
        );
        assert_eq!(parse_amount_text("-0.0"), Some(Decimal::ZERO));
        assert_eq!(
            parse_amount_text("1.5000000").map(|d| d.to_string()),
            Some("1.5".into())
        );
        assert_eq!(parse_amount_text("1.0000001"), None, "超 6 位小数不截断");
        assert_eq!(parse_amount_text("007"), None, "前导零不猜");
        assert_eq!(parse_amount_text("1e3"), None, "文本指数不猜");
    }

    #[test]
    fn numeric_literal_exact_without_f64() {
        let d = |s: &str| parse_numeric_literal(s).map(|v| v.to_string());
        assert_eq!(d("123456.78"), Ok("123456.78".into()));
        assert_eq!(d("-4321.05"), Ok("-4321.05".into()));
        assert_eq!(d("0"), Ok("0".into()));
        assert_eq!(d("-0"), Ok("0".into()), "负零规范为 0");
        assert_eq!(d("12.50"), Ok("12.5".into()));
        assert_eq!(d("1E+3"), Ok("1000".into()));
        assert_eq!(d("2.5e-3"), Ok("0.0025".into()));
        assert_eq!(d("1.25E1"), Ok("12.5".into()));
        assert_eq!(
            d("0.12345678"),
            Err(DecimalPolicyError::PrecisionExceeded { max: 6 }),
            "超输入精度报 decimal_precision_exceeded，不截断"
        );
        assert_eq!(
            d("1.0000000000000001E-3"),
            Err(DecimalPolicyError::PrecisionExceeded { max: 6 })
        );
        assert!(d("NaN").is_err() && d("1,234").is_err() && d("").is_err());
    }

    #[test]
    fn percent_and_units() {
        assert_eq!(
            parse_percent_text("50%", true).map(|d| d.to_string()),
            Some("0.5".into())
        );
        assert_eq!(parse_percent_text("50%", false), None);
        let yuan = normalize_money_to_yuan(Decimal::from(2), MoneyUnit::WanYuan);
        assert_eq!(yuan.to_string(), "20000");
        assert_eq!(MoneyUnit::parse(" 千元 "), Some(MoneyUnit::ThousandYuan));
        assert_eq!(MoneyUnit::parse("美元"), None);
    }

    #[test]
    fn progress_cells() {
        assert_eq!(
            parse_progress_cell("50%", false).map(|d| d.to_string()),
            Some("50".into())
        );
        assert_eq!(
            parse_progress_cell("0.5", true).map(|d| d.to_string()),
            Some("50".into())
        );
        assert_eq!(
            parse_progress_cell("0.5", false).map(|d| d.to_string()),
            Some("0.5".into())
        );
        assert_eq!(parse_progress_cell("100.01", false), None);
    }

    #[test]
    fn merged_anchor_only() {
        let expanded = expand_merged_anchor(Some(Decimal::from(100)), 3);
        assert_eq!(expanded[0], Some(Decimal::from(100)));
        assert!(expanded[1..].iter().all(|v| v.is_none()));
    }
}
