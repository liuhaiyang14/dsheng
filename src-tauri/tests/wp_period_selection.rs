//! Desktop-only period choices and validation, with temporary synthetic local files.
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;
use dsheng_app::contracts::{core::data, env::DesktopEnv};
use dsheng_cli::store::sources;
use fixture::Fixture;
use serde_json::{json, Value};

#[test]
fn desktop_scan_adds_real_period_choices_without_inventing_blank_days() {
    let f = Fixture::new("desktop-period-choices");
    let dir = tempfile::tempdir().unwrap();
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet().set_name("source").unwrap();
    for (col, value) in ["指标", "期间", "实际值"].iter().enumerate() {
        sheet.write_string(0, col as u16, *value).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 1, "2026-10-05").unwrap();
    sheet.write_string(1, 2, "5").unwrap();
    book.save(dir.path().join("合成每日.xlsx")).unwrap();
    let env = DesktopEnv::with_db(&f.db);
    let scan = data::scan(
        &env,
        f.workspace.to_string(),
        dir.path().to_string_lossy().into_owned(),
    );
    assert_eq!(scan["ok"], true, "{scan}");
    assert_eq!(
        scan["data"]["available_periods"],
        json!([{"from":"2026-10-01","to":"2026-10-31","grain":"month"}])
    );
}

#[test]
fn desktop_analyze_rejects_arbitrary_ranges_before_directory_scan() {
    let f = Fixture::new("desktop-period-guard");
    let env = DesktopEnv::with_db(&f.db);
    for (from, to, grain) in [
        ("2026-10-01", "2026-10-05", "month"),
        ("2026-10-05", "2026-10-05", "day"),
    ] {
        let result = data::analyze(
            &env,
            json!({"workspace_id":f.workspace,"directory_label":"/nonexistent-test-directory", "from":from,"to":to,"grain":grain}),
        );
        assert_eq!(result["ok"], false);
        assert_eq!(
            result["error"]["code"], "INVALID_ARGUMENT",
            "期间校验应先于目录读取: {result}"
        );
    }
}

#[test]
fn desktop_uses_current_calendar_for_historical_date_without_rewriting_old_run() {
    let f = Fixture::new("desktop-current-calendar");
    let dir = tempfile::tempdir().unwrap();
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let policy_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "UPDATE period_policies SET effective_to='2026-11-01' WHERE workspace_id=?1",
        [f.workspace.to_string()],
    )
    .unwrap();
    conn.execute("INSERT INTO period_policies (period_policy_id,workspace_id,version,cadence,effective_from,effective_to,week_start_day,fiscal_year_start_month,timezone_id,created_at) VALUES (?1,?2,2,'week','2026-11-01',NULL,7,1,'Asia/Shanghai','t')", rusqlite::params![policy_id, f.workspace.to_string()]).unwrap();
    conn.execute(
        "UPDATE workspaces SET current_period_policy_id=?2 WHERE workspace_id=?1",
        rusqlite::params![f.workspace.to_string(), policy_id],
    )
    .unwrap();
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet().set_name("source").unwrap();
    for (col, value) in ["指标", "期间", "实际值"].iter().enumerate() {
        sheet.write_string(0, col as u16, *value).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 1, "2026-10-01").unwrap();
    sheet.write_string(1, 2, "5").unwrap();
    book.save(dir.path().join("历史日期.xlsx")).unwrap();
    let env = DesktopEnv::with_db(&f.db);
    let scan = data::scan(
        &env,
        f.workspace.to_string(),
        dir.path().to_string_lossy().into_owned(),
    );
    assert_eq!(scan["ok"], true, "{scan}");
    assert_eq!(scan["data"]["available_period_policy_id"], policy_id);
    assert_eq!(
        scan["data"]["available_periods"],
        json!([{"from":"2026-09-27","to":"2026-10-03","grain":"week"}])
    );
    sources::source_mapping_set(
        &conn,
        f.workspace,
        &dir.path().to_string_lossy(),
        scan["data"]["source_profile"]["hash"].as_str().unwrap(),
        vec![],
    )
    .unwrap();
    let result = data::analyze(
        &env,
        json!({"workspace_id":f.workspace,"directory_label":dir.path().to_string_lossy(),"from":"2026-09-27","to":"2026-10-03","grain":"week"}),
    );
    assert_eq!(result["ok"], true, "{result}");
    let runs = dsheng_cli::store::analysis::history_list(&conn, f.workspace).unwrap();
    let new_run = runs
        .iter()
        .find(|run| run["run_id"] == result["data"]["run_id"])
        .unwrap();
    assert_eq!(new_run["policy_version"], 2);
    assert_eq!(new_run["cadence"], "week");
    let old_run = runs.iter().find(|run| run["run_id"] == f.run_id).unwrap();
    assert_eq!(old_run["policy_version"], 1);
    assert_eq!(old_run["cadence"], "month");
    assert_eq!(old_run["current_revision_no"], 1);
}

