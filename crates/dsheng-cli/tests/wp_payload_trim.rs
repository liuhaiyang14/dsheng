//! 生成载荷来源范围裁剪测试（wp/payload-trim；prompt-context-validation.md §4：
//! 收窄“选定范围”本身——预览 manifest/scope 如实反映新范围，用户确认即认可）。
//!
//! 合成夹具层次：临时 SQLite＋合成 XLSX/CSV（复用 CLI 运行器建库）＋库级
//! build_*_payload/create_preview 断言；无网络。
//!
//! 覆盖：
//! - 全表扫描（含远端无关行）→ payload 只含表头块＋已注册指标行；
//! - 用户 --evidence 指向的行保留（evidence 单元格不得丢）；
//! - 推移表转置 sheet 按指标列整列（含 period 列）裁剪；
//! - 两文件冲突候选行保留（冲突证据行不丢）；
//! - CSV 来源（sheet=null、evidence sheet-or-csv=csv）；
//! - 确定性：同一 DB 状态两次构建 → 指纹/字节一致（session id 剔除后同文）；
//! - 预览 source_scope_summary 反映裁剪后范围与行数（settings.schema 字段集）。

use rust_xlsxwriter::Workbook;
use serde_json::{json, Value};

fn run(args: &[&str]) -> (String, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    (String::from_utf8(out).expect("utf8"), code)
}

fn data_of(stdout: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim()).expect("envelope");
    assert_eq!(v["ok"], true, "{stdout}");
    v["data"].clone()
}

/// 建工作区＋指标（sales_net/销售额；可选 gross/毛利额）＋带 Key provider。
struct Setup {
    db: String,
    dir: std::path::PathBuf,
    workspace: uuid::Uuid,
}

