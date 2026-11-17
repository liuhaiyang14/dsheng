//! A custom record adapter owns its matching sheet even when the sheet also
//! resembles a generic table or its record values are not yet usable.
use dsheng_cli::store::{analysis, settings, sources};
use rusqlite::Connection;
use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    conn: Connection,
    workspace: Uuid,
    directory: String,
    profile: sources::SourceProfileBinding,
}

impl Fixture {
    fn new(record_metric: &str, value_field: &str, matcher: &str, tax_basis: &str) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let mut conn = Connection::open(temp.path().join("test.sqlite3")).unwrap();
        dsheng_core::repository::apply_migrations(&mut conn, "profile-record-routing-test")
            .unwrap();
        let created = settings::apply_settings_operation(&mut conn, &serde_json::from_value(json!({
            "operation":"workspace_create", "name":"自定义单据模板",
            "initial_period_policy":{"cadence":"month", "effective_from":"2024-01-01",
                "week_start_day":1,"month_start_day":1,"fiscal_year_start_month":1,"timezone_id":"Asia/Shanghai"}
        })).unwrap(), None).unwrap();
        let workspace = created["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        sources::metric_set(&mut conn, workspace, serde_json::from_value(json!({
            "metric_key":"sales_net", "label":"不含税销售额", "semantic_type":"flow",
            "unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},
            "direction":"neutral", "target_constraint_scope":"none", "aggregation":"sum_disjoint",
            "comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},
            "effective_from":"2024-01-01", "expected_revision":0
        })).unwrap()).unwrap();
        let mut config: Value = serde_json::from_str(include_str!(
            "../../../contracts/v1/source-profiles/xsd_trade_v1.json"
        ))
        .unwrap();
        config["profile_id"] = json!("custom_trade");
        config["normalization"]["tax_basis"] = json!(tax_basis);
        config["record_mappings"] = json!([{
            "mapping_id":"sales", "source_tax_basis":"inclusive", "file_matchers":[matcher],
            "sheet_matchers":["source"], "metric_key":record_metric,
            "value_fields":[value_field], "date_fields":["期间"]
        }]);
        let profile_path = temp.path().join("custom.json");
        std::fs::write(&profile_path, config.to_string()).unwrap();
        let profile =
            sources::resolve_source_profile(Some(profile_path.to_str().unwrap()), false).unwrap();
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("source").unwrap();
        // The last column deliberately has no amount. The generic actual value
        // must not act as a fallback for an incomplete record adapter.
        for (col, value) in ["期间", "指标", "实际值", "含税金额"].iter().enumerate() {
            sheet.write_string(0, col as u16, *value).unwrap();
        }
        for (col, value) in ["2026-09-12", "不含税销售额", "113"].iter().enumerate() {
            sheet.write_string(1, col as u16, *value).unwrap();
        }
        book.save(source.join("source.xlsx")).unwrap();
        Self {
            _temp: temp,
            conn,
            workspace,
            directory: source.to_str().unwrap().into(),
            profile,
        }
    }

    fn confirm(&self, taxes: Value) {
        sources::source_mapping_set(
            &self.conn,
            self.workspace,
            &self.directory,
            &self.profile.hash,
            vec![serde_json::from_value(json!({
                "tool_field":"source", "source_label":"source.xlsx", "source":"source.xlsx",
                "recognized":"已确认", "usage":"已确认文件用途", "target_role":"profile_mapping",
                "tax_treatments":taxes
            }))
            .unwrap()],
        )
        .unwrap();
    }

    fn confirm_column(&self, role: &str, column: &str) {
        sources::source_mapping_set(
            &self.conn,
            self.workspace,
            &self.directory,
            &self.profile.hash,
            vec![serde_json::from_value(json!({
                "tool_field":role, "source_label":"source.xlsx", "source":"source.xlsx",
                "recognized":column, "column":column, "usage":"已确认来源列", "target_role":role
            }))
            .unwrap()],
        )
        .unwrap();
    }

    fn write_source(&self, headers: &[&str], values: &[&str]) {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("source").unwrap();
        for (col, value) in headers.iter().enumerate() {
            sheet.write_string(0, col as u16, *value).unwrap();
        }
        for (col, value) in values.iter().enumerate() {
            sheet.write_string(1, col as u16, *value).unwrap();
        }
        book.save(std::path::Path::new(&self.directory).join("source.xlsx"))
            .unwrap();
    }

    fn analyze(&mut self) -> Value {
        analysis::run_analyze_with_profile(
            &mut self.conn,
            self.workspace,
            &self.directory,
            "2026-09-01",
            "2026-09-30",
            false,
            Some("month".into()),
            None,
            false,
            None,
            self.profile.clone(),
        )
        .unwrap()
    }
}

