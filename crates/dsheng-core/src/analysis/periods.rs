//! 期间、基期算法与周期策略（data-period-import.md §3）。
//!
//! §3.2 基期表：
//!
//! | grain | 环比 | 同期 |
//! | --- | --- | --- |
//! | day | 前 1 当地日 | 上一年同月同日；闰日不存在 → null/`date_not_in_prior_year` |
//! | week | 全周前移 7 天 | 全周前移 364 天，固定标签“52周同期” |
//! | month | 按月度起点配置的前一完整周期 | 上一年同一起点月份的周期 |
//! | quarter | 前一财季（前 3 月） | 同财年配置前 12 月的财季 |
//! | custom | 紧接 from 前的等天数范围 | 两端减 1 年均存在且总天数相同，否则 `prior_year_range_not_comparable` |

use crate::contracts::domain::{CoverageStatus, DateRange, Granularity};
use chrono::{DateTime, Datelike, Duration, NaiveDate, TimeZone, Utc};

/// 周 yoy 固定标签（§3.2）。
pub const FIFTY_TWO_WEEK_LABEL: &str = "52周同期";
/// custom 下一窗口标签（§3.1）。
pub const CUSTOM_WINDOW_LABEL: &str = "等长自定义行动窗口";
/// 工作区默认时区（§3.1）。
pub const DEFAULT_TIMEZONE: &str = "Asia/Shanghai";

/// 周期日历参数（策略快照中与基期相关部分）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarPolicy {
    /// ISO 1–7，默认 1（周一）。
    pub week_start_day: u32,
    /// 月度周期起始日（1–31）；短月截到月末，下月继续原锚点日号。
    pub month_start_day: u32,
    /// 财年起月，默认 1。
    pub fiscal_year_start_month: u32,
    /// IANA 时区标识；默认 Asia/Shanghai。
    pub timezone_id: String,
}

impl Default for CalendarPolicy {
    fn default() -> Self {
        Self {
            week_start_day: 1,
            month_start_day: 1,
            fiscal_year_start_month: 1,
            timezone_id: DEFAULT_TIMEZONE.to_string(),
        }
    }
}

/// 日历层确定原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CalendarReason {
    #[error("date_not_in_prior_year")]
    DateNotInPriorYear,
    #[error("prior_year_range_not_comparable")]
    PriorYearRangeNotComparable,
    #[error("partial_period")]
    PartialPeriod,
    #[error("calendar_mismatch")]
    CalendarMismatch,
    #[error("coverage_incomplete")]
    CoverageIncomplete,
    #[error("cannot_disaggregate")]
    CannotDisaggregate,
    #[error("invalid_excel_date")]
    InvalidExcelDate,
    #[error("ambiguous_date")]
    AmbiguousDate,
    #[error("invalid_date")]
    InvalidDate,
}

impl CalendarReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DateNotInPriorYear => "date_not_in_prior_year",
            Self::PriorYearRangeNotComparable => "prior_year_range_not_comparable",
            Self::PartialPeriod => "partial_period",
            Self::CalendarMismatch => "calendar_mismatch",
            Self::CoverageIncomplete => "coverage_incomplete",
            Self::CannotDisaggregate => "cannot_disaggregate",
            Self::InvalidExcelDate => "invalid_excel_date",
            Self::AmbiguousDate => "ambiguous_date",
            Self::InvalidDate => "invalid_date",
        }
    }
}

/// 基期结果（含固定标签，如“52周同期”）。
#[derive(Debug, Clone, PartialEq)]
pub struct BasePeriod {
    pub range: DateRange,
    pub label: Option<&'static str>,
}

fn month_first(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, 1)
}

fn month_last(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month + 1, 1)
        .or_else(|| NaiveDate::from_ymd_opt(year + 1, 1, 1))
        .map(|d| d - Duration::days(1))
}

fn month_add(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let zero_based = year * 12 + (month as i32 - 1) + delta;
    let y = zero_based.div_euclid(12);
    let m = zero_based.rem_euclid(12) as u32 + 1;
    (y, m)
}

