//! 共享领域枚举与值类型（data-period-import.md §1–§3 / task-lifecycle）。

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// 值状态；只有 `valid` 进入经营计算（§1.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueState {
    Valid,
    Missing,
    PendingInput,
    Invalid,
    Conflict,
    ConfigurationRequired,
    UnsupportedFormula,
    PrecisionUnverified,
}

/// 观察角色：本期实绩 / 目标期计划 / 年度当月分解。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasureRole {
    Actual,
    Plan,
    AnnualMonthPlan,
}

/// 记录种类（§2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Detail,
    PeriodTotal,
    CumulativeTotal,
    ClosingBalance,
    OpeningBalance,
}

/// 覆盖状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    Complete,
    Partial,
    Unknown,
}

/// 覆盖信息（§2 coverage）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Coverage {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub granularity: Granularity,
    pub status: CoverageStatus,
    pub confirmed_by: Option<String>,
    #[serde(default)]
    pub empty_days_are_zero: bool,
}

/// 改善方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Neutral,
}

/// 分析粒度；`custom` 为等长自定义行动窗口。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Granularity {
    Day,
    Week,
    Month,
    Quarter,
    Custom,
}

/// 周期策略 cadence（与 Granularity 分开：策略不含 custom）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    Day,
    Week,
    Month,
    Quarter,
}

impl Cadence {
    pub fn as_granularity(self) -> Granularity {
        match self {
            Cadence::Day => Granularity::Day,
            Cadence::Week => Granularity::Week,
            Cadence::Month => Granularity::Month,
            Cadence::Quarter => Granularity::Quarter,
        }
    }
}

/// 比较类型（§3.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonType {
    Mom,
    Yoy,
}

/// 比较可用性（§1.4 comparison 表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityStatus {
    Available,
    Partial,
    Unavailable,
}

/// 不可比/异常原因枚举（§1.4 reason；日历无基期原因单独见 §3.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonReason {
    ZeroBase,
    NegativeBase,
    MissingCurrent,
    MissingBase,
    CoverageIncomplete,
    DefinitionMismatch,
    UnitMismatch,
    ScopeMismatch,
    CalendarMismatch,
    SourceConflict,
    ConfigurationRequired,
    UnsupportedFormula,
    PrecisionUnverified,
    InvalidInput,
}

/// 异常优先级（§1.4 固定顺序）。
pub const COMPARISON_REASON_PRIORITY: &[ComparisonReason] = &[
    ComparisonReason::SourceConflict,
    ComparisonReason::ConfigurationRequired,
    ComparisonReason::UnsupportedFormula,
    ComparisonReason::PrecisionUnverified,
    ComparisonReason::InvalidInput,
    ComparisonReason::DefinitionMismatch,
    ComparisonReason::UnitMismatch,
    ComparisonReason::ScopeMismatch,
    ComparisonReason::CalendarMismatch,
    ComparisonReason::CoverageIncomplete,
    ComparisonReason::MissingCurrent,
    ComparisonReason::MissingBase,
];

/// 闭合自然日期区间 [from, to]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DateRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
}

impl DateRange {
    pub fn new(from: NaiveDate, to: NaiveDate) -> Result<Self, &'static str> {
        if from > to {
            return Err("date_range_invalid: from > to");
        }
        Ok(Self { from, to })
    }

    /// `YYYY-MM-DD/YYYY-MM-DD`（period 字段固定格式）。
    pub fn to_period_string(&self) -> String {
        format!(
            "{}/{}",
            self.from.format("%Y-%m-%d"),
            self.to.format("%Y-%m-%d")
        )
    }

    pub fn day_count(&self) -> i64 {
        (self.to - self.from).num_days() + 1
    }
}

