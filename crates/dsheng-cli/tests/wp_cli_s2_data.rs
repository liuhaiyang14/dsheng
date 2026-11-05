//! S2 集成测试（测试层次：进程内 CLI＋临时 SQLite＋合成 XLSX；无网络）。
//!
//! 覆盖：metric set/list、scan（含 unsupported pending）、analyze（本地计算＋
//! decimal 字符串＋冲突检测）、analysis show、history list/show/revisions/
//! compare/delete（预览-确认）、manual-input、source-conflict resolve、
//! mapping show/set、task reconcile dry-run/apply、task show/update/step/timeline。

use serde_json::{json, Value};

fn run(args: &[&str]) -> (String, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    (String::from_utf8(out).expect("utf8"), code)
}

fn ok(stdout: &str) -> Value {
    let v: Value = serde_json::from_str(stdout.trim()).expect("单个 JSON envelope");
    assert_eq!(v["ok"], true, "stdout: {stdout}");
    v["data"].clone()
}

fn err_code(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout.trim()).expect("单个 JSON envelope");
    assert_eq!(v["ok"], false, "stdout: {stdout}");
    v["error"]["code"].as_str().expect("code").to_string()
}

/// 测试夹具：workspace＋指标定义＋合成核算表 XLSX（数值写文本格保证词法精确）。
struct Fixture {
    db: String,
    dir: std::path::PathBuf,
    workspace: String,
    metric_id: String,
}

impl Fixture {
    fn new(tag: &str, conflicting_second_file: bool) -> Self {
        let tmp = tempfile::tempdir().expect("临时目录");
        let dir = tmp.keep();
        let db = dir
            .join(format!("{tag}.sqlite3"))
            .to_string_lossy()
            .to_string();
        // workspace
        let ws_input = json!({
            "schema_version": "1", "operation": "workspace_create", "name": "合成门店",
            "initial_period_policy": {"cadence": "month", "effective_from": "2026-01-01"}
        });
        write_file(&dir, "ws.json", &ws_input.to_string());
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
        let data = ok(&out);
        let workspace = data["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .to_string();
        // 指标定义（sales_net：收入下限资格）
        let metric_input = json!({
            "metric_key": "sales_net", "label": "销售额",
            "semantic_type": "flow",
            "unit_signature": {"dimension": "money", "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"},
            "direction": "up", "target_constraint_scope": "monthly_revenue_floor",
            "revenue_semantics": true, "aggregation": "sum_disjoint",
            "required_inputs": [],
            "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
            "effective_from": "2026-01-01", "expected_revision": 0
        });
        write_file(&dir, "metric.json", &metric_input.to_string());
        let (out, code) = run(&[
            "metric",
            "set",
            "--db",
            &db,
            "--workspace",
            &workspace,
            "--input-json",
            &dir.join("metric.json").to_string_lossy(),
            "--json",
        ]);
        assert_eq!(code, 0, "metric set: {out}");
        let metric_id = ok(&out)["metric_id"].as_str().unwrap().to_string();
        // 合成核算表（文本数值：词法精确）
        let mut book = rust_xlsxwriter::Workbook::new();
        let sheet = book.add_worksheet().set_name("核算表").unwrap();
        sheet.write_string(0, 0, "科目").unwrap();
        sheet.write_string(0, 1, "单位").unwrap();
        sheet.write_string(0, 2, "月度预定").unwrap();
        sheet.write_string(0, 3, "月度实绩").unwrap();
        sheet.write_string(0, 4, "年度计划·当月目标").unwrap();
        sheet.write_string(1, 0, "销售额").unwrap();
        sheet.write_string(1, 1, "元").unwrap();
        sheet.write_string(1, 2, "12000.5").unwrap();
        sheet.write_string(1, 3, "11500.25").unwrap();
        sheet.write_string(1, 4, "12500").unwrap();
        sheet.write_string(2, 0, "未注册科目").unwrap();
        sheet.write_string(2, 3, "999").unwrap();
        book.save(dir.join("核算表模板.xlsx")).unwrap();
        if conflicting_second_file {
            let mut book2 = rust_xlsxwriter::Workbook::new();
            let sheet2 = book2.add_worksheet().set_name("核算表").unwrap();
            sheet2.write_string(0, 0, "科目").unwrap();
            sheet2.write_string(0, 1, "单位").unwrap();
            sheet2.write_string(0, 2, "月度预定").unwrap();
            sheet2.write_string(0, 3, "月度实绩").unwrap();
            sheet2.write_string(1, 0, "销售额").unwrap();
            sheet2.write_string(1, 3, "11800.75").unwrap();
            book2.save(dir.join("冲突表.xlsx")).unwrap();
        }
        // unsupported 扩展名文件
        write_file(&dir, "说明.pptx", "ppt");
        Self {
            db,
            dir,
            workspace,
            metric_id,
        }
    }
}

fn write_file(dir: &std::path::Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).expect("写测试文件");
}

