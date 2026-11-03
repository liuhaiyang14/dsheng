//! S4/S5 导出测试（测试层次：CLI 运行器＋共享导出库＋临时 SQLite；无网络）。
//!
//! 往返（§5/§6 验收）：模板包 → 解包核对布局（元数据键位/列名/技术列）→
//! 以模板同布局填合成值（等价于用户在表格软件中填写）→ `analyze` 导入 →
//! 导出（bundle/workbook/meeting/chart）→ 用 calamine 逐格核对
//! 布局/空值保留/decimal 精度（含 >15 位有效数 decimal_text）/flow 累计；
//! 公式注入防护；Key 与本机绝对路径绝不进导出。

mod fixture;

use fixture::Fixture;
use serde_json::Value;

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

fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut reader = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).expect("zip 可读");
    let mut out = Vec::new();
    for i in 0..reader.len() {
        let mut e = reader.by_index(i).expect("条目");
        let name = e.name().map(|n| n.to_string()).unwrap_or_default();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut e, &mut buf).expect("读条目");
        out.push((name, buf));
    }
    out
}

/// calamine 读取 sheet 网格（raw 文本）。
fn read_sheet(path: &std::path::Path, sheet: &str) -> Vec<Vec<String>> {
    use calamine::Reader;
    let mut book: calamine::Xlsx<std::io::BufReader<std::fs::File>> = calamine::Xlsx::new(
        std::io::BufReader::new(std::fs::File::open(path).expect("打开 xlsx")),
    )
    .expect("xlsx 可读");
    let range = book.worksheet_range(sheet).expect("sheet 存在");
    let (rows, cols) = range.get_size();
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| match range.get_value((r as u32, c as u32)) {
                    Some(calamine::Data::Empty) => String::new(),
                    Some(calamine::Data::String(s)) => s.clone(),
                    Some(calamine::Data::Int(i)) => i.to_string(),
                    Some(calamine::Data::Float(f)) => {
                        let s = format!("{f}");
                        if s.ends_with(".0") {
                            s.trim_end_matches(".0").to_string()
                        } else {
                            s
                        }
                    }
                    Some(other) => other.to_string(),
                    None => String::new(),
                })
                .collect()
        })
        .collect()
}

