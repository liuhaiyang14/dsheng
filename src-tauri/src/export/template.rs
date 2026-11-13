//! 模板包（§5）：source profile 输入 XLSX 或固定 two_tables_v1 输出 XLSX＋字段说明 Markdown。
//!
//! 无样例金额/客户/人员；元数据键位固定、值空；列名按 §5 表；推移表空期间列
//! 为“期间1（填写ISO期间）”等。任务单属于分析结果附加导出，不是 source 输入。

use super::cells::{write_headers, write_text};
use super::workbook::{
    write_blank_metadata, ACCOUNTING_HEADERS, CHART_HEADERS, DIFF_HEADERS, EVIDENCE_HEADERS,
    HEADER_ROW, TASK_PLAN_HEADERS, TASK_REVIEW_HEADERS,
};
use super::{
    ExportError, ExportResult, SHEET_ACCOUNTING, SHEET_CHART, SHEET_DIFF, SHEET_TASK_EVIDENCE,
    SHEET_TREND,
};
use rust_xlsxwriter::Workbook;
use std::io::Write;

/// 旧包名仅供历史导出兼容；当前 UI/CLI 不再把它当 source 输入。
pub const TEMPLATE_ZIP_NAME: &str = "import-template-v1.zip";
pub const TEMPLATE_ACCOUNTING: &str = "核算表模板.xlsx";
pub const TEMPLATE_TREND: &str = "推移表模板.xlsx";
pub const TEMPLATE_TASK: &str = "任务单模板.xlsx";
pub const TEMPLATE_FIELDS_MD: &str = "字段说明.md";
pub const OUTPUT_TEMPLATE_ZIP_NAME: &str = "two-tables-output-v1.zip";
pub const SOURCE_TEMPLATE_ZIP_NAME: &str = "source-profile-template-v1.zip";

/// 推移表空期间列（§5：未填不能导成真实期间）。
pub fn blank_trend_headers(period_columns: usize) -> Vec<String> {
    let mut h: Vec<String> = vec!["科目".into(), "单位".into(), "取数来源".into()];
    for i in 1..=period_columns {
        h.push(format!("期间{i}（填写ISO期间）"));
    }
    h.push("累计".into());
    h
}

fn blank_accounting() -> ExportResult<Vec<u8>> {
    let mut book = Workbook::new();
    let ws = book.add_worksheet();
    ws.set_name(SHEET_ACCOUNTING)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    write_headers(ws, HEADER_ROW, &ACCOUNTING_HEADERS)?;
    let ws = book.add_worksheet();
    ws.set_name(SHEET_TREND)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    let trend = blank_trend_headers(3);
    let refs: Vec<&str> = trend.iter().map(String::as_str).collect();
    write_headers(ws, HEADER_ROW, &refs)?;
    let ws = book.add_worksheet();
    ws.set_name(SHEET_DIFF)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    write_headers(ws, HEADER_ROW, &DIFF_HEADERS)?;
    book.save_to_buffer()
        .map_err(|e| ExportError(e.to_string()))
}

fn blank_trend() -> ExportResult<Vec<u8>> {
    let mut book = Workbook::new();
    let ws = book.add_worksheet();
    ws.set_name(SHEET_TREND)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    let trend = blank_trend_headers(3);
    let refs: Vec<&str> = trend.iter().map(String::as_str).collect();
    write_headers(ws, HEADER_ROW, &refs)?;
    let ws = book.add_worksheet();
    ws.set_name(SHEET_CHART)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    write_headers(ws, HEADER_ROW, &CHART_HEADERS)?;
    write_text(ws, HEADER_ROW + 1, 0, "")?;
    book.save_to_buffer()
        .map_err(|e| ExportError(e.to_string()))
}

fn blank_task() -> ExportResult<Vec<u8>> {
    let mut book = Workbook::new();
    let ws = book.add_worksheet();
    ws.set_name("本期任务单检视")
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    // 模板不含技术列（task_id/step_id 由导出产生；字段说明列出）
    write_headers(ws, HEADER_ROW, &TASK_REVIEW_HEADERS[..14])?;
    let ws = book.add_worksheet();
    ws.set_name("下期任务单计划")
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    write_headers(ws, HEADER_ROW, &TASK_PLAN_HEADERS[..13])?;
    let ws = book.add_worksheet();
    ws.set_name(SHEET_TASK_EVIDENCE)
        .map_err(|e| ExportError(e.to_string()))?;
    write_blank_metadata(ws)?;
    write_headers(ws, HEADER_ROW, &EVIDENCE_HEADERS)?;
    book.save_to_buffer()
        .map_err(|e| ExportError(e.to_string()))
}

