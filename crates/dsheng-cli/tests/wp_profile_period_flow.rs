//! Profile date discovery and extraction must agree, using real file readers and SQLite.
use dsheng_cli::store::{analysis, settings, sources};
use rusqlite::{params, Connection};
use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};
use std::path::Path;
use uuid::Uuid;

fn workspace(conn: &mut Connection) -> Uuid {
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")
        .unwrap();
    dsheng_core::repository::apply_migrations(conn, "profile-period-test").unwrap();
    let op = serde_json::from_value(json!({
        "operation": "workspace_create", "name": "期间流程回归",
        "initial_period_policy": {
            "cadence": "month", "effective_from": "2024-01-01",
            "week_start_day": 1, "month_start_day": 1,
            "fiscal_year_start_month": 1, "timezone_id": "Asia/Shanghai"
        }
    }))
    .unwrap();
    let created = settings::apply_settings_operation(conn, &op, None).unwrap();
    let workspace = created["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    for (key, label) in [
        ("labor_cost", "人工费"),
        ("payments_amount", "付款金额"),
        ("sales_net", "不含税销售额"),
        ("order_cost", "订单成本"),
    ] {
        sources::metric_set(conn, workspace, serde_json::from_value(json!({
            "metric_key": key, "label": label, "semantic_type": "flow",
            "unit_signature": {"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},
            "direction":"neutral", "target_constraint_scope":"none", "aggregation":"sum_disjoint",
            "comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},
            "effective_from":"2024-01-01", "expected_revision":0
        })).unwrap()).unwrap();
    }
    workspace
}

fn confirm_files(conn: &Connection, workspace: Uuid, dir: &str, scan: &Value) {
    let entries = scan["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            serde_json::from_value(json!({
                "tool_field":"source", "source_label":file["file_label"],
                "recognized":"已确认", "usage":"业务用途已确认",
                "source_id":file["file_id"], "source":file["file_label"],
                "target_role":"source"
            }))
            .unwrap()
        })
        .collect();
    sources::source_mapping_set(
        conn,
        workspace,
        dir,
        scan["source_profile"]["hash"].as_str().unwrap(),
        entries,
    )
    .unwrap();
}

fn scan(conn: &mut Connection, workspace: Uuid, dir: &str) -> Value {
    sources::scan_summary_desktop_with_profile(
        conn,
        workspace,
        dir,
        &sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap(),
    )
    .unwrap()
}

fn analyze(
    conn: &mut Connection,
    workspace: Uuid,
    dir: &str,
    from: &str,
    to: &str,
    grain: &str,
) -> Value {
    let policy =
        sources::validate_desktop_analysis_period(conn, workspace, from, to, grain).unwrap();
    analysis::run_analyze_with_profile(
        conn,
        workspace,
        dir,
        from,
        to,
        false,
        Some(grain.into()),
        Some(policy),
        false,
        None,
        sources::resolve_source_profile(Some("xsd_trade_v1"), false).unwrap(),
    )
    .unwrap()
}

fn actual<'a>(result: &'a Value, key: &str) -> &'a Value {
    &result["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric_key"] == key)
        .unwrap()["actual_value"]
}

fn salary_file(dir: &Path) {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("工资条").unwrap();
    for (col, header) in ["data_id", "薪资归属年份", "薪资归属月份", "当月实发"]
        .iter()
        .enumerate()
    {
        sheet.write_string(0, col as u16, *header).unwrap();
    }
    for (col, value) in ["pay-1", "2026", "9", "8000"].iter().enumerate() {
        sheet.write_string(1, col as u16, *value).unwrap();
    }
    book.save(dir.join("工资条.xlsx")).unwrap();
}

