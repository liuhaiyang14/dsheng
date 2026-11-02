//! 来源去重、候选观察与冲突（data-period-import.md §2）。
//!
//! - 相同 metric/role/scope/unit/tax/完整期间的多个汇总为候选同一观察，不能相加：
//!   同值去重保留一个经营值及全部证据；不同值 → conflict，经营值 null 保留双方。
//! - 明细与汇总共存须显式选 authoritative representation；只有一种才自动选择。
//! - 明细去重须确认业务行键；同键同值去重、同键不同值冲突；
//!   重叠来源无可靠键 → `overlap_unresolved`。
//! - 累计列是独立范围观察，不与其内部月份重复相加。

use crate::analysis::engine::EngineReason;
use crate::contracts::domain::DateRange;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

/// 单个汇总候选观察（同 metric/role/scope/unit/tax/完整期间）。
#[derive(Debug, Clone, PartialEq)]
pub struct SummaryCandidate {
    pub value: Decimal,
    pub evidence_id: String,
}

/// 汇总候选合并结果。
#[derive(Debug, Clone, PartialEq)]
pub enum SummaryMerge {
    /// 同值去重：保留一个经营值与全部证据（不重复相加）。
    Deduped {
        value: Decimal,
        evidence_ids: Vec<String>,
        /// 语义上已合并（供 DC `summed=false`）。
        summed: bool,
    },
    /// 不同值：经营值 null、保留双方，用户选主来源并填原因后才 valid。
    Conflict { reason: EngineReason },
}

/// 合并同键汇总候选（§2.2）。
pub fn merge_summary_candidates(candidates: &[SummaryCandidate]) -> SummaryMerge {
    let Some(first) = candidates.first() else {
        return SummaryMerge::Conflict {
            reason: EngineReason::InvalidInput,
        };
    };
    if candidates.iter().all(|c| c.value == first.value) {
        return SummaryMerge::Deduped {
            value: first.value,
            evidence_ids: candidates.iter().map(|c| c.evidence_id.clone()).collect(),
            summed: false,
        };
    }
    SummaryMerge::Conflict {
        reason: EngineReason::SourceConflict,
    }
}

/// 明细/汇总的权威表示选择（§2.3；明细不是默认优先）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepresentationChoice {
    Detail,
    Total,
}

/// 明细与汇总聚合结果。
#[derive(Debug, Clone, PartialEq)]
pub struct AuthoritativeAggregation {
    pub value: Decimal,
    /// 选定表示与另一表示的对账差（默认容差 0，仅对账不阻断）。
    pub reconciliation_difference: Option<Decimal>,
}

/// 明细与汇总共存时的聚合（§2.3）。
///
/// 未选 authoritative representation → `source_selection_required`；
/// 选定后另一表示只作对账；返回对账差。
pub fn aggregate_with_authoritative(
    details: &[Decimal],
    total: Decimal,
    choice: Option<RepresentationChoice>,
) -> Result<AuthoritativeAggregation, EngineReason> {
    let detail_sum = details.iter().copied().fold(Decimal::ZERO, |a, v| a + v);
    match choice {
        None => Err(EngineReason::SourceSelectionRequired),
        Some(RepresentationChoice::Detail) => Ok(AuthoritativeAggregation {
            value: detail_sum,
            reconciliation_difference: Some(detail_sum - total),
        }),
        Some(RepresentationChoice::Total) => Ok(AuthoritativeAggregation {
            value: total,
            reconciliation_difference: Some(total - detail_sum),
        }),
    }
}

/// 累计列处理（§2.5）：累计作为独立范围观察；不与内部月份重复相加。
///
/// 返回权威表示（monthly 明细或 cumulative 累计）的经营值与对账差。
pub fn aggregate_cumulative_vs_months(
    months: &[Decimal],
    cumulative: Decimal,
    choice: Option<RepresentationChoice>,
) -> Result<AuthoritativeAggregation, EngineReason> {
    aggregate_with_authoritative(months, cumulative, choice)
}

/// 明细行观察（§2.4）。
#[derive(Debug, Clone, PartialEq)]
pub struct DetailObservation {
    /// 已确认业务行键（document_id+line_id 等）；None = 无键。
    pub row_key: Option<String>,
    pub value: Decimal,
    pub evidence_id: String,
}

/// 明细合并结果。
#[derive(Debug, Clone, PartialEq)]
pub struct DetailMerge {
    pub value: Decimal,
    /// 去重后保留的证据（同键同值合并全部证据）。
    pub evidence_ids: Vec<String>,
    pub deduped_rows: usize,
}