/// 字段说明（source profile 的别名表摘要＋元数据位置＋技术列说明；无样例业务数据）。
pub fn fields_markdown() -> String {
    let mut s = String::new();
    s.push_str("# dsheng 原始 source 模板字段说明（source-profile-template-v1）\n\n");
    s.push_str(
        "## 元数据（每个 source sheet 第 1–4 行；第 5 行空；第 6 行列名；第 7 行起数据）\n\n",
    );
    s.push_str("| 单元格 | 键 | 说明 |\n| --- | --- | --- |\n");
    for (cell, key, note) in [
        ("A1/B1", "dsheng_template_version", "固定 1"),
        ("A2/B2", "workspace_name", "工作区名称"),
        ("C2/D2", "currency", "ISO 大写三字母币种（如 CNY）"),
        (
            "E2/F2",
            "tax_basis",
            "inclusive / exclusive / not_applicable",
        ),
        ("A3/B3", "source_from", "来源期间起（YYYY-MM-DD）"),
        ("C3/D3", "source_to", "来源期间止（YYYY-MM-DD）"),
        ("E3/F3", "grain", "day / week / month / quarter"),
        (
            "K3/L3",
            "target_from",
            "目标期间起（导出可填；任务目标窗口不与来源窗口混用）",
        ),
        ("M3/N3", "target_to", "目标期间止"),
        ("A4/B4", "timezone", "IANA 时区（默认 Asia/Shanghai）"),
        ("C4/D4", "week_start_day", "ISO 1–7（默认 1）"),
        ("E4/F4", "fiscal_year_start_month", "1–12（默认 1）"),
        ("G4/H4", "coverage_status", "complete / partial / unknown"),
        ("I4/J4", "empty_days_are_zero", "true / false（默认 false）"),
        (
            "K4/L4",
            "month_start_day",
            "月度起始日1–31（默认1）；短月取月末，下月仍按原日号",
        ),
    ] {
        s.push_str(&format!("| {cell} | {key} | {note} |\n"));
    }
    s.push_str("\n元数据为空时由运行时导入预览显式补齐；无元数据的旧模板不能硬套行位。\n\n");
    s.push_str("## 核算表模板.xlsx\n\n");
    s.push_str(&format!("- 核算表：{}\n", ACCOUNTING_HEADERS.join("、")));
    s.push_str(&format!(
        "- 推移表：{}\n",
        blank_trend_headers(3).join("、")
    ));
    s.push_str(&format!("- 差异分析：{}\n\n", DIFF_HEADERS.join("、")));
    s.push_str("## 推移表模板.xlsx\n\n");
    s.push_str(&format!(
        "- 推移表：{}（重复来源不能双计）\n",
        blank_trend_headers(3).join("、")
    ));
    s.push_str(&format!(
        "- 图表：{}（空配置，不造趋势）\n\n",
        CHART_HEADERS.join("、")
    ));
    s.push_str("## 任务单模板.xlsx\n\n");
    s.push_str(&format!(
        "- 本期任务单检视：{}\n",
        TASK_REVIEW_HEADERS[..14].join("、")
    ));
    s.push_str(&format!(
        "- 下期任务单计划：{}\n",
        TASK_PLAN_HEADERS[..13].join("、")
    ));
    s.push_str(&format!("- 任务依据：{}\n\n", EVIDENCE_HEADERS.join("、")));
    s.push_str("导出的任务工作簿 sheet 名固定为“任务单检视”“任务单计划”（完整期间在第 3 行），");
    s.push_str("并追加技术列 task_id、step_id（可隐藏；身份以该列为准，不得拿行号作身份）。\n\n");
    s.push_str("## 数值与文本规则\n\n");
    s.push_str(
        "- 金额/数量/工时最多 6 位小数；千分位仅接受三位分组；表头明确 % 才按百分数换算。\n",
    );
    s.push_str("- 空单元格为 missing；“待补/待确认”为 pending_input；NA/N/A/- 保留原文不改零。\n");
    s.push_str("- 以 =、+、-、@ 开头的文本按文本处理（导出写文本类型；CSV 单引号转义）。\n");
    s.push_str(
        "- 超过 15 位有效数的数值导出为精确 decimal 文本（number_storage=decimal_text）。\n",
    );
    s.push_str("- 指标行名按注册指标的 label/aliases exact 匹配；未注册行需用户确认。\n");
    s
}

/// 构建固定 two_tables_v1 输出模板（仅核算表/推移表输出，不作为输入）。
pub fn build_two_tables_v1_zip() -> ExportResult<Vec<u8>> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in [
            (TEMPLATE_ACCOUNTING, blank_accounting()?),
            (TEMPLATE_TREND, blank_trend()?),
        ] {
            zip.start_file(name, options)
                .map_err(|e| ExportError(format!("zip 条目失败: {e}")))?;
            zip.write_all(&bytes)
                .map_err(|e| ExportError(format!("zip 写入失败: {e}")))?;
        }
        zip.finish()
            .map_err(|e| ExportError(format!("zip 完成失败: {e}")))?;
    }
    Ok(buffer.into_inner())
}