#[test]
fn profile_payroll_month_cannot_be_relabelled_as_a_day_or_week() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    salary_file(&source);
    let dir = source.to_str().unwrap();
    let mut conn = Connection::open(temp.path().join("test.sqlite3")).unwrap();
    let workspace = workspace(&mut conn);
    let scanned = scan(&mut conn, workspace, dir);
    assert_eq!(
        scanned["available_periods"],
        json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}])
    );
    confirm_files(&conn, workspace, dir, &scanned);
    let month = analyze(
        &mut conn,
        workspace,
        dir,
        "2026-09-01",
        "2026-09-30",
        "month",
    );
    assert_eq!(actual(&month, "labor_cost"), "8000");
    assert_eq!(analysis::history_list(&conn, workspace).unwrap().len(), 1);
    for grain in ["day", "week"] {
        conn.execute(
            "UPDATE period_policies SET cadence = ?2 WHERE workspace_id = ?1",
            params![workspace.to_string(), grain],
        )
        .unwrap();
        assert_eq!(
            scan(&mut conn, workspace, dir)["available_periods"],
            json!([]),
            "whole payroll month must not become a {grain}"
        );
    }
    conn.execute(
        "UPDATE period_policies SET cadence = 'day' WHERE workspace_id = ?1",
        params![workspace.to_string()],
    )
    .unwrap();
    let day = analyze(&mut conn, workspace, dir, "2026-09-01", "2026-09-01", "day");
    assert!(
        actual(&day, "labor_cost").is_null(),
        "monthly salary cannot be attributed to the first day"
    );
}

#[test]
fn profile_timestamp_scan_and_analysis_agree_and_save_history() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("付款单").unwrap();
    for (col, header) in ["data_id", "付款时间", "付款金额/元"].iter().enumerate() {
        sheet.write_string(0, col as u16, *header).unwrap();
    }
    for (row, (id, date, amount)) in [
        ("a", "2026-09-12 13:45:00", "12"),
        ("b", "2026-09-30T23:59:00", "23"),
    ]
    .iter()
    .enumerate()
    {
        for (col, value) in [id, date, amount].iter().enumerate() {
            sheet
                .write_string(row as u32 + 1, col as u16, **value)
                .unwrap();
        }
    }
    book.save(source.join("付款单.xlsx")).unwrap();
    let dir = source.to_str().unwrap();
    let mut conn = Connection::open(temp.path().join("test.sqlite3")).unwrap();
    let workspace = workspace(&mut conn);
    let generic = sources::scan_summary_desktop(&mut conn, workspace, dir).unwrap();
    assert_eq!(
        generic["available_periods"],
        json!([]),
        "unknown detail fields must not create runnable periods"
    );
    let scanned = scan(&mut conn, workspace, dir);
    assert_eq!(
        scanned["available_periods"],
        json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}])
    );
    confirm_files(&conn, workspace, dir, &scanned);
    let result = analyze(
        &mut conn,
        workspace,
        dir,
        "2026-09-01",
        "2026-09-30",
        "month",
    );
    assert_eq!(actual(&result, "payments_amount"), "35");
    let history = analysis::history_list(&conn, workspace).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["run_id"], result["run_id"]);
    assert_eq!(
        history[0]["period"],
        json!({"from":"2026-09-01","to":"2026-09-30"})
    );
}