#[test]
fn s2_metric_list_and_rejections() {
    let f = Fixture::new("metric", false);
    // list
    let (out, code) = run(&[
        "metric",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--json",
    ]);
    assert_eq!(code, 0);
    let data = ok(&out);
    assert_eq!(data["metrics"].as_array().map(Vec::len), Some(1));
    assert_eq!(data["metrics"][0]["metric_key"], "sales_net");
    assert!(data["metrics"][0]["semantic_hash"].as_str().unwrap().len() == 64);
    // 版本化：expected_revision=1 → version 2
    let v2 = json!({
        "metric_key": "sales_net", "label": "销售额（新口径）",
        "semantic_type": "flow",
        "unit_signature": {"dimension": "money", "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"},
        "direction": "up", "target_constraint_scope": "monthly_revenue_floor",
        "revenue_semantics": true, "aggregation": "sum_disjoint",
        "required_inputs": [],
        "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
        "effective_from": "2026-11-01", "expected_revision": 1
    });
    write_file(&f.dir, "metric2.json", &v2.to_string());
    let (out, code) = run(&[
        "metric",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--input-json",
        &f.dir.join("metric2.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(ok(&out)["version"], 2);
    // 修订冲突
    let (out, code) = run(&[
        "metric",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--input-json",
        &f.dir.join("metric2.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "REVISION_CONFLICT");
    // 未知字段拒绝
    let bad = json!({
        "metric_key": "x", "label": "x", "semantic_type": "flow",
        "unit_signature": {"dimension": "money", "base_unit": "元", "currency": "CNY", "tax_basis": "exclusive"},
        "direction": "up", "target_constraint_scope": "explicit_target_only",
        "aggregation": "sum_disjoint", "comparison_policy": {"positive_base": "relative_change", "zero_or_negative_base": "absolute_only"},
        "effective_from": "2026-01-01", "expected_revision": 0, "surprise": true
    });
    write_file(&f.dir, "metric_bad.json", &bad.to_string());
    let (out, code) = run(&[
        "metric",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--input-json",
        &f.dir.join("metric_bad.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "INVALID_ARGUMENT");
}

#[test]
fn s2_scan_summary_view() {
    let f = Fixture::new("scan", false);
    let (out, code) = run(&[
        "scan",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let data = ok(&out);
    assert_eq!(data["workspace_id"], f.workspace);
    assert_eq!(data["files"].as_array().map(Vec::len), Some(1), "仅 xlsx");
    let file = &data["files"][0];
    assert_eq!(file["file_label"], "核算表模板.xlsx");
    assert_eq!(file["mapping_status"], "mapped");
    assert!(file["sheet_count"].as_u64().unwrap() >= 1);
    assert!(
        file["file_id"].as_str().is_some(),
        "scanFileInfo 用 file_id"
    );
    assert!(
        file["note"]
            .as_str()
            .is_some_and(|n| n.contains("核算表:accounting")),
        "sheet 角色摘要进 note: {}",
        file["note"]
    );
    assert!(data["pending_inputs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| { p.as_str().unwrap().contains("unsupported_file_kind") }));
}

#[test]
fn s2_analyze_local_computation_and_views() {
    let f = Fixture::new("analyze", false);
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "analyze: {out}");
    let data = ok(&out);
    let run_id = data["run_id"].as_str().unwrap().to_string();
    assert_eq!(data["workspace_id"], f.workspace);
    assert_eq!(data["current_revision_no"], 1);
    assert_eq!(data["grain"], "month");
    assert_eq!(data["period"]["from"], "2026-10-01");
    let metrics = data["metrics"].as_array().unwrap();
    assert_eq!(metrics.len(), 1, "未注册科目不参与计算");
    let m = &metrics[0];
    assert_eq!(m["metric_key"], "sales_net");
    assert_eq!(
        m["actual_value"], "11500.25",
        "decimal 字符串（文本词法精确）"
    );
    assert_eq!(
        m["target_value"], "12000.5",
        "显式目标保留原值；下限只影响候选（targets.rs 语义）"
    );
    assert_eq!(m["delta"], "-500.25");
    // mom 无基期观察 → unavailable/missing_base
    assert_eq!(m["mom"]["availability_status"], "unavailable");
    assert_eq!(m["mom"]["reason"], "missing_base");
    assert_eq!(m["mom"]["base_value"], Value::Null);
    // series 单桶
    let series = data["series"].as_array().unwrap();
    assert_eq!(series.len(), 1);
    assert_eq!(series[0]["actual"][0], "11500.25");
    assert_eq!(series[0]["plan"][0], "12000.5");

    // analysis show（当前 revision）
    let (out, code) = run(&[
        "analysis",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(ok(&out)["metrics"].as_array().unwrap().len(), 1);

    // 重做：同 key 追加 revision 2（保留历史快照）
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(ok(&out)["current_revision_no"], 2);
    let same_run = ok(&out)["run_id"].as_str().unwrap().to_string();
    assert_eq!(same_run, run_id, "同 period_key 保留 run_id");

    // history list/show/revisions/compare
    let (out, code) = run(&[
        "history",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--json",
    ]);
    assert_eq!(code, 0);
    let runs = ok(&out)["runs"].as_array().unwrap().clone();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["current_revision_no"], 2);
    assert!(runs[0]["summary"]["metric_count"].as_u64().unwrap() >= 1);

    let (out, code) = run(&[
        "history",
        "revisions",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(ok(&out)["revisions"].as_array().unwrap().len(), 2);

    let (out, code) = run(&[
        "history",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--revision",
        "1",
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(ok(&out)["analysis_revision_no"], 1);

    let (out, code) = run(&[
        "history",
        "compare",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--type",
        "mom",
        "--json",
    ]);
    assert_eq!(code, 0);
    let comps = ok(&out)["comparisons"].as_array().unwrap().clone();
    assert_eq!(comps.len(), 1);
    assert_eq!(comps[0]["comparison_type"], "mom");

    // mapping show（run 当前 revision 快照）
    let (out, code) = run(&[
        "mapping",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    let mapping = ok(&out);
    assert_eq!(mapping["pending_draft"], false);
    assert!(!mapping["mappings"].as_array().unwrap().is_empty());

    // mapping set 草稿（版本化）
    let draft = json!({
        "expected_revision": 2,
        "mappings": [
            {"tool_field": "actual", "source_label": "本月实绩", "recognized": "D", "usage": "月度实绩"}
        ]
    });
    write_file(&f.dir, "mapping.json", &draft.to_string());
    let (out, code) = run(&[
        "mapping",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("mapping.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(ok(&out)["revision"], 3);
    // 错误 expected revision → 冲突
    let (out, code) = run(&[
        "mapping",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("mapping.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "REVISION_CONFLICT");
}

#[test]
fn s2_source_conflict_flow() {
    let f = Fixture::new("conflict", true);
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let run_id = ok(&out)["run_id"].as_str().unwrap().to_string();
    // 冲突：销售额同期间 11500.25 vs 11800.75 → 值 null，冲突组保留双方
    let data = ok(&out);
    assert_eq!(data["metrics"][0]["actual_value"], Value::Null);
    // conflict list
    let (out, code) = run(&[
        "source-conflict",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    let conflicts = ok(&out)["conflicts"].as_array().unwrap().clone();
    assert_eq!(conflicts.len(), 1);
    let candidates = conflicts[0]["candidate_values"].as_array().unwrap().clone();
    assert_eq!(candidates.len(), 2);
    assert_eq!(conflicts[0]["resolution_status"], "unresolved");
    // resolve：选中第一证据
    let selected = candidates[0]["evidence_id"].as_str().unwrap().to_string();
    let resolve = json!({
        "conflict_id": conflicts[0]["conflict_id"], "selected_evidence_id": selected,
        "resolution_note": "以核算表模板为准", "expected_revision": 1
    });
    write_file(&f.dir, "resolve.json", &resolve.to_string());
    let (out, code) = run(&[
        "source-conflict",
        "resolve",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("resolve.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    // 非候选证据拒绝
    let bad = json!({
        "conflict_id": conflicts[0]["conflict_id"], "selected_evidence_id": "cell:不存在",
        "resolution_note": "x", "expected_revision": 1
    });
    write_file(&f.dir, "resolve_bad.json", &bad.to_string());
    let (out, code) = run(&[
        "source-conflict",
        "resolve",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("resolve_bad.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 3);
    assert_eq!(err_code(&out), "DATA_VALIDATION_FAILED");
}

#[test]
fn s2_manual_input_flow() {
    let f = Fixture::new("manual", false);
    let period_key = "pk-manual-1";
    let input = json!({
        "metric_id": f.metric_id, "measure_role": "actual", "scope_key": "全公司",
        "value": "12345.6", "confirmed": true, "source_note": "财务口述补录",
        "expected_revision": 0
    });
    write_file(&f.dir, "mi.json", &input.to_string());
    let (out, code) = run(&[
        "manual-input",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--period-key",
        period_key,
        "--input-json",
        &f.dir.join("mi.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(ok(&out)["input_revision"], 1);
    // list（decimal 字符串）
    let (out, code) = run(&[
        "manual-input",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--period-key",
        period_key,
        "--json",
    ]);
    assert_eq!(code, 0);
    let inputs = ok(&out)["inputs"].as_array().unwrap().clone();
    assert_eq!(inputs[0]["value"], "12345.6");
    assert!(inputs[0]["evidence_id"]
        .as_str()
        .unwrap()
        .starts_with("manual:"));
    // 修订冲突（旧 expected_revision）
    let (out, code) = run(&[
        "manual-input",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--period-key",
        period_key,
        "--input-json",
        &f.dir.join("mi.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "REVISION_CONFLICT");
    // 缺 source_note 拒绝
    let bad = json!({
        "metric_id": f.metric_id, "measure_role": "actual", "scope_key": "全公司",
        "value": "1", "confirmed": true, "source_note": "", "expected_revision": 1
    });
    write_file(&f.dir, "mi_bad.json", &bad.to_string());
    let (out, code) = run(&[
        "manual-input",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--period-key",
        period_key,
        "--input-json",
        &f.dir.join("mi_bad.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 3);
    assert_eq!(err_code(&out), "DATA_VALIDATION_FAILED");
}

#[test]
fn s2_task_reconcile_and_lifecycle() {
    let f = Fixture::new("task", false);
    // 分析 run（reconcile 的指纹门）
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let data = ok(&out);
    let run_id = data["run_id"].as_str().unwrap().to_string();
    let fingerprint = data["analysis_input_fingerprint"]
        .as_str()
        .unwrap()
        .to_string();
    // 持久化合成候选（模拟已保存的模型输出；测试层次：DB 夹具注入）
    let candidate_id = uuid::Uuid::new_v4().to_string();
    let candidate_output = json!({
        "summary": "下月追赶候选",
        "tasks": [{
            "task_key": "t-sales-1", "title": "下月销售额追赶", "summary": "冲刺下月目标",
            "category": "销售最大化", "metric_id": f.metric_id,
            "target_value": 13000, "unit": "元", "owner": "张三",
            "planned_start": "2026-11-01", "due_date": "2026-11-30",
            "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
            "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
            "steps": [{"step_id": "s1", "order": 1, "action": "加大老客拜访", "owner": "张三",
                        "start_date": "2026-11-02", "due_date": "2026-11-10",
                        "done_criteria": "完成10家拜访", "evidence_ids": []}],
            "evidence_ids": [], "assumptions": [], "needs_review": []
        }]
    });
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
        conn.execute(
            "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id,
                    analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                    candidate_revision_no, attempt_id, output_json,
                    envelope_locally_reconstructed, state, prompt_version, schema_version, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, ?5, 1, 'attempt-fixture', ?6, 0, 'candidate', '2.3', '2', ?7)",
            rusqlite::params![
                candidate_id,
                f.workspace,
                run_id,
                fingerprint,
                format!("gen-{fingerprint}"),
                candidate_output.to_string(),
                dsheng_core::repository::utc_iso(chrono::Utc::now())
            ],
        )
        .unwrap();
    }
    // dry-run
    let reconcile_input = json!({
        "schema_version": "1", "workspace_id": f.workspace, "run_id": run_id,
        "analysis_revision_no": 1, "input_fingerprint": fingerprint,
        "candidate_generation_id": candidate_id, "candidate_revision_no": 1,
        "expected_tasks": [], "mutation_id": uuid::Uuid::new_v4().to_string(),
        "mappings": [{"candidate_task_key": "t-sales-1", "action": "new"}]
    });
    write_file(&f.dir, "reconcile.json", &reconcile_input.to_string());
    let (out, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("reconcile.json").to_string_lossy(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "reconcile dry-run: {out}");
    let preview = ok(&out);
    let token = preview["apply_token"].as_str().unwrap().to_string();
    assert_eq!(preview["created_new"].as_array().unwrap().len(), 1);
    // 无 token 的 apply 拒绝（双确认）
    let (out, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("reconcile.json").to_string_lossy(),
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "CONFIRMATION_REQUIRED");
    // apply（带 token）
    let mut with_token = reconcile_input.clone();
    with_token["apply_token"] = json!(token);
    write_file(&f.dir, "reconcile_apply.json", &with_token.to_string());
    let (out, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--input-json",
        &f.dir.join("reconcile_apply.json").to_string_lossy(),
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 0, "reconcile apply: {out}");
    let applied = ok(&out);
    assert_eq!(applied["new_task_ids"].as_array().unwrap().len(), 1);
    let task_id = applied["new_task_ids"][0].as_str().unwrap().to_string();

    // task show（read-views.taskView 字段）
    let (out, code) = run(&[
        "task",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--task",
        &task_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    let task = ok(&out);
    assert_eq!(task["title"], "下月销售额追赶");
    assert_eq!(task["category"], "revenue");
    assert_eq!(task["target_value"], "13000");
    assert_eq!(task["steps"].as_array().unwrap().len(), 1);
    assert_eq!(task["status"], "not_started");
    // task list（run 过滤）
    let (out, code) = run(&[
        "task",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0);
    assert_eq!(ok(&out)["tasks"].as_array().unwrap().len(), 1);

    // sugar 更新（--status/--progress/--expected-revision → 同一 task-edit 合同）
    let (out, code) = run(&[
        "task",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--task",
        &task_id,
        "--status",
        "in_progress",
        "--progress",
        "50",
        "--progress-mode",
        "manual",
        "--adjustment-reason",
        "前半完成",
        "--expected-revision",
        "1",
        "--json",
    ]);
    assert_eq!(code, 0, "sugar update: {out}");
    assert_eq!(ok(&out)["task_revision_no"], 2);
    assert_eq!(ok(&out)["progress"], "50");
    // 缺 expected-revision 拒绝
    let (out, code) = run(&[
        "task",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--task",
        &task_id,
        "--status",
        "in_progress",
        "--progress",
        "60",
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "INVALID_ARGUMENT");

    // step add → step update → step move → step delete（最后一步删除后须保留 ≥1 步）
    let add = json!({"action": "复盘并更新话术", "owner": "李四",
                      "start_date": "2026-11-20", "due_date": "2026-11-25"});
    write_file(&f.dir, "step_add.json", &add.to_string());
    let (out, code) = run(&[
        "task",
        "step",
        "add",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--task",
        &task_id,
        "--input-json",
        &f.dir.join("step_add.json").to_string_lossy(),
        "--json",
    ]);
    assert_eq!(code, 0, "step add: {out}");
    // 读取 step id（show）
    let (out2, _) = run(&[
        "task",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--task",
        &task_id,
        "--json",
    ]);
    let steps = ok(&out2)["steps"].as_array().unwrap().clone();
    assert_eq!(steps.len(), 2);
    let new_step = steps[1]["step_id"].as_str().unwrap().to_string();
    // step update（状态完成）
    let patch = json!({"status": "completed"});
    write_file(&f.dir, "step_patch.json", &patch.to_string());
    let revision = ok(&out2)["task_revision_no"].as_u64().unwrap();
    let (out, code) = run(&[
        "task",
        "step",
        "update",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--step",
        &new_step,
        "--patch-json",
        &f.dir.join("step_patch.json").to_string_lossy(),
        "--expected-revision",
        &revision.to_string(),
        "--json",
    ]);
    assert_eq!(code, 0, "step update: {out}");
    // step delete（保留至少一步）
    let (out, code) = run(&[
        "task",
        "step",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--step",
        &new_step,
        "--expected-revision",
        &(revision + 1).to_string(),
        "--json",
    ]);
    assert_eq!(code, 0, "step delete: {out}");

    // timeline（planned_start/due 覆盖 11 月）
    let (out, code) = run(&[
        "task",
        "timeline",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--from",
        "2026-11-01",
        "--to",
        "2026-11-30",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0);
    let entries = ok(&out)["entries"].as_array().unwrap().clone();
    assert!(
        entries.iter().any(|e| e["kind"] == "planned_start"),
        "{entries:?}"
    );
    assert!(entries.iter().any(|e| e["kind"] == "due"));
}

#[test]
fn s2_history_delete_preview_confirm() {
    let f = Fixture::new("del", false);
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-09-01",
        "--to",
        "2026-09-30",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let run_id = ok(&out)["run_id"].as_str().unwrap().to_string();
    // dry-run：预览 id + scope
    let (out, code) = run(&[
        "history",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let preview = ok(&out);
    assert!(preview["blocked"].is_null(), "无任务/请求不阻断");
    let preview_id = preview["delete_preview_id"].as_str().unwrap().to_string();
    assert!(preview["scope"]["run_revisions"].as_u64().unwrap() >= 1);
    // 无 preview id 的 confirm 拒绝（flag 互斥由 clap/校验保证）
    let (out, code) = run(&[
        "history",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--confirm-delete",
        "00000000-0000-4000-8000-0000000000ff",
        "--json",
    ]);
    assert_eq!(code, 2, "DELETE_PREVIEW_STALE 退出码 2");
    assert_eq!(err_code(&out), "DELETE_PREVIEW_STALE");
    // 确认删除
    let (out, code) = run(&[
        "history",
        "delete",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--confirm-delete",
        &preview_id,
        "--json",
    ]);
    assert_eq!(code, 0, "confirm delete: {out}");
    assert_eq!(ok(&out)["deleted"], true);
    // 已删除 → 再查 run 报错
    let (out, code) = run(&[
        "analysis",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 2);
    assert_eq!(err_code(&out), "WORKSPACE_MISMATCH");
}

#[test]
fn s2_diagnosis_local_read_and_revise() {
    let f = Fixture::new("diag", false);
    let (out, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "auto",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let run_id = ok(&out)["run_id"].as_str().unwrap().to_string();
    // 注入合成 session/report/turn（模拟一次已完成的本地会话；DB 夹具注入）
    let session_id = uuid::Uuid::new_v4().to_string();
    let report_id = uuid::Uuid::new_v4().to_string();
    let turn_id = uuid::Uuid::new_v4().to_string();
    let report_json = json!({
        "schema_version":"3", "report_id":report_id, "generation_session_id":session_id,
        "workspace_id":f.workspace, "run_id":run_id, "framework":"5why",
        "scope":{"problem_statement":"销售额未达下限","focus_metric_ids":[]},
        "period":{"from":"2026-10-01","to":"2026-10-31","period_policy_version":"1"},
        "revision_no":1,"summary":"合成报告","source_manifest":[],
        "findings":[{"finding_id":"f1","level":1,"question":"为什么下降？","answer":"客流减少",
            "fact_status":"hypothesis","evidence_ids":[],"reasoning_summary":"待核对",
            "confidence":"low","validation_question":"核对客流？","status":"candidate"}],
        "facts_and_hypotheses":[],"evidence_refs":[],"clarification_questions":[],"warnings":[],
        "stop_reason":null,"can_continue":true,"created_at":"2026-11-03T00:00:00Z",
        "updated_at":"2026-11-03T00:00:00Z","analysis_revision_no":1,"analysis_input_fingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        dsheng_core::repository::apply_migrations(&conn, "test").unwrap();
        let now = dsheng_core::repository::utc_iso(chrono::Utc::now());
        conn.execute(
            "INSERT INTO generation_sessions (session_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, generation_input_fingerprint, workflow, framework,
                    state, prompt_version, schema_version, input_schema_version,
                    context_policy_version, source_manifest_hash, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 'fp', 'gfp', 'diagnosis', '5why', 'active', '2.0', '3', '2', '2', 'h', ?4, ?4)",
            rusqlite::params![session_id, f.workspace, run_id, now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diagnosis_reports (report_id, workspace_id, run_id, analysis_revision_no,
                    analysis_input_fingerprint, report_revision_no, session_id, framework,
                    problem_statement, report_json, state, created_at, updated_at)
             VALUES (?1, ?2, ?3, 1, 'fp', 1, ?4, '5why', '销售额未达下限', ?5, 'accepted', ?6, ?6)",
            rusqlite::params![
                report_id,
                f.workspace,
                run_id,
                session_id,
                report_json.to_string(),
                now
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO diagnosis_turns (turn_id, session_id, workspace_id, turn_no, round_no,
                    user_message, turn_kind, turn_state, accepted_facts_json, context_manifest_json,
                    prompt_version, schema_version, created_at)
             VALUES (?1, ?2, ?3, 1, 1, '请分析', 'clarification', 'pending', '[]', '{}', '2.0', '3', ?4)",
            rusqlite::params![turn_id, session_id, f.workspace, now],
        )
        .unwrap();
    }
    // list
    let (out, code) = run(&[
        "diagnosis",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--run",
        &run_id,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let reports = ok(&out)["reports"].as_array().unwrap().clone();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["framework"], "5why");
    assert_eq!(reports[0]["can_continue"], true);
    // show
    let (out, code) = run(&[
        "diagnosis",
        "show",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--report",
        &report_id,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert_eq!(ok(&out)["scope"]["problem_statement"], "销售额未达下限");
    // export md
    let out_path = f.dir.join("report.md");
    let (out, code) = run(&[
        "diagnosis",
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--report",
        &report_id,
        "--format",
        "md",
        "--out",
        out_path.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let content = std::fs::read_to_string(out_path).unwrap();
    assert!(content.contains("销售额未达下限"));
    // export pdf 明确拒绝（未实现不冒充）
    let (out, code) = run(&[
        "diagnosis",
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--report",
        &report_id,
        "--format",
        "pdf",
        "--out",
        f.dir.join("x.pdf").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 5);
    assert_eq!(err_code(&out), "EXPORT_FAILED");
    // pending cancel（expected_revision=当前报告 revision 1）
    let (out, code) = run(&[
        "diagnosis",
        "pending",
        "cancel",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--session",
        &session_id,
        "--turn",
        &turn_id,
        "--expected-revision",
        "1",
        "--confirm-local",
        "--json",
    ]);
    assert_eq!(code, 0, "pending cancel: {out}");
    assert_eq!(ok(&out)["cleared"], true);
    // revise（stdin 声明经文件替代不可；此处用 --replacement-stdin 且进程内无 stdin →
    // 校验路径走 read_stdin_text：测试环境 stdin 为空 → 拒绝空声明）
    let (out, code) = run(&[
        "diagnosis",
        "revise",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace,
        "--session",
        &session_id,
        "--fact",
        "fact-1",
        "--replacement-stdin",
        "--json",
    ]);
    // 进程内 stdin 为空：DATA_VALIDATION_FAILED（空更正声明）
    assert_eq!(code, 3);
    assert_eq!(err_code(&out), "DATA_VALIDATION_FAILED");
}
