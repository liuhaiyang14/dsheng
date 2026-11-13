//! Diagnosis evidence comes from the selected analysis revision, independently of comparisons.
//! Synthetic workbook/SQLite only; no Provider dispatch.
mod fixture;

use dsheng_cli::store::analysis::{diagnosis_evidence_ids, run_analyze};
use dsheng_core::contracts::cli::CliErrorCode;
use fixture::Fixture;
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;
use uuid::Uuid;

fn partial_analysis(f: &Fixture) -> Value {
    run_analyze(
        &mut Connection::open(&f.db).unwrap(),
        f.workspace,
        f.dir.to_str().unwrap(),
        "2026-10-01",
        "2026-10-15",
        true,
        None,
        None,
        false,
        None,
    )
    .unwrap()
}

fn ids(f: &Fixture, view: &Value, revision: u32) -> Vec<String> {
    let conn = Connection::open_with_flags(&f.db, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    diagnosis_evidence_ids(
        &conn,
        f.workspace,
        view["run_id"].as_str().unwrap(),
        revision,
        Some(view["metrics"][0]["metric_id"].as_str().unwrap()),
    )
    .unwrap()
}

#[test]
fn partial_period_has_actual_evidence_without_comparisons_or_view_extensions() {
    let f = Fixture::new("diagnosis-evidence-partial");
    let view = partial_analysis(&f);
    assert!(view["metrics"][0]["mom"].is_null());
    assert!(view["metrics"][0]["yoy"].is_null());
    assert!(view.get("diagnosis_evidence_by_metric").is_none());
    assert!(view["metrics"][0].get("evidence_ids").is_none());
    let evidence = ids(&f, &view, 1);
    // Evidence is anchored to the raw source revision; generated two-table
    // sheets are not an import source and therefore cannot appear here.
    assert_eq!(
        evidence,
        vec![
            "cell:source-data.xlsx:source:D2",
            "cell:source-data.xlsx:source:D3"
        ]
    );
    let conn = Connection::open(&f.db).unwrap();
    let run = view["run_id"].as_str().unwrap();
    assert_eq!(
        diagnosis_evidence_ids(&conn, f.workspace, run, 1, None).unwrap(),
        evidence
    );
    assert_eq!(
        diagnosis_evidence_ids(&conn, Uuid::new_v4(), run, 1, None)
            .unwrap_err()
            .code,
        CliErrorCode::WorkspaceMismatch
    );
    assert_eq!(
        diagnosis_evidence_ids(&conn, f.workspace, run, 99, None)
            .unwrap_err()
            .code,
        CliErrorCode::DataValidationFailed
    );
    assert_eq!(
        diagnosis_evidence_ids(&conn, f.workspace, run, 1, Some("not-in-snapshot"))
            .unwrap_err()
            .code,
        CliErrorCode::InputContractInvalid
    );
}

#[test]
fn prior_revision_evidence_survives_source_changes_and_removal() {
    let f = Fixture::new("diagnosis-evidence-history");
    let first = partial_analysis(&f);
    let old = ids(&f, &first, 1);
    let mut book = rust_xlsxwriter::Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("核算表").unwrap();
    for (col, label) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"]
        .iter()
        .enumerate()
    {
        sheet.write_string(0, col as u16, *label).unwrap();
    }
    // Move the synthetic observation from row 2 to row 3 and change the actual value.
    sheet.write_string(2, 0, "销售额").unwrap();
    sheet.write_string(2, 2, "12000.5").unwrap();
    sheet.write_string(2, 3, "11000.25").unwrap();
    sheet.write_string(2, 4, "12500").unwrap();
    book.save(f.dir.join("核算表模板.xlsx")).unwrap();
    let second = partial_analysis(&f);
    assert_eq!(second["run_id"], first["run_id"]);
    assert_eq!(second["current_revision_no"], 2);
    assert_eq!(
        ids(&f, &second, 2),
        vec![
            "cell:source-data.xlsx:source:D2",
            "cell:source-data.xlsx:source:D3"
        ]
    );
    // Generated output sheets are rejected as input; current source caches/files
    // are irrelevant when reading a historical analysis revision.
    let conn = Connection::open(&f.db).unwrap();
    conn.execute("DELETE FROM cli_source_cells", []).unwrap();
    std::fs::remove_file(f.dir.join("核算表模板.xlsx")).unwrap();
    assert_eq!(ids(&f, &first, 1), old);
    assert_eq!(ids(&f, &second, 2), old);
}

#[test]
fn legacy_snapshot_reuses_only_stored_evidence_and_missing_evidence_stays_empty() {
    let f = Fixture::new("diagnosis-evidence-legacy");
    let conn = Connection::open(&f.db).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions WHERE run_id=?1 AND revision_no=1",
            [&f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut legacy: Value = serde_json::from_str(&raw).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("diagnosis_evidence_by_metric");
    let metric_id = legacy["metrics"][0]["metric_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut expected: Vec<String> = ["mom", "yoy"]
        .iter()
        .flat_map(|key| {
            legacy["metrics"][0][*key]["evidence_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    expected.sort();
    expected.dedup();
    assert!(!expected.is_empty());
    conn.execute(
        "UPDATE run_revisions SET normalized_metrics_json=?2 WHERE run_id=?1 AND revision_no=1",
        params![f.run_id, legacy.to_string()],
    )
    .unwrap();
    assert_eq!(
        diagnosis_evidence_ids(&conn, f.workspace, &f.run_id, 1, Some(&metric_id)).unwrap(),
        expected
    );
    legacy["metrics"][0]["mom"] = Value::Null;
    legacy["metrics"][0]["yoy"] = Value::Null;
    conn.execute(
        "UPDATE run_revisions SET normalized_metrics_json=?2 WHERE run_id=?1 AND revision_no=1",
        params![f.run_id, legacy.to_string()],
    )
    .unwrap();
    assert!(
        diagnosis_evidence_ids(&conn, f.workspace, &f.run_id, 1, Some(&metric_id))
            .unwrap()
            .is_empty()
    );
}