/// Opt-in because the user's files must remain local and are not repository fixtures.
#[test]
#[ignore = "set DSHENG_SOURCE_FIXTURE_DIR to a read-only sample directory"]
fn real_source_scan_confirm_monthly_analysis_persists_history() {
    let dir = std::env::var("DSHENG_SOURCE_FIXTURE_DIR").expect("DSHENG_SOURCE_FIXTURE_DIR");
    let temp = tempfile::tempdir().unwrap();
    let mut conn = Connection::open(temp.path().join("sample.sqlite3")).unwrap();
    let workspace = workspace(&mut conn);
    let generic = sources::scan_summary_desktop(&mut conn, workspace, &dir).unwrap();
    assert_eq!(generic["available_periods"], json!([]));
    let scanned = scan(&mut conn, workspace, &dir);
    let available = scanned["available_periods"].as_array().unwrap();
    assert!(available
        .iter()
        .any(|p| p["from"] == "2026-09-01" && p["to"] == "2026-09-30"));
    confirm_files(&conn, workspace, &dir, &scanned);
    let confirmed = scan(&mut conn, workspace, &dir);
    assert!(confirmed["files"]
        .as_array()
        .unwrap()
        .iter()
        .all(|file| file["mapping_status"] == "mapped"));
    let result = analyze(
        &mut conn,
        workspace,
        &dir,
        "2026-09-01",
        "2026-09-30",
        "month",
    );
    // Independently cross-checked against the read-only sample Excel files:
    // September payroll has 6 values; September payments has 41 values.
    assert_eq!(actual(&result, "labor_cost"), "52839");
    assert_eq!(actual(&result, "payments_amount"), "226492.32");
    assert!(
        actual(&result, "sales_net").is_null(),
        "unconfirmed gross sales must not become net revenue"
    );
    drop(conn);
    let conn = Connection::open(temp.path().join("sample.sqlite3")).unwrap();
    let history = analysis::history_list(&conn, workspace).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["run_id"], result["run_id"]);
    assert_eq!(
        history[0]["period"],
        json!({"from":"2026-09-01","to":"2026-09-30"})
    );
    sources::require_two_tables_materialized(
        &conn,
        workspace,
        result["run_id"].as_str().unwrap(),
        1,
    )
    .unwrap();
    println!(
        "real-source: files={}, periods={}, payroll={}, payments={}, history=1, two_tables=ready",
        scanned["files"].as_array().unwrap().len(),
        available.len(),
        actual(&result, "labor_cost"),
        actual(&result, "payments_amount")
    );
}

#[test]
fn profile_gross_sales_require_explicit_tax_and_preserve_old_revision() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("销售出库").unwrap();
    for (col, header) in [
        "data_id",
        "出库时间",
        "出库产品售价总额(含税)/元",
        "出库产品成本总额/元",
        "产品出库确认",
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(0, col as u16, *header).unwrap();
    }
    for (col, value) in ["sale-1", "2026-09-12 13:45:00", "113", "40", "确认"]
        .iter()
        .enumerate()
    {
        sheet.write_string(1, col as u16, *value).unwrap();
    }
    book.save(source.join("销售出库.xlsx")).unwrap();
    let dir = source.to_str().unwrap();
    let mut conn = Connection::open(temp.path().join("test.sqlite3")).unwrap();
    let workspace = workspace(&mut conn);
    let scanned = scan(&mut conn, workspace, dir);
    confirm_files(&conn, workspace, dir, &scanned);
    let pending = analyze(
        &mut conn,
        workspace,
        dir,
        "2026-09-01",
        "2026-09-30",
        "month",
    );
    assert!(actual(&pending, "sales_net").is_null());
    assert!(actual(&pending, "order_cost").is_null());
    let file = &scanned["files"][0];
    let entry = serde_json::from_value(json!({
        "tool_field":"sales_net", "source_label":file["file_label"], "recognized":"已确认", "usage":"销售出库",
        "source_id":file["file_id"], "source":file["file_label"], "target_role":"sales_net",
        "tax_treatments": {"sales_net":{"basis":"inclusive", "rate_percent":"13"}, "order_cost":{"basis":"exclusive"}}
    })).unwrap();
    sources::source_mapping_set(
        &conn,
        workspace,
        dir,
        scanned["source_profile"]["hash"].as_str().unwrap(),
        vec![entry],
    )
    .unwrap();
    let confirmed = analyze(
        &mut conn,
        workspace,
        dir,
        "2026-09-01",
        "2026-09-30",
        "month",
    );
    assert_eq!(actual(&confirmed, "sales_net"), "100");
    assert_eq!(actual(&confirmed, "order_cost"), "40");
    assert_eq!(confirmed["run_id"], pending["run_id"]);
    let old = analysis::analysis_run_view(&conn, workspace, pending["run_id"].as_str().unwrap(), 1)
        .unwrap();
    assert!(actual(&old, "sales_net").is_null());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM run_revisions WHERE run_id = ?1",
            params![pending["run_id"].as_str().unwrap()],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
}

