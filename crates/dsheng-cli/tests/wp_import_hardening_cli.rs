//! 导入器真实文件硬化（CLI 端到端）：合成「真实形态」核算表 → analyze。
//!
//! 夹具在测试内用 zip（deflate）构造 calamine 可读的最小 xlsx，模仿真实核算表：
//! 第 1–2 行整行合并标题/说明、第 4 行表头（含 `计算公式 / 取数来源`、
//! `销售额占比(预定)` 等未映射列）、整行合并 section 行、内联字符串科目列、
//! C 列公式说明文本（`A5=A1+A2+A3+A4`）、`—` 占位、带样式的空数值格、
//! 显式 0、负数与两位小数金额（XML 数值字面量）、`合计` 行、`待补` 行。
//! 全部数值为合成值；真实业务文件绝不入库。

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

fn run(args: &[&str]) -> (Value, i32) {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = dsheng_cli::run_with_output(&owned, &mut out, &mut err);
    let text = String::from_utf8(out).expect("utf8");
    (
        serde_json::from_str(text.trim()).unwrap_or_else(|_| panic!("envelope: {text}")),
        code,
    )
}

// ---- 合成 xlsx（deflate；内联字符串/样式数值/样式空格） ----

enum C {
    T(&'static str, &'static str),
    N(&'static str, &'static str),
    Blank(&'static str),
}

fn cell_xml(c: &C) -> String {
    match c {
        C::T(r, t) => format!(r#"<c r="{r}" s="2" t="inlineStr"><is><t>{t}</t></is></c>"#),
        C::N(r, v) => format!(r#"<c r="{r}" s="1" t="n"><v>{v}</v></c>"#),
        C::Blank(r) => format!(r#"<c r="{r}" s="1" t="n"></c>"#),
    }
}

fn sheet_xml(rows: &[(u32, Vec<C>)], merges: &[&str]) -> String {
    let mut s = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, cells) in rows {
        s.push_str(&format!(r#"<row r="{r}">"#));
        for c in cells {
            s.push_str(&cell_xml(c));
        }
        s.push_str("</row>");
    }
    s.push_str("</sheetData>");
    if !merges.is_empty() {
        s.push_str(&format!(r#"<mergeCells count="{}">"#, merges.len()));
        for m in merges {
            s.push_str(&format!(r#"<mergeCell ref="{m}"/>"#));
        }
        s.push_str("</mergeCells>");
    }
    s.push_str("</worksheet>");
    s
}

fn write_xlsx(path: &Path, sheets: &[(&str, String)]) {
    let mut overrides = String::new();
    let mut entries = String::new();
    let mut rels = String::new();
    for (i, (name, _)) in sheets.iter().enumerate() {
        let n = i + 1;
        overrides.push_str(&format!(
            r#"<Override PartName="/xl/worksheets/sheet{n}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#
        ));
        entries.push_str(&format!(
            r#"<sheet name="{name}" sheetId="{n}" r:id="rId{n}"/>"#
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{n}.xml"/>"#
        ));
    }
    let mut parts: Vec<(String, String)> = vec![
        (
            "[Content_Types].xml".into(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>{overrides}</Types>"#
            ),
        ),
        (
            "_rels/.rels".into(),
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>"#.into(),
        ),
        (
            "xl/workbook.xml".into(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{entries}</sheets></workbook>"#
            ),
        ),
        (
            "xl/_rels/workbook.xml.rels".into(),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}<Relationship Id="rIdS" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#
            ),
        ),
        (
            "xl/styles.xml".into(),
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="1"><font/></fonts><fills count="1"><fill/></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="4" applyNumberFormat="1"/><xf numFmtId="0"/></cellXfs></styleSheet>"#.into(),
        ),
    ];
    for (i, (_, xml)) in sheets.iter().enumerate() {
        parts.push((format!("xl/worksheets/sheet{}.xml", i + 1), xml.clone()));
    }
    let file = std::fs::File::create(path).expect("创建 xlsx");
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in &parts {
        zip.start_file(name.as_str(), options).expect("zip entry");
        zip.write_all(content.as_bytes()).expect("zip write");
    }
    zip.finish().expect("zip finish");
}

/// 合成核算表：行号/列位与真实布局形态一致，数值为合成值。
fn accounting_sheet() -> String {
    use C::*;
    let rows = vec![
        (
            1,
            vec![T("A1", "合成公司 — 2026年10月整体核算表（合成夹具）")],
        ),
        (2, vec![T("A2", "模板二（费用项下含人工费）｜合成说明行")]),
        (
            4,
            vec![
                T("A4", "序号"),
                T("B4", "科目"),
                T("C4", "计算公式 / 取数来源"),
                T("D4", "本月MP"),
                T("E4", "年度计划·当月目标"),
                T("F4", "月度预定"),
                T("G4", "销售额占比(预定)"),
                T("H4", "月度实绩"),
                T("I4", "销售额占比(实绩)"),
                T("J4", "预实差异额"),
                T("K4", "实绩达成预定比"),
                T("L4", "实绩达成年度计划比"),
            ],
        ),
        (5, vec![T("A5", "【先行指标（未来经营信号）】")]),
        (
            7,
            vec![
                T("A7", "A0-1"),
                T("B7", "当月采购签约额（备货）"),
                T("C7", "待补·本次导出无采购订单表"),
                T("D7", "—"),
                Blank("E7"),
                Blank("F7"),
                T("G7", "—"),
                T("H7", "待补"),
            ],
        ),
        (10, vec![T("A10", "【收入部分】")]),
        (
            11,
            vec![
                T("A11", "A1"),
                T("B11", "工业品销售——合成客户"),
                T("C11", "直接取数·销售出库表（出库时间落月）"),
                T("D11", "合成MP-甲"),
                N("E11", "200000"),
                N("F11", "150000"),
                N("H11", "65432.1"),
            ],
        ),
        (
            12,
            vec![
                T("A12", "A5"),
                T("B12", "总销售额"),
                T("C12", "A5=A1+A2+A3+A4"),
                T("D12", "—"),
                N("E12", "360000"),
                N("F12", "200000"),
                N("G12", "1"),
                N("H12", "123456.78"),
                N("I12", "1"),
                N("J12", "-76543.22"),
                N("K12", "0.6172839"),
                N("L12", "0.3429355"),
            ],
        ),
        (13, vec![T("A13", "【费用部分（合成）】")]),
        (
            19,
            vec![
                T("A19", "B1"),
                T("B19", "费用合计（含人工费）"),
                T("C19", "B1=B2+B3+B4+B5+B6+B20"),
                T("D19", "—"),
                Blank("E19"),
                Blank("F19"),
                T("G19", "—"),
                N("H19", "45678.9"),
            ],
        ),
        (
            20,
            vec![
                T("A20", "B20"),
                T("B20", "人工费（不可作为削减对象）"),
                T("C20", "直接取数·工资条（薪资归属当月）"),
                T("D20", "—"),
                Blank("E20"),
                N("F20", "20000"),
                T("G20", "—"),
                N("H20", "18000"),
                N("J20", "-2000"),
            ],
        ),
        (
            21,
            vec![
                T("A21", "C"),
                T("B21", "结算收益（附加价值）"),
                T("C21", "C=A5-B1（=税前利润）"),
                T("D21", "合成MP-乙"),
                N("E21", "40000"),
                N("F21", "30000"),
                N("G21", "0.12"),
                N("H21", "-4321.37"),
            ],
        ),
        (26, vec![T("A26", "【单位时间附加值部分】")]),
        (
            27,
            vec![
                T("A27", "E"),
                T("B27", "单位时间附加值（元/小时）"),
                T("C27", "E=C/D1"),
                T("D27", "合成MP-丙"),
                Blank("E27"),
                N("F27", "100"),
                T("G27", "—"),
                N("H27", "-6.71"),
            ],
        ),
        (
            33,
            vec![
                T("A33", "N"),
                T("B33", "当月库存金额"),
                T("C33", "库存模块未随本次导出"),
                Blank("E33"),
                Blank("F33"),
                T("G33", "—"),
                T("H33", "—"),
            ],
        ),
        (
            34,
            vec![
                T("A34", "O"),
                T("B34", "（迄今为止的）库存余额"),
                T("C34", "合成：期末余额（stock）"),
                Blank("E34"),
                Blank("F34"),
                T("G34", "—"),
                Blank("H34"),
            ],
        ),
        (35, vec![T("A35", "【表外参考（现金与往来）】")]),
        (
            36,
            vec![
                T("A36", "P1"),
                T("B36", "当月收款"),
                T("C36", "直接取数·收款单（收款时间落月）"),
                T("D36", "合成MP-丁"),
                Blank("E36"),
                N("F36", "50000"),
                T("G36", "—"),
                N("H36", "0"),
                N("J36", "-50000"),
                N("K36", "0"),
            ],
        ),
        (
            37,
            vec![
                T("A37", "P2"),
                T("B37", "当月付款（实付）"),
                T("C37", "直接取数·付款单（付款时间落月）"),
                T("D37", "—"),
                Blank("E37"),
                Blank("F37"),
                T("G37", "—"),
                N("H37", "76543.21"),
            ],
        ),
        (
            38,
            vec![
                T("A38", "P3"),
                T("B38", "当月销项开票"),
                T("C38", "直接取数·销项发票（开票时间落月）"),
                T("D38", "合成MP-戊"),
                Blank("E38"),
                Blank("F38"),
                T("G38", "—"),
                Blank("H38"),
                Blank("J38"),
            ],
        ),
        (
            42,
            vec![T("A42", "注1（合成）：本行为说明段，不是交易行。")],
        ),
    ];
    sheet_xml(
        &rows,
        &[
            "A1:L1", "A2:L2", "A5:L5", "A10:L10", "A13:L13", "A26:L26", "A35:L35", "A42:L42",
        ],
    )
}

const FILE_LABEL: &str = "合成核算表.xlsx";

struct Env {
    dir: PathBuf,
    scan: PathBuf,
    db: String,
    workspace: String,
}

impl Env {
    /// workspace（month 策略）+ 合成指标（别名覆盖真实行名形态）+ 合成核算表。
    fn new(tag: &str) -> Self {
        Self::with_files(tag, &[(FILE_LABEL, vec![("核算表", accounting_sheet())])])
    }

    /// 同上，但扫描目录中的文件/sheet 由调用方给定。
    fn with_files(tag: &str, files: &[(&str, Vec<(&str, String)>)]) -> Self {
        let dir = tempfile::tempdir().expect("临时目录").keep();
        let scan = dir.join("scan");
        std::fs::create_dir_all(&scan).unwrap();
        for (label, sheets) in files {
            write_xlsx(&scan.join(label), sheets);
        }
        let db = dir
            .join(format!("{tag}.sqlite3"))
            .to_string_lossy()
            .to_string();
        let ws_json = dir.join("ws.json");
        std::fs::write(
            &ws_json,
            r#"{"schema_version":"1","operation":"workspace_create","name":"硬化合成","initial_period_policy":{"cadence":"month","week_start_day":1,"fiscal_year_start_month":1,"timezone_id":"Asia/Shanghai","effective_from":"2026-01-01"}}"#,
        )
        .unwrap();
        let (env, code) = run(&[
            "workspace",
            "create",
            "--db",
            &db,
            "--input-json",
            &ws_json.to_string_lossy(),
            "--json",
        ]);
        assert_eq!(code, 0, "{env}");
        let workspace = env["data"]["workspace"]["workspace_id"]
            .as_str()
            .unwrap()
            .to_string();
        let me = Self {
            dir,
            scan,
            db,
            workspace,
        };
        let money =
            r#"{"dimension":"money","base_unit":"元","currency":"CNY","tax_basis":"exclusive"}"#;
        let per_hour = r#"{"dimension":"money_per_time","base_unit":"元/小时","currency":"CNY","tax_basis":"exclusive"}"#;
        for (key, label, alias, unit, direction, scope, revenue) in [
            (
                "sales_net",
                "不含税销售额",
                "总销售额",
                money,
                "up",
                "monthly_revenue_floor",
                true,
            ),
            (
                "settlement_income",
                "结算收益",
                "结算收益（附加价值）",
                money,
                "up",
                "explicit_target_only",
                false,
            ),
            (
                "value_added_per_hour",
                "单位时间附加值",
                "单位时间附加值（元/小时）",
                per_hour,
                "up",
                "explicit_target_only",
                false,
            ),
            (
                "receipts_amount",
                "收款金额",
                "当月收款",
                money,
                "up",
                "explicit_target_only",
                false,
            ),
            (
                "payments_amount",
                "付款金额",
                "当月付款（实付）",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
            (
                "invoice_amount",
                "开票金额",
                "当月销项开票",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
            (
                "labor_cost",
                "人工费",
                "人工费（不可作为削减对象）",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
            (
                "operating_expense",
                "经营费用",
                "费用合计（含人工费）",
                money,
                "down",
                "explicit_target_only",
                false,
            ),
            (
                "purchase_signed_amount",
                "采购签约额",
                "当月采购签约额（备货）",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
            (
                "inventory_amount",
                "库存金额",
                "当月库存金额",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
            // stock 指标（空格即使显式确认也不补 0：需期末，缺期初当零是禁止行为）
            (
                "inventory_balance",
                "库存余额",
                "（迄今为止的）库存余额",
                money,
                "neutral",
                "explicit_target_only",
                false,
            ),
        ] {
            let semantic_type = if key == "inventory_balance" {
                "stock"
            } else {
                "flow"
            };
            let aggregation = if key == "inventory_balance" {
                "closing_balance_or_rollforward"
            } else {
                "sum_disjoint"
            };
            let body = format!(
                r#"{{"metric_key":"{key}","label":"{label}","label_aliases":["{alias}"],"semantic_type":"{semantic_type}","unit_signature":{unit},"direction":"{direction}","target_constraint_scope":"{scope}","revenue_semantics":{revenue},"aggregation":"{aggregation}","required_inputs":[],"comparison_policy":{{"positive_base":"relative_change","zero_or_negative_base":"absolute_only"}},"effective_from":"2026-01-01","expected_revision":0}}"#
            );
            let path = me.dir.join(format!("metric_{key}.json"));
            std::fs::write(&path, body).unwrap();
            let (env, code) = run(&[
                "metric",
                "set",
                "--db",
                &me.db,
                "--workspace",
                &me.workspace,
                "--input-json",
                &path.to_string_lossy(),
                "--json",
            ]);
            assert_eq!(code, 0, "metric set {key}: {env}");
        }
        me
    }

    /// analyze 2026-10 月度；`extra` 为可选 --input-json 内容。
    fn analyze(&self, extra: Option<&str>) -> (Value, i32) {
        let scan = self.scan.to_string_lossy().to_string();
        let mut args = vec![
            "analyze",
            "--db",
            &self.db,
            "--workspace",
            &self.workspace,
            "--dir",
            &scan,
            "--from",
            "2026-10-01",
            "--to",
            "2026-10-31",
            "--grain",
            "month",
            "--json",
        ];
        let extra_path = self.dir.join("analyze_extra.json");
        let extra_path_s = extra_path.to_string_lossy().to_string();
        if let Some(body) = extra {
            std::fs::write(&extra_path, body).unwrap();
            args.push("--input-json");
            args.push(&extra_path_s);
        }
        run(&args)
    }
}

fn metric<'a>(data: &'a Value, key: &str) -> Option<&'a Value> {
    data["metrics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["metric_key"] == key)
}

fn actual(data: &Value, key: &str) -> Option<String> {
    metric(data, key).and_then(|m| m["actual_value"].as_str().map(str::to_string))
}

// ---- F2：XML 数值字面量；C 列公式说明/`—` 不排除正常行 ----

#[test]
fn decimal_rows_with_formula_text_in_source_column_are_observed() {
    let env = Env::new("hard-f2");
    let (out, code) = env.analyze(None);
    assert_eq!(code, 0, "{out}");
    let data = &out["data"];
    // 两位小数金额（XML 数值字面量）：精确进入指标，不经 f64
    assert_eq!(actual(data, "sales_net").as_deref(), Some("123456.78"));
    assert_eq!(
        actual(data, "settlement_income").as_deref(),
        Some("-4321.37")
    );
    assert_eq!(
        actual(data, "value_added_per_hour").as_deref(),
        Some("-6.71")
    );
    // C 列写“直接取数”的小数行同理（根因是数值字面量，不是 C 列）
    assert_eq!(actual(data, "payments_amount").as_deref(), Some("76543.21"));
    // `合计` 行是合法范围观察（§4.2），不因关键词或 C 列公式文本被排除
    assert_eq!(
        actual(data, "operating_expense").as_deref(),
        Some("45678.9")
    );
    assert_eq!(actual(data, "labor_cost").as_deref(), Some("18000"));
    // 证据坐标按绝对位置定位到 H12（C 列公式文本不参与行分类）
    let sales = metric(data, "sales_net").unwrap();
    let evidence: Vec<&str> = sales["mom"]["evidence_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(evidence, vec![format!("cell:{FILE_LABEL}:核算表:H12")]);
    std::fs::remove_dir_all(&env.dir).ok();
}

// ---- F1：空格=missing（不产生 0 观察）；显式 0 与负数原样保留 ----

fn target(data: &Value, key: &str) -> Option<String> {
    metric(data, key).and_then(|m| m["target_value"].as_str().map(str::to_string))
}

#[test]
fn blank_cells_are_missing_and_explicit_zero_or_negative_kept() {
    let env = Env::new("hard-f1");
    let (out, code) = env.analyze(None);
    assert_eq!(code, 0, "{out}");
    let data = &out["data"];
    // 当月销项开票：E/F/H 全为带样式空格 → 无任何观察（缺失≠0），不出 0
    assert_eq!(actual(data, "invoice_amount"), None, "空实绩格不得算成 0");
    assert_eq!(target(data, "invoice_amount"), None, "空预定格不得算成 0");
    // 当月收款：H 为显式数值 0 → 0 保留；预定/差异照常
    assert_eq!(actual(data, "receipts_amount").as_deref(), Some("0"));
    assert_eq!(target(data, "receipts_amount").as_deref(), Some("50000"));
    assert_eq!(
        metric(data, "receipts_amount").unwrap()["delta"].as_str(),
        Some("-50000")
    );
    // 当月付款：实绩有值、预定格为空 → target null，delta null（不以 0 代替）
    assert_eq!(actual(data, "payments_amount").as_deref(), Some("76543.21"));
    assert_eq!(target(data, "payments_amount"), None);
    assert!(metric(data, "payments_amount").unwrap()["delta"].is_null());
    // 负数原样
    assert_eq!(
        actual(data, "settlement_income").as_deref(),
        Some("-4321.37")
    );
    // `待补`（pending_input）与 `—` 占位：不是数值，也不是 0
    assert_eq!(actual(data, "purchase_signed_amount"), None);
    assert_eq!(actual(data, "inventory_amount"), None);
    // revision 快照同样不含空格补出的 0
    let conn = rusqlite::Connection::open(&env.db).unwrap();
    let normalized: String = conn
        .query_row(
            "SELECT normalized_metrics_json FROM run_revisions WHERE run_id = ?1",
            rusqlite::params![out["data"]["run_id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let snapshot: Value = serde_json::from_str(&normalized).unwrap();
    assert!(metric(&snapshot, "invoice_amount").is_none());
    std::fs::remove_dir_all(&env.dir).ok();
}

// ---- F3：显式确认后空值可按 0 计（coverage 兼容实现） ----

/// 读取 run 最新 revision 的 period_policy 快照。
fn policy_snapshot(env: &Env, run_id: &str) -> Value {
    let conn = rusqlite::Connection::open(&env.db).unwrap();
    let raw: String = conn
        .query_row(
            "SELECT period_policy_snapshot_json FROM run_revisions
             WHERE run_id = ?1 ORDER BY revision_no DESC LIMIT 1",
            rusqlite::params![run_id],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn empty_as_zero_requires_explicit_confirmation_and_snapshots_coverage() {
    let env = Env::new("hard-f3");

    // 选项非法：empty_as_zero=true 但缺 coverage_confirmed_by（默认缺失≠0）
    let (out, code) = env.analyze(Some(r#"{"empty_as_zero":true}"#));
    assert_ne!(code, 0, "{out}");
    assert_eq!(out["error"]["code"], "DATA_VALIDATION_FAILED");
    assert_eq!(out["error"]["field"], "coverage_confirmed_by");

    // 命令行不带选项：行为不变（空格=缺失）
    let (out, code) = env.analyze(None);
    assert_eq!(code, 0, "{out}");
    let data = &out["data"];
    assert_eq!(actual(data, "invoice_amount"), None);
    assert_eq!(
        target(data, "labor_cost").as_deref(),
        Some("20000"),
        "目标来自有值的预定列；E 列（年度分解）为空不产生目标"
    );

    // 显式确认：空 actual 格按 0 计；coverage 快照记录 empty_days_are_zero=true
    // 与 confirmed_by；目标列空格仍不补 0；stock 指标空格不补 0
    let confirmed = r#"{"empty_as_zero":true,"coverage_confirmed_by":"硬化验证"}"#;
    let (out, code) = env.analyze(Some(confirmed));
    assert_eq!(code, 0, "{out}");
    let data = &out["data"];
    assert_eq!(
        actual(data, "invoice_amount").as_deref(),
        Some("0"),
        "确认后空实绩格按 0 计"
    );
    assert_eq!(
        target(data, "payments_amount"),
        None,
        "plan 空格=未设定，不补 0"
    );
    assert_eq!(
        actual(data, "inventory_balance"),
        None,
        "stock 空格需期末余额，不因确认补 0"
    );
    let snapshot = policy_snapshot(&env, data["run_id"].as_str().unwrap());
    let coverage = &snapshot["coverage"];
    assert_eq!(coverage["status"], "complete");
    assert_eq!(coverage["confirmed_by"], "硬化验证");
    assert_eq!(coverage["empty_days_are_zero"], true);
    assert_eq!(coverage["from"], "2026-10-01");
    assert_eq!(coverage["to"], "2026-10-31");
    assert_eq!(coverage["granularity"], "month");
    let filled: Vec<&str> = coverage["zero_filled_evidence_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        filled,
        vec![format!("cell:{FILE_LABEL}:核算表:H38")],
        "补 0 证据坐标留档（可审计）"
    );

    // 未确认的 revision 快照保持 unknown / false
    let (out2, code) = env.analyze(None);
    assert_eq!(code, 0, "{out2}");
    let snapshot2 = policy_snapshot(&env, out2["data"]["run_id"].as_str().unwrap());
    assert_eq!(snapshot2["coverage"]["status"], "unknown");
    assert_eq!(snapshot2["coverage"]["empty_days_are_zero"], false);
    assert_eq!(
        snapshot2["coverage"]["zero_filled_evidence_ids"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    // 指纹不同：输入集合不同 → 不同 revision（不覆盖历史）
    assert_ne!(
        out["data"]["analysis_input_fingerprint"],
        out2["data"]["analysis_input_fingerprint"]
    );
    std::fs::remove_dir_all(&env.dir).ok();
}

// ---- F4：推移表（合同方向）期间列命中；真实转置形态按 generic 处理 ----

/// 合同 §4.3 方向的推移表：科目行 × 期间列（YYYY-MM / YYYY-MM-DD）。
fn trend_sheet() -> String {
    use C::*;
    let rows = vec![
        (
            1,
            vec![
                T("A1", "科目"),
                T("B1", "单位"),
                T("C1", "2026-09"),
                T("D1", "2026-10-31"),
            ],
        ),
        (
            2,
            vec![
                T("A2", "总销售额"),
                T("B2", "元"),
                N("C2", "111"),
                N("D2", "222.5"),
            ],
        ),
        (
            3,
            vec![T("A3", "人工费"), T("B3", "元"), Blank("C3"), N("D3", "55")],
        ),
    ];
    sheet_xml(&rows, &[])
}

/// 真实企业形态（转置）：A 列期间值（YYYY-MM 数据行）、B 列起为指标名。
fn transposed_trend_sheet() -> String {
    use C::*;
    let rows = vec![
        (
            1,
            vec![
                T("A1", "月份"),
                T("B1", "总销售额(元)"),
                T("C1", "结算收益(元)"),
                T("D1", "人工费(元)"),
            ],
        ),
        (
            2,
            vec![
                T("A2", "2025-07"),
                N("B2", "20"),
                N("C2", "3.5"),
                N("D2", "9"),
            ],
        ),
        (
            3,
            vec![
                T("A3", "2025-08"),
                N("B3", "30"),
                N("C3", "4.5"),
                N("D3", "9"),
            ],
        ),
        (
            4,
            vec![
                T("A4", "2026-09"),
                N("B4", "40"),
                N("C4", "5.5"),
                N("D4", "9"),
            ],
        ),
        (
            5,
            vec![
                T("A5", "2026-10"),
                N("B5", "50"),
                N("C5", "-6.5"),
                N("D5", "9"),
            ],
        ),
    ];
    sheet_xml(&rows, &[])
}

#[test]
fn contract_orientation_trend_sheet_observed_by_period_columns() {
    let env = Env::with_files(
        "hard-f4-trend",
        &[("合成推移表.xlsx", vec![("推移表", trend_sheet())])],
    );
    let scan = env.scan.to_string_lossy().to_string();
    // 适配器命中 trend（scan note 记录角色）
    let (out, code) = run(&[
        "scan",
        "--db",
        &env.db,
        "--workspace",
        &env.workspace,
        "--dir",
        &scan,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out["data"]["files"][0]["note"]
            .as_str()
            .unwrap()
            .contains("推移表:trend"),
        "scan note: {}",
        out["data"]["files"][0]["note"]
    );
    // 观察按列头期间归位：9 月/10 月各一桶；YYYY-MM-DD 列头为单日观察
    let (out, code) = run(&[
        "analyze",
        "--db",
        &env.db,
        "--workspace",
        &env.workspace,
        "--dir",
        &scan,
        "--from",
        "2026-09-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let data = &out["data"];
    assert_eq!(actual(data, "sales_net").as_deref(), Some("333.5"));
    let series = data["series"].as_array().unwrap();
    let sales_series = series
        .iter()
        .find(|s| s["metric_key"] == "sales_net")
        .unwrap();
    assert_eq!(
        sales_series["buckets"],
        json!(["2026-09-01/2026-09-30", "2026-10-31/2026-10-31"])
    );
    assert_eq!(sales_series["actual"], json!(["111", "222.5"]));
    std::fs::remove_dir_all(&env.dir).ok();
}

#[test]
fn real_transposed_trend_layout_stays_generic_table() {
    let env = Env::with_files(
        "hard-f4-generic",
        &[(
            "合成转置推移表.xlsx",
            vec![("推移表", transposed_trend_sheet())],
        )],
    );
    let scan = env.scan.to_string_lossy().to_string();
    let (out, code) = run(&[
        "scan",
        "--db",
        &env.db,
        "--workspace",
        &env.workspace,
        "--dir",
        &scan,
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(
        out["data"]["files"][0]["note"]
            .as_str()
            .unwrap()
            .contains("推移表:generic_table"),
        "转置形态不满足合同 trend 锚点（科目行×期间列），按 generic：{}",
        out["data"]["files"][0]["note"]
    );
    // 不产生观察（不猜转置语义）
    let (out, code) = run(&[
        "analyze",
        "--db",
        &env.db,
        "--workspace",
        &env.workspace,
        "--dir",
        &scan,
        "--from",
        "2026-09-01",
        "--to",
        "2026-10-31",
        "--grain",
        "month",
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    assert!(out["data"]["metrics"].as_array().unwrap().is_empty());
    std::fs::remove_dir_all(&env.dir).ok();
}
