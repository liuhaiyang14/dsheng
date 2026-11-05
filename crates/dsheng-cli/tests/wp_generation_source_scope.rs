//! 生成范围只来自所选不可变版本及显式基期；合成 SQLite，无网络。

mod fixture;

use dsheng_cli::store::generation::trimmed_source_selection;
use dsheng_core::contracts::cli::CliErrorCode;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn source(conn: &Connection, f: &Fixture) -> (String, String) {
    conn.query_row(
        "SELECT source_id, content_hash FROM source_files WHERE workspace_id = ?1 AND kind = 'xlsx'",
        params![f.workspace.to_string()],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn selection(source_id: &str, hash: &str) -> Value {
    json!([{"source_id": source_id, "content_hash": hash, "sheet": "核算表", "range": "A1:E2"}])
}

fn set_scope(conn: &Connection, run: &str, scope: &Value, bases: &Value) {
    conn.execute(
        "UPDATE run_revisions SET source_selection_snapshot_json = ?2,
                comparison_base_refs_json = ?3 WHERE run_id = ?1 AND revision_no = 1",
        params![run, scope.to_string(), bases.to_string()],
    )
    .unwrap();
}

fn clone_source(conn: &Connection, original: &str, hash: &str) -> String {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO source_files
            (source_id, workspace_id, file_label, path_ref, kind, content_hash, created_at, updated_at)
         SELECT ?2, workspace_id, file_label, path_ref, kind, ?3, created_at, updated_at
         FROM source_files WHERE source_id = ?1",
        params![original, id, hash],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cli_source_sheet_layouts
         SELECT workspace_id, ?2, sheet, role, layout_json
         FROM cli_source_sheet_layouts WHERE source_id = ?1",
        params![original, id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cli_source_cells
         SELECT workspace_id, ?2, sheet, row_no, col_no, raw_text, cell_kind,
                value_state, typed_value, formula, merged_range
         FROM cli_source_cells WHERE source_id = ?1",
        params![original, id],
    )
    .unwrap();
    id
}

fn clone_run(conn: &Connection, original: &str, scope: &Value) -> String {
    let id = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO analysis_runs
         SELECT ?2, workspace_id, ?2, 1, analysis_input_fingerprint, created_at, updated_at
         FROM analysis_runs WHERE run_id = ?1",
        params![original, id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO run_revisions
         SELECT ?2, revision_no, analysis_input_fingerprint, canonicalization_version,
                period_policy_snapshot_json, ?3, mapping_snapshot_json,
                manual_inputs_snapshot_json, metric_definitions_snapshot_json,
                normalized_metrics_json, comparisons_json, '[]', action_period_snapshot_json, created_at
         FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
        params![original, id, scope.to_string()],
    )
    .unwrap();
    id
}

#[test]
fn old_revision_with_blank_source_id_excludes_new_file_versions_and_other_sheets() {
    let f = Fixture::new("generation-source-version");
    let conn = Connection::open(&f.db).unwrap();
    let (old_id, old_hash) = source(&conn, &f);
    let new_id = clone_source(&conn, &old_id, "new-file-content");
    set_scope(&conn, &f.run_id, &selection("", &old_hash), &json!([]));
    conn.execute(
        "INSERT INTO cli_source_sheet_layouts
         SELECT workspace_id, source_id, '其他期间', role, layout_json
         FROM cli_source_sheet_layouts WHERE source_id = ?1",
        params![old_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cli_source_cells
         SELECT workspace_id, source_id, '其他期间', row_no, col_no, raw_text,
                cell_kind, value_state, typed_value, formula, merged_range
         FROM cli_source_cells WHERE source_id = ?1",
        params![old_id],
    )
    .unwrap();

    let selected = trimmed_source_selection(
        &conn,
        f.workspace,
        &f.run_id,
        1,
        &["cell:核算表模板.xlsx:其他期间:D2".to_string()],
    )
    .unwrap();
    assert_eq!(selected.manifest.len(), 1);
    assert_eq!(selected.manifest[0].source_id.to_string(), old_id);
    assert_eq!(selected.manifest[0].content_hash, old_hash);
    assert!(!selected.records.is_empty());
    assert!(selected
        .records
        .iter()
        .all(|r| r.source_id.to_string() != new_id && r.sheet.as_deref() == Some("核算表")));
    assert_eq!(selected.scope_rows.len(), 1);
    assert_eq!(selected.scope_rows[0]["content_hash"], json!(old_hash));
}

#[test]
fn only_the_exact_explicit_comparison_revision_adds_baseline_sources() {
    let f = Fixture::new("generation-source-base");
    let conn = Connection::open(&f.db).unwrap();
    let (current_id, current_hash) = source(&conn, &f);
    let base_id = clone_source(&conn, &current_id, "base-file-content");
    let unrelated_id = clone_source(&conn, &current_id, "unrelated-file-content");
    let base_run = clone_run(&conn, &f.run_id, &selection(&base_id, "base-file-content"));
    // The baseline has since been revised; only its explicitly bound r1 may contribute.
    conn.execute(
        "INSERT INTO run_revisions
         SELECT run_id, 2, analysis_input_fingerprint, canonicalization_version,
                period_policy_snapshot_json, ?2, mapping_snapshot_json,
                manual_inputs_snapshot_json, metric_definitions_snapshot_json,
                normalized_metrics_json, comparisons_json, '[]', action_period_snapshot_json, created_at
         FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
        params![base_run, selection(&unrelated_id, "unrelated-file-content").to_string()],
    ).unwrap();
    conn.execute(
        "UPDATE analysis_runs SET current_revision_no = 2 WHERE run_id = ?1",
        params![base_run],
    )
    .unwrap();
    set_scope(
        &conn,
        &f.run_id,
        &selection(&current_id, &current_hash),
        &json!([{"run_id": base_run, "revision_no": 1}]),
    );

    let selected = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]).unwrap();
    let ids: std::collections::BTreeSet<_> = selected
        .manifest
        .iter()
        .map(|m| m.source_id.to_string())
        .collect();
    assert_eq!(ids, [current_id, base_id].into_iter().collect());
    assert!(selected
        .records
        .iter()
        .all(|r| r.source_id.to_string() != unrelated_id));
}

#[test]
fn missing_or_invalid_snapshots_never_fall_back_to_workspace_sources() {
    let f = Fixture::new("generation-source-missing");
    let conn = Connection::open(&f.db).unwrap();
    for raw in ["{}", "not-json"] {
        conn.execute(
            "UPDATE run_revisions SET source_selection_snapshot_json = ?2 WHERE run_id = ?1",
            params![f.run_id, raw],
        )
        .unwrap();
        let result = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]);
        assert!(matches!(result, Err(e) if e.code == CliErrorCode::DataValidationFailed));
    }
    let result = trimmed_source_selection(&conn, f.workspace, &f.run_id, 99, &[]);
    assert!(matches!(result, Err(e) if e.code == CliErrorCode::DataValidationFailed));
}