fn sales(result: &Value) -> &Value {
    &result["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|metric| metric["metric_key"] == "sales_net")
        .unwrap()["actual_value"]
}

#[test]
fn custom_record_tax_conversion_counts_the_source_once_and_requires_confirmation() {
    let mut fixture = Fixture::new("sales_net", "实际值", "source", "exclusive");
    fixture.confirm(json!({"sales_net":{"basis":"inclusive","rate_percent":"13"}}));
    assert_eq!(
        sales(&fixture.analyze()),
        "100",
        "113 / 1.13 must not also include the raw 113"
    );
    fixture.confirm(json!({}));
    assert!(
        sales(&fixture.analyze()).is_null(),
        "unconfirmed tax must stay missing"
    );
}

#[test]
fn incomplete_custom_record_mapping_cannot_fall_back_to_generic_values() {
    for (metric, field) in [
        ("unregistered_metric", "实际值"),
        ("sales_net", "缺少的金额列"),
        ("sales_net", "含税金额"),
    ] {
        let mut fixture = Fixture::new(metric, field, "source", "exclusive");
        fixture.confirm(json!({"sales_net":{"basis":"inclusive","rate_percent":"13"}}));
        assert!(
            sales(&fixture.analyze()).is_null(),
            "metric={metric}, field={field}"
        );
    }
}

#[test]
fn unmatched_generic_sheet_retains_its_existing_tax_rules() {
    let mut known = Fixture::new("sales_net", "实际值", "different_file", "exclusive");
    known.confirm(json!({}));
    assert_eq!(sales(&known.analyze()), "113");
    let mut pending = Fixture::new(
        "sales_net",
        "实际值",
        "different_file",
        "confirm_at_runtime",
    );
    pending.confirm(json!({}));
    assert!(sales(&pending.analyze()).is_null());
}

#[test]
fn built_in_profiles_without_record_adapters_keep_generic_extraction() {
    for profile in ["manufacturing_demo", "service_demo"] {
        let mut fixture = Fixture::new("sales_net", "实际值", "source", "exclusive");
        fixture.profile = sources::resolve_source_profile(Some(profile), false).unwrap();
        fixture.confirm(json!({}));
        assert_eq!(sales(&fixture.analyze()), "113", "profile={profile}");
    }
}

#[test]
fn single_column_confirmation_preserves_other_recognized_fields() {
    for profile in ["manufacturing_demo", "service_demo"] {
        let mut fixture = Fixture::new("sales_net", "实际值", "source", "exclusive");
        fixture.profile = sources::resolve_source_profile(Some(profile), false).unwrap();
        for (role, column) in [("period", "A"), ("actual", "C")] {
            fixture.confirm_column(role, column);
            assert_eq!(
                sales(&fixture.analyze()),
                "113",
                "profile={profile}, role={role}"
            );
        }
    }
}

#[test]
fn manual_columns_override_detected_amounts_and_dates() {
    let mut fixture = Fixture::new("sales_net", "实际值", "source", "exclusive");
    fixture.profile = sources::resolve_source_profile(Some("service_demo"), false).unwrap();
    fixture.write_source(
        &["期间", "指标", "实际值", "复核金额", "复核日期"],
        &["2026-09-12", "不含税销售额", "113", "217", "2026-10-12"],
    );
    fixture.confirm_column("actual", "D");
    assert_eq!(
        sales(&fixture.analyze()),
        "217",
        "manual amount replaces the detected actual column"
    );
    fixture.confirm_column("date", "E");
    assert!(
        sales(&fixture.analyze()).is_null(),
        "manual date moves the value outside September"
    );
}

#[test]
fn unrecognized_required_fields_and_explicit_blank_amounts_stay_missing() {
    let mut fixture = Fixture::new("sales_net", "实际值", "source", "exclusive");
    fixture.profile = sources::resolve_source_profile(Some("service_demo"), false).unwrap();
    fixture.confirm_column("actual", "D");
    assert!(
        sales(&fixture.analyze()).is_null(),
        "explicit blank column must not fall back to raw 113 or zero"
    );
    fixture.write_source(
        &["未识别日期", "指标", "实际值"],
        &["2026-09-12", "不含税销售额", "113"],
    );
    fixture.confirm_column("actual", "C");
    assert!(
        sales(&fixture.analyze()).is_null(),
        "missing date mapping must not assign a value to this month"
    );
}
