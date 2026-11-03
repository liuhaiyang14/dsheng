//! 目标政策（data-period-import.md §1.2 / metric-catalog target_policy）。
//!
//! - `monthly_revenue_floor` 仅适用于明确收入语义、direction=up、月度目标且可比的
//!   指标；下限 = max(有效年度当月分解, 去年同月实绩)；候选目标取
//!   max(合法显式目标, 有效年度当月分解, 去年同月实绩)。
//! - 显式目标低于下限：保留原目标输入不变，候选上调待采纳（needs_review），
//!   不是已批准目标。
//! - 未解决来源冲突的候选值完全不得参与 max。
//! - explicit_target_only：仅取明确确认的计划/改善目标；历史实绩不等于目标；
//!   无显式目标 → null（`confirmed_target_missing`）。年度总计划不自动除 12。

use crate::contracts::domain::{Direction, Granularity};
use crate::contracts::metrics::TargetConstraintScope;
use rust_decimal::Decimal;

/// 候选上调待采纳的固定 needs_review 文案（metric-catalog target_policy）。
pub const CANDIDATE_RAISE_NEEDS_REVIEW: &str = "候选上调待采纳";
/// 无可确认目标。
pub const CONFIRMED_TARGET_MISSING: &str = "confirmed_target_missing";

/// monthly_revenue_floor 目标输入。
#[derive(Debug, Clone, Default)]
pub struct MonthlyFloorInput {
    pub explicit_target: Option<Decimal>,
    /// 有效（无冲突或已解决且明确选定、可比）的年度当月分解。
    pub comparable_annual_month_plan: Option<Decimal>,
    /// 有效且可比的去年同月实绩。
    pub comparable_prior_year_actual: Option<Decimal>,
    /// 不可比值（保持记录，不参与 max）。
    pub incomparable_values: Vec<Decimal>,
}

/// monthly_revenue_floor 目标结果。
#[derive(Debug, Clone, PartialEq)]
pub struct MonthlyFloorOutcome {
    /// 采用的目标：显式目标（原值保留）；无显式时为下限；都没有则 null。
    pub target: Option<Decimal>,
    /// 候选目标 = max(合法显式目标, 有效年度当月分解, 去年同月实绩)。
    pub candidate_target: Option<Decimal>,
    /// 可比下限 = max(有效年度当月分解, 去年同月实绩)。
    pub floor: Option<Decimal>,
    /// 原显式目标输入（不因下限改动）。
    pub original_explicit_target: Option<Decimal>,
    /// 显式目标低于下限时 Some("候选上调待采纳")。
    pub needs_review: Option<&'static str>,
    /// 未参与 max 的不可比值。
    pub excluded: Vec<Decimal>,
}

/// 资格判定：明确收入语义 + direction=up + 月度目标（§1.2）。
pub fn monthly_revenue_floor_eligible(
    revenue_semantics: bool,
    direction: Direction,
    target_grain: Granularity,
) -> bool {
    revenue_semantics && direction == Direction::Up && target_grain == Granularity::Month
}

/// monthly_revenue_floor 计算（§1.2）。
///
/// scope 非 monthly_revenue_floor 或不满足资格时按 explicit_target_only 处理。
pub fn monthly_revenue_floor(
    input: &MonthlyFloorInput,
    scope: TargetConstraintScope,
    eligible: bool,
) -> MonthlyFloorOutcome {
    let excluded = input.incomparable_values.clone();
    if scope != TargetConstraintScope::MonthlyRevenueFloor || !eligible {
        return MonthlyFloorOutcome {
            target: input.explicit_target,
            candidate_target: input.explicit_target,
            floor: None,
            original_explicit_target: input.explicit_target,
            needs_review: None,
            excluded,
        };
    }
    let floor = max_of(&[
        input.comparable_annual_month_plan,
        input.comparable_prior_year_actual,
    ]);
    let candidate = max_of(&[input.explicit_target, floor]);
    let needs_review = match (input.explicit_target, floor) {
        (Some(explicit), Some(f)) if explicit < f => Some(CANDIDATE_RAISE_NEEDS_REVIEW),
        _ => None,
    };
    MonthlyFloorOutcome {
        target: input.explicit_target.or(floor),
        candidate_target: candidate,
        floor,
        original_explicit_target: input.explicit_target,
        needs_review,
        excluded,
    }
}

/// explicit_target_only：仅显式确认目标；无则 `confirmed_target_missing`。
///
/// 历史实绩不是目标；不自行改成 min、不发明降幅、年度总计划不除 12。
pub fn explicit_target_only(explicit_target: Option<Decimal>) -> Result<Decimal, &'static str> {
    explicit_target.ok_or(CONFIRMED_TARGET_MISSING)
}

fn max_of(values: &[Option<Decimal>]) -> Option<Decimal> {
    values.iter().flatten().copied().max()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::decimal::canonical_decimal as cd;

    #[test]
    fn eligibility_requires_revenue_up_month() {
        assert!(monthly_revenue_floor_eligible(
            true,
            Direction::Up,
            Granularity::Month
        ));
        assert!(!monthly_revenue_floor_eligible(
            true,
            Direction::Up,
            Granularity::Day
        ));
        assert!(!monthly_revenue_floor_eligible(
            true,
            Direction::Down,
            Granularity::Month
        ));
        assert!(!monthly_revenue_floor_eligible(
            false,
            Direction::Up,
            Granularity::Month
        ));
    }

    #[test]
    fn explicit_only_never_uses_history() {
        assert_eq!(explicit_target_only(None), Err(CONFIRMED_TARGET_MISSING));
        assert_eq!(
            explicit_target_only(Some(cd("35").unwrap())),
            Ok(cd("35").unwrap())
        );
    }
}
