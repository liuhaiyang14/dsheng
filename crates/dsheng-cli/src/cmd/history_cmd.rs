//! history 命令组（cli-surface 43–47）。

use crate::cli;
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::store::analysis as analysis_store;
use dsheng_core::contracts::cli::CliErrorCode;
use serde_json::json;

pub fn dispatch(action: cli::HistoryAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::HistoryAction::List { workspace } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            Ok(json!({ "runs": analysis_store::history_list(&conn, ws.workspace_id)? }))
        }
        cli::HistoryAction::Show {
            workspace,
            run,
            revision,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            ctx::ensure_object_in_workspace(
                &conn,
                "analysis_runs",
                "run_id",
                &run,
                ws.workspace_id,
                "run",
            )?;
            analysis_store::history_detail(&conn, ws.workspace_id, &run, revision)
        }
        cli::HistoryAction::Revisions { workspace, run } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            analysis_store::history_revisions(&conn, ws.workspace_id, &run)
        }
        cli::HistoryAction::Compare {
            workspace,
            run,
            revision,
            compare_type,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let ctype = match compare_type {
                cli::CompareType::Mom => "mom",
                cli::CompareType::Yoy => "yoy",
            };
            analysis_store::history_compare(&conn, ws.workspace_id, &run, revision, ctype)
        }
        cli::HistoryAction::Delete {
            workspace,
            run,
            dry_run,
            confirm_delete,
        } => {
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let run_id = ctx::uuid_arg(&run, "--run")?;
            if dry_run == confirm_delete.is_some() {
                return Err(
                    CliError::invalid("--dry-run 与 --confirm-delete 必须二选一")
                        .with_field("--dry-run"),
                );
            }
            if dry_run {
                let preview = dsheng_core::repository::history_delete::history_delete_preview(
                    &conn,
                    &ws.workspace_id.to_string(),
                    &run_id,
                    chrono::Utc::now(),
                )
                .map_err(CliError::from)?;
                return serde_json::to_value(preview)
                    .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()));
            }
            let preview_id = confirm_delete.expect("checked");
            let scope = dsheng_core::repository::history_delete::history_delete_confirm(
                &mut conn,
                &ws.workspace_id.to_string(),
                &run_id,
                &preview_id,
                chrono::Utc::now(),
            )
            .map_err(CliError::from)?;
            serde_json::to_value(json!({
                "run_id": run_id.to_string(),
                "deleted": true,
                "scope": scope,
            }))
            .map_err(|e| CliError::new(CliErrorCode::LocalSaveFailed, e.to_string()))
        }
    }
}
