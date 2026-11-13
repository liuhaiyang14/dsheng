//! 一键注册常用指标（目录子集＋真实科目全名别名；CNY/不含税口径确认值）。
//! 与主 Agent CLI 验证流（批次 2026-11-04）完全一致；幂等：已注册的指标跳过。

use super::DesktopEnv;
use serde_json::{json, Value};

const DEFAULTS: &str = r#"[
 {
  "metric_key": "sales_net",
  "label": "不含税销售额",
  "label_aliases": [
   "不含税销售额",
   "不含税订单金额",
   "不含税收入",
   "总销售额"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "monthly_revenue_floor",
  "revenue_semantics": true,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "receipts_amount",
  "label": "收款金额",
  "label_aliases": [
   "收款金额",
   "回款金额",
   "当月收款"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "payments_amount",
  "label": "付款金额",
  "label_aliases": [
   "付款金额",
   "当月付款（实付）",
   "当月付款(实付)"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "down",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "invoice_amount",
  "label": "开票金额",
  "label_aliases": [
   "开票金额",
   "当月销项开票"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "labor_cost",
  "label": "人工费",
  "label_aliases": [
   "人工费",
   "人工费用",
   "人工费（不可作为削减对象）",
   "人工费(不可作为削减对象)"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "down",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "settlement_income",
  "label": "结算收益",
  "label_aliases": [
   "结算收益",
   "结算收益（附加价值）",
   "结算收益(附加价值)"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "value_added_per_hour",
  "label": "单位时间附加值",
  "label_aliases": [
   "单位时间附加值",
   "单位时间附加值（元/小时）",
   "单位时间附加值(元/小时)"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "money_per_time",
   "base_unit": "元/时",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "order_cost",
  "label": "订单成本",
  "label_aliases": [
   "订单成本",
   "采购成本",
   "当月采购签约额（备货）"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "down",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "order_gross_profit",
  "label": "订单毛利",
  "label_aliases": [
   "订单毛利",
   "毛利额"
  ],
  "semantic_type": "configured_derived",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "configured_recompute",
  "formula": {"op":"sub","args":[{"op":"ref","metric_key":"sales_net"},{"op":"ref","metric_key":"order_cost"}]},
  "required_inputs": ["sales_net", "order_cost"],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "operating_expense",
  "label": "经营费用",
  "label_aliases": [
   "经营费用",
   "经营费用合计"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "down",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "value_added",
  "label": "附加值（扣人工前）",
  "label_aliases": ["附加值", "附加值（扣人工前）", "结算收益（附加价值）"],
  "semantic_type": "configured_derived",
  "unit_signature": {"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "configured_recompute",
  "formula": {"op":"sub","args":[{"op":"ref","metric_key":"order_gross_profit"},{"op":"ref","metric_key":"operating_expense"}]},
  "required_inputs": ["order_gross_profit", "operating_expense"],
  "comparison_policy": {"positive_base":"relative_change","zero_or_negative_base":"absolute_only"}
 },
 {
  "metric_key": "operating_income",
  "label": "经营收益（扣人工后）",
  "label_aliases": ["经营收益", "经营收益（扣人工后）", "经营收益（税前）"],
  "semantic_type": "configured_derived",
  "unit_signature": {"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "configured_recompute",
  "formula": {"op":"sub","args":[{"op":"ref","metric_key":"value_added"},{"op":"ref","metric_key":"labor_cost"}]},
  "required_inputs": ["value_added", "labor_cost"],
  "comparison_policy": {"positive_base":"relative_change","zero_or_negative_base":"absolute_only"}
 },
 {
  "metric_key": "order_gross_margin",
  "label": "订单毛利率",
  "label_aliases": [
   "订单毛利率",
   "毛利率"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "ratio",
   "base_unit": "1",
   "currency": null,
   "tax_basis": null
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "settlement_margin",
  "label": "结算收益率",
  "label_aliases": [
   "结算收益率",
   "利润率"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "ratio",
   "base_unit": "1",
   "currency": null,
   "tax_basis": null
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "expense_ratio",
  "label": "费用构成比",
  "label_aliases": [
   "费用构成比"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "ratio",
   "base_unit": "1",
   "currency": null,
   "tax_basis": null
  },
  "direction": "down",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "plan_delta",
  "label": "预定实绩差异额",
  "label_aliases": [
   "预定实绩差异额"
  ],
  "semantic_type": "flow",
  "unit_signature": {
   "dimension": "money",
   "base_unit": "元",
   "currency": "CNY",
   "tax_basis": "exclusive"
  },
  "direction": "neutral",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "sum_disjoint",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "plan_achievement_ratio",
  "label": "实绩达成预定比",
  "label_aliases": [
   "实绩达成预定比"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "ratio",
   "base_unit": "1",
   "currency": null,
   "tax_basis": null
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 },
 {
  "metric_key": "annual_plan_achievement_ratio",
  "label": "实绩达成年度计划比",
  "label_aliases": [
   "实绩达成年度计划比"
  ],
  "semantic_type": "ratio",
  "unit_signature": {
   "dimension": "ratio",
   "base_unit": "1",
   "currency": null,
   "tax_basis": null
  },
  "direction": "up",
  "target_constraint_scope": "explicit_target_only",
  "revenue_semantics": false,
  "aggregation": "ratio_of_aggregated_inputs",
  "formula": null,
  "required_inputs": [],
  "comparison_policy": {
   "positive_base": "relative_change",
   "zero_or_negative_base": "absolute_only"
  }
 }
]"#;

pub fn register_defaults(env: &DesktopEnv, workspace_id: String) -> Value {
    let defs: Vec<Value> = serde_json::from_str(DEFAULTS).unwrap_or_default();
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let mut registered: Vec<Value> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut failed: Vec<Value> = Vec::new();
    let db = match env.db_arg() {
        Ok(p) => p,
        Err(e) => return json!({"schema_version":"1","ok":false,"error":{"code":"DATA_DIRECTORY_UNAVAILABLE","message":e.to_string(),"field":null,"details":null}}),
    };
    let Ok(mut conn) = dsheng_cli::ctx::open_db_at(std::path::Path::new(&db)) else {
        return json!({"schema_version":"1","ok":false,"error":{"code":"LOCAL_SAVE_FAILED","message":"无法打开数据库","field":null,"details":null}});
    };
    let ws: uuid::Uuid = match workspace_id.parse() {
        Ok(w) => w,
        Err(_) => return json!({"schema_version":"1","ok":false,"error":{"code":"INVALID_ARGUMENT","message":"workspace_id 非法","field":null,"details":null}}),
    };
    for def in defs {
        let input = dsheng_cli::store::sources::MetricDefinitionSet {
            metric_key: def["metric_key"].as_str().unwrap_or_default().to_string(),
            label: def["label"].as_str().unwrap_or_default().to_string(),
            label_aliases: def.get("label_aliases").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()),
            semantic_type: def["semantic_type"].as_str().unwrap_or_default().to_string(),
            unit_signature: def["unit_signature"].clone(),
            direction: def["direction"].as_str().unwrap_or_default().to_string(),
            target_constraint_scope: def["target_constraint_scope"].as_str().unwrap_or_default().to_string(),
            revenue_semantics: def["revenue_semantics"].as_bool().unwrap_or(false),
            aggregation: def["aggregation"].as_str().unwrap_or_default().to_string(),
            formula: def.get("formula").filter(|v| !v.is_null()).cloned(),
            required_inputs: def.get("required_inputs").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default(),
            denominator_policy: "positive_only".into(),
            comparison_policy: def["comparison_policy"].clone(),
            effective_from: today.clone(),
            labor_treatment: None,
            scope_key: None,
            expected_revision: 0,
        };
        match dsheng_cli::store::sources::metric_set(&mut conn, ws, input) {
            Ok(v) => registered.push(v),
            Err(e) => {
                let code = format!("{:?}", e.code);
                if code.contains("RevisionConflict") {
                    skipped.push(def["metric_key"].as_str().unwrap_or_default().to_string());
                } else {
                    failed.push(json!({"metric_key": def["metric_key"], "error": e.to_string()}));
                }
            }
        }
    }
    json!({
        "schema_version": "1", "ok": failed.is_empty(),
        "data": {"registered": registered.len(), "skipped_already_present": skipped, "failed": failed},
        "error": null
    })
}