struct ManualPeriodFixture {
    _temp: tempfile::TempDir,
    env: DesktopEnv,
    directory: String,
    workspace: String,
}

impl ManualPeriodFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let env = DesktopEnv::with_db(temp.path().join("manual-period.sqlite3"));
        let directory = temp.path().join("source");
        std::fs::create_dir(&directory).unwrap();
        let created = dsheng_app::contracts::core::settings::workspace_create(&env, json!({
            "schema_version":"1", "operation":"workspace_create", "name":"手工列期间回归",
            "initial_period_policy":{"cadence":"month", "effective_from":"2024-01-01",
                "week_start_day":1, "month_start_day":1, "fiscal_year_start_month":1,
                "timezone_id":"Asia/Shanghai"}
        }));
        assert_eq!(created["ok"], true, "{created}");
        Self {
            _temp: temp, env, directory: directory.to_string_lossy().into_owned(),
            workspace: created["data"]["workspace"]["workspace_id"].as_str().unwrap().into(),
        }
    }

    fn write(&self, file: &str, sheets: &[(&str, [&str; 5])]) {
        self.write_with_headers(file, &["期间", "指标", "实际值", "复核日期", "复核金额"], sheets);
    }

    fn write_with_headers(&self, file: &str, headers: &[&str; 5], sheets: &[(&str, [&str; 5])]) {
        let mut book = rust_xlsxwriter::Workbook::new();
        for (name, values) in sheets {
            let sheet = book.add_worksheet().set_name(*name).unwrap();
            for (column, header) in headers.iter().enumerate() {
                sheet.write_string(0, column as u16, *header).unwrap();
            }
            for (column, value) in values.iter().enumerate() {
                sheet.write_string(1, column as u16, *value).unwrap();
            }
        }
        book.save(std::path::Path::new(&self.directory).join(file)).unwrap();
    }

    fn scan(&self) -> Value {
        let result = data::scan_with_profile(&self.env, self.workspace.clone(), self.directory.clone(), None, true);
        assert_eq!(result["ok"], true, "{result}");
        result["data"].clone()
    }

    fn confirm(&self, scan: &Value, mappings: Value) {
        let result = data::source_mapping_set(&self.env, self.workspace.clone(), self.directory.clone(),
            scan["source_profile"]["hash"].as_str().unwrap().into(), mappings);
        assert_eq!(result["ok"], true, "{result}");
    }

    fn save_choice(&self, choice: &Value, expected: &str) {
        let result = data::analyze(&self.env, json!({
            "workspace_id":self.workspace, "directory_label":self.directory,
            "from":choice["from"], "to":choice["to"], "grain":choice["grain"], "generic":true
        }));
        assert_eq!(result["ok"], true, "{result}");
        let sales = result["data"]["metrics"].as_array().unwrap().iter()
            .find(|metric| metric["metric_key"] == "sales_net").unwrap();
        assert_eq!(sales["actual_value"], expected, "{sales}");
        let history = dsheng_app::contracts::core::history::history_list(&self.env, self.workspace.clone());
        let run = history["data"]["runs"].as_array().unwrap().iter()
            .find(|run| run["run_id"] == result["data"]["run_id"]).unwrap();
        assert_eq!(run["period"], json!({"from":choice["from"], "to":choice["to"]}));
        assert!(run["summary"]["metric_count"].as_u64().unwrap() > 0);
    }
}

