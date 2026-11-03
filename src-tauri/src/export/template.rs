//! 模板包 import-template-v1.zip（§5）：三份空白 XLSX＋字段说明 Markdown。
//!
//! 无样例金额/客户/人员；元数据键位固定、值空；列名按 §5 表；推移表空期间列
//! 为“期间1（填写ISO期间）”等；任务 sheet 用“本期/下期”前缀名（导入器接受
//! 无日期前缀同一后缀）。

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

pub const TEMPLATE_ZIP_NAME: &str = "import-template-v1.zip";
pub const TEMPLATE_ACCOUNTING: &str = "核算表模板.xlsx";
pub const TEMPLATE_TREND: &str = "推移表模板.xlsx";
pub const TEMPLATE_TASK: &str = "任务单模板.xlsx";
pub const TEMPLATE_FIELDS_MD: &str = "字段说明.md";

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

/// 字段说明（§4.2 别名表摘要＋§5 元数据位置＋技术列说明；无样例业务数据）。
pub fn fields_markdown() -> String {
    let mut s = String::new();
    s.push_str("# dsheng 导入模板字段说明（import-template-v1）\n\n");
    s.push_str(
        "## 元数据（三份工作簿每个 sheet 第 1–4 行；第 5 行空；第 6 行列名；第 7 行起数据）\n\n",
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
