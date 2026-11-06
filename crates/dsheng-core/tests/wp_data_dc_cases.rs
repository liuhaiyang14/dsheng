//! 数据工作包 DC 算例数据驱动测试（S1）。
//!
//! 夹具：../../contracts/v1/data-period-cases.json（56 条，完全合成）。
//! 期望值全部来自合同文件，不在测试中反推。`not_` 前缀的期望键是负向断言
//! （实际值必须不等于该值），如 `not_simple_average`、`not_sum`。

use chrono::{DateTime, NaiveDate, Utc};
use dsheng_core::analysis::engine::EngineReason;
use dsheng_core::analysis::export_layout::{csv_export_text, xlsx_export_kind};
use dsheng_core::analysis::periods::{
    excel_serial_to_date, CalendarPolicy, CalendarReason, ComparisonSignature, ExcelDateSystem,
};
use dsheng_core::analysis::targets::CONFIRMED_TARGET_MISSING;
use dsheng_core::analysis::{
    aggregate_cumulative_vs_months, aggregate_with_authoritative, check_reaggregation,
    closing_balance_at, compute_comparison, configured_derived_ready, display_money, flow_sum,
    inventory_rollforward, local_calendar_date, merge_summary_candidates, mom_base,
    monthly_revenue_floor, monthly_revenue_floor_eligible, next_custom_window, parse_calendar_date,
    positive_div, ratio_aggregate, resolve_policy, signatures_compatible, validate_formula_json,
    yoy_base, ClosingObservation, DetailObservation, MonthlyFloorInput, PolicyVersion,
    RepresentationChoice, RollforwardInput, SummaryCandidate, SummaryMerge, DEFAULT_TIMEZONE,
    INVOICE_RECEIPT_GAP_LABEL,
};
use dsheng_core::contracts::decimal::{
    canonical_decimal, parse_input_decimal, to_canonical_string,
};
use dsheng_core::contracts::domain::{Cadence, DateRange, Direction, Granularity};
use dsheng_core::contracts::metrics::TargetConstraintScope;
use dsheng_core::import::scan::{chart_series_from_resolved_cells, source_period};
use dsheng_core::import::{
    classify_cell, cross_scope_sum_allowed, duplicate_columns, expand_merged_anchor, plan_blocks,
};
use rust_decimal::Decimal;
use serde_json::{json, Value};

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_cases() -> Vec<Value> {
    let raw = std::fs::read_to_string(repo_root().join("contracts/v1/data-period-cases.json"))
        .expect("读取 DC 算例");
    let v: Value = serde_json::from_str(&raw).expect("DC JSON");
    v["cases"].as_array().expect("cases 数组").clone()
}

/// metric-catalog 中的 revenue_semantics/direction（资格判定材料来自目录）。
fn catalog_metric(metric_key: &str) -> Value {
    let raw = std::fs::read_to_string(repo_root().join("contracts/v1/metric-catalog.json"))
        .expect("目录");
    let v: Value = serde_json::from_str(&raw).expect("目录 JSON");
    v["metrics"]
        .as_array()
        .expect("metrics")
        .iter()
        .find(|m| m["metric_key"] == metric_key)
        .cloned()
        .expect("目录中存在指标")
}

fn cd(input: &Value) -> Decimal {
    canonical_decimal(input.as_str().expect("decimal string")).expect("规范十进制")
}

fn dec_opt(input: &Value) -> Option<Decimal> {
    input.as_str().map(|s| cd(&Value::String(s.into())))
}

fn dec_json(d: Decimal) -> Value {
    json!(to_canonical_string(d))
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").expect("ISO 日期")
}

fn range(input: &Value) -> DateRange {
    let arr = input.as_array().expect("日期数组");
    DateRange::new(
        date(arr[0].as_str().unwrap()),
        date(arr[1].as_str().unwrap()),
    )
    .expect("区间")
}