/// 单位签名（§1.4 unitSignature）。四字段均 required；序列化时 null 不省略。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnitSignature {
    /// money / time / quantity / ratio / money_per_quantity / money_per_time
    pub dimension: UnitDimension,
    /// ratio 固定 "1"、time 固定 "hour"，其余为已确认规范单位。
    pub base_unit: String,
    /// ISO 大写三字母或 null。
    pub currency: Option<String>,
    /// inclusive / exclusive / not_applicable 或 null。
    pub tax_basis: Option<TaxBasis>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitDimension {
    Money,
    Time,
    Quantity,
    Ratio,
    MoneyPerQuantity,
    MoneyPerTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaxBasis {
    Inclusive,
    Exclusive,
    NotApplicable,
}

impl UnitSignature {
    /// 模型 `unit` 字符串：unitSignature 的规范 JSON（键按字母序，null 不省略）。
    pub fn to_model_unit_string(&self) -> String {
        // 键序：base_unit, currency, dimension, tax_basis
        let currency: String = self
            .currency
            .as_deref()
            .map(json_str)
            .unwrap_or_else(|| "null".to_string());
        let tax: String = match self.tax_basis {
            Some(TaxBasis::Inclusive) => "\"inclusive\"".into(),
            Some(TaxBasis::Exclusive) => "\"exclusive\"".into(),
            Some(TaxBasis::NotApplicable) => "\"not_applicable\"".into(),
            None => "null".into(),
        };
        let dimension = match self.dimension {
            UnitDimension::Money => "\"money\"",
            UnitDimension::Time => "\"time\"",
            UnitDimension::Quantity => "\"quantity\"",
            UnitDimension::Ratio => "\"ratio\"",
            UnitDimension::MoneyPerQuantity => "\"money_per_quantity\"",
            UnitDimension::MoneyPerTime => "\"money_per_time\"",
        };
        format!(
            "{{\"base_unit\":{},\"currency\":{},\"dimension\":{},\"tax_basis\":{}}}",
            json_str(&self.base_unit),
            currency,
            dimension,
            tax
        )
    }

    /// money 及其派生维度要求 currency 非空且 tax_basis 为已确认枚举（§1.4）。
    pub fn validate(&self) -> Result<(), &'static str> {
        let needs_money_meta = matches!(
            self.dimension,
            UnitDimension::Money | UnitDimension::MoneyPerQuantity | UnitDimension::MoneyPerTime
        );
        if needs_money_meta {
            let cur_ok = self
                .currency
                .as_deref()
                .map(|c| c.len() == 3 && c.chars().all(|ch| ch.is_ascii_uppercase()))
                .unwrap_or(false);
            if !cur_ok {
                return Err("unit_signature_invalid: money 维度 currency 必须为 ISO 大写三字母");
            }
            if self.tax_basis != Some(TaxBasis::Inclusive)
                && self.tax_basis != Some(TaxBasis::Exclusive)
                && self.tax_basis != Some(TaxBasis::NotApplicable)
            {
                return Err("unit_signature_invalid: money 维度 tax_basis 必须已确认");
            }
        } else if self.currency.is_some() || self.tax_basis.is_some() {
            return Err("unit_signature_invalid: 非 money 维度 currency/tax_basis 必须为 null");
        }
        if self.dimension == UnitDimension::Ratio && self.base_unit != "1" {
            return Err("unit_signature_invalid: ratio 的 base_unit 固定为 1");
        }
        if self.dimension == UnitDimension::Time && self.base_unit != "hour" {
            return Err("unit_signature_invalid: time 的 base_unit 固定为 hour");
        }
        if self.base_unit.is_empty() {
            return Err("unit_signature_invalid: base_unit 非空");
        }
        Ok(())
    }
}

fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("字符串序列化不会失败")
}

/// 目录/工作区确认占位符（发送前必须替换，§1.4）。
pub const WORKSPACE_CONFIRMED_PLACEHOLDER: &str = "workspace_confirmed";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_string_is_alphabetical_and_explicit_null() {
        let u = UnitSignature {
            dimension: UnitDimension::Ratio,
            base_unit: "1".into(),
            currency: None,
            tax_basis: None,
        };
        assert_eq!(
            u.to_model_unit_string(),
            "{\"base_unit\":\"1\",\"currency\":null,\"dimension\":\"ratio\",\"tax_basis\":null}"
        );
    }

    #[test]
    fn money_requires_currency_and_tax() {
        let bad = UnitSignature {
            dimension: UnitDimension::Money,
            base_unit: "元".into(),
            currency: None,
            tax_basis: None,
        };
        assert!(bad.validate().is_err());
        let good = UnitSignature {
            dimension: UnitDimension::Money,
            base_unit: "元".into(),
            currency: Some("CNY".into()),
            tax_basis: Some(TaxBasis::Exclusive),
        };
        assert!(good.validate().is_ok());
    }

    #[test]
    fn date_range_ordering() {
        let a = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let b = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert!(DateRange::new(a, b).is_ok());
        assert!(DateRange::new(b, a).is_err());
    }
}
