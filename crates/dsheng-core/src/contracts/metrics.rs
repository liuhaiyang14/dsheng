//! 指标注册与有限公式 AST（data-period-import.md §1.2 / §1.4 / metric-catalog）。

use crate::contracts::decimal::canonical_decimal;
use crate::contracts::domain::{Direction, UnitSignature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// 公式 AST 上限（metric-catalog formula_ast）。
pub const AST_MAX_NODES: usize = 64;
pub const AST_MAX_DEPTH: usize = 16;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FormulaError {
    #[error("节点数超过 {AST_MAX_NODES}")]
    TooManyNodes,
    #[error("深度超过 {AST_MAX_DEPTH}")]
    TooDeep,
    #[error("未知引用: {0}")]
    UnknownReference(String),
    #[error("检测到依赖环: {0}")]
    Cycle(String),
    #[error("非法 const: {0}")]
    InvalidConst(String),
    #[error("单位不匹配: {0}")]
    UnitMismatch(String),
}

/// 仅 ref/const/add/sub/mul/div，二元运算，args 恰 2 项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FormulaNode {
    Ref { metric_key: String },
    Const { value: String }, // 规范十进制字符串、无单位
    Add { args: Box<[FormulaNode; 2]> },
    Sub { args: Box<[FormulaNode; 2]> },
    Mul { args: Box<[FormulaNode; 2]> },
    Div { args: Box<[FormulaNode; 2]> },
}

impl FormulaNode {
    /// 计算节点数与最大深度。
    pub fn size_and_depth(&self) -> (usize, usize) {
        match self {
            FormulaNode::Ref { .. } | FormulaNode::Const { .. } => (1, 1),
            FormulaNode::Add { args }
            | FormulaNode::Sub { args }
            | FormulaNode::Mul { args }
            | FormulaNode::Div { args } => {
                let (s1, d1) = args[0].size_and_depth();
                let (s2, d2) = args[1].size_and_depth();
                (1 + s1 + s2, 1 + d1.max(d2))
            }
        }
    }

    /// 收集全部 ref 的 metric_key。
    pub fn collect_refs<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            FormulaNode::Ref { metric_key } => out.push(metric_key),
            FormulaNode::Const { .. } => {}
            FormulaNode::Add { args }
            | FormulaNode::Sub { args }
            | FormulaNode::Mul { args }
            | FormulaNode::Div { args } => {
                args[0].collect_refs(out);
                args[1].collect_refs(out);
            }
        }
    }

    /// 结构校验：节点/深度限制、const 规范性、引用存在。
    /// 单位一致性（add/sub 同单位、最终等于声明单位）由分析工作包在
    /// 具备单位推导时执行；此处只做合同结构部分。
    pub fn validate(
        &self,
        known_metric_keys: &std::collections::HashSet<&str>,
    ) -> Result<(), FormulaError> {
        let (nodes, depth) = self.size_and_depth();
        if nodes > AST_MAX_NODES {
            return Err(FormulaError::TooManyNodes);
        }
        if depth > AST_MAX_DEPTH {
            return Err(FormulaError::TooDeep);
        }
        self.validate_inner(known_metric_keys)
    }

    fn validate_inner(&self, known: &std::collections::HashSet<&str>) -> Result<(), FormulaError> {
        match self {
            FormulaNode::Ref { metric_key } => {
                if known.contains(metric_key.as_str()) {
                    Ok(())
                } else {
                    Err(FormulaError::UnknownReference(metric_key.clone()))
                }
            }
            FormulaNode::Const { value } => canonical_decimal(value)
                .map(|_| ())
                .map_err(|_| FormulaError::InvalidConst(value.clone())),
            FormulaNode::Add { args }
            | FormulaNode::Sub { args }
            | FormulaNode::Mul { args }
            | FormulaNode::Div { args } => {
                args[0].validate_inner(known)?;
                args[1].validate_inner(known)
            }
        }
    }
}

/// 聚合方式枚举（§1.4 表）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregation {
    SumDisjoint,
    ConfiguredRecompute,
    RecomputeAfterAggregation,
    RatioOfAggregatedInputs,
    OpeningBoundary,
    ClosingBalanceOrRollforward,
    ClosingBoundary,
    SumItemClosingValues,
}

/// 指标语义类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticType {
    Flow,
    Stock,
    Ratio,
    ConfiguredDerived,
}

/// 目标约束范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetConstraintScope {
    MonthlyRevenueFloor,
    ExplicitTargetOnly,
    None,
}

/// 分母政策（合同固定 positive_only）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenominatorPolicy {
    PositiveOnly,
}