#[test]
fn s4_template_zip_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("import-template-v1.zip");
    let (stdout, code) = run(&[
        "template",
        "download",
        "--package",
        "import",
        "--out",
        out.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let entries = unzip(&std::fs::read(&out).unwrap());
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"核算表模板.xlsx"), "{names:?}");
    assert!(names.contains(&"推移表模板.xlsx"));
    assert!(names.contains(&"任务单模板.xlsx"));
    assert!(names.contains(&"字段说明.md"));
    // 空白布局：A1 键、B1=1、第 6 行列名、第 7 行起无数据
    let acc = tmp.path().join("核算表模板.xlsx");
    std::fs::write(
        &acc,
        entries
            .iter()
            .find(|(n, _)| n == "核算表模板.xlsx")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    let grid = read_sheet(&acc, "核算表");
    assert_eq!(grid[0][0], "dsheng_template_version");
    assert_eq!(grid[0][1], "1");
    assert_eq!(grid[2][0], "source_from");
    assert_eq!(grid[3][6], "coverage_status");
    assert_eq!(grid[5][0], "科目");
    assert_eq!(grid[5][6], "月度实绩");
    assert!(grid.len() == 6, "空白模板第 7 行起无数据: {}", grid.len());
    // 字段说明含技术列与防护规则
    let md = String::from_utf8(
        entries
            .iter()
            .find(|(n, _)| n == "字段说明.md")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    assert!(md.contains("task_id"));
    assert!(md.contains("decimal_text"));
}

#[test]
fn s4_roundtrip_template_fill_import_export() {
    let f = Fixture::new("s4-roundtrip");
    // 1) 导出业务包与任务工作簿（导入侧数据由 Fixture 的 analyze 产生）
    let out_dir = &f.dir;
    let bundle = out_dir.join("bundle.zip");
    let (stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--bundle",
        "business",
        "--out",
        bundle.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(data_of(&stdout)["kind"], "bundle");
    let entries = unzip(&std::fs::read(bundle).unwrap());
    let by_name = |n: &str| -> Vec<u8> { entries.iter().find(|(k, _)| k == n).expect(n).1.clone() };
    // 2) 布局核对：核算表 sheet 元数据/列名/数据行
    let acc_path = out_dir.join("核算表.xlsx");
    std::fs::write(&acc_path, by_name("核算表.xlsx")).unwrap();
    let grid = read_sheet(&acc_path, "核算表");
    assert_eq!(grid[0][0], "dsheng_template_version");
    assert_eq!(grid[1][0], "workspace_name");
    assert_eq!(grid[2][0], "source_from");
    assert_eq!(grid[2][1], "2026-10-01");
    assert_eq!(grid[2][5], "month");
    assert_eq!(grid[3][0], "timezone");
    assert_eq!(grid[5][6], "月度实绩");
    assert_eq!(grid[6][0], "销售额", "第 7 行数据");
    assert_eq!(grid[6][6], "11500.25", "decimal 词法精确往返");
    // 空值保留：年度计划·当月目标列为空（视图无年度分解时不造值）
    assert_eq!(grid[6][4], "");
    // 3) 推移 sheet：单桶＋flow 累计
    let trend = read_sheet(&acc_path, "推移表");
    assert_eq!(trend[5][0], "科目");
    let data_rows: Vec<&Vec<String>> = trend
        .iter()
        .skip(6)
        .filter(|r| !r[0].is_empty() && !r[0].starts_with("注："))
        .collect();
    assert_eq!(data_rows[0][0], "销售额");
    let last_non_empty = data_rows[0]
        .iter()
        .rfind(|c| !c.is_empty())
        .expect("累计列非空");
    assert_eq!(last_non_empty, "11500.25", "flow 单桶累计=该桶值");
    // 4) metadata.json：结构/指纹/decimal_text 政策；无 Key；无本机绝对路径
    let metadata: Value = serde_json::from_slice(&by_name("metadata.json")).expect("metadata json");
    assert_eq!(metadata["run_id"], f.run_id);
    assert_eq!(metadata["period"]["source_from"], "2026-10-01");
    assert!(
        metadata["analysis_input_fingerprint"]
            .as_str()
            .map(str::len)
            == Some(64)
    );
    assert!(!metadata["source_manifests"].as_array().unwrap().is_empty());
    let raw = String::from_utf8(by_name("metadata.json")).unwrap();
    assert!(
        !raw.contains("api_key") && !raw.contains("test-key-not-real"),
        "Key 绝不进导出"
    );
    assert!(
        !raw.contains("/Users/") && !raw.contains(out_dir.to_string_lossy().as_ref() as &str),
        "无本机绝对路径"
    );
    // 5) meeting.json：10 页；第 7 页未提供
    let meeting: Value = serde_json::from_slice(&by_name("meeting.json")).unwrap();
    assert_eq!(meeting["page_count"], 10);
    assert_eq!(meeting["pages"].as_array().unwrap().len(), 10);
    assert_eq!(meeting["pages"][6]["content"]["status"], "未提供");
    // 6) 模板往返：以模板同布局填合成值 → analyze 导入 → 与导出值一致
    let fill_dir = out_dir.join("filled");
    std::fs::create_dir_all(&fill_dir).unwrap();
    let filled = fill_dir.join("核算表填写.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("核算表").unwrap();
        // 模板同布局：元数据键值对 + 第 6 行列名 + 第 7 行数据（等价用户填写）
        for (r, c, k, v) in [
            (0u32, 0u16, "dsheng_template_version", "1"),
            (1, 0, "workspace_name", "合成门店"),
            (1, 2, "currency", "CNY"),
            (1, 4, "tax_basis", "exclusive"),
            (2, 0, "source_from", "2026-10-01"),
            (2, 2, "source_to", "2026-10-31"),
            (2, 4, "grain", "month"),
            (3, 0, "timezone", "Asia/Shanghai"),
            (3, 2, "week_start_day", "1"),
            (3, 4, "fiscal_year_start_month", "1"),
            (3, 6, "coverage_status", "complete"),
            (3, 8, "empty_days_are_zero", "false"),
        ] {
            ws.write_string(r, c, k).unwrap();
            if !v.is_empty() {
                ws.write_string(r, c + 1, v).unwrap();
            }
        }
        for (c, h) in [
            "科目",
            "单位",
            "计算公式/取数来源",
            "本月MP",
            "年度计划·当月目标",
            "月度预定",
            "月度实绩",
            "预实差异额",
            "预定比",
            "年度计划比",
        ]
        .iter()
        .enumerate()
        {
            ws.write_string(5, c as u16, *h).unwrap();
        }
        ws.write_string(6, 0, "销售额").unwrap();
        ws.write_string(6, 1, "元").unwrap();
        ws.write_string(6, 5, "12000.5").unwrap();
        ws.write_string(6, 6, "11777.75").unwrap();
        book.save(&filled).unwrap();
    }
    let (stdout, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--dir",
        fill_dir.to_string_lossy().as_ref(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "填写模板导入: {stdout}");
    let imported = data_of(&stdout);
    assert_eq!(
        imported["metrics"][0]["actual_value"], "11777.75",
        "纯填写模板（单来源）导入值精确"
    );
    // 冲突场景：同目录两个来源同期间不同值 → 冲突（值 null，不造均值）
    std::fs::copy(f.dir.join("核算表模板.xlsx"), fill_dir.join("原始表.xlsx")).unwrap();
    let (stdout, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--dir",
        fill_dir.to_string_lossy().as_ref(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let conflicted = data_of(&stdout);
    assert!(
        conflicted["metrics"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["actual_value"].is_null()),
        "同键不同值必须进冲突（不造均值）"
    );
    let conflict_run = conflicted["run_id"].as_str().unwrap().to_string();
    let (stdout, code) = run(&[
        "source-conflict",
        "list",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &conflict_run,
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(
        data_of(&stdout)["conflicts"].as_array().unwrap().len(),
        1,
        "冲突组保留双方"
    );
    // 7) 精度：>15 位有效数导出为 decimal 文本并标记
    let big_dir = out_dir.join("big");
    std::fs::create_dir_all(&big_dir).unwrap();
    let big_file = big_dir.join("大数.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("核算表").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        // 17 位有效数（≤28 有效位、6 位小数上限内）
        ws.write_string(1, 2, "1234567890123456.5").unwrap();
        book.save(&big_file).unwrap();
    }
    // 独立 workspace（避免与 Fixture 来源冲突）
    std::fs::write(
        out_dir.join("ws2.json"),
        r#"{"schema_version":"1","operation":"workspace_create","name":"精度工作区","initial_period_policy":{"cadence":"month","effective_from":"2026-01-01"}}"#,
    )
    .unwrap();
    let (stdout, code) = run(&[
        "workspace",
        "create",
        "--db",
        &f.db,
        "--input-json",
        out_dir.join("ws2.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let ws2 = data_of(&stdout)["workspace"]["workspace_id"]
        .as_str()
        .unwrap()
        .to_string();
    std::fs::write(
        out_dir.join("metric2.json"),
        r#"{"metric_key":"sales_net","label":"销售额","semantic_type":"flow","unit_signature":{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"},"direction":"up","target_constraint_scope":"explicit_target_only","revenue_semantics":false,"aggregation":"sum_disjoint","required_inputs":[],"comparison_policy":{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"},"effective_from":"2026-01-01","expected_revision":0}"#,
    )
    .unwrap();
    let (stdout, code) = run(&[
        "metric",
        "set",
        "--db",
        &f.db,
        "--workspace",
        &ws2,
        "--input-json",
        out_dir.join("metric2.json").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let (stdout, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &ws2,
        "--dir",
        big_dir.to_string_lossy().as_ref(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "大数导入: {stdout}");
    let big_run = data_of(&stdout)["run_id"].as_str().unwrap().to_string();
    let big_wb = out_dir.join("big.xlsx");
    let (stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &ws2,
        "--run",
        &big_run,
        "--workbook",
        "accounting",
        "--out",
        big_wb.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    assert!(
        data_of(&stdout)["decimal_text_cells"].as_u64().unwrap() >= 1,
        "17 位有效数须标记 decimal_text"
    );
    let grid = read_sheet(&big_wb, "核算表");
    assert_eq!(grid[6][5], "1234567890123456.5", "decimal 文本保真");
}

#[test]
fn s4_formula_injection_guard_in_task_export() {
    let f = Fixture::new("s4-guard");
    // 注入公式样式标题的任务（经 reconcile 夹具落库）
    let run_id = f.run_id.clone();
    let fingerprint: String = {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        conn.query_row(
            "SELECT analysis_input_fingerprint FROM analysis_runs WHERE run_id = ?1",
            rusqlite::params![run_id],
            |r| r.get(0),
        )
        .unwrap()
    };
    let candidate_id = uuid::Uuid::new_v4().to_string();
    {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        let output = serde_json::json!({
            "summary": "注入防护夹具",
            "tasks": [{
                "task_key": "k-guard", "title": "=SUM(A1:A9)", "summary": "s", "category": "其他事项",
                "source_period": {"from": "2026-10-01", "to": "2026-10-31"},
                "target_period": {"from": "2026-11-01", "to": "2026-11-30"},
                "steps": [{"step_id": "s1", "order": 1, "action": "=cmd|'/c calc'",
                            "owner": "@SUM(1)"}],
                "evidence_ids": [], "assumptions": [], "needs_review": []
            }]
        });
        conn.execute(
            "INSERT INTO task_candidates (candidate_generation_id, workspace_id, run_id,
                    analysis_revision_no, analysis_input_fingerprint, generation_input_fingerprint,
                    candidate_revision_no, attempt_id, output_json, envelope_locally_reconstructed,
                    state, prompt_version, schema_version, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, 'g', 1, 'a', ?5, 0, 'candidate', '2.3', '2', ?6)",
            rusqlite::params![
                candidate_id,
                f.workspace.to_string(),
                run_id,
                fingerprint,
                output.to_string(),
                dsheng_core::repository::utc_iso(chrono::Utc::now())
            ],
        )
        .unwrap();
    }
    let dir = &f.dir;
    let mutation_id = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        dir.join("recon.json"),
        format!(
            r#"{{"schema_version":"1","workspace_id":"{}","run_id":"{run_id}","analysis_revision_no":1,"input_fingerprint":"{fingerprint}","candidate_generation_id":"{candidate_id}","candidate_revision_no":1,"expected_tasks":[],"mutation_id":"{mutation_id}","mappings":[{{"candidate_task_key":"k-guard","action":"new"}}]}}"#,
            f.workspace,
        ),
    )
    .unwrap();
    let (stdout, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run_id,
        "--input-json",
        dir.join("recon.json").to_string_lossy().as_ref(),
        "--dry-run",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let token = data_of(&stdout)["apply_token"]
        .as_str()
        .unwrap()
        .to_string();
    let with_token = format!(
        r#"{{"schema_version":"1","workspace_id":"{}","run_id":"{run_id}","analysis_revision_no":1,"input_fingerprint":"{fingerprint}","candidate_generation_id":"{candidate_id}","candidate_revision_no":1,"expected_tasks":[],"mutation_id":"{mutation_id}","mappings":[{{"candidate_task_key":"k-guard","action":"new"}}],"apply_token":"{token}"}}"#,
        f.workspace,
    );
    std::fs::write(dir.join("recon_apply.json"), with_token).unwrap();
    let (stdout, code) = run(&[
        "task",
        "reconcile",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run_id,
        "--input-json",
        dir.join("recon_apply.json").to_string_lossy().as_ref(),
        "--apply",
        "--json",
    ]);
    assert_eq!(code, 0, "应用候选: {stdout}");
    // 导出任务工作簿：=+-@ 开头文本必须是文本单元格
    let out = dir.join("任务单.xlsx");
    let (stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run_id,
        "--workbook",
        "task",
        "--out",
        out.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    // 目标期 2026-11 → 计划 sheet（完整期间在第 3 行）
    let plan = read_sheet(&out, "任务单计划");
    assert_eq!(plan[6][1], "=SUM(A1:A9)", "标题以 = 开头 → 文本格");
    assert_eq!(plan[6][5], "=cmd|'/c calc'", "步骤以 = 开头 → 文本格");
    assert_eq!(
        plan[6][6], "@SUM(1)",
        "责任人以 @ 开头 → 文本格（CSV 变体单引号转义）"
    );
    // CSV 防护（chart csv 变体口径）：csv_export_text 以 ' 前缀
    assert_eq!(
        dsheng_core::analysis::export_layout::csv_export_text("@SUM(1)"),
        "'@SUM(1)"
    );
}

#[test]
fn s4_meeting_markdown_and_chart_variants() {
    let f = Fixture::new("s4-meeting");
    let dir = &f.dir;
    // 双桶序列（两月推移）：手写趋势表
    let trend_dir = dir.join("trend");
    std::fs::create_dir_all(&trend_dir).unwrap();
    let trend_file = trend_dir.join("推移.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("推移表").unwrap();
        for (c, h) in ["科目", "单位", "2026-09", "2026-10"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        ws.write_string(1, 2, "11000.5").unwrap();
        ws.write_string(1, 3, "11500.25").unwrap();
        book.save(&trend_file).unwrap();
    }
    let (stdout, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--dir",
        trend_dir.to_string_lossy().as_ref(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let run2 = data_of(&stdout)["run_id"].as_str().unwrap().to_string();
    // meeting md：10 个二级标题
    let md_path = dir.join("meeting.md");
    let (stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run2,
        "--meeting",
        "--format",
        "md",
        "--out",
        md_path.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let md = std::fs::read_to_string(&md_path).unwrap();
    assert_eq!(md.matches("## ").count(), 10, "固定 10 页");
    assert!(md.contains("## 7. 已确认追赶目标/行动"));
    assert!(md.contains("未提供"));
    // chart csv / xlsx / png
    for (fmt, suffix) in [("csv", "csv"), ("xlsx", "xlsx"), ("png", "png")] {
        let out = dir.join(format!("chart.{suffix}"));
        let (stdout, code) = run(&[
            "export",
            "--db",
            &f.db,
            "--workspace",
            &f.workspace.to_string(),
            "--run",
            &run2,
            "--chart",
            "sales_net",
            "--format",
            fmt,
            "--out",
            out.to_string_lossy().as_ref(),
            "--json",
        ]);
        assert_eq!(code, 0, "chart {fmt}: {stdout}");
        assert!(std::fs::metadata(&out).unwrap().len() > 0);
    }
    let csv = std::fs::read_to_string(dir.join("chart.csv")).unwrap();
    assert!(csv.contains("11500.25") && csv.contains("11000.5"));
    // 非法：--format 与选择器不符
    let (_stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run2,
        "--meeting",
        "--format",
        "png",
        "--out",
        dir.join("x").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 2);
    // 多选择器拒绝
    let (_stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &run2,
        "--meeting",
        "--chart",
        "sales_net",
        "--format",
        "csv",
        "--out",
        dir.join("y").to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 2);
}

#[test]
fn s4_bundle_no_key_no_abs_paths() {
    let f = Fixture::new("s4-secrets");
    let out = f.dir.join("b.zip");
    let (stdout, code) = run(&[
        "export",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--run",
        &f.run_id,
        "--bundle",
        "business",
        "--out",
        out.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    for (name, bytes) in unzip(&std::fs::read(&out).unwrap()) {
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("test-key-not-real"), "{name} 泄漏 API Key");
        assert!(!text.contains("/Users/"), "{name} 含本机绝对路径");
        assert!(
            !text.contains(f.dir.to_string_lossy().as_ref() as &str),
            "{name} 含本机路径"
        );
    }
}
