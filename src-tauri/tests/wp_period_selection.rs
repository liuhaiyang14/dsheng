//! Desktop-only period choices and validation, with temporary synthetic local files.
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;
use dsheng_app::contracts::{core::data, env::DesktopEnv};
use dsheng_cli::store::sources;
use fixture::Fixture;
use serde_json::json;

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
