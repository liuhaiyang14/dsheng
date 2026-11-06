//! Natural-period selection using only temporary SQLite and synthetic workbooks.
mod fixture;

use dsheng_cli::store::{analysis, sources};
use dsheng_core::contracts::domain::{DateRange, Granularity};
use fixture::Fixture;
use rusqlite::{params, Connection};
use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};
use std::path::Path;

fn set_grain(conn: &Connection, f: &Fixture, grain: &str) {
    conn.execute(
        "UPDATE period_policies SET cadence = ?2 WHERE workspace_id = ?1",
        params![f.workspace.to_string(), grain],
    )
    .unwrap();
}

fn daily_file(dir: &Path) {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("核算表").unwrap();
    for (col, value) in ["科目", "日期", "本期实绩"].iter().enumerate() {
        sheet.write_string(0, col as u16, *value).unwrap();
    }
    for (row, (date, value)) in [
        ("2026-10-01", "1"),
        ("2026/10/05", "5"),
        ("2026-10-03", ""),
        ("2026-11-01", "100"),
        ("2026-10-04", "待补"),
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(row as u32 + 1, 0, "销售额").unwrap();
        sheet.write_string(row as u32 + 1, 1, *date).unwrap();
        if !value.is_empty() {
            sheet.write_string(row as u32 + 1, 2, *value).unwrap();
        }
    }
    book.save(dir.join("每日.xlsx")).unwrap();
}

fn analyze(
    conn: &mut Connection,
    f: &Fixture,
    dir: &Path,
    from: &str,
    to: &str,
    grain: &str,
) -> Value {
    analysis::run_analyze(
        conn,
        f.workspace,
        &dir.to_string_lossy(),
        from,
        to,
        false,
        Some(grain.into()),
        None,
        false,
        None,
    )
    .unwrap()
}

fn sales(view: &Value) -> &Value {
    view["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric_key"] == "sales_net")
        .expect("sales metric")
}

