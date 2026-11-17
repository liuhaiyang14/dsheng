//! Dated pending source rows must never leak into other periods or partial totals.
use dsheng_cli::store::{analysis, settings, sources};
use rusqlite::Connection;
use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    dir: String,
    conn: Connection,
    workspace: Uuid,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let mut conn = Connection::open(temp.path().join("audit.sqlite3")).unwrap();
        dsheng_core::repository::apply_migrations(&mut conn, "profile-isolation-test").unwrap();
        let created = settings::apply_settings_operation(&mut conn, &serde_json::from_value(json!({
            "operation":"workspace_create", "name":"来源期间隔离",
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
        Self {
            _temp: temp,
            dir: source.to_str().unwrap().into(),
            conn,
            workspace,
        }
    }

    fn sales(&self, suffix: &str, date: &str, amount: &str, status: Option<&str>) -> String {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("销售出库").unwrap();
        for (col, text) in ["data_id", "出库时间", "出库产品售价总额(含税)/元"]
            .iter()
            .enumerate()
        {
            sheet.write_string(0, col as u16, *text).unwrap();
        }
        for (col, text) in [suffix, date, amount].iter().enumerate() {
            sheet.write_string(1, col as u16, *text).unwrap();
        }
        if let Some(status) = status {
            sheet.write_string(0, 3, "产品出库确认").unwrap();
            sheet.write_string(1, 3, status).unwrap();
        }
        let name = format!("销售出库{suffix}.xlsx");
        book.save(std::path::Path::new(&self.dir).join(&name))
            .unwrap();
        name
    }

    fn scan(&mut self) -> Value {
        sources::scan_summary_desktop_with_profile(
            &mut self.conn,
            self.workspace,
            &self.dir,
            &sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap(),
        )
        .unwrap()
    }

    fn confirm(&mut self, tax_confirmed: &[&str]) {
        let scan = self.scan();
        let entries = scan["files"].as_array().unwrap().iter().map(|file| {
            let name = file["file_label"].as_str().unwrap();
            let taxes = if tax_confirmed.contains(&name) {
                json!({"sales_net":{"basis":"inclusive","rate_percent":"13"}})
            } else { json!({}) };
            serde_json::from_value(json!({
                "tool_field":"source", "source_label":name, "source_id":name, "source":name,
                "recognized":"已确认", "usage":"用途已确认", "target_role":"profile_mapping", "tax_treatments":taxes
            })).unwrap()
        }).collect();
        sources::source_mapping_set(
            &self.conn,
            self.workspace,
            &self.dir,
            scan["source_profile"]["hash"].as_str().unwrap(),
            entries,
        )
        .unwrap();
    }

    fn analyze(&mut self, from: &str, to: &str, grain: &str) -> Value {
        analysis::run_analyze_with_profile(
            &mut self.conn,
            self.workspace,
            &self.dir,
            from,
            to,
            false,
            Some(grain.into()),
            None,
            false,
            None,
            sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap(),
        )
        .unwrap()
    }
}

fn sales(result: &Value) -> &Value {
    result["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["metric_key"] == "sales_net")
        .unwrap()
}

#[test]
fn required_status_column_is_reported_and_never_treated_as_approved() {
    for status in [None, Some(""), Some("作废")] {
        let mut fixture = Fixture::new();
        let name = fixture.sales("审核", "2026-09-12", "113", status);
        let scan = fixture.scan();
        assert_eq!(scan["available_periods"], json!([]), "status={status:?}");
        let review = scan["files"][0]["review"]["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["metric_key"] == "sales_net")
            .unwrap();
        if status.is_none() {
            assert!(review["missing_fields"]
                .as_array()
                .unwrap()
                .contains(&json!("产品出库确认")));
        }
        fixture.confirm(&[&name]);
        let result = fixture.analyze("2026-09-01", "2026-09-30", "month");
        assert!(
            sales(&result)["actual_value"].is_null(),
            "status={status:?}"
        );
        if status != Some("作废") {
            assert!(sales(&result)["pending_reasons"]
                .as_array()
                .unwrap()
                .contains(&json!("status_missing_or_unconfirmed")));
        }
    }
}

#[test]
fn missing_required_status_cannot_leave_a_partial_month_total() {
    let mut fixture = Fixture::new();
    let approved = fixture.sales("已审核", "2026-09-12", "113", Some("确认"));
    let missing = fixture.sales("无审核列", "2026-09-13", "226", None);
    fixture.confirm(&[&approved, &missing]);
    assert!(sales(&fixture.analyze("2026-09-01", "2026-09-30", "month"))["actual_value"].is_null());
}

#[test]
fn unrelated_month_tax_does_not_change_current_amount_or_old_revision() {
    let mut fixture = Fixture::new();
    let september = fixture.sales("九月", "2026-09-12", "113", Some("确认"));
    fixture.confirm(&[&september]);
    let original = fixture.analyze("2026-09-01", "2026-09-30", "month");
    assert_eq!(sales(&original)["actual_value"], "100");
    fixture.sales("十月待确认", "2026-10-12", "226", Some("确认"));
    fixture.confirm(&[&september]);
    let current = fixture.analyze("2026-09-01", "2026-09-30", "month");
    assert_eq!(sales(&current)["actual_value"], "100");
    assert!(!sales(&current)["pending_reasons"]
        .as_array()
        .unwrap()
        .contains(&json!("configuration_required_tax_basis")));
    let old = analysis::analysis_run_view(
        &fixture.conn,
        fixture.workspace,
        original["run_id"].as_str().unwrap(),
        1,
    )
    .unwrap();
    assert_eq!(sales(&old)["actual_value"], "100");
    assert!(sales(&fixture.analyze("2026-10-01", "2026-10-31", "month"))["actual_value"].is_null());
}

#[test]
fn tax_pending_rows_block_only_overlapping_totals_including_comparison_bases() {
    let mut fixture = Fixture::new();
    let current = fixture.sales("本期", "2026-09-12", "113", Some("确认"));
    let mom = fixture.sales("环比已确认", "2026-08-12", "113", Some("确认"));
    let yoy = fixture.sales("同比已确认", "2025-09-12", "113", Some("确认"));
    let mom_pending = fixture.sales("环比待确认", "2026-08-13", "226", Some("确认"));
    let yoy_pending = fixture.sales("同比待确认", "2025-09-13", "226", Some("确认"));
    fixture.confirm(&[&current, &mom, &yoy]);
    let result = fixture.analyze("2026-09-01", "2026-09-30", "month");
    assert_eq!(sales(&result)["actual_value"], "100");
    for comparison in ["mom", "yoy"] {
        assert!(
            sales(&result)[comparison]["base_value"].is_null(),
            "{comparison} cannot use partial base"
        );
        assert!(sales(&result)[comparison]["absolute_change"].is_null());
    }
    assert!(sales(&fixture.analyze("2026-08-01", "2026-08-31", "month"))["actual_value"].is_null());
    // A custom window on the confirmed row stays valid; including the next day's
    // pending row must invalidate the whole sum rather than silently return 100.
    assert_eq!(
        sales(&fixture.analyze("2026-08-12", "2026-08-12", "custom"))["actual_value"],
        "100"
    );
    assert!(
        sales(&fixture.analyze("2026-08-12", "2026-08-13", "custom"))["actual_value"].is_null()
    );
    fixture.confirm(&[&current, &mom, &yoy, &mom_pending, &yoy_pending]);
    let complete = fixture.analyze("2026-09-01", "2026-09-30", "month");
    for comparison in ["mom", "yoy"] {
        assert_eq!(sales(&complete)[comparison]["base_value"], "300");
    }
}

#[test]
fn pending_tax_on_the_same_day_keeps_both_total_and_trend_bucket_missing() {
    let mut fixture = Fixture::new();
    let approved = fixture.sales("已确认同日", "2026-09-12", "113", Some("确认"));
    fixture.sales("待确认同日", "2026-09-12", "226", Some("确认"));
    fixture.confirm(&[&approved]);
    let result = fixture.analyze("2026-09-01", "2026-09-30", "month");
    assert!(sales(&result)["actual_value"].is_null());
    let trend = result["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["metric_key"] == "sales_net")
        .unwrap();
    assert_eq!(trend["actual"], json!([null]));
}