#[test]
fn generic_manual_date_rescan_exposes_correct_month_and_saves_it_without_changing_other_sources() {
    for locator in ["source_id", "source"] {
        let fixture = ManualPeriodFixture::new();
        fixture.write("corrected.xlsx", &[
            ("corrected", ["2026-09-12", "不含税销售额", "113", "2026-10-12", "999"]),
            ("unchanged", ["2026-11-12", "不含税销售额", "7", "2026-12-12", "999"]),
        ]);
        fixture.write("other.xlsx", &[
            ("corrected", ["2026-01-12", "不含税销售额", "5", "2026-02-12", "999"]),
        ]);
        let scan = fixture.scan();
        let mut correction = json!({"tool_field":"period", "source_label":"corrected.xlsx",
            "recognized":"D", "column":"D", "target_role":"period", "sheet":"corrected", "usage":"已核对归属日期"});
        correction[locator] = json!("corrected.xlsx");
        fixture.confirm(&scan, json!([correction,
            {"tool_field":"profile_mapping", "source_label":"other.xlsx", "source":"other.xlsx",
                "recognized":"已确认", "target_role":"profile_mapping", "usage":"保留自动列"}]));
        let rescanned = fixture.scan();
        assert_eq!(rescanned["available_periods"], json!([
            {"from":"2026-01-01","to":"2026-01-31","grain":"month"},
            {"from":"2026-10-01","to":"2026-10-31","grain":"month"},
            {"from":"2026-11-01","to":"2026-11-30","grain":"month"}
        ]), "locator={locator}");
        // Submit exactly the choices a desktop select receives, through the
        // Tauri command. Periods from the untouched sheet/file stay available.
        for (choice, amount) in rescanned["available_periods"].as_array().unwrap().iter().zip(["5", "113", "7"]) {
            fixture.save_choice(choice, amount);
        }
    }
}

#[test]
fn generic_manual_amount_controls_available_periods_without_falling_back_to_blank_columns() {
    let fixture = ManualPeriodFixture::new();
    fixture.write("source.xlsx", &[("source", ["2026-09-12", "不含税销售额", "", "", "217"])]);
    let scan = fixture.scan();
    assert_eq!(scan["available_periods"], json!([]));
    let mapping = |column: &str| json!([{"tool_field":"actual", "source_label":"source.xlsx", "source":"source.xlsx",
        "recognized":column, "column":column, "target_role":"actual", "usage":"已核对实际值"}]);
    fixture.confirm(&scan, mapping("E"));
    let rescanned = fixture.scan();
    assert_eq!(rescanned["available_periods"], json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}]));
    fixture.save_choice(&rescanned["available_periods"][0], "217");
    fixture.confirm(&rescanned, mapping("C"));
    assert_eq!(fixture.scan()["available_periods"], json!([]));
}

#[test]
fn generic_coverage_summary_uses_the_confirmed_date_column() {
    let fixture = ManualPeriodFixture::new();
    fixture.write("source.xlsx", &[("source", ["2026-09-12", "不含税销售额", "113", "2026-10-12", "217"])]);
    let scan = fixture.scan();
    fixture.confirm(&scan, json!([{"tool_field":"period", "source_label":"source.xlsx", "source":"source.xlsx",
        "recognized":"D", "column":"D", "target_role":"period", "usage":"已核对归属日期"}]));
    let rescanned = fixture.scan();
    assert_eq!(rescanned["coverage"]["from"], "2026-10-12");
    assert_eq!(rescanned["coverage"]["to"], "2026-10-12");
    assert_eq!(rescanned["available_periods"][0]["from"], "2026-10-01");
    fixture.save_choice(&rescanned["available_periods"][0], "113");
}