#[test]
fn empty_snapshot_keeps_an_empty_file_scope() {
    let f = Fixture::new("generation-source-empty");
    let conn = Connection::open(&f.db).unwrap();
    set_scope(&conn, &f.run_id, &json!([]), &json!([]));
    let selected = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]).unwrap();
    assert!(selected.manifest.is_empty());
    assert!(selected.records.is_empty());
    assert!(selected.scope_rows.is_empty());
}

#[test]
fn missing_or_foreign_base_references_fail_closed() {
    let f = Fixture::new("generation-source-foreign-base");
    let conn = Connection::open(&f.db).unwrap();
    let (id, hash) = source(&conn, &f);
    let base_run = clone_run(&conn, &f.run_id, &selection(&id, &hash));
    let other_workspace = Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO workspaces (workspace_id, name, created_at, updated_at)
         VALUES (?1, '另一合成企业', '2026-11-05T00:00:00Z', '2026-11-05T00:00:00Z')",
        params![other_workspace],
    )
    .unwrap();
    conn.execute(
        "UPDATE analysis_runs SET workspace_id = ?2 WHERE run_id = ?1",
        params![base_run, other_workspace],
    )
    .unwrap();
    for run_id in [base_run, "missing-run".to_string()] {
        set_scope(
            &conn,
            &f.run_id,
            &selection(&id, &hash),
            &json!([{"run_id": run_id, "revision_no": 1}]),
        );
        let result = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]);
        assert!(matches!(result, Err(e) if e.code == CliErrorCode::DataValidationFailed));
    }
}

#[test]
fn a_known_source_id_cannot_substitute_a_different_content_hash() {
    let f = Fixture::new("generation-source-hash");
    let conn = Connection::open(&f.db).unwrap();
    let (id, _) = source(&conn, &f);
    clone_source(&conn, &id, "different-content");
    set_scope(
        &conn,
        &f.run_id,
        &selection(&id, "different-content"),
        &json!([]),
    );
    let result = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]);
    assert!(matches!(result, Err(e) if e.code == CliErrorCode::DataValidationFailed));
}