#[test]
fn profile_excel_serial_dates_use_workbook_epoch_and_ignore_bare_times() {
    use rust_xlsxwriter::{ExcelDateTime, Format};
    use std::io::{Cursor, Read, Write};
    for is_1904 in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let mut book = Workbook::new();
        let sheet = book.add_worksheet().set_name("付款单").unwrap();
        for (col, header) in ["data_id", "付款时间", "付款金额/元"].iter().enumerate() {
            sheet.write_string(0, col as u16, *header).unwrap();
        }
        let date = ExcelDateTime::from_ymd(2026, 9, 12)
            .unwrap()
            .and_hms(13, 45, 0)
            .unwrap();
        let serial = date.to_excel() - if is_1904 { 1462.0 } else { 0.0 };
        let format = Format::new().set_num_format("yyyy-mm-dd hh:mm:ss");
        sheet.write_string(1, 0, "dated").unwrap();
        sheet
            .write_number_with_format(1, 1, serial, &format)
            .unwrap();
        sheet.write_number(1, 2, 35).unwrap();
        sheet.write_string(2, 0, "time-only").unwrap();
        sheet
            .write_number_with_format(2, 1, 0.5, &Format::new().set_num_format("hh:mm:ss"))
            .unwrap();
        sheet.write_number(2, 2, 999).unwrap();
        sheet.write_string(3, 0, "duration").unwrap();
        sheet
            .write_number_with_format(3, 1, 2.5, &Format::new().set_num_format("[h]:mm:ss"))
            .unwrap();
        sheet.write_number(3, 2, 999).unwrap();
        let bytes = book.save_to_buffer().unwrap();
        let file = source.join("付款单.xlsx");
        if is_1904 {
            // rust_xlsxwriter writes the 1900 epoch; set real workbook metadata
            // in the ZIP while keeping the matching 1904 numeric date serial.
            let mut input = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
            let mut output = zip::ZipWriter::new(std::fs::File::create(&file).unwrap());
            for index in 0..input.len() {
                let mut entry = input.by_index(index).unwrap();
                let name = entry.name().unwrap().into_owned();
                let mut data = Vec::new();
                entry.read_to_end(&mut data).unwrap();
                if name == "xl/workbook.xml" {
                    let text = String::from_utf8(data).unwrap();
                    assert!(text.contains("<workbookPr "));
                    data = text
                        .replacen("<workbookPr ", "<workbookPr date1904=\"1\" ", 1)
                        .into_bytes();
                }
                output
                    .start_file(
                        name,
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored),
                    )
                    .unwrap();
                output.write_all(&data).unwrap();
            }
            output.finish().unwrap();
        } else {
            std::fs::write(&file, bytes).unwrap();
        }
        let sheets = dsheng_core::import::fileio::read_workbook(&file).unwrap();
        assert_eq!(sheets[0].rows[1][1].raw_text(), "2026-09-12 13:45:00.000");
        assert!(matches!(
            sheets[0].rows[2][1],
            dsheng_core::import::fileio::CellValue::Error(_)
        ));
        assert!(matches!(
            sheets[0].rows[3][1],
            dsheng_core::import::fileio::CellValue::Error(_)
        ));
        let dir = source.to_str().unwrap();
        let mut conn = Connection::open(temp.path().join("test.sqlite3")).unwrap();
        let workspace = workspace(&mut conn);
        let scanned = scan(&mut conn, workspace, dir);
        assert_eq!(
            scanned["available_periods"],
            json!([{"from":"2026-09-01","to":"2026-09-30","grain":"month"}])
        );
        confirm_files(&conn, workspace, dir, &scanned);
        let result = analyze(
            &mut conn,
            workspace,
            dir,
            "2026-09-01",
            "2026-09-30",
            "month",
        );
        assert_eq!(actual(&result, "payments_amount"), "35");
    }
}
