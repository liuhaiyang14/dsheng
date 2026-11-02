//! Decimal 计算引擎（data-period-import.md §1.2 / §1.3 / metric-catalog numeric_policy）。
//!
//! - flow 求无重叠完整和；stock 取确认期末或期初+完整净变动；
//!   ratio 合并分子/分母后重算；configured_derived 只执行已确认有限 AST。
//! - 所有 div 分母须 >0；0 → `zero_denominator`，负 → `negative_denominator`。
//! - 分子可负、比例可大于 100%，不截平；库存允许负值并给警告。
//! - 全部运算走 [`crate::contracts::decimal`]，禁止 f64 中转。

use crate::contracts::decimal::{
    canonical_decimal, contract_div, display_round, to_canonical_string,
    DISPLAY_MONEY_FRACTION_DIGITS,
};
use crate::contracts::domain::ComparisonReason;
use crate::contracts::metrics::FormulaNode;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// 引擎层确定原因词法（合同用词，逐一对应 spec/算例）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EngineReason {
    #[error("zero_denominator")]
    ZeroDenominator,
    #[error("negative_denominator")]
    NegativeDenominator,
    #[error("source_conflict")]
    SourceConflict,
    #[error("source_selection_required")]
    SourceSelectionRequired,
    #[error("overlap_unresolved")]
    OverlapUnresolved,
    #[error("missing_opening_balance")]
    MissingOpeningBalance,
    #[error("missing_closing_balance")]
    MissingClosingBalance,
    #[error("configuration_required")]
    ConfigurationRequired,
    #[error("unsupported_formula_op")]
    UnsupportedFormulaOp,
    #[error("invalid_input")]
    InvalidInput,
}

impl EngineReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ZeroDenominator => "zero_denominator",
            Self::NegativeDenominator => "negative_denominator",
            Self::SourceConflict => "source_conflict",
            Self::SourceSelectionRequired => "source_selection_required",
            Self::OverlapUnresolved => "overlap_unresolved",
            Self::MissingOpeningBalance => "missing_opening_balance",
            Self::MissingClosingBalance => "missing_closing_balance",
            Self::ConfigurationRequired => "configuration_required",
            Self::UnsupportedFormulaOp => "unsupported_formula_op",
            Self::InvalidInput => "invalid_input",
        }
    }
}

/// 正分母除法（§1.2：所有 div 分母须>0；12 位小数 half-even）。
pub fn positive_div(numerator: Decimal, denominator: Decimal) -> Result<Decimal, EngineReason> {
    if denominator.is_zero() {
        return Err(EngineReason::ZeroDenominator);
    }
    if denominator < Decimal::ZERO {
        return Err(EngineReason::NegativeDenominator);
    }
    contract_div(numerator, denominator).map_err(|_| EngineReason::InvalidInput)
}

/// flow 聚合：仅对已证明不重叠且完整覆盖的观察求和（§1.2 表）。
///
/// 明细与汇总共存须先选 authoritative representation（见 [`crate::analysis::observations`]）；
/// 累计列是独立范围观察，不在此重复相加。
pub fn flow_sum(values: &[Decimal], disjoint_complete: bool) -> Result<Decimal, EngineReason> {
    if !disjoint_complete {
        return Err(EngineReason::OverlapUnresolved);
    }
    Ok(values.iter().copied().fold(Decimal::ZERO, |acc, v| acc + v))
}

/// ratio 聚合：汇总原分子/分母后重算（§1.2 表；禁止百分比求和/简单平均）。
pub fn ratio_aggregate(
    numerators: &[Decimal],
    denominators: &[Decimal],
) -> Result<Decimal, EngineReason> {
    if numerators.len() != denominators.len() || numerators.is_empty() {
        return Err(EngineReason::InvalidInput);
    }
    let n = numerators.iter().copied().fold(Decimal::ZERO, |a, v| a + v);
    let d = denominators
        .iter()
        .copied()
        .fold(Decimal::ZERO, |a, v| a + v);
    positive_div(n, d)
}

