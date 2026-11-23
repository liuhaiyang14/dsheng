//! 数据源/经营分析 command 实现（UI scan/fieldMappings/analyze；
//! CLI scan/mapping show/analyze——本地计算，无 Provider）。

use super::{analyze_grain_of, dispatch, envelope, req_str};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use serde_json::Value;

/// dsheng scan（directory_label = UI 数据源目录标签）。
pub fn scan_with_profile(
    env: &DesktopEnv,
    workspace_id: String,
    directory_label: String,
    source_profile: Option<String>,
    generic: bool,
) -> Value {
    let __t0 = std::time::Instant::now();
    let __ws = workspace_id.clone();
    let __dir = directory_label.clone();
    let __log = move |msg: &str| {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/dsheng-cmd.log")
        {
            let _ = writeln!(f, "[scan] {msg} dir={__dir} ws={__ws}");
        }
    };
    __log("enter");
    let __out = envelope((|| {
        let db = env.db_arg()?;
        let mut conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let profile =
            dsheng_cli::store::sources::resolve_source_profile(source_profile.as_deref(), generic)?;
        super::metric_defaults::ensure_defaults(&mut conn, workspace.workspace_id)?;
        dsheng_cli::store::sources::scan_summary_desktop_with_profile(
            &mut conn,
            workspace.workspace_id,
            &directory_label,
            &profile,
        )
    })());
    __log(&format!(
        "exit ok={:?} {:?}",
        __out.get("ok").and_then(|v| v.as_bool()),
        __t0.elapsed()
    ));
    __out
}

/// 旧测试/示例的三参数兼容入口；桌面 command 使用带 profile 的入口。
pub fn scan(env: &DesktopEnv, workspace_id: String, directory_label: String) -> Value {
    scan_with_profile(env, workspace_id, directory_label, None, false)
}

/// dsheng mapping show。
pub fn mapping_show(env: &DesktopEnv, workspace_id: String, run_id: String) -> Value {
    envelope((|| {
        let db = env.db_arg()?;
        let conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let mut result =
            dsheng_cli::store::sources::mapping_show(&conn, workspace.workspace_id, &run_id)?;
        let metrics = dsheng_cli::store::sources::metric_list(&conn, workspace.workspace_id)?;
        result["mappings"] = mapping_display_rows(&result["mappings"], &metrics);
        Ok(result)
    })())
}

// Record snapshots retain cell-level evidence. The screen needs one readable
// field row per source/metric, with the same four-field contract as legacy maps.
fn mapping_display_rows(snapshot: &Value, metrics: &[Value]) -> Value {
    let mut rows = std::collections::BTreeMap::new();
    for entry in snapshot.as_array().into_iter().flatten() {
        let key = entry["metric_key"]
            .as_str()
            .or_else(|| entry["tool_field"].as_str())
            .unwrap_or_default();
        let label = metrics
            .iter()
            .find(|metric| metric["metric_key"].as_str() == Some(key))
            .and_then(|metric| metric["label"].as_str())
            .unwrap_or_else(|| if key.is_ascii() { "来源用途" } else { key });
        let source = entry["source_label"]
            .as_str()
            .or_else(|| entry["source_id"].as_str())
            .unwrap_or("来源数据");
        let sheet = entry["sheet"].as_str().unwrap_or_default();
        let recognized = entry["recognized"].as_str().unwrap_or("已识别");
        let usage = entry["usage"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("用于计算{label}；计算结果以经营分析为准"));
        rows.entry((source.to_owned(), sheet.to_owned(), key.to_owned())).or_insert_with(|| serde_json::json!({
            "tool_field": label, "source_label": if sheet.is_empty() { source.to_owned() } else { format!("{source} · {sheet}") },
            "recognized": recognized, "usage": usage,
        }));
    }
    Value::Array(rows.into_values().collect())
}

#[cfg(test)]
mod mapping_display_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn record_evidence_is_grouped_into_readable_field_rows() {
        let snapshot = json!([
            {"metric_key":"labor_cost", "source_id":"工资.xlsx", "sheet":"工资", "evidence_id":"A2"},
            {"metric_key":"labor_cost", "source_id":"工资.xlsx", "sheet":"工资", "evidence_id":"A3"}
        ]);
        let rows = mapping_display_rows(
            &snapshot,
            &[json!({"metric_key":"labor_cost", "label":"人工费"})],
        );
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["tool_field"], "人工费");
        assert_eq!(rows[0]["recognized"], "已识别");
        assert!(rows[0]["usage"].as_str().unwrap().contains("人工费"));
    }
}

/// dsheng analyze（本地计算；UI AnalyzeInput 五字段 → CLI flags）。
///
/// 注：CLI 另有 `--policy/--recalculate/--input-json`（历史回算等），
/// UI AnalyzeInput 未暴露——合同缺口见工作包报告。
pub fn analyze(env: &DesktopEnv, input: Value) -> Value {
    envelope((|| {
        let workspace = Some(req_str(&input, "workspace_id")?);
        let dir = req_str(&input, "directory_label")?;
        let from = req_str(&input, "from")?;
        let to = req_str(&input, "to")?;
        let grain_name = req_str(&input, "grain")?;
        let grain = analyze_grain_of(&grain_name)?;
        let source_profile = input
            .get("source_profile")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let generic = input
            .get("generic")
            .and_then(Value::as_bool)
            .unwrap_or(source_profile.is_none());
        let db = env.db_arg()?;
        let mut conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let resolved = dsheng_cli::ctx::resolve_workspace(&conn, workspace.as_ref())?;
        let policy_version = dsheng_cli::store::sources::validate_desktop_analysis_period(
            &conn,
            resolved.workspace_id,
            &from,
            &to,
            &grain_name,
        )?;
        // Older workspaces may already have confirmed files but no metric
        // definitions. Keep their normal Analyze action sufficient to recover.
        super::metric_defaults::ensure_defaults(&mut conn, resolved.workspace_id)?;
        drop(conn);
        dispatch(
            env,
            args::Command::Analyze {
                workspace,
                dir,
                from,
                to,
                grain,
                policy: Some(policy_version),
                recalculate: false,
                input_json: None,
                source_profile,
                generic,
            },
        )
    })())
}