#[test]
fn generic_stale_header_confirmation_keeps_detected_dates_in_scan_and_analysis() {
    let fixture = ManualPeriodFixture::new();
    fixture.write("source.xlsx", &[("source", ["2026-09-12", "不含税销售额", "113", "2026-10-12", "217"])]);
    let scan = fixture.scan();
    fixture.confirm(&scan, json!([{"tool_field":"period", "source_label":"source.xlsx", "source":"source.xlsx",
        "recognized":"D", "column":"D", "target_role":"period", "header_signature":"obsolete-header",
        "usage":"旧表头确认"}]));
    let rescanned = fixture.scan();
    assert_eq!(rescanned["available_periods"], json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}]));
    fixture.save_choice(&rescanned["available_periods"][0], "113");
}

#[test]
fn sequential_generic_corrections_preserve_amount_date_and_other_worksheets() {
    let fixture = ManualPeriodFixture::new();
    fixture.write_with_headers("source.xlsx", &["期间", "指标", "实际值", "复核金额", "复核日期"], &[
        ("corrected", ["2026-09-12", "不含税销售额", "113", "217", "2026-10-12"]),
        ("other", ["2026-11-12", "不含税销售额", "7", "19", "2026-12-12"]),
    ]);
    let mapping = |role: &str, column: &str, sheet: &str| json!([{
        "source":"source.xlsx", "source_label":"source.xlsx", "tool_field":role,
        "target_role":role, "column":column, "recognized":column, "sheet":sheet, "usage":"已核对来源列"
    }]);
    let scan = fixture.scan();
    fixture.confirm(&scan, mapping("actual", "D", "corrected"));
    fixture.confirm(&fixture.scan(), mapping("period", "E", "corrected"));
    let corrected = fixture.scan();
    assert_eq!(corrected["available_periods"], json!([
        {"from":"2026-10-01","to":"2026-10-31","grain":"month"},
        {"from":"2026-11-01","to":"2026-11-30","grain":"month"}
    ]));
    assert_eq!(corrected["files"][0]["review"]["column_overrides"], json!([
        {"role":"actual","column":"D","sheet":"corrected"},
        {"role":"period","column":"E","sheet":"corrected"}
    ]));
    fixture.save_choice(&corrected["available_periods"][0], "217");
    fixture.save_choice(&corrected["available_periods"][1], "7");

    fixture.confirm(&corrected, mapping("actual", "D", "other"));
    fixture.confirm(&fixture.scan(), mapping("actual", "C", "corrected"));
    let revised = fixture.scan();
    assert_eq!(revised["available_periods"], corrected["available_periods"]);
    assert_eq!(revised["files"][0]["review"]["column_overrides"].as_array().unwrap().len(), 3);
    fixture.save_choice(&revised["available_periods"][0], "113");
    fixture.save_choice(&revised["available_periods"][1], "19");

    // Empty legacy confirmations must not revert either worksheet's columns.
    fixture.confirm(&revised, json!([]));
    fixture.confirm(&revised, json!([{"source":"source.xlsx", "source_label":"source.xlsx",
        "tool_field":"source", "recognized":"已确认", "usage":"旧客户端确认"}]));
    let legacy = fixture.scan();
    assert_eq!(legacy["files"][0]["review"]["column_overrides"], revised["files"][0]["review"]["column_overrides"]);
    fixture.save_choice(&legacy["available_periods"][0], "113");
    fixture.save_choice(&legacy["available_periods"][1], "19");
}