/// 该日期所在自然月的完整闭区间。
pub fn natural_month_span(d: NaiveDate) -> DateRange {
    DateRange {
        from: month_first(d.year(), d.month()).expect("自然月"),
        to: month_last(d.year(), d.month()).expect("自然月"),
    }
}

/// 指定起点月份的月度周期。每个端点独立按原始日号钳制，不沿上月日期漂移。
fn month_span_from_anchor(year: i32, month: u32, month_start_day: u32) -> DateRange {
    fn anchor(year: i32, month: u32, day: u32) -> NaiveDate {
        let last = month_last(year, month).expect("有效月份");
        NaiveDate::from_ymd_opt(year, month, day.min(last.day())).expect("月度起点日为1至31")
    }
    let (next_year, next_month) = month_add(year, month, 1);
    DateRange {
        from: anchor(year, month, month_start_day),
        to: anchor(next_year, next_month, month_start_day) - Duration::days(1),
    }
}

/// 日期所属月度周期；标签归属区间起点所在月，不按截止月命名。
pub fn monthly_period_span(date: NaiveDate, month_start_day: u32) -> DateRange {
    let current = month_span_from_anchor(date.year(), date.month(), month_start_day);
    if date >= current.from {
        current
    } else {
        let (year, month) = month_add(date.year(), date.month(), -1);
        month_span_from_anchor(year, month, month_start_day)
    }
}

/// 财季区间：返回（财年起始年 FY、季度序号 1–4、区间）。
///
/// FY 按起始年命名（§3.1）：财年 2026-04 起的 2027-01 属 FY2026 Q4。
pub fn fiscal_quarter_span(d: NaiveDate, fiscal_year_start_month: u32) -> (i32, u32, DateRange) {
    let m = d.month() as i32;
    let fs = fiscal_year_start_month as i32;
    let fy_start_year = d.year() - i32::from(m < fs);
    let offset = (m - fs).rem_euclid(12); // 年内自财年起点的月偏移
    let q = (offset / 3 + 1) as u32;
    let (qy, qm) = month_add(fy_start_year, fiscal_year_start_month, (q as i32 - 1) * 3);
    let (ey, em) = month_add(qy, qm, 2);
    let range = DateRange {
        from: month_first(qy, qm).expect("财季首月"),
        to: month_last(ey, em).expect("财季末月"),
    };
    (fy_start_year, q, range)
}

/// 财季显示标签 `FY{年} Q{序号}`。
pub fn fiscal_quarter_label(d: NaiveDate, fiscal_year_start_month: u32) -> String {
    let (fy, q, _) = fiscal_quarter_span(d, fiscal_year_start_month);
    format!("FY{fy} Q{q}")
}

/// 标准粒度完整桶校验（§3.2：标准 grain 不覆盖完整桶 → `partial_period`）。
pub fn check_full_coverage(
    range: DateRange,
    grain: Granularity,
    policy: &CalendarPolicy,
) -> Result<(), CalendarReason> {
    match grain {
        Granularity::Day => Ok(()), // 闭区间日窗口即完整日桶集合
        Granularity::Week => {
            let weekday = range.from.weekday().number_from_monday(); // 1=周一
            if range.day_count() == 7 && weekday == policy.week_start_day {
                Ok(())
            } else {
                Err(CalendarReason::PartialPeriod)
            }
        }
        Granularity::Month => {
            if monthly_period_span(range.from, policy.month_start_day) == range {
                Ok(())
            } else {
                Err(CalendarReason::PartialPeriod)
            }
        }
        Granularity::Quarter => {
            let (_, _, span) = fiscal_quarter_span(range.from, policy.fiscal_year_start_month);
            if span == range {
                Ok(())
            } else {
                Err(CalendarReason::PartialPeriod)
            }
        }
        Granularity::Custom => Ok(()),
    }
}