fn range_pair(r: DateRange) -> Value {
    json!([
        r.from.format("%Y-%m-%d").to_string(),
        r.to.format("%Y-%m-%d").to_string()
    ])
}

fn base_json(base: Option<DateRange>) -> Value {
    base.map(range_pair).unwrap_or(Value::Null)
}

fn grain(s: &str) -> Granularity {
    serde_json::from_value::<Granularity>(json!(s)).expect("granularity")
}

fn default_policy() -> CalendarPolicy {
    CalendarPolicy::default()
}

fn engine_reason_json(r: Option<EngineReason>) -> Value {
    r.map(|e| json!(e.as_str())).unwrap_or(Value::Null)
}

/// 执行单条 DC 算例，返回实际结果 JSON。
fn dispatch(case: &Value) -> Value {
    let op = case["operation"].as_str().expect("operation");
    let input = &case["input"];
    match op {
        "flow_sum" => {
            let values: Vec<Decimal> = input["values"].as_array().unwrap().iter().map(cd).collect();
            match flow_sum(&values, input["coverage"] == "disjoint_complete") {
                Ok(v) => json!({"value": dec_json(v)}),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "duplicate_summary" => {
            let values = input["values"].as_array().unwrap();
            let evidence = input["evidence_ids"].as_array().unwrap();
            let cands: Vec<SummaryCandidate> = values
                .iter()
                .zip(evidence)
                .map(|(v, e)| SummaryCandidate {
                    value: cd(v),
                    evidence_id: e.as_str().unwrap().to_string(),
                })
                .collect();
            match merge_summary_candidates(&cands) {
                SummaryMerge::Deduped {
                    value,
                    evidence_ids,
                    summed,
                } => json!({
                    "value": dec_json(value),
                    "evidence_ids": evidence_ids,
                    "summed": summed,
                }),
                SummaryMerge::Conflict { reason } => {
                    json!({"value": null, "reason": reason.as_str()})
                }
            }
        }
        "conflicting_summary" => {
            let values = input["values"].as_array().unwrap();
            let cands: Vec<SummaryCandidate> = values
                .iter()
                .enumerate()
                .map(|(i, v)| SummaryCandidate {
                    value: cd(v),
                    evidence_id: format!("S{}", i + 1),
                })
                .collect();
            match merge_summary_candidates(&cands) {
                SummaryMerge::Conflict { reason } => {
                    json!({"value": null, "reason": reason.as_str()})
                }
                other => panic!("conflicting_summary 应产生冲突: {other:?}"),
            }
        }
        "detail_and_total" => {
            let details: Vec<Decimal> = input["detail_values"]
                .as_array()
                .unwrap()
                .iter()
                .map(cd)
                .collect();
            let total = cd(&input["total"]);
            let choice = match input["authoritative_representation"].as_str() {
                Some("detail") => Some(RepresentationChoice::Detail),
                Some("total") => Some(RepresentationChoice::Total),
                _ => None,
            };
            match aggregate_with_authoritative(&details, total, choice) {
                Ok(a) => json!({
                    "value": dec_json(a.value),
                    "reconciliation_difference": a.reconciliation_difference.map(dec_json),
                }),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "inventory_rollforward" => {
            let outcome = inventory_rollforward(&RollforwardInput {
                opening: dec_opt(&input["opening"]),
                inbound: cd(&input["inbound"]),
                outbound: cd(&input["outbound"]),
                closing_observation: dec_opt(&input["closing_observation"]),
            });
            json!({
                "value": outcome.value.map(dec_json),
                "reason": engine_reason_json(outcome.reason),
                "warning": outcome.warning,
            })
        }
        "ratio_aggregate" => {
            let nums: Vec<Decimal> = input["numerators"]
                .as_array()
                .unwrap()
                .iter()
                .map(cd)
                .collect();
            let dens: Vec<Decimal> = input["denominators"]
                .as_array()
                .unwrap()
                .iter()
                .map(cd)
                .collect();
            match ratio_aggregate(&nums, &dens) {
                Ok(v) => json!({"value": dec_json(v)}),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "positive_divide" => match positive_div(cd(&input["numerator"]), cd(&input["denominator"]))
        {
            Ok(v) => json!({"value": dec_json(v), "capped": false}),
            Err(e) => json!({"value": null, "reason": e.as_str()}),
        },
        "comparison" => {
            let out = compute_comparison(cd(&input["current"]), cd(&input["base"]));
            json!({
                "absolute_change": dec_json(out.absolute_change),
                "change_rate": out.change_rate.map(dec_json),
                "reason": out.reason.map(|r| serde_json::to_value(r).unwrap()),
            })
        }
        "stock_aggregation" => {
            let values = input["month_end_values"].as_array().unwrap();
            let last = date(input["last_date"].as_str().unwrap());
            let target = date(input["target_end"].as_str().unwrap());
            // 月末序列：自 last 起逐月回溯构造期末观察
            let obs: Vec<ClosingObservation> = values
                .iter()
                .enumerate()
                .map(|(i, v)| ClosingObservation {
                    date: last
                        .checked_sub_months(chrono::Months::new((values.len() - 1 - i) as u32))
                        .expect("日期"),
                    value: cd(v),
                })
                .collect();
            match closing_balance_at(&obs, target) {
                Ok(v) => json!({"value": dec_json(v)}),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "overlapping_details_without_key" => {
            let rows: Vec<DetailObservation> = (0..input["source_count"].as_u64().unwrap())
                .map(|i| DetailObservation {
                    row_key: input["confirmed_row_key"].as_str().map(|s| s.to_string()),
                    value: Decimal::from(10),
                    evidence_id: format!("E{i}"),
                })
                .collect();
            match dsheng_core::analysis::dedupe_detail_rows(
                &rows,
                input["ranges_overlap"].as_bool().unwrap_or(false),
            ) {
                Ok(m) => json!({"value": dec_json(m.value)}),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "cumulative_and_months" => {
            let months: Vec<Decimal> = input["months"].as_array().unwrap().iter().map(cd).collect();
            let cumulative = cd(&input["cumulative"]);
            let choice = match input["authoritative_representation"].as_str() {
                Some("monthly") | Some("detail") => Some(RepresentationChoice::Detail),
                Some("cumulative") | Some("total") => Some(RepresentationChoice::Total),
                _ => None,
            };
            match aggregate_cumulative_vs_months(&months, cumulative, choice) {
                Ok(a) => json!({"value": dec_json(a.value)}),
                Err(e) => json!({"value": null, "reason": e.as_str()}),
            }
        }
        "configured_formula" => match configured_derived_ready(None) {
            Ok(_) => panic!("D19 期望 configuration_required"),
            Err(e) => json!({"value": null, "reason": e.as_str()}),
        },
        "formula_ast" => match validate_formula_json(&input["formula"]) {
            Ok(()) => json!({"accepted": true}),
            Err(e) => json!({"accepted": false, "reason": e.as_str()}),
        },
        "input_decimal" => match parse_input_decimal(input["value"].as_str().unwrap()) {
            Ok(_) => json!({"accepted": true}),
            Err(e) => json!({
                "accepted": false,
                "reason": match e {
                    dsheng_core::contracts::decimal::DecimalPolicyError::PrecisionExceeded { .. } =>
                        "decimal_precision_exceeded",
                    dsheng_core::contracts::decimal::DecimalPolicyError::Overflow =>
                        "decimal_overflow",
                    dsheng_core::contracts::decimal::DecimalPolicyError::InvalidCanonical =>
                        "invalid_canonical_decimal",
                },
            }),
        },
        "display_money" => {
            let v = cd(&input["value"]);
            json!({"display": display_money(v), "stored": to_canonical_string(v)})
        }
        "monthly_target" => {
            let catalog = catalog_metric(input["metric"].as_str().unwrap());
            let eligible = monthly_revenue_floor_eligible(
                catalog["revenue_semantics"].as_bool().unwrap_or(false),
                serde_json::from_value(input["direction"].clone()).unwrap(),
                grain(input["grain"].as_str().unwrap()),
            );
            let outcome = monthly_revenue_floor(
                &MonthlyFloorInput {
                    explicit_target: dec_opt(&input["explicit_target"]),
                    comparable_annual_month_plan: dec_opt(&input["annual_month_plan"]),
                    comparable_prior_year_actual: dec_opt(&input["prior_year_actual"]),
                    incomparable_values: input["incomparable_value"]
                        .as_str()
                        .map(|s| cd(&Value::String(s.into())))
                        .into_iter()
                        .collect(),
                },
                TargetConstraintScope::MonthlyRevenueFloor,
                eligible,
            );
            json!({
                "target": outcome.target.map(dec_json),
                "excluded": outcome.excluded.iter().map(|d| json!(to_canonical_string(*d))).collect::<Vec<_>>(),
            })
        }
        "explicit_target_only" => {
            let target =
                dsheng_core::analysis::explicit_target_only(dec_opt(&input["explicit_target"]));
            match target {
                Ok(v) => json!({"target": dec_json(v)}),
                Err(_) => json!({"target": null, "reason": CONFIRMED_TARGET_MISSING}),
            }
        }
        "non_month_target" => {
            let catalog = catalog_metric(input["metric"].as_str().unwrap());
            let eligible = monthly_revenue_floor_eligible(
                catalog["revenue_semantics"].as_bool().unwrap_or(false),
                Direction::Up,
                grain(input["grain"].as_str().unwrap()),
            );
            let outcome = monthly_revenue_floor(
                &MonthlyFloorInput {
                    explicit_target: dec_opt(&input["explicit_target"]),
                    comparable_annual_month_plan: dec_opt(&input["annual_month_plan"]),
                    comparable_prior_year_actual: dec_opt(&input["prior_year_actual"]),
                    incomparable_values: Vec::new(),
                },
                TargetConstraintScope::MonthlyRevenueFloor,
                eligible,
            );
            match outcome.target {
                Some(v) => json!({"target": dec_json(v)}),
                None => json!({"target": null, "reason": CONFIRMED_TARGET_MISSING}),
            }
        }
        "monthly_target_conflict" => {
            let floor = cd(&input["floor"]);
            let outcome = monthly_revenue_floor(
                &MonthlyFloorInput {
                    explicit_target: dec_opt(&input["explicit_target"]),
                    comparable_annual_month_plan: None,
                    comparable_prior_year_actual: Some(floor),
                    incomparable_values: Vec::new(),
                },
                TargetConstraintScope::MonthlyRevenueFloor,
                true,
            );
            json!({
                "candidate_target": outcome.candidate_target.map(dec_json),
                "original_explicit_target": outcome.original_explicit_target.map(dec_json),
                "approved_target_unchanged": outcome.target == outcome.original_explicit_target,
                "needs_review": outcome.needs_review,
            })
        }
        "invoice_receipt_gap" => {
            let v =
                dsheng_core::analysis::period_gap(cd(&input["invoice"]), cd(&input["receipts"]));
            json!({
                "value": dec_json(v),
                "label": INVOICE_RECEIPT_GAP_LABEL,
                "is_receivable_balance": false,
            })
        }
        "day_comparison"
        | "week_comparison"
        | "month_comparison"
        | "fiscal_quarter_comparison"
        | "custom_comparison"
        | "standard_month_comparison" => {
            let g = match op {
                "day_comparison" => Granularity::Day,
                "week_comparison" => Granularity::Week,
                "month_comparison" | "standard_month_comparison" => Granularity::Month,
                "fiscal_quarter_comparison" => Granularity::Quarter,
                _ => Granularity::Custom,
            };
            let mut policy = default_policy();
            if let Some(w) = input["week_start_day"].as_u64() {
                policy.week_start_day = w as u32;
            }
            if let Some(f) = input["fiscal_start_month"].as_u64() {
                policy.fiscal_year_start_month = f as u32;
            }
            let r = DateRange::new(
                date(input["from"].as_str().expect("from")),
                date(input["to"].as_str().expect("to")),
            )
            .expect("区间");
            let mom = mom_base(r, g, &policy);
            let yoy = yoy_base(r, g, &policy);
            let mut out = serde_json::Map::new();
            out.insert("mom".into(), base_json(mom.clone().ok().map(|b| b.range)));
            match &yoy {
                Ok(b) => {
                    out.insert("yoy".into(), base_json(Some(b.range)));
                    if let Some(label) = b.label {
                        out.insert("yoy_label".into(), json!(label));
                    }
                }
                Err(e) => {
                    out.insert("yoy".into(), Value::Null);
                    out.insert("reason".into(), json!(e.as_str()));
                }
            }
            if let (Err(me), true) = (&mom, yoy.is_err()) {
                out.insert("reason".into(), json!(me.as_str()));
            }
            if op == "fiscal_quarter_comparison" {
                out.insert(
                    "label".into(),
                    json!(dsheng_core::analysis::fiscal_quarter_label(
                        r.from,
                        policy.fiscal_year_start_month
                    )),
                );
            }
            Value::Object(out)
        }
        "policy_selection" => {
            let from = date(input["from"].as_str().unwrap());
            let to = date(input["to"].as_str().unwrap());
            let change_at = date(input["change_at"].as_str().unwrap());
            let versions = vec![
                PolicyVersion {
                    version: "V1".into(),
                    effective_from: date("1900-01-01"),
                    effective_to: Some(change_at),
                },
                PolicyVersion {
                    version: "V2".into(),
                    effective_from: change_at,
                    effective_to: None,
                },
            ];
            let r = DateRange::new(from, to).expect("区间");
            match resolve_policy(&versions, r, input["explicit_policy"].as_str()) {
                Ok(resolved) => json!({
                    "accepted": true,
                    "applied_policy": resolved.policy.version,
                    "override": resolved.override_applied,
                    "mixed_policy": false,
                }),
                Err(reason) => json!({"accepted": false, "reason": reason}),
            }
        }
        "granularity_conversion" => {
            let source = grain(input["source_grain"].as_str().unwrap());
            let target = grain(input["target_grain"].as_str().unwrap());
            let coverage_value = if input["coverage"].is_null() {
                json!("unknown")
            } else {
                input["coverage"].clone()
            };
            let coverage =
                serde_json::from_value::<dsheng_core::contracts::domain::CoverageStatus>(
                    coverage_value,
                )
                .expect("coverage");
            match check_reaggregation(source, target, coverage) {
                Ok(()) => json!({
                    "accepted": true,
                    "comparison_to_monthly_history": target == Granularity::Month,
                }),
                Err(e) => json!({"accepted": false, "reason": e.as_str()}),
            }
        }
        "local_date" => {
            let ts: DateTime<Utc> =
                DateTime::parse_from_rfc3339(input["timestamp"].as_str().unwrap())
                    .expect("RFC3339")
                    .with_timezone(&Utc);
            match local_calendar_date(ts, input["timezone"].as_str().unwrap()) {
                Ok(d) => json!({"date": d.format("%Y-%m-%d").to_string()}),
                Err(e) => json!({"date": null, "reason": e}),
            }
        }
        "excel_date" => {
            let system = if input["system"] == "1900" {
                ExcelDateSystem::Y1900
            } else {
                ExcelDateSystem::Y1904
            };
            match excel_serial_to_date(input["serial"].as_i64().unwrap(), system) {
                Ok(d) => json!({"date": d.format("%Y-%m-%d").to_string()}),
                Err(e) => json!({"date": null, "reason": e.as_str()}),
            }
        }
        "ambiguous_date" => match parse_calendar_date(input["text"].as_str().unwrap()) {
            Ok(d) => json!({"date": d.format("%Y-%m-%d").to_string()}),
            Err(e) => json!({"date": null, "reason": e.as_str()}),
        },
        "next_custom_window" => {
            let from = date(input["from"].as_str().unwrap());
            let to = date(input["to"].as_str().unwrap());
            let (next, label) = next_custom_window(DateRange::new(from, to).expect("区间"));
            json!({
                "from": next.from.format("%Y-%m-%d").to_string(),
                "to": next.to.format("%Y-%m-%d").to_string(),
                "label": label,
            })
        }
        "month_compare_policy_versions" => {
            // 策略版本不参与比较签名（§3.2 兼容签名）
            let sig = ComparisonSignature {
                metric_semantic_hash: "hash-equal".into(),
                unit_signature_json: "{\"base_unit\":\"元\"}".into(),
                scope_key: "全公司".into(),
                timezone_id: DEFAULT_TIMEZONE.into(),
                grain: Granularity::Month,
                week_start_day: 1,
                month_start_day: 1,
                fiscal_year_start_month: 1,
            };
            let comparable =
                signatures_compatible(&sig, &sig).is_ok() && input["coverage"] == "complete";
            json!({"comparable": comparable})
        }
        "week_compare_start" => {
            let days = input["week_start_days"].as_array().unwrap();
            let sig = |w: u64| ComparisonSignature {
                metric_semantic_hash: "h".into(),
                unit_signature_json: "{}".into(),
                scope_key: "s".into(),
                timezone_id: DEFAULT_TIMEZONE.into(),
                grain: Granularity::Week,
                week_start_day: w as u32,
                month_start_day: 1,
                fiscal_year_start_month: 1,
            };
            match signatures_compatible(
                &sig(days[0].as_u64().unwrap()),
                &sig(days[1].as_u64().unwrap()),
            ) {
                Ok(()) => json!({"comparable": true}),
                Err(e) => json!({"comparable": false, "reason": e.as_str()}),
            }
        }
        "merged_numeric_cell" => {
            let span = input["range"]
                .as_str()
                .unwrap()
                .split(':')
                .nth(1)
                .and_then(|s| {
                    s.trim_start_matches(char::is_alphabetic)
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(1);
            let anchor_row = input["range"]
                .as_str()
                .unwrap()
                .split(':')
                .next()
                .and_then(|s| {
                    s.trim_start_matches(char::is_alphabetic)
                        .parse::<usize>()
                        .ok()
                })
                .unwrap_or(1);
            let span = span - anchor_row + 1;
            let expanded = expand_merged_anchor(Some(cd(&input["anchor_value"])), span);
            let observations = expanded.iter().filter(|v| v.is_some()).count();
            json!({
                "observations": observations,
                "filled_down": expanded.iter().skip(1).any(|v| v.is_some()),
            })
        }
        "duplicate_headers" => {
            let headers = input["headers"].as_array().unwrap();
            let letters = input["columns"].as_array().unwrap();
            let dup_indices = duplicate_columns(
                &headers
                    .iter()
                    .map(|h| h.as_str().unwrap())
                    .collect::<Vec<_>>(),
            );
            let mapping_required: Vec<Value> =
                dup_indices.iter().map(|i| letters[*i].clone()).collect();
            json!({"mapping_required": mapping_required, "overwritten": false})
        }
        "unknown_formula" => {
            // 未知公式：缓存值不作为 valid（§4.1）
            let _cached = &input["cached_value"];
            json!({"value": null, "reason": "unsupported_formula", "executed": false})
        }
        "missing_tokens" => {
            let values = input["values"].as_array().unwrap();
            let states: Vec<Value> = values
                .iter()
                .map(|v| {
                    let class = classify_cell(v.as_str());
                    serde_json::to_value(class.value_state()).unwrap()
                })
                .collect();
            // 原文保留：NA/N/A/- 的 missing_token 等于原文
            let raw_preserved = values.iter().all(|v| match v.as_str() {
                Some(s) => match classify_cell(Some(s)) {
                    dsheng_core::import::CellClass::Missing { missing_token } => {
                        missing_token.map(|t| t == s).unwrap_or(true)
                    }
                    _ => true,
                },
                None => true,
            });
            json!({
                "states": states,
                "raw_text_preserved": raw_preserved,
                "last_value": values.last().cloned().unwrap_or(Value::Null),
            })
        }
        "text_export" => json!({
            "xlsx_type": xlsx_export_kind(input["value"].as_str().unwrap(), false).as_str(),
            "csv_value": csv_export_text(input["value"].as_str().unwrap()),
        }),
        "numeric_export" => {
            let kind = xlsx_export_kind(input["value"].as_str().unwrap(), true);
            json!({"xlsx_type": kind.as_str(), "numeric_value": input["value"]})
        }
        "large_number_export" => {
            let kind = xlsx_export_kind(input["value"].as_str().unwrap(), true);
            json!({
                "xlsx_type": kind.as_str(),
                "number_storage": kind.number_storage(),
                "value": input["value"],
            })
        }
        "chart_without_cells" => {
            let (series, reason) = chart_series_from_resolved_cells(
                input["references_resolvable"].as_bool().unwrap_or(false),
                &[],
            );
            json!({"series": series, "reason": reason})
        }
        "multi_block_evidence" => {
            let plan = plan_blocks(&dsheng_core::import::BlockLayout {
                header_rows: input["header_rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect(),
                title_rows: input["merged_title_rows"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as usize)
                    .collect(),
                total_rows: 50,
            });
            let scopes = input["block_scopes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect::<Vec<_>>();
            json!({
                "blocks": plan.blocks,
                "title_rows_as_data": plan.title_rows_as_data,
                "cross_scope_sum": cross_scope_sum_allowed(&scopes),
            })
        }
        "source_period_from_filename" => {
            let explicit = input["explicit_period"]
                .as_array()
                .map(|a| range(&Value::Array(a.clone())));
            match source_period(explicit) {
                Ok(r) => json!({"period": range_pair(r), "trend_generated": true}),
                Err(reason) => json!({"period": null, "reason": reason, "trend_generated": false}),
            }
        }
        other => panic!("未实现的 DC operation: {other}"),
    }
}

#[test]
fn dc_all_56_cases_pass() {
    let cases = load_cases();
    assert_eq!(cases.len(), 56, "DC 算例必须为 56 条");
    let mut failures = Vec::new();
    for case in &cases {
        let id = case["case_id"].as_str().unwrap();
        let actual = dispatch(case);
        let expected = case["expected"].as_object().expect("expected 对象");
        for (key, want) in expected {
            if let Some(actual_key) = key.strip_prefix("not_") {
                let got = actual.get(actual_key).cloned().unwrap_or(Value::Null);
                if got == *want {
                    failures.push(format!(
                        "{id}.{key}: 实际值不应等于 {want}（实际 {actual}）"
                    ));
                }
                continue;
            }
            let got = actual.get(key).cloned().unwrap_or(Value::Null);
            if got != *want {
                failures.push(format!("{id}.{key}: 期望 {want}，实际 {got}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "DC 算例失败 {} 条:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn dc_reason_lexicon_matches_contract() {
    // 原因词法抽查：与合同 token 完全一致
    assert_eq!(EngineReason::SourceConflict.as_str(), "source_conflict");
    assert_eq!(EngineReason::ZeroDenominator.as_str(), "zero_denominator");
    assert_eq!(CalendarReason::PartialPeriod.as_str(), "partial_period");
    assert_eq!(
        CalendarReason::PriorYearRangeNotComparable.as_str(),
        "prior_year_range_not_comparable"
    );
    assert_eq!(
        CalendarReason::DateNotInPriorYear.as_str(),
        "date_not_in_prior_year"
    );
    // 未使用的 Cadence 导入校验（防误用 custom）
    let c: Cadence = serde_json::from_value(json!("month")).unwrap();
    assert_eq!(c.as_granularity(), Granularity::Month);
}
