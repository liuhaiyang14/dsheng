//! 用户确认的来源用途与金额口径。规则来自版本化模板，确认随映射写入 revision 指纹。
use super::sources::{FieldMappingEntry, ScannedFile, SourceProfileBinding};
use crate::error::{CliError, CliResult};
use dsheng_core::import::headers::normalize_header;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaxTreatment {
    pub basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_percent: Option<String>,
}

impl TaxTreatment {
    pub(super) fn equivalent(&self, other: &Self) -> bool {
        self.basis == other.basis
            && (self.basis == "exclusive" || self.rate().ok() == other.rate().ok())
    }

    pub fn validate(&self) -> CliResult<()> {
        match self.basis.as_str() {
            "exclusive" if self.rate_percent.is_none() => Ok(()),
            "inclusive" => self.rate().map(|_| ()),
            _ => Err(CliError::invalid(
                "请选择金额是否含税；不含税金额无需填写税率",
            )),
        }
    }

    fn rate(&self) -> CliResult<Decimal> {
        let rate = self
            .rate_percent
            .as_deref()
            .and_then(|raw| Decimal::from_str_exact(raw.trim()).ok())
            .filter(|rate| *rate >= Decimal::ZERO && *rate <= Decimal::from(100));
        rate.ok_or_else(|| {
            CliError::invalid("含税金额须填写明确的税率（0 到 100）；不确定或混合税率请保留待确认")
        })
    }
}

pub fn confirmation<'a>(
    entries: Option<&'a [FieldMappingEntry]>,
    file: &ScannedFile,
) -> Option<&'a FieldMappingEntry> {
    entries?.iter().find(|entry| confirms_file(entry, file))
}

fn confirms_file(entry: &FieldMappingEntry, file: &ScannedFile) -> bool {
    entry.source_label == file.file_label
        || entry.source.as_deref() == Some(file.file_label.as_str())
        || entry.source_id.as_deref() == Some(file.file_label.as_str())
}

pub fn reference_only(entries: &[FieldMappingEntry], file: &ScannedFile) -> bool {
    confirmation(Some(entries), file)
        .is_some_and(|entry| entry.target_role.as_deref() == Some("reference_only"))
}

pub fn matches(mapping: &Value, file_label: &str, sheet_name: &str) -> bool {
    [
        ("file_matchers", file_label),
        ("sheet_matchers", sheet_name),
    ]
    .into_iter()
    .all(|(key, label)| {
        mapping
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.is_empty()
                    || items
                        .iter()
                        .filter_map(Value::as_str)
                        .any(|matcher| label.to_lowercase().contains(&matcher.to_lowercase()))
            })
    })
}

pub fn tax_basis<'a>(profile: &'a SourceProfileBinding, mapping: &'a Value) -> &'a str {
    mapping
        .get("source_tax_basis")
        .and_then(Value::as_str)
        .or_else(|| {
            profile
                .config
                .as_ref()?
                .get("normalization")?
                .get("tax_basis")?
                .as_str()
        })
        .unwrap_or("confirm_at_runtime")
}