/// 环比基期（§3.2 表）。标准 grain 不完整时先给 `partial_period`。
pub fn mom_base(
    range: DateRange,
    grain: Granularity,
    policy: &CalendarPolicy,
) -> Result<BasePeriod, CalendarReason> {
    check_full_coverage(range, grain, policy)?;
    let base = match grain {
        Granularity::Day => DateRange {
            from: range.from - Duration::days(1),
            to: range.to - Duration::days(1),
        },
        Granularity::Week => DateRange {
            from: range.from - Duration::days(7),
            to: range.to - Duration::days(7),
        },
        Granularity::Month => {
            let (py, pm) = month_add(range.from.year(), range.from.month(), -1);
            month_span_from_anchor(py, pm, policy.month_start_day)
        }
        Granularity::Quarter => {
            // 前一财季 = 本财季起点前移 3 个自然月
            let (_, _, span) = fiscal_quarter_span(range.from, policy.fiscal_year_start_month);
            let (qy, qm) = month_add(span.from.year(), span.from.month(), -3);
            let (ey, em) = month_add(qy, qm, 2);
            DateRange {
                from: month_first(qy, qm).expect("财季"),
                to: month_last(ey, em).expect("财季"),
            }
        }
        Granularity::Custom => {
            // 紧接 from 前的等天数范围
            let n = range.day_count();
            DateRange {
                from: range.from - Duration::days(n),
                to: range.from - Duration::days(1),
            }
        }
    };
    Ok(BasePeriod {
        range: base,
        label: None,
    })
}

/// 同期基期（§3.2 表）。
pub fn yoy_base(
    range: DateRange,
    grain: Granularity,
    policy: &CalendarPolicy,
) -> Result<BasePeriod, CalendarReason> {
    check_full_coverage(range, grain, policy)?;
    match grain {
        Granularity::Day => {
            let from = shift_year(range.from).ok_or(CalendarReason::DateNotInPriorYear)?;
            let to = shift_year(range.to).ok_or(CalendarReason::DateNotInPriorYear)?;
            Ok(BasePeriod {
                range: DateRange { from, to },
                label: None,
            })
        }
        Granularity::Week => Ok(BasePeriod {
            range: DateRange {
                from: range.from - Duration::days(364),
                to: range.to - Duration::days(364),
            },
            label: Some(FIFTY_TWO_WEEK_LABEL),
        }),
        Granularity::Month => Ok(BasePeriod {
            range: month_span_from_anchor(
                range.from.year() - 1,
                range.from.month(),
                policy.month_start_day,
            ),
            label: None,
        }),
        Granularity::Quarter => {
            // 同财年配置前 12 月的财季：财季起点年-1，季内月序不变
            let (_, _, span) = fiscal_quarter_span(range.from, policy.fiscal_year_start_month);
            let (qy, qm) = month_add(span.from.year(), span.from.month(), -12);
            let (ey, em) = month_add(qy, qm, 2);
            Ok(BasePeriod {
                range: DateRange {
                    from: month_first(qy, qm).expect("财季"),
                    to: month_last(ey, em).expect("财季"),
                },
                label: None,
            })
        }
        Granularity::Custom => {
            // 两端减 1 年均存在且总天数相同
            let from = shift_year(range.from).ok_or(CalendarReason::PriorYearRangeNotComparable)?;
            let to = shift_year(range.to).ok_or(CalendarReason::PriorYearRangeNotComparable)?;
            let base = DateRange { from, to };
            if base.day_count() == range.day_count() {
                Ok(BasePeriod {
                    range: base,
                    label: None,
                })
            } else {
                Err(CalendarReason::PriorYearRangeNotComparable)
            }
        }
    }
}

fn shift_year(d: NaiveDate) -> Option<NaiveDate> {
    let year = d.year() - 1;
    NaiveDate::from_ymd_opt(year, d.month(), d.day())
}

/// custom 下一窗口：紧接 to 的同天数范围（§3.1）。
pub fn next_custom_window(range: DateRange) -> (DateRange, &'static str) {
    let n = range.day_count();
    (
        DateRange {
            from: range.to + Duration::days(1),
            to: range.to + Duration::days(n),
        },
        CUSTOM_WINDOW_LABEL,
    )
}

/// 周期策略版本（[from,to) 左闭右开，最后 to=null；拒绝重叠）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyVersion {
    pub version: String,
    pub effective_from: NaiveDate,
    pub effective_to: Option<NaiveDate>,
}

impl PolicyVersion {
    /// [from,to) 覆盖判定。
    fn covers(&self, d: NaiveDate) -> bool {
        d >= self.effective_from && self.effective_to.is_none_or(|t| d < t)
    }
}