#[test]
fn scan_buckets_use_actual_dates_and_monthly_analysis_includes_only_that_month() {
    let f = Fixture::new("period-buckets");
    let dir = tempfile::tempdir().unwrap();
    daily_file(dir.path());
    let mut conn = Connection::open(&f.db).unwrap();
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert!(scan["available_period_policy_id"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    assert_eq!(
        scan["available_periods"],
        json!([
            {"from":"2026-10-01","to":"2026-10-31","grain":"month"},
            {"from":"2026-11-01","to":"2026-11-30","grain":"month"}
        ])
    );
    let month = analyze(
        &mut conn,
        &f,
        dir.path(),
        "2026-10-01",
        "2026-10-31",
        "month",
    );
    assert_eq!(
        sales(&month)["actual_value"],
        "6",
        "仅汇总10月已有的1日和5日，不加11月或填空日"
    );
    set_grain(&conn, &f, "day");
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert_eq!(
        scan["available_periods"],
        json!([
            {"from":"2026-10-01","to":"2026-10-01","grain":"day"},
            {"from":"2026-10-05","to":"2026-10-05","grain":"day"},
            {"from":"2026-11-01","to":"2026-11-01","grain":"day"}
        ])
    );
    let day = analyze(&mut conn, &f, dir.path(), "2026-10-05", "2026-10-05", "day");
    assert_eq!(sales(&day)["actual_value"], "5");
    for comparison in ["mom", "yoy"] {
        let evidence = sales(&day)[comparison]["evidence_ids"].as_array().unwrap();
        assert_eq!(
            evidence.len(),
            1,
            "日比较只引用当日和实际基期，不能混入10月1日或11月1日"
        );
        assert!(evidence[0].as_str().unwrap().ends_with(":C3"));
    }
    let cli_scan =
        sources::scan_summary(&mut conn, f.workspace, &dir.path().to_string_lossy()).unwrap();
    assert!(
        cli_scan.get("available_periods").is_none(),
        "冻结CLI读视图不增加字段"
    );
}

#[test]
fn monthly_and_undated_values_never_become_daily_values_or_daily_choices() {
    let f = Fixture::new("no-month-disaggregation");
    let dir = tempfile::tempdir().unwrap();
    daily_file(dir.path());
    // 无日期核算汇总不得绑定到所选的任何一天。
    std::fs::copy(
        f.dir.join("核算表模板.xlsx"),
        dir.path().join("无日期汇总.xlsx"),
    )
    .unwrap();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("推移表").unwrap();
    for (col, value) in ["科目", "单位", "2026-10", "2026-12"].iter().enumerate() {
        sheet.write_string(0, col as u16, *value).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 2, "1000").unwrap();
    sheet.write_string(1, 3, "2000").unwrap();
    book.save(dir.path().join("月汇总.xlsx")).unwrap();
    let mut conn = Connection::open(&f.db).unwrap();
    set_grain(&conn, &f, "day");
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert_eq!(
        scan["available_periods"].as_array().unwrap().len(),
        3,
        "月总值不能展开31个日选项"
    );
    let day = analyze(&mut conn, &f, dir.path(), "2026-10-01", "2026-10-01", "day");
    assert_eq!(
        sales(&day)["actual_value"],
        "1",
        "日实绩不包含月汇总1000或无日期汇总11500.25"
    );
}

#[test]
fn transposed_months_keep_actual_zero_and_skip_blank_or_cumulative_rows() {
    let f = Fixture::new("transposed-periods");
    let dir = tempfile::tempdir().unwrap();
    let mut book = Workbook::new();
    let sheet = book.add_worksheet().set_name("推移表").unwrap();
    sheet.write_string(0, 0, "月份").unwrap();
    sheet.write_string(0, 1, "销售额").unwrap();
    for (row, (period, value)) in [
        ("2026-10", "30"),
        ("2026-11", ""),
        ("2026-12", "0"),
        ("累计2026年1-12月", "30"),
    ]
    .iter()
    .enumerate()
    {
        sheet.write_string(row as u32 + 1, 0, *period).unwrap();
        if !value.is_empty() {
            sheet.write_string(row as u32 + 1, 1, *value).unwrap();
        }
    }
    book.save(dir.path().join("转置.xlsx")).unwrap();
    let mut conn = Connection::open(&f.db).unwrap();
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert_eq!(
        scan["available_periods"],
        json!([
            {"from":"2026-10-01","to":"2026-10-31","grain":"month"},
            {"from":"2026-12-01","to":"2026-12-31","grain":"month"}
        ])
    );
    set_grain(&conn, &f, "day");
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert_eq!(scan["available_periods"], json!([]));
}

#[test]
fn manual_values_are_bound_to_the_exact_analysis_period_key() {
    let f = Fixture::new("manual-period-boundary");
    let dir = tempfile::tempdir().unwrap();
    daily_file(dir.path());
    let mut conn = Connection::open(&f.db).unwrap();
    set_grain(&conn, &f, "day");
    let range = DateRange {
        from: "2026-10-05".parse().unwrap(),
        to: "2026-10-05".parse().unwrap(),
    };
    let period_key = dsheng_core::repository::analysis_runs::build_period_key(
        "1",
        Granularity::Day,
        range,
        "全公司",
    );
    conn.execute("INSERT INTO manual_inputs (input_id, workspace_id, metric_id, period_key, measure_role, scope_key, value, unit, confirmed, source_note, input_revision, raw_text, created_at, updated_at) VALUES (?1,?2,?3,?4,'plan','全公司','50','元',1,'本地测试',1,'50','t','t')", params![uuid::Uuid::new_v4().to_string(),f.workspace.to_string(),f.metric_id,period_key]).unwrap();
    let day1 = analyze(&mut conn, &f, dir.path(), "2026-10-01", "2026-10-01", "day");
    assert!(sales(&day1)["target_value"].is_null(), "5日目标不能跑到1日");
    let day5 = analyze(&mut conn, &f, dir.path(), "2026-10-05", "2026-10-05", "day");
    assert_eq!(sales(&day5)["target_value"], "50");
}

#[test]
fn desktop_selection_requires_one_natural_bucket_using_current_system_grain() {
    let f = Fixture::new("period-validation");
    let conn = Connection::open(&f.db).unwrap();
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-01",
        "2026-10-31",
        "auto"
    )
    .is_ok());
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-01",
        "2026-10-05",
        "month"
    )
    .is_err());
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-01",
        "2026-10-01",
        "day"
    )
    .is_err());
    set_grain(&conn, &f, "day");
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-05",
        "2026-10-05",
        "day"
    )
    .is_ok());
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-01",
        "2026-10-05",
        "day"
    )
    .is_err());
}