/// 比较政策（两字段 required）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComparisonPolicy {
    pub positive_base: PositiveBasePolicy,
    pub zero_or_negative_base: ZeroOrNegativeBasePolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PositiveBasePolicy {
    RelativeChange,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZeroOrNegativeBasePolicy {
    AbsoluteOnly,
}

/// 劳动费用处理（本地冻结定义的哈希材料；无此配置为 null）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaborTreatment {
    IncludedInExpense,
    SeparateFromExpense,
}

/// 模型 definition 投影（§1.4 投影表，全部字段）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDefinitionProjection {
    pub metric_id: uuid::Uuid,
    pub metric_key: String,
    pub version: u32,
    pub semantic_hash: String,
    pub label: String,
    pub semantic_type: SemanticType,
    pub unit_signature: UnitSignature,
    pub direction: Direction,
    pub target_constraint_scope: TargetConstraintScope,
    pub revenue_semantics: bool,
    pub aggregation: Aggregation,
    pub formula: Option<FormulaNode>,
    pub required_inputs: Vec<String>,
    pub denominator_policy: DenominatorPolicy,
    pub comparison_policy: ComparisonPolicy,
    pub effective_from: chrono::NaiveDate,
    pub scope_key: String,
}

/// semantic hash 材料键序（canonical JSON，SHA-256）。
/// 不含身份 ID、version、label、effective_from、scope_key（§1.4）。
fn semantic_hash_material(
    def: &MetricDefinitionProjection,
    labor_treatment: Option<LaborTreatment>,
    dependency_semantic_hashes: &BTreeMap<String, String>,
) -> serde_json::Value {
    let labor = match labor_treatment {
        Some(LaborTreatment::IncludedInExpense) => serde_json::json!("included_in_expense"),
        Some(LaborTreatment::SeparateFromExpense) => serde_json::json!("separate_from_expense"),
        None => serde_json::Value::Null,
    };
    serde_json::json!({
        "aggregation": def.aggregation,
        "comparison_policy": def.comparison_policy,
        "dependency_semantic_hashes": dependency_semantic_hashes,
        "denominator_policy": def.denominator_policy,
        "direction": def.direction,
        "formula": def.formula,
        "labor_treatment": labor,
        "required_inputs": def.required_inputs,
        "revenue_semantics": def.revenue_semantics,
        "semantic_type": def.semantic_type,
        "target_constraint_scope": def.target_constraint_scope,
        "unit_signature": {
            "base_unit": def.unit_signature.base_unit,
            "currency": def.unit_signature.currency,
            "dimension": def.unit_signature.dimension,
            "tax_basis": def.unit_signature.tax_basis,
        },
    })
}

/// canonical JSON：对象键按 Unicode 码点升序（serde_json Map 需 BTree 排序序列化）。
pub fn canonical_json_sha256(value: &serde_json::Value) -> String {
    let canonical = to_canonical_json(value);
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn to_canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = String::from("{");
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(k).expect("key"));
                out.push(':');
                out.push_str(&to_canonical_json(&map[*k]));
            }
            out.push('}');
            out
        }
        serde_json::Value::Array(items) => {
            let mut out = String::from("[");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&to_canonical_json(item));
            }
            out.push(']');
            out
        }
        // Number 在 arbitrary_precision 下保留词法；字符串转义走 serde
        other => serde_json::to_string(other).expect("scalar"),
    }
}

/// 计算 semantic hash（自叶向根的依赖 hash 由调用方按拓扑序提供）。
pub fn compute_semantic_hash(
    def: &MetricDefinitionProjection,
    labor_treatment: Option<LaborTreatment>,
    dependency_semantic_hashes: &BTreeMap<String, String>,
) -> String {
    canonical_json_sha256(&semantic_hash_material(
        def,
        labor_treatment,
        dependency_semantic_hashes,
    ))
}

/// monthly_revenue_floor 资格：收入语义且 direction=up（§1.2/§1.4）。
pub fn monthly_revenue_floor_eligible(def: &MetricDefinitionProjection) -> bool {
    def.revenue_semantics && def.direction == Direction::Up
}

/// 依赖闭环检查：直接 AST 引用 ∪ required_inputs（§1.4）。
pub fn direct_dependencies(def: &MetricDefinitionProjection) -> Vec<String> {
    let mut refs = Vec::new();
    if let Some(f) = &def.formula {
        let mut r = Vec::new();
        f.collect_refs(&mut r);
        refs.extend(r.into_iter().map(|s| s.to_string()));
    }
    refs.extend(def.required_inputs.iter().cloned());
    let mut seen = std::collections::BTreeSet::new();
    refs.retain(|k| seen.insert(k.clone()));
    refs
}

/// 目录单位：允许合同占位值，本地注册时替换（§1.4：占位必须在发送前替换成实际确认值）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogUnit {
    pub dimension: crate::contracts::domain::UnitDimension,
    pub base_unit: String,
    #[serde(default)]
    pub currency: Option<String>,
    #[serde(default)]
    pub tax_basis: Option<String>,
}