/// 策略版本重叠校验（§3.1：拒绝重叠）。
pub fn validate_policy_no_overlap(versions: &[PolicyVersion]) -> Result<(), &'static str> {
    for (i, a) in versions.iter().enumerate() {
        for b in versions.iter().skip(i + 1) {
            let overlap = match (a.effective_to, b.effective_to) {
                (None, None) => true,
                (None, Some(bt)) => bt > a.effective_from,
                (Some(at), None) => at > b.effective_from,
                (Some(at), Some(bt)) => a.effective_from < bt && b.effective_from < at,
            };
            if overlap {
                return Err("policy_overlap_rejected");
            }
        }
    }
    Ok(())
}

/// 策略选择结果。
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPolicy<'a> {
    pub policy: &'a PolicyVersion,
    /// 用户显式选择整段范围作用时为 true（标 override 并保存快照）。
    pub override_applied: bool,
}

/// 为分析范围选择策略版本（§3.1）。
///
/// 默认：from 与 to 命中同一版本；跨界 → `policy_selection_required`，
/// 用户显式选一版本作用于整个范围（标 override），不自动分段混算。
/// 显式选择始终优先，包括用当前策略重新分析旧策略生效期间的数据。
pub fn resolve_policy<'a>(
    versions: &'a [PolicyVersion],
    range: DateRange,
    explicit: Option<&str>,
) -> Result<ResolvedPolicy<'a>, &'static str> {
    if versions.is_empty() {
        return Err("policy_selection_required");
    }
    if let Some(explicit_version) = explicit {
        let chosen = versions
            .iter()
            .find(|v| v.version == explicit_version)
            .ok_or("policy_selection_required")?;
        return Ok(ResolvedPolicy {
            policy: chosen,
            override_applied: true,
        });
    }
    let from_hit = versions.iter().find(|v| v.covers(range.from));
    let to_hit = versions.iter().find(|v| v.covers(range.to));
    match (from_hit, to_hit) {
        (Some(a), Some(b)) if a.version == b.version => Ok(ResolvedPolicy {
            policy: a,
            override_applied: false,
        }),
        _ => Err("policy_selection_required"),
    }
}

/// 重聚合方向校验（§2 coverage / §3.2）。
///
/// 允许：同粒度；完整日 → 周/月/季/custom；完整月 → 财季。
/// 月不能拆日/周、季度不能拆月；coverage 非 complete → `coverage_incomplete`。
pub fn check_reaggregation(
    source: Granularity,
    target: Granularity,
    coverage: CoverageStatus,
) -> Result<(), CalendarReason> {
    use Granularity::*;
    let allowed = match (source, target) {
        (Day, Week | Month | Quarter | Custom) => true,
        (Month, Quarter) => true,
        (a, b) => a == b,
    };
    if !allowed {
        return Err(CalendarReason::CannotDisaggregate);
    }
    if source != target && coverage != CoverageStatus::Complete {
        return Err(CalendarReason::CoverageIncomplete);
    }
    Ok(())
}

/// 比较兼容签名（§3.2）：metric semantic hash + unit/currency/tax + scope +
/// timezone + 目标 grain + week_start（仅 week）+ fiscal_start（仅 quarter）。
///
/// 不含 policy 版本、文件名或原输入粒度。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonSignature {
    pub metric_semantic_hash: String,
    pub unit_signature_json: String,
    pub scope_key: String,
    pub timezone_id: String,
    pub grain: Granularity,
    pub week_start_day: u32,
    pub month_start_day: u32,
    pub fiscal_year_start_month: u32,
}