/// 构建 profile 来源模板。来源模板只包含 generic source sheet，不包含
/// 核算表/推移表/差异分析/图表等生成结果工作表。
pub fn build_source_profile_zip(profile_id: &str) -> ExportResult<Vec<u8>> {
    let (files, sheet_name, headers) = profile_template(profile_id);
    let fields = format!(
        "# source profile template\n\n- profile_id: `{profile_id}`\n- output_contract_id: `two_tables_v1`\n- files: {}\n\n列：{}\n",
        files.join("、"),
        headers.join("、")
    );
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for file_name in files {
            let mut book = Workbook::new();
            let ws = book.add_worksheet();
            ws.set_name(&sheet_name)
                .map_err(|e| ExportError(e.to_string()))?;
            write_blank_metadata(ws)?;
            write_headers(ws, HEADER_ROW, &headers.iter().map(String::as_str).collect::<Vec<_>>())?;
            let workbook = book
                .save_to_buffer()
                .map_err(|e| ExportError(e.to_string()))?;
            zip.start_file(file_name, options)
                .map_err(|e| ExportError(format!("zip 条目失败: {e}")))?;
            zip.write_all(&workbook)
                .map_err(|e| ExportError(format!("zip 写入失败: {e}")))?;
        }
        zip.start_file(TEMPLATE_FIELDS_MD, options)
            .map_err(|e| ExportError(format!("zip 条目失败: {e}")))?;
        zip.write_all(&fields.into_bytes())
            .map_err(|e| ExportError(format!("zip 写入失败: {e}")))?;
        zip.finish()
            .map_err(|e| ExportError(format!("zip 完成失败: {e}")))?;
    }
    Ok(buffer.into_inner())
}

/// 返回 source profile 模板包的实际文件清单，供 CLI/UI 结果摘要使用。
pub fn source_profile_template_files(profile_id: &str) -> Vec<String> {
    let (files, _, _) = profile_template(profile_id);
    files.into_iter().chain(std::iter::once(TEMPLATE_FIELDS_MD.to_string())).collect()
}

fn profile_template(profile_id: &str) -> (Vec<String>, String, Vec<String>) {
    let builtin = match profile_id {
        "manufacturing_demo" | "manufacturing_erp_v1" => Some(include_str!("../../../contracts/v1/source-profiles/manufacturing_demo.json")),
        "service_demo" => Some(include_str!("../../../contracts/v1/source-profiles/service_demo.json")),
        "xsd_trade_v1" => Some(include_str!("../../../contracts/v1/source-profiles/xsd_trade_v1.json")),
        _ => None,
    };
    if let Some(bytes) = builtin {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(bytes) {
            if let Some(template) = value.get("template") {
                let files = template
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|xs| xs.iter().filter_map(|v| v.as_str()).map(str::to_string).collect::<Vec<_>>())
                    .filter(|xs| !xs.is_empty())
                    .unwrap_or_else(|| vec!["source-template.xlsx".into()]);
                let sheet = template.get("sheet_name").and_then(|v| v.as_str()).unwrap_or("source").to_string();
                let headers = template
                    .get("columns")
                    .and_then(|v| v.as_array())
                    .map(|xs| xs.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
                    .filter(|xs| xs.len() >= 2)
                    .unwrap_or_else(|| vec!["期间".into(), "指标".into(), "单位".into(), "计划值".into(), "实际值".into(), "数据来源".into()]);
                return (files, sheet, headers);
            }
        }
    }
    let path = std::path::Path::new(profile_id);
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            let template = value.get("template");
            let files = template
                .and_then(|v| v.get("files"))
                .and_then(|v| v.as_array())
                .map(|xs| {
                    xs.iter()
                        .filter_map(|v| v.as_str())
                        .filter_map(|v| std::path::Path::new(v).file_name().map(|n| n.to_string_lossy().to_string()))
                        .filter(|v| !v.is_empty())
                        .collect::<Vec<_>>()
                })
                .filter(|xs| !xs.is_empty())
                .unwrap_or_else(|| vec!["source-template.xlsx".into()]);
            let sheet = template
                .and_then(|v| v.get("sheet_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("source")
                .to_string();
            let headers = template
                .and_then(|v| v.get("columns"))
                .and_then(|v| v.as_array())
                .map(|xs| xs.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
                .filter(|xs| xs.len() >= 2)
                .unwrap_or_else(|| vec!["期间".into(), "指标".into(), "单位".into(), "计划值".into(), "实际值".into(), "数据来源".into()]);
            return (files, sheet, headers);
        }
    }
    (vec!["source-template.xlsx".into()], "source".into(), vec!["期间".into(), "指标".into(), "单位".into(), "计划值".into(), "实际值".into(), "数据来源".into()])
}

/// 构建模板 ZIP 字节。
pub fn build_template_zip() -> ExportResult<Vec<u8>> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in [
            (TEMPLATE_ACCOUNTING, blank_accounting()?),
            (TEMPLATE_TREND, blank_trend()?),
            (TEMPLATE_TASK, blank_task()?),
            (TEMPLATE_FIELDS_MD, fields_markdown().into_bytes()),
        ] {
            zip.start_file(name, options)
                .map_err(|e| ExportError(format!("zip 条目失败: {e}")))?;
            zip.write_all(&bytes)
                .map_err(|e| ExportError(format!("zip 写入失败: {e}")))?;
        }
        zip.finish()
            .map_err(|e| ExportError(format!("zip 完成失败: {e}")))?;
    }
    Ok(buffer.into_inner())
}
