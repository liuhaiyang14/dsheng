//! 合成 SQLite 旁表与本地 payload 组装；不派发 Provider。
mod fixture;
use dsheng_cli::store::generation::{build_diagnosis_payload, build_task_payload};
use dsheng_core::contracts::provider::Framework;
use dsheng_core::generation::input::SessionStateProjection;
use fixture::Fixture;
use rusqlite::{params, Connection};
use serde_json::{json, Value};

fn check_layout(kind: &str) {
    let f = Fixture::new(&format!("period-payload-{kind}"));
    let conn = Connection::open(&f.db).unwrap();
    let (sid, label): (String, String) = conn
        .query_row(
            "SELECT source_id, file_label FROM source_files WHERE workspace_id=?1 AND kind='xlsx'",
            params![f.workspace.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let (layout, rows, current, outside, base) = match kind {
        "transposed" => (
            json!({"transposed":{"header_row":1,"metric_columns":[[1,"销售额"]],"period_rows":[[1,"2026-10-02/2026-10-02"],[2,"2026-10-03/2026-10-03"],[3,"2026-10-01/2026-10-01"]],"cumulative_rows":[]}}),
            vec![
                vec!["日期", "销售额"],
                vec!["2026-10-02", "22"],
                vec!["2026-10-03", "333033"],
                vec!["2026-10-01", "11"],
            ],
            "B2",
            "B3",
            "B4",
        ),
        "row" => (
            json!({"header":{"row":1,"row_count":1,"columns":{"metric_label":0,"period":1,"actual":2}},"label_col":0,"period_col":1,"trend_period_cols":[]}),
            vec![
                vec!["科目", "日期", "实绩"],
                vec!["销售额", "2026-10-02", "22"],
                vec!["销售额", "2026-10-03", "333033"],
                vec!["销售额", "2026-10-01", "11"],
            ],
            "C2",
            "C3",
            "C4",
        ),
        _ => (
            json!({"header":{"row":1,"row_count":1,"columns":{"metric_label":0}},"label_col":0,"period_col":null,"trend_period_cols":[[1,"2026-10-02/2026-10-02"],[2,"2026-10-03/2026-10-03"],[3,"2026-10-01/2026-10-01"]]}),
            vec![
                vec!["科目", "2026-10-02", "2026-10-03", "2026-10-01"],
                vec!["销售额", "22", "333033", "11"],
            ],
            "B2",
            "C2",
            "D2",
        ),
    };
    conn.execute(
        "UPDATE cli_source_sheet_layouts SET layout_json=?2 WHERE source_id=?1 AND sheet='source'",
        params![sid, layout.to_string()],
    )
    .unwrap();
    conn.execute(
        "DELETE FROM cli_source_cells WHERE source_id=?1",
        params![sid],
    )
    .unwrap();
    for (row, cells) in rows.iter().enumerate() {
        for (col, value) in cells.iter().enumerate() {
            conn.execute("INSERT INTO cli_source_cells(workspace_id,source_id,sheet,row_no,col_no,raw_text,cell_kind,value_state,typed_value,formula,merged_range) VALUES(?1,?2,'source',?3,?4,?5,'text','valid',?5,NULL,NULL)",params![f.workspace.to_string(),sid,row+1,col,value]).unwrap();
        }
    }
    let raw: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions WHERE run_id=?1 AND revision_no=1",
            params![f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut normalized: Value = serde_json::from_str(&raw).unwrap();
    normalized["period"] = json!({"from":"2026-10-02","to":"2026-10-02"});
    normalized["grain"] = json!("day");
    let ids: Vec<String> = [current, outside, base]
        .iter()
        .map(|a1| format!("cell:{label}:source:{a1}"))
        .collect();
    for metric in normalized["metrics"].as_array_mut().unwrap() {
        metric["actual_value"] = json!("22");
        metric["mom"] = json!({"comparison_type":"mom","label":"前一日","base_from":"2026-10-01","base_to":"2026-10-01","base_value":"11","absolute_change":"11","change_rate":"1","availability_status":"available","reason":null,"base_refs":[],"evidence_ids":ids});
        metric["yoy"] = Value::Null;
    }
    conn.execute(
        "UPDATE run_revisions SET normalized_metrics_json=?2 WHERE run_id=?1 AND revision_no=1",
        params![f.run_id, normalized.to_string()],
    )
    .unwrap();
    let current_key: String = conn
        .query_row(
            "SELECT period_key FROM analysis_runs WHERE run_id=?1",
            params![f.run_id],
            |r| r.get(0),
        )
        .unwrap();
    let mut manual_evidence = Vec::new();
    for (key, value) in [(current_key.as_str(), "44"), ("other-period", "333034")] {
        let input_id = uuid::Uuid::new_v4().to_string();
        manual_evidence.push(format!("manual:{input_id}:r1"));
        conn.execute("INSERT INTO manual_inputs(input_id,workspace_id,metric_id,period_key,measure_role,scope_key,value,unit,confirmed,source_note,input_revision,raw_text,created_at,updated_at) VALUES(?1,?2,?3,?4,'actual','all',?5,'元',1,'合成输入',1,?5,'test','test')",params![input_id,f.workspace.to_string(),f.metric_id,key,value]).unwrap();
    }
    for (date, value, evidence, manual_id, manual_value) in [
        ("2026-10-02", "22", &ids[0], &manual_evidence[0], "44"),
        (
            "2026-10-03",
            "333033",
            &ids[1],
            &manual_evidence[1],
            "333034",
        ),
    ] {
        conn.execute("INSERT INTO source_conflicts(conflict_id,workspace_id,run_id,metric_id,measure_role,period_from,period_to,scope_key,unit,candidate_values_json,resolution_status,created_at) VALUES(?1,?2,?3,?4,'actual',?5,?5,'all','元',?6,'unresolved','test')",params![uuid::Uuid::new_v4().to_string(),f.workspace.to_string(),f.run_id,f.metric_id,date,json!([{"value":value,"evidence_id":evidence,"source_label":label},{"value":manual_value,"evidence_id":manual_id,"source_label":"人工输入"}]).to_string()]).unwrap();
    }
    let task = build_task_payload(&conn, f.workspace, &f.run_id, "只分析选定当天", None).unwrap();
    let diagnosis = build_diagnosis_payload(
        &conn,
        f.workspace,
        &f.run_id,
        Framework::W5,
        "只分析选定当天",
        &[],
        &[ids[1].clone()],
        SessionStateProjection {
            accepted_facts: vec![],
            prior_report_summary: None,
            unresolved_questions: vec![],
            pending_clarification: None,
        },
        None,
        "只分析选定当天",
    )
    .unwrap();
    for material in [task, diagnosis] {
        assert!(
            !material.payload_json.contains("333033"),
            "{kind}: other-day value leaked"
        );
        let payload: Value = serde_json::from_str(&material.payload_json).unwrap();
        assert_eq!(payload["manual_inputs"].as_array().unwrap().len(), 1);
        assert_eq!(payload["manual_inputs"][0]["value"], "44");
        assert_eq!(
            payload["manual_inputs"][0]["period"],
            "2026-10-02/2026-10-02"
        );
        assert_eq!(payload["source_conflicts"].as_array().unwrap().len(), 1);
        let records = payload["source_records"].as_array().unwrap();
        let candidate = &payload["source_conflicts"][0]["candidate_values"][0]["evidence_id"];
        assert!(records.iter().any(|r| r["evidence_id"] == *candidate));
        assert!(records.iter().any(|r| r["values"]["raw_text"] == "22"));
        assert!(
            records.iter().any(|r| r["values"]["raw_text"] == "11"),
            "real baseline anchor kept"
        );
        let comparison = &payload["metrics"][0]["mom"]["evidence_ids"];
        assert_eq!(comparison.as_array().unwrap().len(), 2);
        assert!(comparison
            .as_array()
            .unwrap()
            .iter()
            .all(|id| records.iter().any(|r| r["evidence_id"] == *id)));
        assert!(
            comparison.to_string().contains(&sid),
            "IDs normalized to source UUID"
        );
    }
}

#[test]
fn date_rows_filter_other_days_in_both_workflows() {
    check_layout("row");
}
#[test]
fn date_columns_filter_other_days_in_both_workflows() {
    check_layout("column");
}
#[test]
fn transposed_dates_filter_other_days_in_both_workflows() {
    check_layout("transposed");
}