/// 签名兼容性（仅 week 比较周起点、仅 quarter 比较财年起月）。
pub fn signatures_compatible(
    a: &ComparisonSignature,
    b: &ComparisonSignature,
) -> Result<(), CalendarReason> {
    if a.metric_semantic_hash != b.metric_semantic_hash
        || a.unit_signature_json != b.unit_signature_json
        || a.scope_key != b.scope_key
        || a.timezone_id != b.timezone_id
        || a.grain != b.grain
    {
        return Err(CalendarReason::CalendarMismatch);
    }
    match a.grain {
        Granularity::Month if a.month_start_day != b.month_start_day => {
            Err(CalendarReason::CalendarMismatch)
        }
        Granularity::Week if a.week_start_day != b.week_start_day => {
            Err(CalendarReason::CalendarMismatch)
        }
        Granularity::Quarter if a.fiscal_year_start_month != b.fiscal_year_start_month => {
            Err(CalendarReason::CalendarMismatch)
        }
        _ => Ok(()),
    }
}

/// UTC 事件时间按工作区时区转换当地日历日（§3.1；禁止宿主时区隐式参与）。
pub fn local_calendar_date(ts: DateTime<Utc>, tz_id: &str) -> Result<NaiveDate, &'static str> {
    let tz: chrono_tz::Tz = tz_id.parse().map_err(|_| "unknown_timezone")?;
    let local = tz.from_utc_datetime(&ts.naive_utc());
    Ok(local.date_naive())
}

/// 严格日历文本解析：仅 YYYY-MM-DD / YYYY/MM/DD（§3.1）。
///
/// `01/02/26` 等歧义格式 → `ambiguous_date`；形状匹配但非真实公历日 →
/// `invalid_date`。
pub fn parse_calendar_date(text: &str) -> Result<NaiveDate, CalendarReason> {
    let s = text.trim();
    let b = s.as_bytes();
    let dash = s.len() == 10 && b[4] == b'-' && b[7] == b'-';
    let slash = s.len() == 10 && b[4] == b'/' && b[7] == b'/';
    if !dash && !slash {
        return Err(CalendarReason::AmbiguousDate);
    }
    if !s[..4].chars().all(|c| c.is_ascii_digit())
        || !s[5..7].chars().all(|c| c.is_ascii_digit())
        || !s[8..10].chars().all(|c| c.is_ascii_digit())
    {
        return Err(CalendarReason::AmbiguousDate);
    }
    let year: i32 = s[..4].parse().map_err(|_| CalendarReason::InvalidDate)?;
    let month: u32 = s[5..7].parse().map_err(|_| CalendarReason::InvalidDate)?;
    let day: u32 = s[8..10].parse().map_err(|_| CalendarReason::InvalidDate)?;
    NaiveDate::from_ymd_opt(year, month, day).ok_or(CalendarReason::InvalidDate)
}

/// Excel 日期系统（按文件元数据，§3.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExcelDateSystem {
    Y1900,
    Y1904,
}