/// 所有可进入净额公式的输入均显式转换；空税率不是 0%，原始现金流不换算。
pub fn normalized_amount(
    profile: &SourceProfileBinding,
    mapping: &Value,
    confirmation: Option<&FieldMappingEntry>,
    value: Decimal,
) -> Option<Decimal> {
    let source_basis = tax_basis(profile, mapping);
    match source_basis {
        "exclusive" | "not_applicable" => Some(value),
        "inclusive" | "confirm_at_runtime" => {
            let key = mapping.get("metric_key")?.as_str()?;
            let treatment = confirmation?.tax_treatments.as_ref()?.get(key)?;
            match treatment.basis.as_str() {
                "exclusive" if source_basis == "confirm_at_runtime" => Some(value),
                "inclusive" => {
                    value.checked_div(Decimal::ONE + treatment.rate().ok()? / Decimal::from(100))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// 桌面扩展视图，CLI 冻结合同保持原形。列名来自模板和实际表头，不推断企业税率。
pub fn file_review(
    file: &ScannedFile,
    profile: &SourceProfileBinding,
    entries: Option<&[FieldMappingEntry]>,
) -> Value {
    let mut mappings = Vec::new();
    if let Some(config) = &profile.config {
        let max_header = config["row_rules"]["header_rows_max"]
            .as_u64()
            .unwrap_or(20) as usize;
        for (data, sheet) in &file.sheets {
            if sheet.blocked_reason.is_some() {
                continue;
            }
            for mapping in config["record_mappings"].as_array().into_iter().flatten() {
                if !matches(mapping, &file.file_label, &data.name) {
                    continue;
                }
                let fields = |key: &str| -> Vec<String> {
                    mapping[key]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                };
                let values = fields("value_fields");
                let dates = fields("date_fields");
                let required_status = mapping["accepted_statuses"]
                    .as_array()
                    .filter(|statuses| !statuses.is_empty())
                    .and_then(|_| mapping["status_field"].as_str());
                let missing: Vec<&str> = values
                    .iter()
                    .chain(dates.iter())
                    .map(String::as_str)
                    .chain(required_status)
                    .filter(|field| {
                        !data.rows.iter().take(max_header).flatten().any(|cell| {
                            normalize_header(&cell.raw_text()) == normalize_header(field)
                        })
                    })
                    .collect();
                mappings.push(json!({"metric_key": mapping["metric_key"], "value_fields": values,
                    "date_fields": dates, "missing_fields": missing, "source_tax_basis": tax_basis(profile, mapping),
                    "requires_confirmation": mapping["requires_confirmation"].as_bool().unwrap_or(false),
                    "status_field": mapping["status_field"],
                    "accepted_statuses": mapping["accepted_statuses"].as_array().cloned().unwrap_or_default()}));
            }
        }
    }
    let kind = if profile.mode != "profile"
        || (mappings.is_empty() && file.sheets.iter().any(|(_, s)| s.header.is_some()))
    {
        "generic"
    } else if mappings.is_empty() {
        "auxiliary"
    } else {
        "records"
    };
    let column_overrides: Vec<Value> = entries
        .into_iter()
        .flatten()
        .filter(|entry| confirms_file(entry, file))
        .filter_map(|entry| {
            let (role, column) = entry.column_override()?;
            Some(json!({"role": role, "column": column.to_ascii_uppercase(), "sheet": entry.sheet}))
        })
        .collect();
    json!({"kind": kind, "mappings": mappings, "confirmation": confirmation(entries, file),
        "column_overrides": column_overrides})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn missing_tax_rate_is_not_zero_and_explicit_rate_converts_only_its_metric() {
        let profile =
            super::super::sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap();
        let mapping = json!({"metric_key":"sales_net", "source_tax_basis":"inclusive"});
        let mut entry: FieldMappingEntry = serde_json::from_value(json!({
            "tool_field":"source", "source_label":"销售出库.xlsx", "recognized":"已确认", "usage":"销售数据"
        })).unwrap();
        assert_eq!(
            normalized_amount(&profile, &mapping, Some(&entry), Decimal::from(113)),
            None
        );
        let mut treatment = TaxTreatment {
            basis: "inclusive".into(),
            rate_percent: Some("".into()),
        };
        assert!(treatment.validate().is_err());
        treatment.rate_percent = Some("13".into());
        entry.tax_treatments = Some(BTreeMap::from([("sales_net".into(), treatment.clone())]));
        assert_eq!(
            normalized_amount(&profile, &mapping, Some(&entry), Decimal::from(113)),
            Some(Decimal::from(100))
        );
        let cost = json!({"metric_key":"order_cost", "source_tax_basis":"confirm_at_runtime"});
        assert_eq!(
            normalized_amount(&profile, &cost, Some(&entry), Decimal::from(40)),
            None
        );
        let cash = json!({"metric_key":"payments_amount", "source_tax_basis":"not_applicable"});
        assert_eq!(
            normalized_amount(&profile, &cash, None, Decimal::from(113)),
            Some(Decimal::from(113))
        );
        treatment.rate_percent = Some("0".into());
        assert!(treatment.validate().is_ok());
        for invalid in ["-1", "101", "NaN", "13%"] {
            treatment.rate_percent = Some(invalid.into());
            assert!(treatment.validate().is_err(), "{invalid}");
        }
    }
}
