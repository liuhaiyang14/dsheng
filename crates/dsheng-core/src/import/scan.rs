//! 扫描层纯逻辑：图表序列与文件名期间（data-period-import.md §4.3 / DC I08、I10）。
//!
//! - 图表 sheet 可能只有标题/内嵌 chart：只取 chart 引用的已解析单元格作证据，
//!   引用不可解析时 series 为空并给 `chart_source_unavailable`，不从图像猜值。
//! - 文件名不证明期间：不从文件名猜日期/单位（§4.1/§4.2 period）。

use crate::contracts::domain::DateRange;
use rust_decimal::Decimal;

/// 图表来源不可解析原因。
pub const CHART_SOURCE_UNAVAILABLE: &str = "chart_source_unavailable";
/// 期间需显式确认。
pub const PERIOD_CONFIRMATION_REQUIRED: &str = "period_confirmation_required";

/// 由已解析单元格构造图表序列。
///
/// `references_resolvable`=chart 引用可在已选范围内解析；
/// 数值序列只含真实已解析值（缺桶不造值）。
pub fn chart_series_from_resolved_cells(
    references_resolvable: bool,
    numeric_values: &[Decimal],
) -> (Vec<Decimal>, Option<&'static str>) {
    if !references_resolvable || numeric_values.is_empty() {
        return (Vec::new(), Some(CHART_SOURCE_UNAVAILABLE));
    }
    (numeric_values.to_vec(), None)
}

/// 来源期间：仅接受显式确认的期间；文件名中的日期段一律忽略（DC I10）。
pub fn source_period(explicit: Option<DateRange>) -> Result<DateRange, &'static str> {
    explicit.ok_or(PERIOD_CONFIRMATION_REQUIRED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chart_unavailable_without_cells() {
        let (series, reason) = chart_series_from_resolved_cells(false, &[]);
        assert!(series.is_empty());
        assert_eq!(reason, Some(CHART_SOURCE_UNAVAILABLE));
        let ok = chart_series_from_resolved_cells(true, &[Decimal::ONE, Decimal::from(2)]);
        assert_eq!(ok.1, None);
    }

    #[test]
    fn filename_period_not_trusted() {
        assert_eq!(source_period(None), Err(PERIOD_CONFIRMATION_REQUIRED));
    }
}
