//! 业务包 ZIP（§5）：三工作簿＋结构化 JSON＋导出元数据。
//!
//! 元数据含 workspace/run/revision/as_of、policy、指标 hash、source manifests、
//! 基期 run/revision、过滤器、图表数据来源、decimal_text 单元格清单；
//! 不写本机绝对路径/Key。

use super::cells::DecimalTextLog;
use super::workbook::{build_workbook, WorkbookKind};
use super::{ExportData, ExportError, ExportResult};
use serde_json::json;
use std::io::Write;

pub const BUNDLE_METADATA: &str = "metadata.json";
pub const BUNDLE_ANALYSIS: &str = "analysis.json";
pub const BUNDLE_TASKS: &str = "tasks.json";
pub const BUNDLE_MEETING: &str = "meeting.json";

/// 元数据 JSON（§5 字段；decimal_text 单元格清单来自三工作簿写出记录）。
pub fn metadata_json(data: &ExportData, logs: &[(&str, &DecimalTextLog)]) -> serde_json::Value {
    let m = &data.meta;
    json!({
        "schema_version": "1",
        "kind": "business_bundle",
        "workspace_id": m.workspace_id,
        "workspace_name": m.workspace_name,
        "run_id": m.run_id,
        "revision_no": m.revision_no,
        "as_of": m.as_of,
        "analysis_input_fingerprint": m.analysis_input_fingerprint,
        "policy": m.policy,
        "period": {"source_from": m.source_from, "source_to": m.source_to,
                   "target_from": m.target_from, "target_to": m.target_to, "grain": m.grain},
        "coverage_status": m.coverage_status,
        "empty_days_are_zero": m.empty_days_are_zero,
        "metric_hashes": m.metric_hashes,
        "source_manifests": m.source_manifests,
        "base_refs": m.base_refs,
        "filters": m.filters,
        "chart_sources": data.series.iter().map(|s| json!({
            "chart_id": format!("chart:{}", s["metric_key"].as_str().unwrap_or_default()),
            "metric_key": s["metric_key"],
            "buckets": s["buckets"],
        })).collect::<Vec<_>>(),
        "number_storage": {
            "policy": ">15 位有效数写 decimal 文本（decimal_text）；其余数值类型",
            "decimal_text_cells": logs.iter().flat_map(|(book, log)| {
                log.cells.iter().map(move |c| json!({
                    "workbook": book, "sheet": c.sheet, "cell": c.cell, "value": c.value,
                    "number_storage": "decimal_text",
                }))
            }).collect::<Vec<_>>(),
        },
        "files": [
            WorkbookKind::Accounting.file_name(),
            WorkbookKind::Trend.file_name(),
            WorkbookKind::Task.file_name(),
            BUNDLE_ANALYSIS, BUNDLE_TASKS, BUNDLE_MEETING, BUNDLE_METADATA,
        ],
    })
}

/// 构建业务包 ZIP 字节。
pub fn build_bundle(data: &ExportData) -> ExportResult<Vec<u8>> {
    let (acc, acc_log) = build_workbook(WorkbookKind::Accounting, data)?;
    let (trend, trend_log) = build_workbook(WorkbookKind::Trend, data)?;
    let (task, task_log) = build_workbook(WorkbookKind::Task, data)?;
    let metadata = metadata_json(
        data,
        &[
            (WorkbookKind::Accounting.file_name(), &acc_log),
            (WorkbookKind::Trend.file_name(), &trend_log),
            (WorkbookKind::Task.file_name(), &task_log),
        ],
    );
    let analysis = json!({
        "schema_version": "1",
        "workspace_id": data.meta.workspace_id,
        "run_id": data.meta.run_id,
        "revision_no": data.meta.revision_no,
        "analysis_input_fingerprint": data.meta.analysis_input_fingerprint,
        "grain": data.meta.grain,
        "period": {"from": data.meta.source_from, "to": data.meta.source_to},
        "metrics": data.metrics,
        "series": data.series,
    });
    let tasks = json!({
        "schema_version": "1",
        "run_id": data.meta.run_id,
        "tasks": data.tasks,
        "evidence": data.evidence,
    });
    let meeting = super::meeting::meeting_json(data)?;
    let mut buffer = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut buffer);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let entries: Vec<(&str, Vec<u8>)> = vec![
            (WorkbookKind::Accounting.file_name(), acc),
            (WorkbookKind::Trend.file_name(), trend),
            (WorkbookKind::Task.file_name(), task),
            (
                BUNDLE_ANALYSIS,
                serde_json::to_vec_pretty(&analysis).map_err(|e| ExportError(e.to_string()))?,
            ),
            (
                BUNDLE_TASKS,
                serde_json::to_vec_pretty(&tasks).map_err(|e| ExportError(e.to_string()))?,
            ),
            (
                BUNDLE_MEETING,
                serde_json::to_vec_pretty(&meeting).map_err(|e| ExportError(e.to_string()))?,
            ),
            (
                BUNDLE_METADATA,
                serde_json::to_vec_pretty(&metadata).map_err(|e| ExportError(e.to_string()))?,
            ),
        ];
        for (name, bytes) in entries {
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
