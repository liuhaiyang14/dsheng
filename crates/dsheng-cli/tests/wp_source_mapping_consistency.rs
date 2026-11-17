//! File-level tax decisions and record-adapter routing survive incremental edits.
//! Every source file and database in these tests is synthetic and temporary.
use dsheng_cli::store::{analysis, settings, sources};
use rusqlite::{params, Connection};
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
    fn new(record_amount: Option<&str>, record_date: Option<&str>) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let mut conn = Connection::open_in_memory().unwrap();
        dsheng_core::repository::apply_migrations(&mut conn, "source-mapping-consistency").unwrap();
        let created = settings::apply_settings_operation(&mut conn, &serde_json::from_value(json!({
            "operation":"workspace_create", "name":"映射一致性回归",
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
            "../../../contracts/v1/source-profiles/service_demo.json"
        ))
        .unwrap();
        config["profile_id"] = json!("source_mapping_consistency");
        config["normalization"]["tax_basis"] = json!("exclusive");
        config["record_mappings"] = json!([{
            "mapping_id":"sales", "source_tax_basis":"inclusive", "file_matchers":["source"],
            "sheet_matchers":["record"], "metric_key":"sales_net",
            "value_fields":["明细金额"], "date_fields":["明细日期"]
        }]);
        let path = temp.path().join("profile.json");
        std::fs::write(&path, config.to_string()).unwrap();
        let profile = sources::resolve_source_profile(Some(path.to_str().unwrap()), false).unwrap();
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("record").unwrap();
        for (col, header) in ["期间", "指标", "实际值", "明细日期", "明细金额"]
            .iter()
            .enumerate()
        {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (col, value) in ["2026-09-12", "不含税销售额", "113"].iter().enumerate() {
            sheet.write_string(1, col as u16, *value).unwrap();
        }
        if let Some(date) = record_date {
            sheet.write_string(1, 3, date).unwrap();
        }
        if let Some(amount) = record_amount {
            sheet.write_string(1, 4, amount).unwrap();
        }
        let sheet = book.add_worksheet().set_name("generic").unwrap();
        for (col, header) in ["期间", "指标", "实际值"].iter().enumerate() {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        for (col, value) in ["2026-11-12", "不含税销售额", "50"].iter().enumerate() {
            sheet.write_string(1, col as u16, *value).unwrap();
        }
        book.save(source.join("source.xlsx")).unwrap();
        Self {
            _temp: temp,
            conn,
            workspace,
            directory: source.to_string_lossy().into_owned(),
            profile,
        }
    }

    fn save(&self, entries: Value) {
        sources::source_mapping_set(
            &self.conn,
            self.workspace,
            &self.directory,
            &self.profile.hash,
            serde_json::from_value(entries).unwrap(),
        )
        .unwrap();
    }

    fn saved(&self) -> Value {
        serde_json::to_value(
            sources::source_mapping_confirmed(
                &self.conn,
                self.workspace,
                &self.directory,
                &self.profile.hash,
            )
            .unwrap()
            .unwrap(),
        )
        .unwrap()
    }

    fn scan(&mut self) -> Value {
        sources::scan_summary_desktop_with_profile(
            &mut self.conn,
            self.workspace,
            &self.directory,
            &self.profile,
        )
        .unwrap()
    }

    fn actual(&mut self, month: &str, last_day: &str) -> Value {
        let result = analysis::run_analyze_with_profile(
            &mut self.conn,
            self.workspace,
            &self.directory,
            &format!("2026-{month}-01"),
            &format!("2026-{month}-{last_day}"),
            false,
            Some("month".into()),
            None,
            false,
            None,
            self.profile.clone(),
        )
        .unwrap();
        result["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .find(|metric| metric["metric_key"] == "sales_net")
            .unwrap()["actual_value"]
            .clone()
    }
}

fn column(role: &str, column: &str, sheet: &str, taxes: Option<Value>) -> Value {
    let mut entry = json!({
        "tool_field":role, "source_label":"source.xlsx", "source":"source.xlsx",
        "recognized":column, "column":column, "sheet":sheet, "usage":"确认来源列", "target_role":role
    });
    if let Some(taxes) = taxes {
        entry["tax_treatments"] = taxes;
    }
    entry
}

fn taxes(rate: &str) -> Value {
    json!({"sales_net":{"basis":"inclusive","rate_percent":rate}})
}

#[test]
fn clearing_taxes_on_one_column_clears_all_saved_roles_and_changes_calculation() {
    let mut fixture = Fixture::new(Some("113"), Some("2026-10-12"));
    fixture.save(json!([column("actual", "C", "record", Some(taxes("13")))]));
    fixture.save(json!([column("period", "A", "record", None)]));
    fixture.save(json!([column("actual", "C", "generic", None)]));
    assert_eq!(fixture.actual("10", "31"), "100");
    fixture.save(json!([column("date", "D", "record", Some(json!({})))]));
    let saved = fixture.saved();
    assert_eq!(saved.as_array().unwrap().len(), 3);
    assert!(saved
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["tax_treatments"] == json!({})));
    assert!(fixture.actual("10", "31").is_null());
    assert_eq!(
        fixture.scan()["files"][0]["review"]["confirmation"]["tax_treatments"],
        json!({})
    );
    fixture.save(json!([column("actual", "E", "record", None)]));
    assert!(
        fixture.actual("10", "31").is_null(),
        "omission must retain the cleared decision"
    );
    fixture.save(json!([column("period", "D", "record", Some(taxes("13")))]));
    assert!(fixture
        .saved()
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["tax_treatments"] == taxes("13")));
    assert_eq!(fixture.actual("10", "31"), "100");
}

#[test]
fn contradictory_tax_decisions_in_one_save_fail_without_changing_saved_mappings() {
    let fixture = Fixture::new(Some("113"), Some("2026-10-12"));
    fixture.save(json!([column("actual", "C", "record", Some(taxes("13")))]));
    let before = fixture.saved();
    let result = sources::source_mapping_set(
        &fixture.conn,
        fixture.workspace,
        &fixture.directory,
        &fixture.profile.hash,
        serde_json::from_value(json!([
            column("actual", "E", "record", Some(taxes("13"))),
            column("period", "D", "record", Some(json!({})))
        ]))
        .unwrap(),
    );
    assert!(result.is_err());
    assert_eq!(fixture.saved(), before);
    // Equivalent decimal spellings are one decision, not a contradiction.
    fixture.save(json!([
        column("actual", "E", "record", Some(taxes("13"))),
        column("period", "D", "record", Some(taxes("13.0")))
    ]));
    let saved = fixture.saved();
    assert_eq!(saved[0]["tax_treatments"], saved[1]["tax_treatments"]);
    // A later explicit decision in the same request also updates earlier omissions.
    fixture.save(json!([
        column("actual", "E", "record", None),
        column("period", "D", "record", Some(json!({})))
    ]));
    assert!(fixture
        .saved()
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["tax_treatments"] == json!({})));
}