/// 目录中的占位标记。
pub const PLACEHOLDER_PREFIX: &str = "workspace_confirmed";
pub const PLACEHOLDER_TAX_EXCLUSIVE: &str = "exclusive_confirmed";
pub const PLACEHOLDER_TAX_PER_METRIC: &str = "explicit_per_metric";
pub const PLACEHOLDER_EFFECTIVE_FROM: &str = "workspace_initial_confirmation_date";

impl CatalogUnit {
    pub fn has_placeholder(&self) -> bool {
        self.base_unit.starts_with(PLACEHOLDER_PREFIX)
            || self.currency.as_deref() == Some(PLACEHOLDER_PREFIX)
            || matches!(
                self.tax_basis.as_deref(),
                Some(PLACEHOLDER_TAX_EXCLUSIVE) | Some(PLACEHOLDER_TAX_PER_METRIC)
            )
    }

    /// 用工作区确认值替换占位，产出有效 UnitSignature。
    pub fn materialize(
        &self,
        currency: Option<String>,
        tax_basis: Option<crate::contracts::domain::TaxBasis>,
        item_base_unit: Option<String>,
    ) -> Result<UnitSignature, &'static str> {
        let base_unit = if self.base_unit.starts_with(PLACEHOLDER_PREFIX) {
            item_base_unit.ok_or("item_base_unit_required")?
        } else {
            self.base_unit.clone()
        };
        let sig = UnitSignature {
            dimension: self.dimension,
            base_unit,
            currency: if self.currency.as_deref() == Some(PLACEHOLDER_PREFIX) {
                currency
            } else {
                self.currency.clone()
            },
            tax_basis: match self.tax_basis.as_deref() {
                Some(PLACEHOLDER_TAX_EXCLUSIVE) | Some(PLACEHOLDER_TAX_PER_METRIC) => tax_basis,
                other => other.and_then(|s| match s {
                    "inclusive" => Some(crate::contracts::domain::TaxBasis::Inclusive),
                    "exclusive" => Some(crate::contracts::domain::TaxBasis::Exclusive),
                    "not_applicable" => Some(crate::contracts::domain::TaxBasis::NotApplicable),
                    _ => None,
                }),
            },
        };
        sig.validate()?;
        Ok(sig)
    }
}

/// 目录条目（contracts/v1/metric-catalog.json 的 metrics[]）。
/// catalog 是注册模板；占位值（workspace_confirmed 等）在本地注册时替换。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricCatalogEntry {
    pub metric_key: String,
    pub label: String,
    #[serde(default)]
    pub label_aliases: Vec<String>,
    pub semantic_type: SemanticType,
    pub unit: CatalogUnit,
    pub direction: Direction,
    pub target_constraint_scope: TargetConstraintScope,
    pub aggregation: Aggregation,
    pub formula: Option<FormulaNode>,
    #[serde(default)]
    pub required_inputs: Vec<String>,
    pub denominator_policy: DenominatorPolicy,
    pub comparison_policy: ComparisonPolicy,
    pub version: u32,
    #[serde(default)]
    pub configuration_required: bool,
    #[serde(default)]
    pub notes: String,
    /// 目录内为 `workspace_initial_confirmation_date` 占位；注册时替换为实际日期。
    pub effective_from: String,
    #[serde(default)]
    pub revenue_semantics: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known() -> std::collections::HashSet<&'static str> {
        ["a", "b"].into_iter().collect()
    }

    #[test]
    fn ast_limits_and_refs() {
        let ok = FormulaNode::Add {
            args: Box::new([
                FormulaNode::Ref {
                    metric_key: "a".into(),
                },
                FormulaNode::Const {
                    value: "1.5".into(),
                },
            ]),
        };
        assert!(ok.validate(&known()).is_ok());
        let bad_ref = FormulaNode::Ref {
            metric_key: "zzz".into(),
        };
        assert!(matches!(
            bad_ref.validate(&known()),
            Err(FormulaError::UnknownReference(_))
        ));
        let bad_const = FormulaNode::Const {
            value: "1.50".into(),
        };
        assert!(matches!(
            bad_const.validate(&known()),
            Err(FormulaError::InvalidConst(_))
        ));
        // 深度 17：左倾链
        let mut node = FormulaNode::Ref {
            metric_key: "a".into(),
        };
        for _ in 0..16 {
            node = FormulaNode::Add {
                args: Box::new([node, FormulaNode::Const { value: "1".into() }]),
            };
        }
        assert!(matches!(
            node.validate(&known()),
            Err(FormulaError::TooDeep)
        ));
    }

    #[test]
    fn canonical_json_sorted_keys() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"b":1,"a":{"d":null,"c":[1,2]}}"#).unwrap();
        assert_eq!(to_canonical_json(&v), r#"{"a":{"c":[1,2],"d":null},"b":1}"#);
    }
}