/// 重叠来源的明细去重（§2.4）。
///
/// - 重叠且存在无键行 → `overlap_unresolved`（须选源或明确不重叠范围）；
/// - 同键同全部业务字段 → 去重（保留一值、合并证据）；
/// - 同键不同值 → `source_conflict`；
/// - 无键同值不代表重复，不重叠时按行求和。
pub fn dedupe_detail_rows(
    rows: &[DetailObservation],
    ranges_overlap: bool,
) -> Result<DetailMerge, EngineReason> {
    if ranges_overlap && rows.iter().any(|r| r.row_key.is_none()) {
        return Err(EngineReason::OverlapUnresolved);
    }
    let mut keyed: BTreeMap<String, (Decimal, Vec<String>)> = BTreeMap::new();
    let mut unkeyed_sum = Decimal::ZERO;
    let mut unkeyed_evidence = Vec::new();
    for row in rows {
        match &row.row_key {
            Some(key) => match keyed.get(key) {
                Some((v, _)) if *v == row.value => {
                    keyed
                        .get_mut(key)
                        .expect("已存在")
                        .1
                        .push(row.evidence_id.clone());
                }
                Some((_, _)) => return Err(EngineReason::SourceConflict),
                None => {
                    keyed.insert(key.clone(), (row.value, vec![row.evidence_id.clone()]));
                }
            },
            None => {
                unkeyed_sum += row.value;
                unkeyed_evidence.push(row.evidence_id.clone());
            }
        }
    }
    let mut value = unkeyed_sum;
    let mut evidence_ids = unkeyed_evidence;
    let deduped_rows = keyed.len();
    for (v, evs) in keyed.into_values() {
        value += v;
        evidence_ids.extend(evs);
    }
    evidence_ids.sort();
    Ok(DetailMerge {
        value,
        evidence_ids,
        deduped_rows,
    })
}

/// 观察键（metric/role/scope/unit/tax/完整期间）——候选同一观察的分组键（§2.2）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationKey {
    pub metric_key: String,
    pub measure_role: String,
    pub scope_key: String,
    pub unit: String,
    pub tax_basis: Option<String>,
    pub period: DateRange,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::decimal::canonical_decimal as cd;

    #[test]
    fn duplicate_summary_dedupes() {
        let m = merge_summary_candidates(&[
            SummaryCandidate {
                value: cd("100").unwrap(),
                evidence_id: "S1".into(),
            },
            SummaryCandidate {
                value: cd("100").unwrap(),
                evidence_id: "S2".into(),
            },
        ]);
        let SummaryMerge::Deduped {
            value,
            evidence_ids,
            summed,
        } = m
        else {
            panic!("同值候选应去重");
        };
        assert_eq!(value, cd("100").unwrap());
        assert_eq!(evidence_ids, vec!["S1", "S2"]);
        assert!(!summed);
    }

    #[test]
    fn conflicting_summary_blocks() {
        let m = merge_summary_candidates(&[
            SummaryCandidate {
                value: cd("100").unwrap(),
                evidence_id: "S1".into(),
            },
            SummaryCandidate {
                value: cd("105").unwrap(),
                evidence_id: "S2".into(),
            },
        ]);
        assert_eq!(
            m,
            SummaryMerge::Conflict {
                reason: EngineReason::SourceConflict
            }
        );
    }

    #[test]
    fn detail_total_requires_choice() {
        let d = [cd("40").unwrap(), cd("60").unwrap()];
        assert_eq!(
            aggregate_with_authoritative(&d, cd("100").unwrap(), None),
            Err(EngineReason::SourceSelectionRequired)
        );
        let chosen = aggregate_with_authoritative(
            &d,
            cd("100").unwrap(),
            Some(RepresentationChoice::Detail),
        )
        .unwrap();
        assert_eq!(chosen.value, cd("100").unwrap());
        assert_eq!(chosen.reconciliation_difference, Some(cd("0").unwrap()));
    }

    #[test]
    fn detail_key_dedupe_and_conflict() {
        // 同键同值去重
        let m = dedupe_detail_rows(
            &[
                DetailObservation {
                    row_key: Some("D1-L1".into()),
                    value: cd("10").unwrap(),
                    evidence_id: "A".into(),
                },
                DetailObservation {
                    row_key: Some("D1-L1".into()),
                    value: cd("10").unwrap(),
                    evidence_id: "B".into(),
                },
            ],
            true,
        )
        .unwrap();
        assert_eq!(m.value, cd("10").unwrap());
        assert_eq!(m.deduped_rows, 1);
        assert_eq!(m.evidence_ids, vec!["A", "B"]);
        // 同键不同值冲突
        assert_eq!(
            dedupe_detail_rows(
                &[
                    DetailObservation {
                        row_key: Some("D1-L1".into()),
                        value: cd("10").unwrap(),
                        evidence_id: "A".into(),
                    },
                    DetailObservation {
                        row_key: Some("D1-L1".into()),
                        value: cd("11").unwrap(),
                        evidence_id: "B".into(),
                    },
                ],
                true,
            ),
            Err(EngineReason::SourceConflict)
        );
    }
}