#[test]
fn omitted_taxes_recover_the_last_saved_file_decision_from_legacy_mixed_entries() {
    let fixture = Fixture::new(Some("113"), Some("2026-10-12"));
    fixture.save(json!([column("actual", "C", "record", Some(taxes("13")))]));
    let legacy = json!([
        column("actual", "C", "record", Some(taxes("13"))),
        column("period", "A", "record", Some(json!({})))
    ]);
    fixture
        .conn
        .execute(
            "UPDATE cli_source_mapping_sets SET mapping_json=?1 WHERE workspace_id=?2",
            params![legacy.to_string(), fixture.workspace.to_string()],
        )
        .unwrap();
    fixture.save(json!([column("actual", "E", "record", None)]));
    assert!(fixture
        .saved()
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry["tax_treatments"] == json!({})));
}

#[test]
fn record_periods_never_fall_back_to_generic_dates_but_other_sheets_still_contribute() {
    for (amount, date, has_record) in [
        (None, Some("2026-10-12"), false),
        (Some("113"), None, false),
        (Some("113"), Some("2026-10-12"), true),
    ] {
        let mut fixture = Fixture::new(amount, date);
        fixture.save(json!([column("actual", "E", "record", Some(taxes("13")))]));
        let scan = fixture.scan();
        let starts: Vec<_> = scan["available_periods"]
            .as_array()
            .unwrap()
            .iter()
            .map(|period| period["from"].as_str().unwrap())
            .collect();
        assert_eq!(
            starts,
            if has_record {
                vec!["2026-10-01", "2026-11-01"]
            } else {
                vec!["2026-11-01"]
            }
        );
        assert_eq!(
            scan["coverage"]["from"],
            if has_record {
                "2026-10-12"
            } else {
                "2026-11-12"
            }
        );
        assert_eq!(scan["coverage"]["to"], "2026-11-12");
        assert!(fixture.actual("09", "30").is_null());
        assert_eq!(
            fixture.actual("10", "31"),
            if has_record {
                json!("100")
            } else {
                Value::Null
            }
        );
        assert_eq!(fixture.actual("11", "30"), "50");
    }
}