#[test]
fn csv_snapshots_accept_named_or_null_sheet_without_selecting_other_sheets() {
    let f = Fixture::new("generation-source-csv");
    let conn = Connection::open(&f.db).unwrap();
    let (id, hash) = source(&conn, &f);
    conn.execute(
        "UPDATE source_files SET kind = 'csv' WHERE source_id = ?1",
        params![id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cli_source_sheet_layouts
         SELECT workspace_id, source_id, 'csv', role, layout_json
         FROM cli_source_sheet_layouts WHERE source_id = ?1",
        params![id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO cli_source_cells
         SELECT workspace_id, source_id, 'csv', row_no, col_no, raw_text,
                cell_kind, value_state, typed_value, formula, merged_range
         FROM cli_source_cells WHERE source_id = ?1",
        params![id],
    )
    .unwrap();
    for sheet in [json!("csv"), Value::Null] {
        set_scope(
            &conn,
            &f.run_id,
            &json!([{"source_id": "", "content_hash": hash, "sheet": sheet, "range": "A1:E2"}]),
            &json!([]),
        );
        let selected = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]).unwrap();
        assert_eq!(selected.manifest.len(), 1);
        assert_eq!(selected.manifest[0].sheet, None);
        assert!(!selected.records.is_empty());
        assert!(selected
            .records
            .iter()
            .all(|r| r.sheet.is_none() && r.evidence_id.contains(":csv:")));
    }
}

#[test]
fn missing_or_malformed_sidecars_cannot_silently_drop_selected_sources() {
    let f = Fixture::new("generation-source-sidecars");
    let conn = Connection::open(&f.db).unwrap();
    for corrupt in [
        "DELETE FROM cli_source_sheet_layouts",
        "DELETE FROM cli_source_cells",
        "UPDATE cli_source_sheet_layouts SET layout_json = 'not-json'",
        "UPDATE cli_source_sheet_layouts SET layout_json = '[]'",
    ] {
        conn.execute_batch("SAVEPOINT corrupt_sidecar").unwrap();
        conn.execute_batch(corrupt).unwrap();
        let result = trimmed_source_selection(&conn, f.workspace, &f.run_id, 1, &[]);
        assert!(
            matches!(result, Err(e) if e.code == CliErrorCode::DataValidationFailed),
            "{corrupt}"
        );
        conn.execute_batch("ROLLBACK TO corrupt_sidecar; RELEASE corrupt_sidecar")
            .unwrap();
    }
}

#[test]
fn real_csv_scan_is_version_bound_and_blank_sheets_are_not_selected() {
    let f = Fixture::new("generation-source-real-csv");
    std::fs::write(
        f.dir.join("核算.csv"),
        "科目,月度实绩\n销售额,11500.25\n无关科目,1\n",
    )
    .unwrap();
    let mut blank = rust_xlsxwriter::Workbook::new();
    blank.add_worksheet().set_name("空白").unwrap();
    blank.save(f.dir.join("空白.xlsx")).unwrap();
    std::fs::write(f.dir.join("不支持.pptx"), "synthetic unsupported file").unwrap();
    let args = vec![
        "analyze".to_string(),
        "--db".to_string(),
        f.db.clone(),
        "--workspace".to_string(),
        f.workspace.to_string(),
        "--dir".to_string(),
        f.dir.to_string_lossy().to_string(),
        "--from".to_string(),
        "2026-10-01".to_string(),
        "--to".to_string(),
        "2026-10-31".to_string(),
        "--grain".to_string(),
        "auto".to_string(),
        "--json".to_string(),
    ];
    let mut out = Vec::new();
    let mut err = Vec::new();
    assert_eq!(
        dsheng_cli::run_with_output(&args, &mut out, &mut err),
        0,
        "{}",
        String::from_utf8_lossy(&out)
    );

    let conn = Connection::open(&f.db).unwrap();
    let (revision, raw): (u32, String) = conn
        .query_row(
            "SELECT ar.current_revision_no, rr.source_selection_snapshot_json
         FROM analysis_runs ar JOIN run_revisions rr ON rr.run_id = ar.run_id
            AND rr.revision_no = ar.current_revision_no WHERE ar.run_id = ?1",
            params![f.run_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let snapshot: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert_eq!(
        snapshot.len(),
        2,
        "only the populated workbook and CSV are selected"
    );
    assert!(snapshot
        .iter()
        .all(|s| Uuid::parse_str(s["source_id"].as_str().unwrap()).is_ok()));
    assert!(snapshot.iter().any(|s| s["sheet"] == "csv"));
    let selected = trimmed_source_selection(&conn, f.workspace, &f.run_id, revision, &[]).unwrap();
    let csv_records: Vec<_> = selected
        .records
        .iter()
        .filter(|r| r.file_label == "核算.csv")
        .collect();
    assert_eq!(
        csv_records.len(),
        4,
        "CSV retains only header and registered metric rows"
    );
    assert!(csv_records
        .iter()
        .all(|r| r.sheet.is_none() && r.evidence_id.contains(":csv:")));
    assert!(selected
        .manifest
        .iter()
        .all(|m| m.file_label != "空白.xlsx" && m.file_label != "不支持.pptx"));
}
