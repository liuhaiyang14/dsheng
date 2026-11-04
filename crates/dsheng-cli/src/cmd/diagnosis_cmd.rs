//! diagnosis 命令组（cli-surface 23–29）。
//!
//! 本文件：list/show/export（本地读）与 revise/pending cancel（本地写）。
//! generate/continue（双门禁）在 S3 由 generation 门禁层接线。

use crate::cli;
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::jsonio;
use crate::out;
use crate::store::diagnosis as diagnosis_store;
use dsheng_core::contracts::cli::CliErrorCode;

pub fn dispatch(action: cli::DiagnosisAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    dispatch_with_progress(action, db, None)
}

/// 可选注入进度 sink（桌面壳 confirm 路径；None＝CLI stderr 节流）。
pub fn dispatch_with_progress(
    action: cli::DiagnosisAction,
    db: Option<&String>,
    progress: Option<crate::store::generation::ProgressSink>,
) -> CliResult<serde_json::Value> {
    match action {
        cli::DiagnosisAction::List { workspace, run } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            diagnosis_store::report_list(&conn, ws.workspace_id, &run)
        }
        cli::DiagnosisAction::Show {
            workspace,
            report,
            revision,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            diagnosis_store::report_show(&conn, ws.workspace_id, &report, revision)
        }
        cli::DiagnosisAction::Export {
            workspace,
            report,
            revision,
            format,
            out,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let value = diagnosis_store::report_show(&conn, ws.workspace_id, &report, revision)?;
            let bytes: Vec<u8> = match format {
                cli::ExportDocFormat::Json => serde_json::to_vec_pretty(&value)
                    .map_err(|e| CliError::new(CliErrorCode::ExportFailed, e.to_string()))?,
                cli::ExportDocFormat::Md => diagnosis_store::report_markdown(&value).into_bytes(),
                cli::ExportDocFormat::Pdf => {
                    return Err(CliError::new(
                        CliErrorCode::ExportFailed,
                        "PDF 导出未实现（合同枚举含 pdf；当前构建仅 md/json）",
                    ))
                }
            };
            let size = out::write_out_file(&out, &bytes)?;
            Ok(out::export_written(&out, size))
        }
        cli::DiagnosisAction::Revise {
            workspace,
            session,
            fact,
            replacement_stdin,
        } => {
            if !replacement_stdin {
                return Err(
                    CliError::invalid("revise 需要 --replacement-stdin 提供完整更正声明")
                        .with_field("--replacement-stdin"),
                );
            }
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let replacement = jsonio::read_stdin_text()?;
            diagnosis_store::revise(&mut conn, ws.workspace_id, &session, &fact, &replacement)
        }
        cli::DiagnosisAction::Pending { action } => match action {
            cli::DiagnosisPendingAction::Cancel {
                workspace,
                session,
                turn,
                expected_revision,
                confirm_local,
            } => {
                let mut conn = ctx::open_db(db)?;
                let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
                diagnosis_store::pending_cancel(
                    &mut conn,
                    ws.workspace_id,
                    &session,
                    &turn,
                    expected_revision,
                    confirm_local,
                )
            }
        },
        cli::DiagnosisAction::Generate { .. } | cli::DiagnosisAction::Continue { .. } => {
            super::generation_cmd::diagnosis_generation_with_progress(action, db, progress)
        }
    }
}