fn setup(tag: &str, metrics: &[(&str, &str, &str, bool)]) -> Setup {
    let tmp = tempfile::tempdir().expect("临时目录");
    let dir = tmp.keep();
    let db = dir
        .join(format!("{tag}.sqlite3"))
        .to_string_lossy()
        .to_string();
    std::fs::write(
        dir.join("ws.json"),
        r#"{"schema_version":"1","operation":"workspace_create","name":"裁剪测试门店","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
    )
    .unwrap();
    let (out, code) = run(&[
        "workspace",
        "create",
        "--db",
        &db,
        "--input-json",
        &dir.join("ws.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let workspace: uuid::Uuid = serde_json::from_str::<Value>(&out).unwrap()["data"]["workspace"]
        ["workspace_id"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    for (i, (key, label, dimension, revenue)) in metrics.iter().enumerate() {
        let unit_signature = if *dimension == "ratio" {
            json!({"dimension": dimension, "base_unit": "1", "currency": null, "tax_basis": null})
        } else {
            json!({"dimension": dimension, "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"})
        };
        std::fs::write(
            dir.join(format!("metric{i}.json")),
            json!({
                "metric_key": key,
                "label": label,
                "semantic_type": "flow",
                "unit_signature": unit_signature,
                "direction": "up",
                "target_constraint_scope": if *revenue { "monthly_revenue_floor" } else { "explicit_target_only" },
                "revenue_semantics": revenue,
                "aggregation": "sum_disjoint",
                "required_inputs": [],
                "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
                "effective_from": "2026-01-01",
                "expected_revision": 0
            })
            .to_string(),
        )
        .unwrap();
        let (out, code) = run(&[
            "metric",
            "set",
            "--db",
            &db,
            "--workspace",
            &workspace.to_string(),
            "--input-json",
            &dir.join(format!("metric{i}.json")).to_string_lossy(),
            "--json",
        ]);
        assert_eq!(code, 0, "{out}");
    }
    let provider = uuid::Uuid::new_v4();
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
        conn.execute(
            "INSERT INTO providers (provider_id, workspace_id, revision, name, protocol,
                    base_url, model, stream, api_key, connect_seconds, response_seconds,
                    total_seconds, context_window_tokens, output_reserve_tokens,
                    created_at, updated_at)
             VALUES (?1, ?2, 1, '裁剪测试Provider', 'chat_completions',
                     'https://api.example.com/v1', 'synthetic-model', 0, 'test-key-not-real',
                     15, 180, 240, 65536, 8192, '2026-11-04T00:00:00Z', '2026-11-04T00:00:00Z')",
            rusqlite::params![provider.to_string(), workspace.to_string()],
        )
        .unwrap();
        conn.execute(
            "UPDATE workspaces SET active_provider_id = ?2 WHERE workspace_id = ?1",
            rusqlite::params![workspace.to_string(), provider.to_string()],
        )
        .unwrap();
    }
    Setup { db, dir, workspace }
}

fn analyze(s: &Setup, from: &str, to: &str) -> String {
    let (out, code) = run(&[
        "analyze",
        "--db",
        &s.db,
        "--workspace",
        &s.workspace.to_string(),
        "--dir",
        &s.dir.to_string_lossy(),
        "--from",
        from,
        "--to",
        to,
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    data_of(&out)["run_id"].as_str().unwrap().to_string()
}

/// 合成核算表：表头行＋指标行（销售额）＋N 行无关科目＋远端备注行。
fn accounting_workbook(path: &std::path::Path, junk_rows: usize, note_row: u32) {
    let mut book = Workbook::new();
    let sheet = book.add_worksheet();
    sheet.set_name("核算表").unwrap();
    for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标"]
        .iter()
        .enumerate()
    {
        sheet.write_string(0, c as u16, *h).unwrap();
    }
    sheet.write_string(1, 0, "销售额").unwrap();
    sheet.write_string(1, 2, "12000.5").unwrap();
    sheet.write_string(1, 3, "11500.25").unwrap();
    sheet.write_string(1, 4, "12500").unwrap();
    for i in 0..junk_rows {
        sheet
            .write_string((2 + i) as u32, 0, format!("无关科目{i:02}"))
            .unwrap();
    }
    sheet
        .write_string(note_row - 1, 0, "注：本表由财务部维护")
        .unwrap();
    book.save(path).unwrap();
}

fn payload_of(material: &dsheng_cli::store::generation::GenerationMaterial) -> Value {
    serde_json::from_str(&material.payload_json).expect("payload json")
}

fn rows_of(payload: &Value) -> std::collections::BTreeSet<u32> {
    payload["source_records"]
        .as_array()
        .expect("source_records 数组")
        .iter()
        .map(|r| {
            let a1 = r["cell_or_row"].as_str().unwrap();
            let digits: String = a1.chars().filter(|c| c.is_ascii_digit()).collect();
            digits.parse().expect("行号")
        })
        .collect()
}

#[test]
fn full_sheet_scan_trims_to_relevant_rows() {
    let s = setup(
        "trim-rows",
        &[
            ("sales_net", "销售额", "money", true),
            ("unused_metric", "未使用指标", "money", false),
        ],
    );
    accounting_workbook(&s.dir.join("核算表模板.xlsx"), 40, 200);
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");

    let conn = rusqlite::Connection::open(&s.db).unwrap();
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        s.workspace,
        &run_id,
        "（裁剪测试）",
        None,
    )
    .unwrap();
    let payload = payload_of(&material);
    let records = payload["source_records"].as_array().unwrap();
    assert!(!records.is_empty(), "相关行必须产生 source_records");
    // §1.4：metric_definitions 只发本次使用指标（未使用的不发）
    let defs = payload["workspace"]["metric_definitions"]
        .as_array()
        .unwrap();
    assert_eq!(defs.len(), 1, "未使用指标定义不得发送");
    assert_eq!(defs[0]["metric_key"], json!("sales_net"));
    assert_eq!(payload["metrics"].as_array().unwrap().len(), 1);
    // 只含表头行（1）＋销售额行（2）；40 行无关科目＋远端备注行（200）不发
    assert_eq!(rows_of(&payload), [1, 2].into_iter().collect());
    // 表头与指标行均在（逐格）
    assert!(records
        .iter()
        .any(|r| r["cell_or_row"] == json!("A1") && r["values"]["raw_text"] == json!("科目")));
    assert!(records.iter().any(
        |r| r["cell_or_row"] == json!("D2") && r["values"]["typed_value"] == json!("11500.25")
    ));
    // evidence_id 合同形状：cell:<uuid>:<sheet>:<A1>
    let evidence = records[0]["evidence_id"].as_str().unwrap();
    assert!(evidence.starts_with("cell:") && evidence.contains(":核算表:A1"));
    // manifest 如实反映裁剪后范围（收窄选定范围，而非偷偷丢行）
    let manifest = payload["source_manifest"].as_array().unwrap();
    assert_eq!(manifest.len(), 1);
    let range = manifest[0]["selected_range"].as_str().unwrap();
    assert!(
        range.contains("A1:E2") && range.contains("已按相关行裁剪：2/43 行"),
        "selected_range={range}"
    );
    assert_eq!(manifest[0]["sheet"], json!("核算表"));
    // 预览 scope 行（settings.schema 严格字段集）
    assert_eq!(material.scope_rows.len(), 1);
    assert_eq!(material.scope_rows[0]["row_count"], json!(2));
    assert_eq!(
        material.scope_rows[0]["selected_range"],
        manifest[0]["selected_range"]
    );
}

#[test]
fn user_evidence_rows_are_kept() {
    let s = setup("trim-evidence", &[("sales_net", "销售额", "money", true)]);
    accounting_workbook(&s.dir.join("核算表模板.xlsx"), 3, 12);
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");
    let conn = rusqlite::Connection::open(&s.db).unwrap();
    let state = dsheng_core::generation::input::SessionStateProjection {
        accepted_facts: vec![],
        prior_report_summary: None,
        unresolved_questions: vec![],
        pending_clarification: None,
    };
    let material = dsheng_cli::store::generation::build_diagnosis_payload(
        &conn,
        s.workspace,
        &run_id,
        dsheng_core::contracts::provider::Framework::Why5,
        "销售额为何低于预定",
        &[],
        &["cell:核算表模板.xlsx:核算表:A12".to_string()],
        state,
        None,
        "销售额为何低于预定",
    )
    .unwrap();
    let payload = payload_of(&material);
    // 表头(1)＋指标行(2)＋用户证据行(12)；3 行无关科目(3..=5)不发
    assert_eq!(rows_of(&payload), [1, 2, 12].into_iter().collect());
    assert!(payload["source_records"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["cell_or_row"] == json!("A12")));
}

#[test]
fn scanned_transposed_trend_sheet_keeps_current_and_baseline_rows() {
    let s = setup(
        "trim-transposed",
        &[
            ("sales_net", "销售额", "money", true),
            ("gross_profit", "毛利额", "money", false),
        ],
    );
    {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.set_name("推移表").unwrap();
        // 表头行（A=period，B/C=指标列·带单位后缀）
        sheet.write_string(0, 0, "月份").unwrap();
        sheet.write_string(0, 1, "销售额(元)").unwrap();
        sheet.write_string(0, 2, "毛利额(元)").unwrap();
        let periods = ["2026-07", "2026-08", "2026-09", "2026-10"];
        for (i, p) in periods.iter().enumerate() {
            sheet.write_string((i + 1) as u32, 0, *p).unwrap();
            sheet
                .write_number((i + 1) as u32, 1, 1000.0 + i as f64)
                .unwrap();
            sheet
                .write_number((i + 1) as u32, 2, 300.0 + i as f64)
                .unwrap();
        }
        sheet.write_string(5, 0, "累计2026-07~2026-09").unwrap();
        sheet.write_number(5, 1, 3003.0).unwrap();
        sheet.write_number(5, 2, 903.0).unwrap();
        sheet.write_string(7, 0, "注：口径见封面").unwrap();
        book.save(s.dir.join("推移表模板.xlsx")).unwrap();
    }
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");
    let conn = rusqlite::Connection::open(&s.db).unwrap();
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        s.workspace,
        &run_id,
        "（转置裁剪测试）",
        None,
    )
    .unwrap();
    let payload = payload_of(&material);
    let records = payload["source_records"].as_array().unwrap();
    assert!(!records.is_empty());
    // 指标列（B/C）＋period 列（A）；不存在其他列的记录
    for r in records {
        let col: String = r["cell_or_row"]
            .as_str()
            .unwrap()
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        assert!(
            col == "A" || col == "B" || col == "C",
            "非指标列不得发送：{}",
            r["cell_or_row"]
        );
    }
    // 只发送表头、本期十月及环比九月锚点；累计和无期间注释不发送。
    let rows: std::collections::BTreeSet<u32> = rows_of(&payload);
    assert_eq!(rows, [1, 4, 5].into_iter().collect());
    assert!(records
        .iter()
        .any(|r| r["cell_or_row"] == "B5" && r["values"]["raw_text"] == "1003"));
    assert!(records
        .iter()
        .any(|r| r["cell_or_row"] == "B4" && r["values"]["raw_text"] == "1002"));
    assert!(!records
        .iter()
        .any(|r| r["values"]["raw_text"] == "1000" || r["values"]["raw_text"] == "3003"));
    let manifest = payload["source_manifest"].as_array().unwrap();
    assert_eq!(manifest.len(), 1);
    let range = manifest[0]["selected_range"].as_str().unwrap();
    assert!(
        range.contains("已按指标列裁剪：3/3 列"),
        "selected_range={range}"
    );
}

#[test]
fn conflict_candidate_rows_kept_across_sources() {
    let s = setup("trim-conflict", &[("sales_net", "销售额", "money", true)]);
    // 两文件同指标同期间不同值 → 冲突候选（两文件的相关行都保留）
    accounting_workbook(&s.dir.join("核算表模板.xlsx"), 1, 8);
    {
        let mut book = Workbook::new();
        let sheet = book.add_worksheet();
        sheet.set_name("核算表").unwrap();
        for (c, h) in ["科目", "单位", "月度实绩"].iter().enumerate() {
            sheet.write_string(0, c as u16, *h).unwrap();
        }
        sheet.write_string(1, 0, "销售额").unwrap();
        sheet.write_string(1, 2, "9999.99").unwrap();
        sheet.write_string(2, 0, "无关科目").unwrap();
        book.save(s.dir.join("核算表备份.xlsx")).unwrap();
    }
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");
    let conn = rusqlite::Connection::open(&s.db).unwrap();
    // 冲突确实产生（同 metric/role/period/scope 不同值）
    let conflicts: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_conflicts WHERE run_id = ?1",
            rusqlite::params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(conflicts, 1, "合成夹具须产生一处来源冲突");
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        s.workspace,
        &run_id,
        "（冲突裁剪测试）",
        None,
    )
    .unwrap();
    let payload = payload_of(&material);
    // 两个来源都在 manifest（各自表头＋指标行；无关行裁掉）
    let manifest = payload["source_manifest"].as_array().unwrap();
    let labels: Vec<&str> = manifest
        .iter()
        .filter_map(|m| m["file_label"].as_str())
        .collect();
    assert_eq!(labels.len(), 2, "两来源均须保留：{payload:?}");
    assert!(labels.contains(&"核算表模板.xlsx") && labels.contains(&"核算表备份.xlsx"));
    let records = payload["source_records"].as_array().unwrap();
    assert!(records
        .iter()
        .any(|r| r["values"]["raw_text"] == json!("9999.99")));
    assert!(records
        .iter()
        .all(|r| r["values"]["raw_text"] != json!("无关科目")));
    // 冲突投影仍在（不得丢冲突候选）
    assert_eq!(payload["source_conflicts"].as_array().unwrap().len(), 1);
}

#[test]
fn csv_source_records_use_null_sheet_and_csv_namespace() {
    let s = setup("trim-csv", &[("sales_net", "销售额", "money", true)]);
    std::fs::write(
        s.dir.join("核算.csv"),
        "科目,月度实绩\n销售额,11500.25\n无关科目,1\n",
    )
    .unwrap();
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");
    let conn = rusqlite::Connection::open(&s.db).unwrap();
    let material = dsheng_cli::store::generation::build_task_payload(
        &conn,
        s.workspace,
        &run_id,
        "（CSV 裁剪测试）",
        None,
    )
    .unwrap();
    let payload = payload_of(&material);
    let records = payload["source_records"].as_array().unwrap();
    assert_eq!(rows_of(&payload), [1, 2].into_iter().collect());
    assert!(records.iter().all(|r| r["sheet"].is_null()));
    assert!(records[0]["evidence_id"]
        .as_str()
        .unwrap()
        .contains(":csv:"));
    let manifest = payload["source_manifest"].as_array().unwrap();
    assert_eq!(manifest[0]["sheet"], json!(null));
}

#[test]
fn rebuild_is_deterministic_and_preview_reflects_range() {
    let s = setup(
        "trim-determinism",
        &[("sales_net", "销售额", "money", true)],
    );
    accounting_workbook(&s.dir.join("核算表模板.xlsx"), 20, 30);
    let run_id = analyze(&s, "2026-10-01", "2026-10-31");
    let conn = rusqlite::Connection::open(&s.db).unwrap();

    let build = || {
        dsheng_cli::store::generation::build_diagnosis_payload(
            &conn,
            s.workspace,
            &run_id,
            dsheng_core::contracts::provider::Framework::Why5,
            "确定性",
            &[],
            &["cell:核算表模板.xlsx:核算表:A30".to_string()],
            dsheng_core::generation::input::SessionStateProjection {
                accepted_facts: vec![],
                prior_report_summary: None,
                unresolved_questions: vec![],
                pending_clarification: None,
            },
            None,
            "确定性",
        )
        .unwrap()
    };
    let m1 = build();
    let m2 = build();
    // 指纹/manifest 哈希逐字节一致；payload 剔除 session id 后同文
    assert_eq!(m1.input_hash, m2.input_hash);
    assert_eq!(m1.source_manifest_hash, m2.source_manifest_hash);
    let blank = |mut v: Value| {
        v["request"]["generation_session_id"] = json!("");
        v
    };
    assert_eq!(blank(payload_of(&m1)), blank(payload_of(&m2)));

    // CLI 预览：source_scope_summary 反映裁剪后范围与行数
    let (out, code) = run(&[
        "diagnosis",
        "generate",
        "--db",
        &s.db,
        "--workspace",
        &s.workspace.to_string(),
        "--run",
        &run_id,
        "--framework",
        "5why",
        "--problem",
        "销售额为何低于预定",
        "--evidence",
        "cell:核算表模板.xlsx:核算表:A30",
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let data = data_of(&out);
    let scope = data["source_scope_summary"].as_array().unwrap();
    assert_eq!(scope.len(), 1);
    let range = scope[0]["selected_range"].as_str().unwrap();
    assert!(
        range.contains("已按相关行裁剪：3/23 行") && range.contains("A1:E30"),
        "scope={range}"
    );
    assert_eq!(scope[0]["row_count"], json!(3));
    assert!(data["request_bytes"].as_u64().unwrap() > 0);
    assert!(data["conservative_input_tokens"].as_u64().unwrap() > 0);
}