/// 比较计算（§3.2）：absolute_change = current − base；base>0 才有 change_rate。
///
/// base=0/负只给绝对变化（zero_base/negative_base）；不使用 abs(base)；
/// direction 仅决定是否有利，不改公式。
#[derive(Debug, Clone, PartialEq)]
pub struct ComparisonOutcome {
    pub absolute_change: Decimal,
    pub change_rate: Option<Decimal>,
    pub reason: Option<ComparisonReason>,
}

pub fn compute_comparison(current: Decimal, base: Decimal) -> ComparisonOutcome {
    let absolute_change = current - base;
    if base > Decimal::ZERO {
        let rate = contract_div(absolute_change, base)
            .map_err(|_| ComparisonReason::InvalidInput)
            .unwrap_or(Decimal::ZERO);
        ComparisonOutcome {
            absolute_change,
            change_rate: Some(rate),
            reason: None,
        }
    } else if base.is_zero() {
        ComparisonOutcome {
            absolute_change,
            change_rate: None,
            reason: Some(ComparisonReason::ZeroBase),
        }
    } else {
        ComparisonOutcome {
            absolute_change,
            change_rate: None,
            reason: Some(ComparisonReason::NegativeBase),
        }
    }
}

/// configured_derived：按已确认 AST 对已聚合输入重算。
///
/// ref 缺输入按 invalid_input 拒绝（缺数据不得当有效值）；div 分母走
/// [`positive_div`] 的 zero/negative_denominator 规则。
pub fn eval_formula(
    node: &FormulaNode,
    inputs: &BTreeMap<String, Decimal>,
) -> Result<Decimal, EngineReason> {
    match node {
        FormulaNode::Ref { metric_key } => inputs
            .get(metric_key)
            .copied()
            .ok_or(EngineReason::InvalidInput),
        FormulaNode::Const { value } => {
            canonical_decimal(value).map_err(|_| EngineReason::InvalidInput)
        }
        FormulaNode::Add { args } => {
            Ok(eval_formula(&args[0], inputs)? + eval_formula(&args[1], inputs)?)
        }
        FormulaNode::Sub { args } => {
            Ok(eval_formula(&args[0], inputs)? - eval_formula(&args[1], inputs)?)
        }
        FormulaNode::Mul { args } => {
            Ok(eval_formula(&args[0], inputs)? * eval_formula(&args[1], inputs)?)
        }
        FormulaNode::Div { args } => {
            let n = eval_formula(&args[0], inputs)?;
            let d = eval_formula(&args[1], inputs)?;
            positive_div(n, d)
        }
    }
}

/// 公式 JSON 的封闭结构校验（§1.2：仅 ref/const/add/sub/mul/div、字段精确匹配）。
///
/// 未认识的操作（如 call/fetch）返回 `unsupported_formula_op`，整个候选公式拒绝。
pub fn validate_formula_json(v: &serde_json::Value) -> Result<(), EngineReason> {
    let Some(obj) = v.as_object() else {
        return Err(EngineReason::UnsupportedFormulaOp);
    };
    let Some(op) = obj.get("op").and_then(|x| x.as_str()) else {
        return Err(EngineReason::UnsupportedFormulaOp);
    };
    let fields_ok = match op {
        "ref" => obj.len() == 2 && obj.get("metric_key").is_some_and(|x| x.is_string()),
        "const" => obj.len() == 2 && obj.get("value").is_some_and(|x| x.is_string()),
        "add" | "sub" | "mul" | "div" => {
            obj.len() == 2
                && obj.get("args").is_some_and(|x| {
                    x.as_array()
                        .is_some_and(|a| a.len() == 2 && a.iter().all(validate_formula_json_ok))
                })
        }
        _ => return Err(EngineReason::UnsupportedFormulaOp),
    };
    if !fields_ok {
        return Err(EngineReason::UnsupportedFormulaOp);
    }
    Ok(())
}

fn validate_formula_json_ok(v: &serde_json::Value) -> bool {
    validate_formula_json(v).is_ok()
}