/// dsheng template download：source profile 模板或固定 two_tables_v1 输出模板。
pub fn template_download(
    env: &DesktopEnv,
    out_path: String,
    package: Option<String>,
    profile: Option<String>,
    contract: Option<String>,
) -> Value {
    envelope(dispatch(
        env,
        args::Command::Template {
            action: args::TemplateAction::Download {
                package: package.unwrap_or_else(|| "source".into()),
                contract: contract.or_else(|| Some("two_tables_v1".into())),
                profile,
                out: out_path,
            },
        },
    ))
}

/// 确认 generic source 的字段映射。首次分析尚无 run，因此确认绑定目录和 profile hash。
pub fn source_mapping_set(
    env: &DesktopEnv,
    workspace_id: String,
    directory_label: String,
    profile_hash: String,
    mappings: Value,
) -> Value {
    envelope((|| {
        let db = env.db_arg()?;
        let conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let mappings = serde_json::from_value(mappings)
            .map_err(|e| dsheng_cli::error::CliError::invalid(format!("mappings 输入非法: {e}")))?;
        dsheng_cli::store::sources::source_mapping_set(
            &conn,
            workspace.workspace_id,
            &directory_label,
            &profile_hash,
            mappings,
        )
    })())
}

/// 导出已生成的两表输出（UI 与 CLI 复用同一 ExportData/Workbook 实现）。
/// 不允许把 source 模板或任意输入文件作为两表结果；必须先通过 two_tables_v1 物化门禁。
pub fn export_tables(
    env: &DesktopEnv,
    workspace_id: String,
    run_id: String,
    revision: Option<u32>,
    kind: String,
    out_path: String,
) -> Value {
    envelope((|| {
        let db = env.db_arg()?;
        let conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let workspace = dsheng_cli::ctx::resolve_workspace(&conn, Some(&workspace_id))?;
        let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
            &conn,
            &workspace.workspace_id.to_string(),
            &run_id,
        )
        .map_err(dsheng_cli::error::CliError::from)?;
        let revision_no = revision.unwrap_or(current);
        dsheng_cli::store::sources::require_two_tables_materialized(
            &conn,
            workspace.workspace_id,
            &run_id,
            revision_no,
        )?;
        let cli_data = dsheng_cli::cmd::export_cmd::build_export_data(
            &conn,
            workspace.workspace_id,
            &run_id,
            Some(revision_no),
        )?;
        // CLI 与 Tauri 挂载同一导出源文件，但 Rust 模块各自编译后类型身份不同；
        // 通过已冻结的序列化合同转换，避免桌面端复制一套导出逻辑。
        let data: crate::export::ExportData = serde_json::from_value(
            serde_json::to_value(cli_data)
                .map_err(|e| dsheng_cli::error::CliError::invalid(e.to_string()))?,
        )
        .map_err(|e| dsheng_cli::error::CliError::invalid(e.to_string()))?;
        let (bytes, output_kind) = match kind.as_str() {
            "accounting" => (
                crate::export::workbook::build_workbook(
                    crate::export::workbook::WorkbookKind::Accounting,
                    &data,
                )
                .map_err(|e| {
                    dsheng_cli::error::CliError::new(
                        dsheng_core::contracts::cli::CliErrorCode::ExportFailed,
                        e.to_string(),
                    )
                })?
                .0,
                "workbook",
            ),
            "trend" => (
                crate::export::workbook::build_workbook(
                    crate::export::workbook::WorkbookKind::Trend,
                    &data,
                )
                .map_err(|e| {
                    dsheng_cli::error::CliError::new(
                        dsheng_core::contracts::cli::CliErrorCode::ExportFailed,
                        e.to_string(),
                    )
                })?
                .0,
                "workbook",
            ),
            "bundle" => (
                crate::export::bundle::build_bundle(&data).map_err(|e| {
                    dsheng_cli::error::CliError::new(
                        dsheng_core::contracts::cli::CliErrorCode::ExportFailed,
                        e.to_string(),
                    )
                })?,
                "bundle",
            ),
            _ => {
                return Err(dsheng_cli::error::CliError::invalid(
                    "kind 仅支持 accounting|trend|bundle",
                ))
            }
        };
        let bytes_len = bytes.len() as u64;
        dsheng_cli::out::write_out_file(&out_path, &bytes)?;
        let mut result = dsheng_cli::out::export_written(&out_path, bytes_len);
        result["kind"] = serde_json::json!(output_kind);
        result["contract"] = serde_json::json!("two_tables_v1");
        result["revision_no"] = serde_json::json!(revision_no);
        Ok(result)
    })())
}
