//! export / template 命令（cli-surface 48–52）：从 run revision 组装 ExportData
//! 后调用共享导出库（src-tauri/src/export）。

use crate::cli;
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::export::{self as lib, ExportData, ExportMeta};
use crate::out;
use crate::store::analysis as analysis_store;
use crate::store::views;
use dsheng_core::contracts::cli::CliErrorCode;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use uuid::Uuid;

fn export_err(e: lib::ExportError) -> CliError {
    CliError::new(CliErrorCode::ExportFailed, e.0)
}

/// 由 run revision 组装导出输入（read-views 视图＋元数据；无绝对路径/Key）。
pub fn build_export_data(
    conn: &Connection,
    workspace_id: Uuid,
    run_id: &str,
    revision: Option<u32>,
) -> CliResult<ExportData> {
    ctx::ensure_object_in_workspace(conn, "analysis_runs", "run_id", run_id, workspace_id, "run")?;
    let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
        conn,
        &workspace_id.to_string(),
        run_id,
    )
    .map_err(CliError::from)?;
    let revision = revision.unwrap_or(current);
    // 所有业务导出均以固定 two_tables_v1 为基础；没有完整物化登记时，
    // 不能写出空/半成品两表或带有误导性依据的会议包。
    crate::store::sources::require_two_tables_materialized(
        conn,
        workspace_id,
        run_id,
        revision,
    )?;
    let run_view = analysis_store::analysis_run_view(conn, workspace_id, run_id, revision)?;
    let (
        policy_raw,
        sources_raw,
        defs_raw,
        base_refs_raw,
        action_raw,
        created_at,
        fingerprint,
    ): (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT period_policy_snapshot_json, source_selection_snapshot_json,
                    metric_definitions_snapshot_json, comparison_base_refs_json,
                    action_period_snapshot_json, created_at, analysis_input_fingerprint
             FROM run_revisions WHERE run_id = ?1 AND revision_no = ?2",
            params![run_id, revision as i64],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .map_err(|_| CliError::new(CliErrorCode::DataValidationFailed, "revision 不存在"))?;
    let policy: Value = serde_json::from_str(&policy_raw).unwrap_or_else(|_| json!({}));
    let sources: Vec<Value> = serde_json::from_str(&sources_raw).unwrap_or_default();
    let defs: Vec<Value> = serde_json::from_str(&defs_raw).unwrap_or_default();
    let base_refs: Vec<Value> = serde_json::from_str(&base_refs_raw).unwrap_or_default();
    let action: Option<Value> = action_raw.and_then(|s| serde_json::from_str(&s).ok());
    let (workspace_name, cadence): (String, String) = conn
        .query_row(
            "SELECT w.name, COALESCE(p.cadence, 'month') FROM workspaces w
             LEFT JOIN period_policies p ON p.period_policy_id = w.current_period_policy_id
             WHERE w.workspace_id = ?1",
            params![workspace_id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(CliError::from)?;
    // 指标定义：semantic_type/unit signature（series 累计语义、币种/税口径）
    let mut semantic_types: std::collections::BTreeMap<String, String> = Default::default();
    let mut currency: Option<String> = None;
    let mut tax_basis: Option<String> = None;
    // 新 revision 快照带有当时的语义；优先使用快照，避免指标升版后旧导出
    // 被当前定义重新解释。旧快照没有这些可选字段时再走兼容回退。
    for def in &defs {
        let Some(key) = def.get("metric_key").and_then(Value::as_str) else {
            continue;
        };
        if let Some(st) = def.get("semantic_type").and_then(Value::as_str) {
            semantic_types.insert(key.to_string(), st.to_string());
        }
        if let Some(unit) = def.get("unit_signature") {
            if currency.is_none() {
                currency = unit.get("currency").and_then(Value::as_str).map(str::to_string);
            }
            if tax_basis.is_none() {
                tax_basis = unit.get("tax_basis").and_then(Value::as_str).map(str::to_string);
            }
        }
    }
    {
        let mut stmt = conn.prepare(
            "SELECT metric_key, semantic_type, unit_signature_json FROM metric_definitions m
             WHERE workspace_id = ?1 AND version =
                   (SELECT MAX(version) FROM metric_definitions m2
                    WHERE m2.workspace_id = m.workspace_id AND m2.metric_key = m.metric_key)",
        )?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map(params![workspace_id.to_string()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<Result<_, _>>()
            .map_err(CliError::from)?;
        for (key, st, unit_json) in rows {
            semantic_types.entry(key).or_insert(st);
            if let Ok(u) = serde_json::from_str::<Value>(&unit_json) {
                if currency.is_none() {
                    currency = u["currency"].as_str().map(str::to_string);
                }
                if tax_basis.is_none() {
                    tax_basis = u["tax_basis"].as_str().map(str::to_string);
                }
            }
        }
    }
    // revision 自己的 source profile/coverage 口径优先于当前工作区定义；尤其
    // confirm_at_runtime 不能被默认 money unit 的 exclusive 覆盖。
    if let Some(profile_tax_basis) = policy
        .get("source_profile")
        .and_then(|profile| profile.get("config"))
        .and_then(|config| config.get("normalization"))
        .and_then(|normalization| normalization.get("tax_basis"))
        .and_then(Value::as_str)
    {
        tax_basis = Some(profile_tax_basis.to_string());
    }
    let coverage_status = policy
        .get("coverage")
        .and_then(|coverage| coverage.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let empty_days_are_zero = policy
        .get("coverage")
        .and_then(|coverage| coverage.get("empty_days_are_zero"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let series: Vec<Value> = run_view["series"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|mut s| {
            if let Some(obj) = s.as_object_mut() {
                let key = obj
                    .get("metric_key")
                    .and_then(|k| k.as_str())
                    .unwrap_or("")
                    .to_string();
                if let Some(st) = semantic_types.get(&key) {
                    obj.insert("semantic_type".into(), json!(st));
                }
            }
            s
        })
        .collect();
    let metrics: Vec<Value> = run_view["metrics"].as_array().cloned().unwrap_or_default();
    // 任务（run 关联）＋证据行
    let task_ids = views::task_ids_for_revision(conn, workspace_id, run_id, revision)?;
    let tasks: Vec<Value> = task_ids
        .iter()
        .map(|id| views::task_view(conn, workspace_id, *id))
        .collect::<Result<_, _>>()?;
    let mut evidence: Vec<Value> = Vec::new();
    for m in &metrics {
        for ctype in ["mom", "yoy"] {
            let c = &m[ctype];
            if c.is_null() {
                continue;
            }
            for e in c["evidence_ids"].as_array().cloned().unwrap_or_default() {
                let eid = e.as_str().unwrap_or_default().to_string();
                let (file_label, sheet, cell) = split_cell_evidence(&eid);
                evidence.push(json!({
                    "evidence_id": eid,
                    "metric_label": m["metric_label"],
                    "source_period": format!("{}/{}", run_view["period"]["from"].as_str().unwrap_or_default(),
                                              run_view["period"]["to"].as_str().unwrap_or_default()),
                    "target_period": action.as_ref().map(|a| format!("{}/{}",
                        a["from"].as_str().unwrap_or_default(), a["to"].as_str().unwrap_or_default())),
                    "file_label": file_label,
                    "sheet": sheet,
                    "cell_or_row": cell,
                    "comparison_type": ctype,
                    "base_period": format!("{}/{}", c["base_from"].as_str().unwrap_or_default(),
                                            c["base_to"].as_str().unwrap_or_default()),
                    "base_value": c["base_value"],
                    "current_value": m["actual_value"],
                    "absolute_change": c["absolute_change"],
                    "change_rate": c["change_rate"],
                    "pending_reason": c["reason"],
                }));
            }
        }
    }
    for t in &tasks {
        for e in t["evidence"].as_array().cloned().unwrap_or_default() {
            evidence.push(json!({
                "evidence_id": e["evidence_id"],
                "metric_label": t["metric_label"],
                "source_period": Value::Null,
                "target_period": t["target_period"]["from"].as_str().map(|f| format!("{f}/{}", t["target_period"]["to"].as_str().unwrap_or_default())),
                "file_label": e["source_label"],
                "sheet": Value::Null,
                "cell_or_row": e["locator"],
                "comparison_type": Value::Null,
                "base_period": Value::Null,
                "base_value": Value::Null,
                "current_value": Value::Null,
                "absolute_change": Value::Null,
                "change_rate": Value::Null,
                "pending_reason": Value::Null,
            }));
        }
    }
    // 诊断报告（最新）
    let diagnosis: Option<Value> = conn
        .query_row(
            "SELECT report_json FROM diagnosis_reports WHERE run_id = ?1 AND workspace_id = ?2
             AND analysis_revision_no = ?3
             ORDER BY updated_at DESC, report_revision_no DESC LIMIT 1",
            params![run_id, workspace_id.to_string(), revision as i64],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let meta = ExportMeta {
        output_contract_id: "two_tables_v1".into(),
        workspace_id: workspace_id.to_string(),
        workspace_name,
        run_id: run_id.to_string(),
        revision_no: revision,
        as_of: created_at,
        analysis_input_fingerprint: fingerprint,
        policy: lib::PolicyMeta {
            version: policy["version"]
                .as_u64()
                .map(|v| v.to_string())
                .or_else(|| policy["version"].as_str().map(str::to_string))
                .unwrap_or_else(|| "1".into()),
            cadence,
            timezone_id: policy["timezone_id"]
                .as_str()
                .unwrap_or("Asia/Shanghai")
                .to_string(),
            week_start_day: policy["week_start_day"].as_u64().unwrap_or(1) as u32,
            month_start_day: policy["month_start_day"].as_u64().unwrap_or(1) as u32,
            fiscal_year_start_month: policy["fiscal_year_start_month"].as_u64().unwrap_or(1) as u32,
        },
        currency,
        tax_basis,
        source_from: run_view["period"]["from"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        source_to: run_view["period"]["to"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        target_from: action
            .as_ref()
            .and_then(|a| a["from"].as_str().map(str::to_string)),
        target_to: action
            .as_ref()
            .and_then(|a| a["to"].as_str().map(str::to_string)),
        grain: run_view["grain"].as_str().unwrap_or("month").to_string(),
        coverage_status,
        empty_days_are_zero,
        metric_hashes: defs
            .iter()
            .map(|d| lib::MetricHashMeta {
                metric_key: d["metric_key"].as_str().unwrap_or_default().to_string(),
                version: d["version"].as_u64().unwrap_or(1) as u32,
                semantic_hash: d["semantic_hash"].as_str().unwrap_or_default().to_string(),
            })
            .collect(),
        source_manifests: sources
            .iter()
            .map(|s| lib::SourceManifestMeta {
                source_id: s["source_id"].as_str().unwrap_or_default().to_string(),
                file_label: source_label_of(conn, s["source_id"].as_str().unwrap_or_default()),
                sheet: s["sheet"].as_str().map(str::to_string),
                selected_range: s["range"].as_str().unwrap_or_default().to_string(),
                content_hash: s["content_hash"].as_str().unwrap_or_default().to_string(),
            })
            .collect(),
        base_refs: base_refs
            .iter()
            .map(|b| lib::BaseRefMeta {
                run_id: b["run_id"].as_str().unwrap_or_default().to_string(),
                revision_no: b["revision_no"].as_u64().unwrap_or(1) as u32,
            })
            .collect(),
        filters: json!({"revision": revision, "run_id": run_id}),
    };
    Ok(ExportData {
        meta,
        metrics,
        series,
        tasks,
        evidence,
        diagnosis,
    })
}

fn source_label_of(conn: &Connection, source_id: &str) -> String {
    conn.query_row(
        "SELECT file_label FROM source_files WHERE source_id = ?1",
        params![source_id],
        |r| r.get::<_, String>(0),
    )
    .unwrap_or_else(|_| source_id.to_string())
}

fn split_cell_evidence(eid: &str) -> (String, String, String) {
    // cell:<file_label>:<sheet>:<A1> / manual:<id>:r<n>
    let parts: Vec<&str> = eid.splitn(4, ':').collect();
    match parts.as_slice() {
        ["cell", file, sheet, cell] => (file.to_string(), sheet.to_string(), cell.to_string()),
        ["manual", id, rev] => (format!("用户手工输入 {id}"), String::new(), rev.to_string()),
        _ => (eid.to_string(), String::new(), String::new()),
    }
}

pub fn run(args: cli::ExportArgs, db: Option<&String>) -> CliResult<serde_json::Value> {
    let selectors = [
        args.bundle.is_some(),
        args.workbook.is_some(),
        args.meeting,
        args.chart.is_some(),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    if selectors != 1 {
        return Err(CliError::invalid(
            "export 须且仅须一个选择器：--bundle business | --workbook <kind> | --meeting | --chart <id>",
        ));
    }
    let conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, args.workspace.as_ref())?;
    let data = build_export_data(&conn, ws.workspace_id, &args.run, args.revision)?;
    let out_path = args.out.clone();
    if let Some(bundle) = args.bundle.as_deref() {
        if bundle != "business" {
            return Err(CliError::invalid("--bundle 仅支持 business").with_field("--bundle"));
        }
        let bytes = lib::bundle::build_bundle(&data).map_err(export_err)?;
        let size = out::write_out_file(&out_path, &bytes)?;
        let mut written = out::export_written(&out_path, size);
        written["kind"] = json!("bundle");
        written["entries"] = json!([
            "核算表.xlsx",
            "推移表.xlsx",
            "任务单.xlsx",
            "analysis.json",
            "tasks.json",
            "meeting.json",
            "metadata.json"
        ]);
        return Ok(written);
    }
    if let Some(kind) = args.workbook {
        let kind = match kind {
            cli::WorkbookKind::Accounting => lib::workbook::WorkbookKind::Accounting,
            cli::WorkbookKind::Trend => lib::workbook::WorkbookKind::Trend,
            cli::WorkbookKind::Task => lib::workbook::WorkbookKind::Task,
        };
        let (bytes, log) = lib::workbook::build_workbook(kind, &data).map_err(export_err)?;
        let size = out::write_out_file(&out_path, &bytes)?;
        let mut written = out::export_written(&out_path, size);
        written["kind"] = json!("workbook");
        written["decimal_text_cells"] = json!(log.cells.len());
        return Ok(written);
    }
    if args.meeting {
        let format = args.format.ok_or_else(|| {
            CliError::invalid("--meeting 需要 --format md|json").with_field("--format")
        })?;
        let bytes = match format {
            cli::ExportFormat::Md => lib::meeting::meeting_markdown(&data)
                .map_err(export_err)?
                .into_bytes(),
            cli::ExportFormat::Json => {
                serde_json::to_vec_pretty(&lib::meeting::meeting_json(&data).map_err(export_err)?)
                    .map_err(|e| CliError::new(CliErrorCode::ExportFailed, e.to_string()))?
            }
            _ => return Err(CliError::invalid("--meeting 仅支持 md|json").with_field("--format")),
        };
        let size = out::write_out_file(&out_path, &bytes)?;
        let mut written = out::export_written(&out_path, size);
        written["kind"] = json!("meeting");
        written["page_count"] = json!(10);
        return Ok(written);
    }
    let chart_id = args.chart.expect("checked");
    let format = args.format.ok_or_else(|| {
        CliError::invalid("--chart 需要 --format png|csv|xlsx").with_field("--format")
    })?;
    let bytes = match format {
        cli::ExportFormat::Png => lib::chart::chart_png(&data, &chart_id).map_err(export_err)?,
        cli::ExportFormat::Csv => lib::chart::chart_csv(&data, &chart_id).map_err(export_err)?,
        cli::ExportFormat::Xlsx => {
            let mut log = lib::cells::DecimalTextLog::default();
            lib::chart::chart_xlsx(&data, &chart_id, &mut log).map_err(export_err)?
        }
        _ => return Err(CliError::invalid("--chart 仅支持 png|csv|xlsx").with_field("--format")),
    };
    let size = out::write_out_file(&out_path, &bytes)?;
    let mut written = out::export_written(&out_path, size);
    written["kind"] = json!("chart");
    written["chart_id"] = json!(chart_id);
    Ok(written)
}

pub fn template(action: cli::TemplateAction, _db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::TemplateAction::Download {
            package,
            contract,
            profile,
            out: out_path,
        } => {
            let (bytes, package_name, entries): (Vec<u8>, &str, Vec<String>) = match package.as_str()
            {
                "output" => {
                    if contract.as_deref() != Some("two_tables_v1") {
                        return Err(CliError::invalid(
                            "--package output 需要 --contract two_tables_v1",
                        )
                        .with_field("--contract"));
                    }
                    (
                        lib::template::build_two_tables_v1_zip().map_err(export_err)?,
                        lib::template::OUTPUT_TEMPLATE_ZIP_NAME,
                        vec![
                            lib::template::TEMPLATE_ACCOUNTING.to_string(),
                            lib::template::TEMPLATE_TREND.to_string(),
                        ],
                    )
                }
                "source" => {
                    let profile_id = profile.as_deref().unwrap_or("generic");
                    // 模板下载与 source 扫描使用同一 profile 解析规则：未知 ID
                    // 必须在入口处失败，不能静默退回 generic 模板。
                    crate::store::sources::resolve_source_profile(Some(profile_id), false)?;
                    (
                        lib::template::build_source_profile_zip(profile_id).map_err(export_err)?,
                        lib::template::SOURCE_TEMPLATE_ZIP_NAME,
                        lib::template::source_profile_template_files(profile_id),
                    )
                }
                _ => {
                    return Err(CliError::invalid("--package 仅支持 source|output")
                        .with_field("--package"))
                }
            };
            let size = out::write_out_file(&out_path, &bytes)?;
            let mut written = out::export_written(&out_path, size);
            written["kind"] = json!("template");
            written["package"] = json!(package_name);
            if let Some(contract) = contract {
                written["contract"] = json!(contract);
            }
            if let Some(profile) = profile {
                written["profile"] = json!(profile);
            }
            written["entries"] = json!(entries);
            Ok(written)
        }
    }
}