/// 库存 rollforward 输入（§1.3）。
#[derive(Debug, Clone, Default)]
pub struct RollforwardInput {
    /// 期初；None 表示无期初（不假设为 0）。
    pub opening: Option<Decimal>,
    pub inbound: Decimal,
    pub outbound: Decimal,
    /// 已证实期末观察（closing_balance）；与 rollforward 冲突时须人工选源。
    pub closing_observation: Option<Decimal>,
}

/// rollforward 结果；value=None 时 reason 给出确定原因。
#[derive(Debug, Clone, PartialEq)]
pub struct RollforwardOutcome {
    pub value: Option<Decimal>,
    pub reason: Option<EngineReason>,
    /// 负库存警告；值保留不改 0。
    pub warning: Option<&'static str>,
}

/// 库存量 = 期初 + 入库 − 出库（§1.3）。
///
/// - 无期初且无已证实期末 → `missing_opening_balance`；
/// - 有已证实期末：作为权威值；与可算 rollforward 不同 → `source_selection_required`；
/// - 负库存允许保存并给 `negative_inventory` 警告。
pub fn inventory_rollforward(input: &RollforwardInput) -> RollforwardOutcome {
    let rollforward = input.opening.map(|o| o + input.inbound - input.outbound);
    match (rollforward, input.closing_observation) {
        (None, None) => RollforwardOutcome {
            value: None,
            reason: Some(EngineReason::MissingOpeningBalance),
            warning: None,
        },
        (None, Some(closing)) => RollforwardOutcome {
            value: Some(closing),
            reason: None,
            warning: None,
        },
        (Some(computed), Some(closing)) => {
            if computed == closing {
                RollforwardOutcome {
                    value: Some(closing),
                    reason: None,
                    warning: negative_warning(computed),
                }
            } else {
                RollforwardOutcome {
                    value: None,
                    reason: Some(EngineReason::SourceSelectionRequired),
                    warning: None,
                }
            }
        }
        (Some(computed), None) => RollforwardOutcome {
            value: Some(computed),
            reason: None,
            warning: negative_warning(computed),
        },
    }
}

fn negative_warning(v: Decimal) -> Option<&'static str> {
    if v < Decimal::ZERO {
        Some("negative_inventory")
    } else {
        None
    }
}

/// 已证实期末余额观察（date, value）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClosingObservation {
    pub date: NaiveDate,
    pub value: Decimal,
}

/// stock 聚合（§1.2 表）：选定期末已证实余额；禁止月末余额求和/平均。
///
/// 仅当存在恰好 `target_end` 当日的观察才认定已证实期末；
/// 无期末观察 → `missing_closing_balance`。
pub fn closing_balance_at(
    observations: &[ClosingObservation],
    target_end: NaiveDate,
) -> Result<Decimal, EngineReason> {
    observations
        .iter()
        .find(|o| o.date == target_end)
        .map(|o| o.value)
        .ok_or(EngineReason::MissingClosingBalance)
}

/// “开票−收款”期间差额标签（§1.3）；不是应收余额。
pub const INVOICE_RECEIPT_GAP_LABEL: &str = "开票收款差额（期间）";
/// “入库−付款”期间差额标签（§1.3）；不是应付余额。
pub const INBOUND_PAYMENT_GAP_LABEL: &str = "入库付款差额（期间）";

/// 期间差额 = first − second（开票−收款 / 入库−付款）。
///
/// 可能跨期；调用方不得将其解释为账户余额/账龄/逾期欠款。
pub fn period_gap(first: Decimal, second: Decimal) -> Decimal {
    first - second
}

/// 金额展示（2 位 half-even）；存储仍是规范十进制字符串（§1.1）。
pub fn display_money(value: Decimal) -> String {
    to_canonical_string(display_round(value, DISPLAY_MONEY_FRACTION_DIGITS))
}

