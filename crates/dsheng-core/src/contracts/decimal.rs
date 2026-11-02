//! Decimal 数值合同（data-period-import.md §1.1 / metric-catalog numeric_policy）。
//!
//! - 运算 Decimal 至少 28 位有效数字；输入金额/数量/工时/单价最多 6 位小数。
//! - 除法保留 12 位小数、half-even；中间值不按展示位数截断。
//! - SQLite / 业务 JSON 存规范十进制字符串（无千分位/指数/多余尾零，负零为 0）。
//! - 模型 number 边界用 decimal-aware 序列化/解析，不经 f64。

use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy::MidpointNearestEven;
use serde::{Deserialize, Serialize};

/// 输入小数位上限（金额/数量/工时/单价）。
pub const INPUT_MAX_FRACTION_DIGITS: u32 = 6;
/// 除法结果小数位。
pub const DIVISION_FRACTION_DIGITS: u32 = 12;
/// 金额展示小数位。
pub const DISPLAY_MONEY_FRACTION_DIGITS: u32 = 2;
/// 比例展示小数位。
pub const DISPLAY_PERCENT_FRACTION_DIGITS: u32 = 1;
/// 任务进度最大小数位（task-lifecycle §5.1/§4.2）。
pub const PROGRESS_MAX_FRACTION_DIGITS: u32 = 2;
/// 进度值上限。
pub const PROGRESS_MAX: Decimal = Decimal::ONE_HUNDRED;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecimalPolicyError {
    #[error("decimal_precision_exceeded: 输入小数位超过 {max} 位")]
    PrecisionExceeded { max: u32 },
    #[error("decimal_overflow: 超过 28 位有效数字范围")]
    Overflow,
    #[error("invalid_canonical_decimal: 不是规范十进制字符串")]
    InvalidCanonical,
}