#[test]
fn configured_month_start_controls_scan_guard_totals_and_comparisons() {
    let f = Fixture::new("custom-month-bucket");
    let dir = tempfile::tempdir().unwrap();
    daily_file(dir.path());
    let mut conn = Connection::open(&f.db).unwrap();
    conn.execute(
        "UPDATE period_policies SET month_start_day=15 WHERE workspace_id=?1",
        [f.workspace.to_string()],
    )
    .unwrap();
    let scan = sources::scan_summary_desktop(&mut conn, f.workspace, &dir.path().to_string_lossy())
        .unwrap();
    assert_eq!(
        scan["available_periods"],
        json!([
            {"from":"2026-09-15","to":"2026-10-14","grain":"month"},
            {"from":"2026-10-15","to":"2026-11-14","grain":"month"}
        ])
    );
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-15",
        "2026-11-14",
        "month"
    )
    .is_ok());
    assert!(sources::validate_desktop_analysis_period(
        &conn,
        f.workspace,
        "2026-10-01",
        "2026-10-31",
        "month"
    )
    .is_err());
    let view = analyze(
        &mut conn,
        &f,
        dir.path(),
        "2026-10-15",
        "2026-11-14",
        "month",
    );
    let metric = sales(&view);
    assert_eq!(metric["actual_value"], "100");
    assert_eq!(metric["mom"]["base_value"], "6");
    assert_eq!(metric["mom"]["base_from"], "2026-09-15");
    assert_eq!(metric["mom"]["base_to"], "2026-10-14");
    assert_eq!(metric["yoy"]["base_from"], "2025-10-15");
    assert_eq!(metric["yoy"]["base_to"], "2025-11-14");
    let saved: String = conn.query_row("SELECT period_policy_snapshot_json FROM run_revisions WHERE run_id=?1 AND revision_no=1", [view["run_id"].as_str().unwrap()],|row|row.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&saved).unwrap()["month_start_day"],
        15
    );
}

#[test]
fn setting_month_start_creates_new_policy_and_preserves_old_default() {
    let f = Fixture::new("custom-month-setting");
    let mut conn = Connection::open(&f.db).unwrap();
    let before = dsheng_cli::store::settings::list_policies(&conn, f.workspace).unwrap();
    assert_eq!(before[0]["policy"]["month_start_day"], 1);
    let op = dsheng_cli::jsonio::parse_settings_operation(json!({"schema_version":"1","operation":"period_policy_set", "workspace_id":f.workspace,"expected_revision":1, "policy":{"cadence":"month","month_start_day":31,"effective_from":"2026-11-01"}})).unwrap();
    let changed =
        dsheng_cli::store::settings::apply_settings_operation(&mut conn, &op, None).unwrap();
    assert_eq!(changed["policy"]["policy"]["month_start_day"], 31);
    assert_eq!(changed["policy"]["version"], 2);
    let after = dsheng_cli::store::settings::list_policies(&conn, f.workspace).unwrap();
    assert_eq!(after.len(), 2);
    assert_eq!(after[0]["policy"]["month_start_day"], 1);
    assert_eq!(after[0]["period_policy_id"], before[0]["period_policy_id"]);
    for invalid in [0, 32] {
        assert!(dsheng_cli::jsonio::parse_settings_operation(json!({"schema_version":"1","operation":"period_policy_set", "workspace_id":f.workspace,"expected_revision":2,"policy":{"cadence":"month","month_start_day":invalid,"effective_from":"2026-12-01"}})).is_err());
    }
    assert_eq!(
        dsheng_cli::store::settings::list_policies(&conn, f.workspace)
            .unwrap()
            .len(),
        2
    );
}