/// 配置型指标（settlement_income 等）无可确认公式/口径时的判定（§1.2）。
pub fn configured_derived_ready(formula: Option<&FormulaNode>) -> Result<(), EngineReason> {
    match formula {
        Some(_) => Ok(()),
        None => Err(EngineReason::ConfigurationRequired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::decimal::canonical_decimal as cd;

    #[test]
    fn flow_and_ratio() {
        let vals = [cd("0.1").unwrap(), cd("0.2").unwrap()];
        assert_eq!(flow_sum(&vals, true).unwrap(), cd("0.3").unwrap());
        assert_eq!(flow_sum(&vals, false), Err(EngineReason::OverlapUnresolved));
        let r = ratio_aggregate(
            &[cd("10").unwrap(), cd("60").unwrap()],
            &[cd("100").unwrap(), cd("300").unwrap()],
        )
        .unwrap();
        assert_eq!(r, cd("0.175").unwrap());
    }

    #[test]
    fn divide_rules() {
        assert_eq!(
            positive_div(cd("12").unwrap(), cd("10").unwrap()).unwrap(),
            cd("1.2").unwrap()
        );
        assert_eq!(
            positive_div(cd("12").unwrap(), Decimal::ZERO),
            Err(EngineReason::ZeroDenominator)
        );
        assert_eq!(
            positive_div(cd("12").unwrap(), cd("-10").unwrap()),
            Err(EngineReason::NegativeDenominator)
        );
        // 分子可负、比例可超 100%，不截平
        assert_eq!(
            positive_div(cd("-12").unwrap(), cd("10").unwrap()).unwrap(),
            cd("-1.2").unwrap()
        );
        assert_eq!(
            positive_div(cd("150").unwrap(), cd("100").unwrap()).unwrap(),
            cd("1.5").unwrap()
        );
    }

    #[test]
    fn comparison_rules() {
        let ok = compute_comparison(cd("120").unwrap(), cd("100").unwrap());
        assert_eq!(ok.absolute_change, cd("20").unwrap());
        assert_eq!(ok.change_rate, Some(cd("0.2").unwrap()));
        assert_eq!(ok.reason, None);
        let zero = compute_comparison(cd("20").unwrap(), Decimal::ZERO);
        assert_eq!(zero.change_rate, None);
        assert_eq!(zero.reason, Some(ComparisonReason::ZeroBase));
        let neg = compute_comparison(cd("-20").unwrap(), cd("-40").unwrap());
        assert_eq!(neg.absolute_change, cd("20").unwrap());
        assert_eq!(neg.reason, Some(ComparisonReason::NegativeBase));
    }

    #[test]
    fn inventory_and_stock() {
        let none = inventory_rollforward(&RollforwardInput {
            opening: None,
            inbound: cd("20").unwrap(),
            outbound: cd("5").unwrap(),
            closing_observation: None,
        });
        assert_eq!(none.reason, Some(EngineReason::MissingOpeningBalance));
        let ok = inventory_rollforward(&RollforwardInput {
            opening: Some(cd("10").unwrap()),
            inbound: cd("20").unwrap(),
            outbound: cd("5").unwrap(),
            closing_observation: None,
        });
        assert_eq!(ok.value, Some(cd("25").unwrap()));
        let neg = inventory_rollforward(&RollforwardInput {
            opening: Some(cd("2").unwrap()),
            inbound: cd("1").unwrap(),
            outbound: cd("5").unwrap(),
            closing_observation: None,
        });
        assert_eq!(neg.value, Some(cd("-2").unwrap()));
        assert_eq!(neg.warning, Some("negative_inventory"));
    }

    #[test]
    fn formula_ast_eval() {
        // (a + b) / c
        let node: FormulaNode = serde_json::from_value(serde_json::json!({
            "op": "div",
            "args": [
                {"op": "add", "args": [
                    {"op": "ref", "metric_key": "a"},
                    {"op": "ref", "metric_key": "b"}
                ]},
                {"op": "const", "value": "2"}
            ]
        }))
        .unwrap();
        let mut inputs = BTreeMap::new();
        inputs.insert("a".to_string(), cd("10").unwrap());
        inputs.insert("b".to_string(), cd("20").unwrap());
        assert_eq!(eval_formula(&node, &inputs).unwrap(), cd("15").unwrap());
    }

    #[test]
    fn gap_labels() {
        let v = period_gap(cd("100").unwrap(), cd("120").unwrap());
        assert_eq!(to_canonical_string(v), "-20");
        assert_eq!(INVOICE_RECEIPT_GAP_LABEL, "开票收款差额（期间）");
    }
}