/// 解析规范十进制字符串（合同输入路径：SQLite / CLI / UI / 模型边界还原）。
///
/// 规范形式：`-?(0|[1-9][0-9]*)(\.[0-9]*[1-9])?`，无指数/千分位/前导零/
/// 尾随小数零/负零。此函数只负责语法与规范化校验，不检查小数位上限
/// （输入路径用 [`parse_input_decimal`]，进度用 [`parse_progress_value`]）。
pub fn canonical_decimal(input: &str) -> Result<Decimal, DecimalPolicyError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    // 拒绝指数、加号、下划线、千分位逗号、空白、负零
    if s.contains(['e', 'E', '+', '_', ',']) || s.len() != input.trim().len() || s == "-0" {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    let body = s.strip_prefix('-').unwrap_or(s);
    let invalid_shape = body.is_empty()
        || !body.chars().all(|c| c.is_ascii_digit() || c == '.')
        || body.matches('.').count() > 1;
    if invalid_shape {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    let (int_part, frac_part) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    let has_dot = body.contains('.');
    // 前导零只允许单个 0；小数部分不允许尾随零；有圆点则小数部分必须非空（"1." 非法）
    let int_ok = int_part.len() == 1 || (!int_part.starts_with('0') && !int_part.is_empty());
    let int_digits_ok = !int_part.is_empty() && int_part.chars().all(|c| c.is_ascii_digit());
    let frac_digits_ok = !frac_part.is_empty() && frac_part.chars().all(|c| c.is_ascii_digit());
    let frac_ok = if has_dot {
        frac_digits_ok && !frac_part.ends_with('0')
    } else {
        true
    };
    if !int_ok || !int_digits_ok || !frac_ok {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    let d = Decimal::from_str_exact(s).map_err(|_| DecimalPolicyError::InvalidCanonical)?;
    Ok(normalize_zero(d))
}

fn normalize_zero(d: Decimal) -> Decimal {
    if d.is_zero() {
        Decimal::ZERO
    } else {
        d
    }
}

/// 输入路径解析：规范字符串 + 输入小数位 ≤6 + 28 位有效数字。
pub fn parse_input_decimal(input: &str) -> Result<Decimal, DecimalPolicyError> {
    let d = canonical_decimal(input)?;
    if d.scale() > INPUT_MAX_FRACTION_DIGITS {
        return Err(DecimalPolicyError::PrecisionExceeded {
            max: INPUT_MAX_FRACTION_DIGITS,
        });
    }
    check_significant_digits(&d)?;
    Ok(d)
}

/// 任务进度解析：0–100、最多 2 位小数、规范字符串。
pub fn parse_progress_value(input: &str) -> Result<Decimal, DecimalPolicyError> {
    let d = canonical_decimal(input)?;
    if d.scale() > PROGRESS_MAX_FRACTION_DIGITS {
        return Err(DecimalPolicyError::PrecisionExceeded {
            max: PROGRESS_MAX_FRACTION_DIGITS,
        });
    }
    if d < Decimal::ZERO || d > PROGRESS_MAX {
        return Err(DecimalPolicyError::InvalidCanonical);
    }
    Ok(d)
}

/// 28 位有效数字检查。
pub fn check_significant_digits(d: &Decimal) -> Result<(), DecimalPolicyError> {
    let mantissa = d.mantissa().unsigned_abs();
    let digits = decimal_digit_count(mantissa);
    if digits > 28 {
        return Err(DecimalPolicyError::Overflow);
    }
    Ok(())
}

fn decimal_digit_count(mut m: u128) -> u32 {
    if m == 0 {
        return 1;
    }
    let mut n = 0;
    while m > 0 {
        m /= 10;
        n += 1;
    }
    n
}

/// 输出规范十进制字符串：去尾随零、负零为 0。
pub fn to_canonical_string(d: Decimal) -> String {
    let d = normalize_zero(d).normalize();
    d.to_string()
}

/// 合同除法：12 位小数、half-even。
pub fn contract_div(
    numerator: Decimal,
    denominator: Decimal,
) -> Result<Decimal, DecimalPolicyError> {
    if denominator.is_zero() {
        return Err(DecimalPolicyError::InvalidCanonical); // 调用方应映射 zero_denominator 领域错误
    }
    let q = numerator / denominator;
    Ok(q.round_dp_with_strategy(DIVISION_FRACTION_DIGITS, MidpointNearestEven))
}

/// 展示舍入：金额 2 位 / 比例 1 位（half-even）。
pub fn display_round(d: Decimal, fraction_digits: u32) -> Decimal {
    d.round_dp_with_strategy(fraction_digits, MidpointNearestEven)
}

/// 模型 number 边界序列化：把规范字符串转为 JSON 数字字面量（不经 f64）。
///
/// 依赖 serde_json `arbitrary_precision`：Number 保留数值词法。
pub fn decimal_to_model_number(input: &str) -> Result<serde_json::Number, DecimalPolicyError> {
    let d = canonical_decimal(input)?;
    let canonical = to_canonical_string(d);
    serde_json::from_str::<serde_json::Number>(&canonical)
        .map_err(|_| DecimalPolicyError::InvalidCanonical)
}

/// 模型 number 边界解析：从 JSON 数字词法直接还原规范十进制字符串。
///
/// 使用原始词法（serde_json arbitrary precision 关闭时 Number 仅暴露 f64，
/// 因此本函数只接受由 [`decimal_to_model_number`] 产生的规范形式——
/// Provider 响应解析层必须保留数值词法再调用本函数，不得走 f64）。
pub fn model_number_to_decimal_string(n: &serde_json::Number) -> Option<String> {
    // 规范形式已保证：无指数、无尾随零
    let s = n.to_string();
    if s.contains(['e', 'E']) {
        return None;
    }
    canonical_decimal(&s).ok().map(to_canonical_string)
}

/// 带序列化的规范十进制字符串新类型（SQLite / 业务 JSON / 编辑合同用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CanonicalDecimal(#[serde(skip)] Decimal);

impl TryFrom<String> for CanonicalDecimal {
    type Error = DecimalPolicyError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Ok(Self(parse_input_decimal(&value)?))
    }
}

impl From<CanonicalDecimal> for String {
    fn from(value: CanonicalDecimal) -> String {
        to_canonical_string(value.0)
    }
}

impl CanonicalDecimal {
    pub fn as_decimal(&self) -> Decimal {
        self.0
    }
    pub fn as_string(&self) -> String {
        to_canonical_string(self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_forms() {
        assert!(canonical_decimal("0").is_ok());
        assert!(canonical_decimal("-3").is_ok());
        assert!(canonical_decimal("0.5").is_ok());
        assert!(canonical_decimal("123456.789123").is_ok());
        assert!(canonical_decimal("-0").is_err(), "负零非法");
        assert!(canonical_decimal("01.5").is_err(), "前导零非法");
        assert!(canonical_decimal("1.50").is_err(), "尾随小数零非法");
        assert!(canonical_decimal("1e3").is_err(), "指数非法");
        assert!(canonical_decimal("1,234.5").is_err(), "千分位非法");
        assert!(canonical_decimal("1.").is_err());
        assert!(canonical_decimal(".5").is_err());
        assert!(canonical_decimal("").is_err());
    }

    #[test]
    fn input_precision() {
        assert!(parse_input_decimal("0.123456").is_ok());
        assert!(matches!(
            parse_input_decimal("0.1234567"),
            Err(DecimalPolicyError::PrecisionExceeded { .. })
        ));
    }

    #[test]
    fn division_policy() {
        let a = canonical_decimal("1").unwrap();
        let b = canonical_decimal("3").unwrap();
        let q = contract_div(a, b).unwrap();
        assert_eq!(q.to_string(), "0.333333333333");
        // half-even: 2.5e-12 边界
        let x = canonical_decimal("0.0000000000025").unwrap();
        let one = Decimal::ONE;
        let q2 = contract_div(x, one).unwrap();
        assert_eq!(q2.to_string(), "0.000000000002"); // 12 位后舍去，half-even 向偶
    }

    #[test]
    fn progress_bounds() {
        assert!(parse_progress_value("100").is_ok());
        assert!(parse_progress_value("33.33").is_ok());
        assert!(parse_progress_value("100.01").is_err());
        assert!(parse_progress_value("33.333").is_err());
    }

    #[test]
    fn model_number_roundtrip() {
        for s in ["0", "1.5", "-210000.123456", "0.333333333333"] {
            let n = decimal_to_model_number(s).unwrap();
            let back = model_number_to_decimal_string(&n).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn significant_digits_overflow() {
        let d = Decimal::from_str_exact("12345678901234567890123456789").unwrap(); // 29 位
        assert!(check_significant_digits(&d).is_err());
    }
}
