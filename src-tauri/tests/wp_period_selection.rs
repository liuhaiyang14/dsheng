//! Desktop-only period choices and validation, with temporary synthetic local files.
#[path = "../../crates/dsheng-cli/tests/fixture/mod.rs"]
mod fixture;
use dsheng_app::contracts::{core::data, env::DesktopEnv};
use fixture::Fixture;
use serde_json::json;

#[test]
fn desktop_scan_adds_real_period_choices_without_inventing_blank_days() {
    let f = Fixture::new("desktop-period-choices");
    let dir = tempfile::tempdir().unwrap();
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet().set_name("核算表").unwrap();
    for (col, value) in ["科目", "日期", "本期实绩"].iter().enumerate() {
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