/// Excel 序列日 → 公历日。
///
/// 1900 系统序列 60 为伪闰日 1900-02-29 → `invalid_excel_date`；
/// 序列 <1 无效。1904 系统序列 0 = 1904-01-01。
pub fn excel_serial_to_date(
    serial: i64,
    system: ExcelDateSystem,
) -> Result<NaiveDate, CalendarReason> {
    match system {
        ExcelDateSystem::Y1900 => {
            if serial < 1 || serial == 60 {
                return Err(CalendarReason::InvalidExcelDate);
            }
            let epoch = if serial < 60 {
                NaiveDate::from_ymd_opt(1899, 12, 31).expect("epoch")
            } else {
                NaiveDate::from_ymd_opt(1899, 12, 30).expect("epoch")
            };
            epoch
                .checked_add_signed(Duration::days(serial))
                .ok_or(CalendarReason::InvalidExcelDate)
        }
        ExcelDateSystem::Y1904 => {
            if serial < 0 {
                return Err(CalendarReason::InvalidExcelDate);
            }
            let epoch = NaiveDate::from_ymd_opt(1904, 1, 1).expect("epoch");
            epoch
                .checked_add_signed(Duration::days(serial))
                .ok_or(CalendarReason::InvalidExcelDate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn monthly_anchor_clamps_each_month_without_drifting() {
        for (date, day, from, to) in [
            ("2026-10-14", 15, "2026-09-15", "2026-10-14"),
            ("2026-10-15", 15, "2026-10-15", "2026-11-14"),
            ("2023-02-27", 31, "2023-01-31", "2023-02-27"),
            ("2023-02-28", 31, "2023-02-28", "2023-03-30"),
            ("2023-03-30", 31, "2023-02-28", "2023-03-30"),
            ("2023-03-31", 31, "2023-03-31", "2023-04-29"),
            ("2024-02-29", 31, "2024-02-29", "2024-03-30"),
            ("2026-01-01", 31, "2025-12-31", "2026-01-30"),
        ] {
            assert_eq!(
                monthly_period_span(d(date), day),
                DateRange {
                    from: d(from),
                    to: d(to)
                }
            );
        }
        for year in [2023, 2024] {
            for start in [1, 15, 28, 29, 30, 31] {
                for month in 1..=12 {
                    let range = month_span_from_anchor(year, month, start);
                    let (next_year, next_month) = month_add(year, month, 1);
                    let next = month_span_from_anchor(next_year, next_month, start);
                    assert_eq!(range.to + Duration::days(1), next.from, "月桶连续且无重叠");
                    assert_eq!(range.from.month(), month, "标签归属起点月");
                }
            }
        }
    }

    #[test]
    fn month_comparisons_use_same_anchor_across_short_and_leap_months() {
        let policy = CalendarPolicy {
            month_start_day: 31,
            ..Default::default()
        };
        let current = monthly_period_span(d("2024-03-01"), 31);
        assert_eq!(
            current,
            DateRange {
                from: d("2024-02-29"),
                to: d("2024-03-30")
            }
        );
        assert_eq!(
            mom_base(current, Granularity::Month, &policy)
                .unwrap()
                .range,
            DateRange {
                from: d("2024-01-31"),
                to: d("2024-02-28")
            }
        );
        assert_eq!(
            yoy_base(current, Granularity::Month, &policy)
                .unwrap()
                .range,
            DateRange {
                from: d("2023-02-28"),
                to: d("2023-03-30")
            }
        );
        let ordinary = monthly_period_span(d("2024-02-01"), 1);
        assert_eq!(
            yoy_base(ordinary, Granularity::Month, &CalendarPolicy::default())
                .unwrap()
                .range,
            DateRange {
                from: d("2023-02-01"),
                to: d("2023-02-28")
            }
        );
        assert_eq!(
            check_full_coverage(
                natural_month_span(d("2024-02-01")),
                Granularity::Month,
                &policy
            ),
            Err(CalendarReason::PartialPeriod)
        );
    }

    #[test]
    fn fiscal_quarter_math() {
        // FY2026 Q4（财年 4 月起）
        let (fy, q, span) = fiscal_quarter_span(d("2027-02-15"), 4);
        assert_eq!((fy, q), (2026, 4));
        assert_eq!(span.from, d("2027-01-01"));
        assert_eq!(span.to, d("2027-03-31"));
        assert_eq!(fiscal_quarter_label(d("2027-01-01"), 4), "FY2026 Q4");
        // 自然财年（1 月起）
        let (fy, q, span) = fiscal_quarter_span(d("2026-04-10"), 1);
        assert_eq!((fy, q), (2026, 2));
        assert_eq!(span.from, d("2026-04-01"));
        assert_eq!(span.to, d("2026-06-30"));
    }

    #[test]
    fn coverage_checks() {
        let p = CalendarPolicy::default();
        assert!(check_full_coverage(
            DateRange::new(d("2026-03-01"), d("2026-03-31")).unwrap(),
            Granularity::Month,
            &p
        )
        .is_ok());
        assert_eq!(
            check_full_coverage(
                DateRange::new(d("2026-10-01"), d("2026-10-15")).unwrap(),
                Granularity::Month,
                &p
            ),
            Err(CalendarReason::PartialPeriod)
        );
        // 周未对齐
        assert_eq!(
            check_full_coverage(
                DateRange::new(d("2026-10-27"), d("2026-11-02")).unwrap(),
                Granularity::Week,
                &p
            ),
            Err(CalendarReason::PartialPeriod)
        );
    }

    #[test]
    fn date_parsing_strict() {
        assert_eq!(parse_calendar_date("2026-10-05"), Ok(d("2026-10-05")));
        assert_eq!(parse_calendar_date("2026/10/05"), Ok(d("2026-10-05")));
        assert_eq!(
            parse_calendar_date("01/02/26"),
            Err(CalendarReason::AmbiguousDate)
        );
        assert_eq!(
            parse_calendar_date("2026-02-30"),
            Err(CalendarReason::InvalidDate)
        );
    }

    #[test]
    fn excel_dates() {
        assert_eq!(
            excel_serial_to_date(60, ExcelDateSystem::Y1900),
            Err(CalendarReason::InvalidExcelDate)
        );
        assert_eq!(
            excel_serial_to_date(1, ExcelDateSystem::Y1900),
            Ok(d("1900-01-01"))
        );
        assert_eq!(
            excel_serial_to_date(59, ExcelDateSystem::Y1900),
            Ok(d("1900-02-28"))
        );
        assert_eq!(
            excel_serial_to_date(61, ExcelDateSystem::Y1900),
            Ok(d("1900-03-01"))
        );
        assert_eq!(
            excel_serial_to_date(0, ExcelDateSystem::Y1904),
            Ok(d("1904-01-01"))
        );
    }

    #[test]
    fn policy_resolution() {
        let versions = vec![
            PolicyVersion {
                version: "V1".into(),
                effective_from: d("1900-01-01"),
                effective_to: Some(d("2026-10-16")),
            },
            PolicyVersion {
                version: "V2".into(),
                effective_from: d("2026-10-16"),
                effective_to: None,
            },
        ];
        let range = DateRange::new(d("2026-10-01"), d("2026-10-31")).unwrap();
        assert_eq!(
            resolve_policy(&versions, range, None).unwrap_err(),
            "policy_selection_required"
        );
        let resolved = resolve_policy(&versions, range, Some("V1")).unwrap();
        assert_eq!(resolved.policy.version, "V1");
        assert!(resolved.override_applied);
        // 未跨界：无需显式
        let same = resolve_policy(
            &versions,
            DateRange::new(d("2026-11-01"), d("2026-11-30")).unwrap(),
            None,
        )
        .unwrap();
        assert_eq!(same.policy.version, "V2");
        assert!(!same.override_applied);
        // 旧日期完整位于V1生效范围，也必须使用显式指定的当前V2。
        let historical = DateRange::new(d("2026-09-27"), d("2026-10-03")).unwrap();
        let current_policy = resolve_policy(&versions, historical, Some("V2")).unwrap();
        assert_eq!(current_policy.policy.version, "V2");
        assert!(current_policy.override_applied);
        assert_eq!(
            resolve_policy(&versions, historical, None)
                .unwrap()
                .policy
                .version,
            "V1",
            "未显式选择时保留原自动策略规则"
        );
        assert_eq!(
            resolve_policy(&versions, historical, Some("unknown")).unwrap_err(),
            "policy_selection_required",
            "无效显式策略不能静默回退到旧策略"
        );
    }

    #[test]
    fn signature_week_start_matters_only_for_week() {
        fn sig(grain: Granularity, week_start: u32) -> ComparisonSignature {
            ComparisonSignature {
                metric_semantic_hash: "h".into(),
                unit_signature_json: "{}".into(),
                scope_key: "全公司".into(),
                timezone_id: DEFAULT_TIMEZONE.into(),
                grain,
                week_start_day: week_start,
                month_start_day: 1,
                fiscal_year_start_month: 1,
            }
        }
        assert_eq!(
            signatures_compatible(&sig(Granularity::Week, 1), &sig(Granularity::Week, 7)),
            Err(CalendarReason::CalendarMismatch)
        );
        // month 粒度下周起点差异不参与
        assert!(
            signatures_compatible(&sig(Granularity::Month, 1), &sig(Granularity::Month, 7)).is_ok()
        );
        let mut shifted_month = sig(Granularity::Month, 1);
        shifted_month.month_start_day = 15;
        assert_eq!(
            signatures_compatible(&sig(Granularity::Month, 1), &shifted_month),
            Err(CalendarReason::CalendarMismatch)
        );
        // quarter 才比较财年起月
        assert_eq!(
            signatures_compatible(&sig(Granularity::Quarter, 1), &sig(Granularity::Quarter, 7)),
            Ok(())
        );
    }
}
