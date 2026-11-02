//! 指标计算、周期、比较、聚合（数据与 SQLite 工作包所有权）。
//!
//! 合同：docs/specs/data-period-import.md §1（数值/指标/库存/资金差额）、
//! §3（期间与比较规则）、DC 算例（contracts/v1/data-period-cases.json）。
//! 全部运算走 [`crate::contracts::decimal`]，禁止 f64 中转业务数值。

pub mod engine;
pub mod export_layout;
pub mod fingerprint;
pub mod observations;
pub mod periods;
pub mod targets;

pub use engine::{
    closing_balance_at, compute_comparison, configured_derived_ready, display_money, eval_formula,
    flow_sum, inventory_rollforward, period_gap, positive_div, ratio_aggregate,
    validate_formula_json, ClosingObservation, ComparisonOutcome, EngineReason, RollforwardInput,
    RollforwardOutcome, INBOUND_PAYMENT_GAP_LABEL, INVOICE_RECEIPT_GAP_LABEL,
};
pub use fingerprint::{
    canonical_json, sha256_hex, AnalysisRevisionInput, BaseRef, ConflictResolutionSnapshot,
    ManualInputSnapshot, MetricDefSnapshot, SourceSelection, CANONICALIZATION_VERSION,
};
pub use observations::{
    aggregate_cumulative_vs_months, aggregate_with_authoritative, dedupe_detail_rows,
    merge_summary_candidates, DetailMerge, DetailObservation, ObservationKey, RepresentationChoice,
    SummaryCandidate, SummaryMerge,
};
pub use periods::{
    check_full_coverage, check_reaggregation, excel_serial_to_date, fiscal_quarter_label,
    fiscal_quarter_span, local_calendar_date, mom_base, natural_month_span, next_custom_window,
    parse_calendar_date, resolve_policy, signatures_compatible, validate_policy_no_overlap,
    yoy_base, BasePeriod, CalendarPolicy, CalendarReason, ComparisonSignature, ExcelDateSystem,
    PolicyVersion, ResolvedPolicy, CUSTOM_WINDOW_LABEL, DEFAULT_TIMEZONE, FIFTY_TWO_WEEK_LABEL,
};
pub use targets::{
    explicit_target_only, monthly_revenue_floor, monthly_revenue_floor_eligible, MonthlyFloorInput,
    MonthlyFloorOutcome, CANDIDATE_RAISE_NEEDS_REVIEW, CONFIRMED_TARGET_MISSING,
};
