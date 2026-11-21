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
    let out = tmp.path().join("source-template-v1.zip");
    let (stdout, code) = run(&[
        "template",
        "download",
        "--package", "source",
        "--profile", "manufacturing_demo",
        "--out",
        out.to_string_lossy().as_ref(),
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let entries = unzip(&std::fs::read(&out).unwrap());
    let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
    assert!(names.contains(&"manufacturing-source.xlsx"), "{names:?}");
    assert!(names.contains(&"字段说明.md"));
    // source profile 空白布局：A1 键、B1=1、第 6 行列名、第 7 行起无数据
    let acc = tmp.path().join("manufacturing-source.xlsx");
    std::fs::write(
        &acc,
        entries
            .iter()
            .find(|(n, _)| n == "manufacturing-source.xlsx")
            .unwrap()
            .1
            .clone(),
    )
    .unwrap();
    let grid = read_sheet(&acc, "source");
    assert_eq!(grid[0][0], "dsheng_template_version");
    assert_eq!(grid[0][1], "1");
    assert_eq!(grid[2][0], "source_from");
    assert_eq!(grid[3][6], "coverage_status");
    assert_eq!(grid[5][0], "期间");
    assert_eq!(grid[5][4], "月度实绩");
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
    assert!(md.contains("profile_id"));
    assert!(md.contains("output_contract_id"));
    assert!(md.contains("期间") && md.contains("月度实绩"));
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
    assert_eq!(grid[6][6], "24100.5", "两桶累计值保持十进制词法");
    // 空值保留：年度计划·当月目标列为空（视图无年度分解时不造值）
    assert_eq!(grid[6][4], "");
    // 3) 推移 sheet：双桶＋flow 累计
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
    assert_eq!(last_non_empty, "24100.5", "flow 双桶累计=两桶之和");
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
    // 6) 原始 source 往返：填写 source profile 布局 → analyze → 生成两表
    let fill_dir = out_dir.join("filled");
    std::fs::create_dir_all(&fill_dir).unwrap();
    let filled = fill_dir.join("source-filled.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("source").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标", "期间"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        ws.write_string(1, 2, "12000.5").unwrap();
        ws.write_string(1, 3, "11777.75").unwrap();
        ws.write_string(1, 4, "12500").unwrap();
        ws.write_string(1, 5, "2026-10-01").unwrap();
        ws.write_string(2, 0, "销售额").unwrap();
        ws.write_string(2, 1, "元").unwrap();
        ws.write_string(2, 2, "0").unwrap();
        ws.write_string(2, 4, "0").unwrap();
        ws.write_string(2, 5, "2026-10-02").unwrap();
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
    // 冲突场景：同目录两个 source 来源同期间不同值 → 冲突（值 null，不造均值）
    let conflict_file = fill_dir.join("source-conflict.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("source").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标", "期间"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        ws.write_string(1, 2, "12000.5").unwrap();
        ws.write_string(1, 3, "11888.75").unwrap();
        ws.write_string(1, 4, "12500").unwrap();
        ws.write_string(1, 5, "2026-10-01").unwrap();
        ws.write_string(2, 0, "销售额").unwrap();
        ws.write_string(2, 1, "元").unwrap();
        ws.write_string(2, 2, "0").unwrap();
        ws.write_string(2, 4, "0").unwrap();
        ws.write_string(2, 5, "2026-10-02").unwrap();
        book.save(&conflict_file).unwrap();
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
        ws.set_name("source").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标", "期间"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        // 17 位有效数（≤28 有效位、6 位小数上限内）
        ws.write_string(1, 2, "1234567890123456.5").unwrap();
        ws.write_string(1, 3, "1234567890123456.5").unwrap();
        ws.write_string(1, 4, "0").unwrap();
        ws.write_string(1, 5, "2026-10-01").unwrap();
        ws.write_string(2, 0, "销售额").unwrap();
        ws.write_string(2, 1, "元").unwrap();
        ws.write_string(2, 2, "0").unwrap();
        ws.write_string(2, 3, "0").unwrap();
        ws.write_string(2, 4, "0").unwrap();
        ws.write_string(2, 5, "2026-10-02").unwrap();
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
    assert_eq!(grid[6][5], "1,234,567,890,123,456.50", "超安全精度金额以展示文本保存");
    let metadata = read_sheet(&big_wb, "_metadata");
    assert!(metadata.iter().any(|row| row.get(1).is_some_and(|v| v == "核算表")
        && row.get(2).is_some_and(|v| v == "F7")
        && row.get(3).is_some_and(|v| v == "1234567890123456.5")), "完整原值保留在元数据");
}

fn xlsx_xml(bytes: &[u8], path: &str) -> String {
    String::from_utf8(unzip(bytes).into_iter().find(|(name, _)| name == path).unwrap().1).unwrap()
}

/// 读取真实 XLSX XML：不能只给文本格附 number format，必须验证数值载体和样式绑定。
fn assert_numeric_cell_format(bytes: &[u8], sheet: u32, cell: &str, value: &str, format: &str) {
    let xml = xlsx_xml(bytes, &format!("xl/worksheets/sheet{sheet}.xml"));
    let re = regex::Regex::new(&format!(r#"<c r="{cell}" s="(\d+)"[^>]*><v>{}</v></c>"#, regex::escape(value))).unwrap();
    let capture = re.captures(&xml).unwrap_or_else(|| panic!("{cell} 数值/样式错误: {xml}"));
    assert!(!capture[0].contains("t=\"s\""), "展示值必须是数值类型");
    let style: usize = capture[1].parse().unwrap();
    let styles = xlsx_xml(bytes, "xl/styles.xml");
    let xfs = styles.split("<cellXfs").nth(1).unwrap().split("</cellXfs>").next().unwrap();
    let xf = xfs.split("<xf ").nth(style + 1).unwrap();
    let format_id = xf.split("numFmtId=\"").nth(1).unwrap().split('"').next().unwrap();
    let encoded_format = format.replace('&', "&amp;").replace('"', "&quot;");
    assert!(styles.contains(&format!(r#"numFmtId="{format_id}" formatCode="{encoded_format}""#)), "{cell} 样式须为 {format}: {styles}");
}

fn assert_column_min_width(bytes: &[u8], sheet: u32, column: u16, width: f64) {
    let xml = xlsx_xml(bytes, &format!("xl/worksheets/sheet{sheet}.xml"));
    let re = regex::Regex::new(r#"<col min="(\d+)" max="(\d+)" width="([\d.]+)""#).unwrap();
    assert!(re.captures_iter(&xml).any(|c| {
        c[1].parse::<u16>().unwrap() <= column && c[2].parse::<u16>().unwrap() >= column
            && c[3].parse::<f64>().unwrap() >= width
    }), "列 {column} 须至少 {width} 宽，避免展示为 ####：{xml}");
}

#[test]
fn s4_visual_decimal_rounding_keeps_raw_metadata_and_machine_exports() {
    use dsheng_cli::export::{bundle, cells::DecimalTextLog, chart, workbook::{self, WorkbookKind}};
    use serde_json::json;
    let f = Fixture::new("s4-display-precision");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let mut data = dsheng_cli::cmd::export_cmd::build_export_data(&conn, f.workspace, &f.run_id, None).unwrap();
    let raw_money = "455022.79220779220779220779221";
    data.metrics = vec![
        json!({"metric_key":"money", "metric_label":"金额", "unit":"元", "unit_signature":{"dimension":"money"},
            "target_value":"481590", "actual_value":raw_money, "delta":"-1.225", "annual_month_plan":null,
            "source_note":"原始证据", "target_basis":{"kind":"prior_year_actual","label":"去年同期参考下限","is_reference":true,"from":"2025-10-01","to":"2025-10-31"},
            "yoy":{"base_from":"2025-10-01","base_to":"2025-10-31","absolute_change":"1.235","change_rate":"0.94485"},
            "mom":{"base_from":"2026-09-01","base_to":"2026-09-30","absolute_change":"-1.235","change_rate":"-0.1225"}}),
        json!({"metric_key":"ratio", "metric_label":"比率", "unit":"1", "unit_signature":{"dimension":"ratio"},
            "actual_value":"0.94485", "target_value":"0.82235", "delta":"0.1225",
            "yoy":{"absolute_change":"-0.1235", "change_rate":"-0.1225"},
            "mom":{"absolute_change":"0.1225", "change_rate":"0.1235"}}),
        json!({"metric_key":"huge", "metric_label":"大金额", "unit":"元", "unit_signature":{"dimension":"money"}, "actual_value":"1234567890123456.555"}),
        json!({"metric_key":"hours", "metric_label":"工时", "unit":"hour", "unit_signature":{"dimension":"time"}, "actual_value":"1.234567"}),
        json!({"metric_key":"empty", "metric_label":"空值", "unit":"元", "unit_signature":{"dimension":"money"}, "actual_value":null}),
        json!({"metric_key":"zero", "metric_label":"零值", "unit":"元", "unit_signature":{"dimension":"money"}, "actual_value":"0"}),
        json!({"metric_key":"legacy", "metric_label":"旧记录未确认单位", "unit":"元", "actual_value":"1.234567"}),
        json!({"metric_key":"hour_cost", "metric_label":"每小时费用", "unit":"元/小时", "unit_signature":{"dimension":"money_per_time"}, "actual_value":"-1.235"}),
        json!({"metric_key":"unit_cost", "metric_label":"单价", "unit":"元/件", "unit_signature":{"dimension":"money_per_quantity"}, "actual_value":"1.225"}),
        json!({"metric_key":"quantity", "metric_label":"数量", "unit":"件", "unit_signature":{"dimension":"quantity"}, "actual_value":"1.234567"}),
        json!({"metric_key":"huge_ratio", "metric_label":"大比例", "unit":"1", "unit_signature":{"dimension":"ratio"}, "delta":"1234567890123456.555"}),
    ];
    data.series = vec![
        json!({"metric_key":"money", "metric_label":"金额", "unit":"元", "unit_signature":{"dimension":"money"}, "semantic_type":"flow",
            "buckets":["2026-10-01","2026-10-02"], "actual":[raw_money,"1.225"], "plan":[null,"1.235"]}),
        json!({"metric_key":"ratio", "metric_label":"比率", "unit":"1", "unit_signature":{"dimension":"ratio"}, "semantic_type":"ratio",
            "buckets":["2026-10-01","2026-10-02"], "actual":["0.94485",null], "plan":["0.1225","-0.1235"]}),
    ];
    data.tasks = vec![json!({"task_id":"ratio-review", "unit_signature":{"dimension":"ratio"},
        "target_value":"0.1225", "actual_value":"0.94485", "progress":"99.99"})];
    data.evidence = vec![json!({"evidence_id":"ratio-comparison", "unit_signature":{"dimension":"ratio"},
        "base_value":"0.1225", "current_value":"0.94485", "absolute_change":"-0.1235", "change_rate":"0.1225"})];
    let unchanged = data.clone();
    let (bytes, log) = workbook::build_workbook(WorkbookKind::Accounting, &data).unwrap();
    let path = f.dir.join("precision.xlsx");
    std::fs::write(&path, &bytes).unwrap();
    let grid = read_sheet(&path, "核算表");
    assert_eq!(grid[5][..10], workbook::ACCOUNTING_HEADERS.map(str::to_string), "固定业务列不变");
    assert_eq!(grid[6][6], "455022.79");
    assert_eq!(grid[7][6], "0.945");
    assert_eq!(grid[8][6], "1,234,567,890,123,456.56");
    assert_eq!(grid[9][6], "1.234567", "工时保留原精度");
    assert_eq!(grid[10][6], "", "缺失值不能变成零");
    assert_eq!(grid[11][6], "0");
    assert_eq!(grid[12][6], "1.234567", "旧记录无冻结单位语义时不猜金额");
    assert_eq!(grid[13][6], "-1.24", "货币/时间保留两位，负数半偶舍入");
    assert_eq!(grid[14][6], "1.22", "货币/数量保留两位，正数半偶舍入");
    assert_eq!(grid[15][6], "1.234567", "数量保留原精度");
    assert_eq!(grid[16][7], "123456789012345655.5 个百分点", "大百分点必须文本回退");
    assert!(grid[6][2].contains("月度预定来源：去年同期参考下限"));
    assert!(grid[6][2].contains("参考值，非已确认目标"));
    assert_numeric_cell_format(&bytes, 1, "G7", "455022.79", "#,##0.00");
    assert_numeric_cell_format(&bytes, 1, "G8", "0.945", "0.0%");
    assert_numeric_cell_format(&bytes, 1, "H7", "-1.22", "#,##0.00");
    assert_numeric_cell_format(&bytes, 1, "H8", "12.2", "0.0\" 个百分点\"");
    assert_numeric_cell_format(&bytes, 2, "D7", "455022.79", "#,##0.00");
    assert_numeric_cell_format(&bytes, 2, "E7", "1.22", "#,##0.00");
    assert_numeric_cell_format(&bytes, 3, "H7", "1.24", "#,##0.00");
    assert_numeric_cell_format(&bytes, 3, "I7", "0.945", "0.0%");
    assert_numeric_cell_format(&bytes, 3, "K7", "-1.24", "#,##0.00");
    assert_numeric_cell_format(&bytes, 3, "L7", "-0.122", "0.0%");
    assert_numeric_cell_format(&bytes, 3, "E8", "12.2", "0.0\" 个百分点\"");
    assert_numeric_cell_format(&bytes, 3, "H8", "-12.4", "0.0\" 个百分点\"");
    assert_numeric_cell_format(&bytes, 3, "I8", "-0.122", "0.0%");
    assert_numeric_cell_format(&bytes, 3, "K8", "12.2", "0.0\" 个百分点\"");
    assert_numeric_cell_format(&bytes, 3, "L8", "0.124", "0.0%");
    assert_column_min_width(&bytes, 1, 7, 26.0);
    assert_column_min_width(&bytes, 1, 8, 31.0);
    let metadata = read_sheet(&path, "_metadata");
    assert!(metadata.iter().any(|row| row.get(2).is_some_and(|v| v == "G7")
        && row.get(3).is_some_and(|v| v == raw_money)
        && row.get(4).is_some_and(|v| v == "455,022.79")));
    assert!(metadata.iter().any(|row| row.get(2).is_some_and(|v| v == "G8")
        && row.get(3).is_some_and(|v| v == "0.94485")
        && row.get(4).is_some_and(|v| v == "94.5%")));
    assert!(metadata.iter().any(|row| row.get(0).is_some_and(|v| v == "target_basis")
        && row.get(3).is_some_and(|v| v.contains("prior_year_actual"))));
    assert!(metadata.iter().any(|row| row.get(2).is_some_and(|v| v == "H8")
        && row.get(3).is_some_and(|v| v == "0.1225")
        && row.get(4).is_some_and(|v| v == "12.2 个百分点")));
    assert!(log.cells.iter().any(|cell| cell.cell == "G9" && cell.value == "1234567890123456.555"));
    let chart = chart::chart_xlsx(&data, "ratio", &mut DecimalTextLog::default()).unwrap();
    assert_numeric_cell_format(&chart, 1, "B7", "0.945", "0.0%");
    assert_numeric_cell_format(&chart, 1, "C7", "0.122", "0.0%");
    assert_numeric_cell_format(&chart, 1, "C8", "-0.124", "0.0%");
    assert!(xlsx_xml(&chart, "xl/charts/chart1.xml").contains(r#"formatCode="0.0%""#));
    let (tasks, _) = workbook::build_workbook(WorkbookKind::Task, &data).unwrap();
    assert_numeric_cell_format(&tasks, 1, "E7", "0.122", "0.0%");
    assert_numeric_cell_format(&tasks, 1, "F7", "0.945", "0.0%");
    assert_numeric_cell_format(&tasks, 3, "J7", "0.122", "0.0%");
    assert_numeric_cell_format(&tasks, 3, "K7", "0.945", "0.0%");
    assert_numeric_cell_format(&tasks, 3, "L7", "-12.4", "0.0\" 个百分点\"");
    assert_numeric_cell_format(&tasks, 3, "M7", "0.122", "0.0%");
    assert!(xlsx_xml(&tasks, "xl/worksheets/sheet1.xml").contains(r#"<c r="L7"><v>99.99</v></c>"#), "任务进度不因舍入显示为100");
    let csv = String::from_utf8(chart::chart_csv(&data, "money").unwrap()).unwrap();
    assert!(csv.contains(raw_money), "机器 CSV 原值不变");
    let bundle = bundle::build_bundle(&data).unwrap();
    let entries = unzip(&bundle);
    let analysis: Value = serde_json::from_slice(&entries.iter().find(|(name, _)| name == "analysis.json").unwrap().1).unwrap();
    assert_eq!(analysis["metrics"][0]["actual_value"], raw_money);
    assert_eq!(data, unchanged, "所有导出都不能改写业务快照");
}

#[test]
fn s4_legacy_export_projects_only_selected_revision_metadata_without_writes() {
    use dsheng_cli::export::workbook::{build_workbook, WorkbookKind};
    use rusqlite::params;
    use serde_json::json;
    let f = Fixture::new("s4-frozen-display");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let read_snapshot = || -> (String, String, String) {
        conn.query_row(
            "SELECT normalized_metrics_json, metric_definitions_snapshot_json, analysis_input_fingerprint
             FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
            params![f.run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap()
    };
    let (raw, defs, _) = read_snapshot();
    let mut normalized: Value = serde_json::from_str(&raw).unwrap();
    for field in ["metrics", "series"] {
        for row in normalized[field].as_array_mut().unwrap() {
            row.as_object_mut().unwrap().remove("unit_signature");
            row.as_object_mut().unwrap().remove("semantic_type");
            row.as_object_mut().unwrap().remove("target_basis");
        }
    }
    normalized["metrics"][0]["actual_value"] = json!("1.234567");
    conn.execute(
        "UPDATE run_revisions SET normalized_metrics_json = ?2 WHERE run_id = ?1 AND revision_no = 1",
        params![f.run_id, normalized.to_string()],
    ).unwrap();
    // 只改临时夹具的现行定义：旧 revision 的单位/币种/累计语义必须不受影响。
    conn.execute(
        "UPDATE metric_definitions SET semantic_type = 'stock', unit_signature_json = ?2 WHERE workspace_id = ?1",
        params![f.workspace.to_string(), json!({"dimension":"money","base_unit":"USD","currency":"USD","tax_basis":"inclusive"}).to_string()],
    ).unwrap();
    let before = read_snapshot();
    let data = dsheng_cli::cmd::export_cmd::build_export_data(&conn, f.workspace, &f.run_id, Some(1)).unwrap();
    assert_eq!(data.metrics[0]["unit_signature"]["dimension"], "money");
    assert_eq!(data.metrics[0]["unit_signature"]["currency"], "CNY");
    assert_eq!(data.series[0]["unit_signature"]["currency"], "CNY");
    assert_eq!(data.series[0]["semantic_type"], "flow");
    assert_eq!(data.meta.currency.as_deref(), Some("CNY"));
    assert_eq!(data.metrics[0]["target_basis"]["kind"], "unconfirmed_legacy");
    let (book, _) = build_workbook(WorkbookKind::Accounting, &data).unwrap();
    assert_numeric_cell_format(&book, 1, "G7", "1.23", "#,##0.00");
    let path = f.dir.join("legacy-frozen.xlsx");
    std::fs::write(&path, &book).unwrap();
    assert!(read_sheet(&path, "核算表")[6][2].contains("历史来源待确认"));
    let entries = unzip(&dsheng_cli::export::bundle::build_bundle(&data).unwrap());
    let analysis: Value = serde_json::from_slice(&entries.iter().find(|(name, _)| name == "analysis.json").unwrap().1).unwrap();
    assert_eq!(analysis["metrics"][0]["actual_value"], "1.234567");
    assert_eq!(analysis["metrics"][0]["target_basis"]["kind"], "unconfirmed_legacy");
    assert_eq!(read_snapshot(), before, "只读 projection 不能改写历史快照或指纹");

    // 更老的定义快照本身没有单位/累计语义时保留未知，不能借用现行定义。
    let mut legacy_defs: Value = serde_json::from_str(&defs).unwrap();
    for def in legacy_defs.as_array_mut().unwrap() {
        def.as_object_mut().unwrap().remove("unit_signature");
        def.as_object_mut().unwrap().remove("semantic_type");
    }
    conn.execute(
        "UPDATE run_revisions SET metric_definitions_snapshot_json = ?2 WHERE run_id = ?1 AND revision_no = 1",
        params![f.run_id, legacy_defs.to_string()],
    ).unwrap();
    let before = read_snapshot();
    let unknown = dsheng_cli::cmd::export_cmd::build_export_data(&conn, f.workspace, &f.run_id, Some(1)).unwrap();
    assert!(unknown.metrics[0]["unit_signature"].is_null());
    assert!(unknown.series[0]["unit_signature"].is_null());
    assert!(unknown.series[0]["semantic_type"].is_null());
    assert_eq!(unknown.meta.currency, None);
    let (book, _) = build_workbook(WorkbookKind::Accounting, &unknown).unwrap();
    std::fs::write(&path, &book).unwrap();
    assert_eq!(read_sheet(&path, "核算表")[6][6], "1.234567");
    assert_eq!(read_snapshot(), before);
}

#[test]
fn s4_task_and_evidence_display_bind_frozen_metric_identity() {
    use dsheng_cli::export::workbook::{build_workbook, WorkbookKind};
    use rusqlite::params;
    use serde_json::json;
    let f = Fixture::new("s4-task-display");
    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let (raw, fingerprint): (String, String) = conn.query_row(
        "SELECT normalized_metrics_json, analysis_input_fingerprint FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
        params![f.run_id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    let raw_money = "455022.792207";
    let mut normalized: Value = serde_json::from_str(&raw).unwrap();
    normalized["metrics"][0]["actual_value"] = json!(raw_money);
    normalized["metrics"][0]["mom"] = json!({"base_from":"2026-09-01", "base_to":"2026-09-30",
        "base_value":"1.225", "absolute_change":"1.235", "change_rate":"0.1225", "evidence_ids":["fixture-money-evidence"]});
    conn.execute("UPDATE run_revisions SET normalized_metrics_json = ?2 WHERE run_id = ?1 AND revision_no = 1",
        params![f.run_id, normalized.to_string()]).unwrap();
    for (role, title, metric_id, target, unit) in [
        ("review", "关联指标检视", Some(f.metric_id.as_str()), "1.225", "元"),
        ("plan", "关联指标计划", Some(f.metric_id.as_str()), "1.235", "元"),
        ("review", "未关联指标检视", None, "1.234567", "元"),
        ("review", "单位冲突检视", Some(f.metric_id.as_str()), "1.234567", "hour"),
    ] {
        let input = json!({"schema_version":"1", "workspace_id":f.workspace, "run_id":f.run_id,
            "analysis_revision_no":1, "input_fingerprint":fingerprint, "mutation_id":uuid::Uuid::new_v4(),
            "cycle_role":role, "source_note":"合成展示测试", "content":{"title":title, "summary":"合成测试",
                "category":"other", "metric_id":metric_id, "target_value":target, "actual_value":raw_money, "unit":unit},
            "steps":[{"action":"合成核对", "owner":null, "start_date":null, "due_date":null, "done_criteria":"核对完成"}]});
        let path = f.dir.join("task-display.json");
        std::fs::write(&path, input.to_string()).unwrap();
        let (stdout, code) = run(&["task", "create", "--db", &f.db, "--workspace", &f.workspace.to_string(),
            "--input-json", &path.to_string_lossy(), "--json"]);
        assert_eq!(code, 0, "{stdout}");
    }
    conn.execute("UPDATE metric_definitions SET label = '现行标签', unit_signature_json = ?2 WHERE workspace_id = ?1",
        params![f.workspace.to_string(), json!({"dimension":"ratio","base_unit":"1","currency":null,"tax_basis":null}).to_string()]).unwrap();
    let data = dsheng_cli::cmd::export_cmd::build_export_data(&conn, f.workspace, &f.run_id, Some(1)).unwrap();
    for task in &data.tasks {
        assert_eq!(task["actual_value"], raw_money);
        if task["metric_id"].is_null() || task["unit"] == "hour" {
            assert!(task["unit_signature"].is_null(), "缺少指标关联或明确单位冲突时保留未知");
        } else {
            assert_eq!(task["metric_label"], "销售额");
            assert_eq!(task["unit_signature"]["dimension"], "money");
            assert_eq!(task["unit_signature"]["currency"], "CNY");
        }
    }
    let evidence_index = data.evidence.iter().position(|e| e["evidence_id"] == "fixture-money-evidence").unwrap();
    assert_eq!(data.evidence[evidence_index]["metric_id"], f.metric_id);
    assert_eq!(data.evidence[evidence_index]["unit_signature"]["dimension"], "money");
    let (book, _) = build_workbook(WorkbookKind::Task, &data).unwrap();
    let path = f.dir.join("task-display.xlsx");
    std::fs::write(&path, &book).unwrap();
    let review = read_sheet(&path, "任务单检视");
    let row = review.iter().position(|r| r[1] == "关联指标检视").unwrap() + 1;
    assert_numeric_cell_format(&book, 1, &format!("E{row}"), "1.22", "#,##0.00");
    assert_numeric_cell_format(&book, 1, &format!("F{row}"), "455022.79", "#,##0.00");
    assert_eq!(review.iter().find(|r| r[1] == "未关联指标检视").unwrap()[4], "1.234567");
    assert_eq!(review.iter().find(|r| r[1] == "单位冲突检视").unwrap()[4], "1.234567");
    assert_numeric_cell_format(&book, 2, "E7", "1.24", "#,##0.00");
    let row = evidence_index + 7;
    assert_numeric_cell_format(&book, 3, &format!("J{row}"), "1.22", "#,##0.00");
    assert_numeric_cell_format(&book, 3, &format!("K{row}"), "455022.79", "#,##0.00");
    assert_numeric_cell_format(&book, 3, &format!("L{row}"), "1.24", "#,##0.00");
    assert_numeric_cell_format(&book, 3, &format!("M{row}"), "0.122", "0.0%");
    let entries = unzip(&dsheng_cli::export::bundle::build_bundle(&data).unwrap());
    let tasks: Value = serde_json::from_slice(&entries.iter().find(|(name, _)| name == "tasks.json").unwrap().1).unwrap();
    assert!(tasks["tasks"].as_array().unwrap().iter().all(|t| t["actual_value"] == raw_money));
    let persisted: String = conn.query_row("SELECT normalized_metrics_json FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
        params![f.run_id], |r| r.get(0)).unwrap();
    assert_eq!(persisted, normalized.to_string(), "导出不回写原始快照");
}

#[test]
fn historical_revision_keeps_its_own_input_fingerprint() {
    let f = Fixture::new("s4-revision-fingerprint");
    let rev1_fingerprint: String = {
        let conn = rusqlite::Connection::open(&f.db).unwrap();
        conn.query_row(
            "SELECT analysis_input_fingerprint FROM run_revisions WHERE run_id = ?1 AND revision_no = 1",
            rusqlite::params![f.run_id],
            |row| row.get(0),
        )
        .unwrap()
    };

    // 新增一份同期间 source，触发同一 run 的 revision 2；revision 1 仍保留。
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("source").unwrap();
        for (c, h) in ["科目", "单位", "月度预定", "月度实绩", "年度计划·当月目标", "期间"]
            .iter()
            .enumerate()
        {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        ws.write_string(1, 2, "12000").unwrap();
        ws.write_string(1, 3, "11999").unwrap();
        ws.write_string(1, 4, "12500").unwrap();
        ws.write_string(1, 5, "2026-10-03").unwrap();
        book.save(f.dir.join("source-data-v2.xlsx")).unwrap();
    }
    let (stdout, code) = run(&[
        "analyze",
        "--db",
        &f.db,
        "--workspace",
        &f.workspace.to_string(),
        "--dir",
        &f.dir.to_string_lossy(),
        "--from",
        "2026-10-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "{stdout}");
    let latest = data_of(&stdout);
    assert_eq!(latest["run_id"], f.run_id);
    assert_eq!(latest["current_revision_no"], 2);

    let conn = rusqlite::Connection::open(&f.db).unwrap();
    let rev2_fingerprint: String = conn
        .query_row(
            "SELECT analysis_input_fingerprint FROM run_revisions WHERE run_id = ?1 AND revision_no = 2",
            rusqlite::params![f.run_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(rev1_fingerprint, rev2_fingerprint);

    let view = dsheng_cli::store::analysis::analysis_run_view(
        &conn,
        f.workspace,
        &f.run_id,
        1,
    )
    .unwrap();
    assert_eq!(view["current_revision_no"], 2);
    assert_eq!(view["analysis_input_fingerprint"], rev1_fingerprint);
    let exported = dsheng_cli::cmd::export_cmd::build_export_data(
        &conn,
        f.workspace,
        &f.run_id,
        Some(1),
    )
    .unwrap();
    assert_eq!(exported.meta.analysis_input_fingerprint, rev1_fingerprint);
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
    // 双桶序列：手写原始来源表，推移表由 analyze 生成
    let trend_dir = dir.join("trend");
    std::fs::create_dir_all(&trend_dir).unwrap();
    let trend_file = trend_dir.join("source-data.xlsx");
    {
        use rust_xlsxwriter::*;
        let mut book = Workbook::new();
        let ws = book.add_worksheet();
        ws.set_name("source").unwrap();
        for (c, h) in ["科目", "单位", "月度实绩", "期间"].iter().enumerate() {
            ws.write_string(0, c as u16, *h).unwrap();
        }
        ws.write_string(1, 0, "销售额").unwrap();
        ws.write_string(1, 1, "元").unwrap();
        ws.write_string(1, 2, "11000.5").unwrap();
        ws.write_string(1, 3, "2026-10-01").unwrap();
        ws.write_string(2, 0, "销售额").unwrap();
        ws.write_string(2, 1, "元").unwrap();
        ws.write_string(2, 2, "11500.25").unwrap();
        ws.write_string(2, 3, "2026-10-02").unwrap();
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
