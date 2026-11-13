//! scan / mapping / manual-input / metric / source-conflict / analyze / analysis show
//! （cli-surface 12–22）。

use crate::cli;
use crate::ctx;
use crate::error::{CliError, CliResult};
use crate::jsonio;
use crate::store::analysis as analysis_store;
use crate::store::sources as sources_store;
use dsheng_core::contracts::cli::CliErrorCode;

pub fn scan(
    workspace: Option<String>,
    dir: String,
    source_profile: Option<String>,
    generic: bool,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let mut conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    let profile = sources_store::resolve_source_profile(source_profile.as_deref(), generic)?;
    sources_store::scan_summary_with_profile(&mut conn, ws.workspace_id, &dir, &profile)
}

pub fn mapping(action: cli::MappingAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    let conn = ctx::open_db(db)?;
    match action {
        cli::MappingAction::Show { workspace, run } => {
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            sources_store::mapping_show(&conn, ws.workspace_id, &run)
        }
        cli::MappingAction::Set {
            workspace,
            run,
            input_json,
        } => {
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let value = jsonio::read_json(&input_json)?;
            let input: sources_store::MappingSetInput = serde_json::from_value(value)
                .map_err(|e| CliError::invalid(format!("mapping set 输入非法: {e}")))?;
            sources_store::mapping_set(&conn, ws.workspace_id, &run, input)
        }
    }
}

pub fn manual_input(
    action: cli::ManualInputAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    match action {
        cli::ManualInputAction::List {
            workspace,
            period_key,
        } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            sources_store::manual_input_list(&conn, ws.workspace_id, &period_key)
        }
        cli::ManualInputAction::Set {
            workspace,
            period_key,
            input_json,
        } => {
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let value = jsonio::read_json(&input_json)?;
            let input: sources_store::ManualInputSet = serde_json::from_value(value)
                .map_err(|e| CliError::invalid(format!("manual-input set 输入非法: {e}")))?;
            sources_store::manual_input_set(&mut conn, ws.workspace_id, &period_key, input)
        }
    }
}

pub fn metric(action: cli::MetricAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::MetricAction::List { workspace } => {
            let conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            Ok(
                serde_json::json!({ "metrics": sources_store::metric_list(&conn, ws.workspace_id)? }),
            )
        }
        cli::MetricAction::Set {
            workspace,
            input_json,
        } => {
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            let value = jsonio::read_json(&input_json)?;
            let input: sources_store::MetricDefinitionSet = serde_json::from_value(value)
                .map_err(|e| CliError::invalid(format!("metric set 输入非法: {e}")))?;
            let view = sources_store::metric_set(&mut conn, ws.workspace_id, input)?;
            Ok(view)
        }
    }
}

pub fn source_conflict(
    action: cli::SourceConflictAction,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    match action {
        cli::SourceConflictAction::List { workspace, run } => {
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
            sources_store::conflict_list(&conn, ws.workspace_id, &run)
        }
        cli::SourceConflictAction::Resolve {
            workspace,
            run,
            input_json,
        } => {
            let mut conn = ctx::open_db(db)?;
            let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
            ctx::ensure_object_in_workspace(
                &conn,
                "analysis_runs",
                "run_id",
                &run,
                ws.workspace_id,
                "run",
            )?;
            let value = jsonio::read_json(&input_json)?;
            let input: sources_store::ConflictResolveInput = serde_json::from_value(value)
                .map_err(|e| CliError::invalid(format!("source-conflict resolve 输入非法: {e}")))?;
            sources_store::conflict_resolve(&mut conn, ws.workspace_id, &run, input)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn analyze(
    workspace: Option<String>,
    dir: String,
    from: String,
    to: String,
    grain: cli::AnalyzeGrain,
    policy: Option<String>,
    recalculate: bool,
    input_json: Option<String>,
    source_profile: Option<String>,
    generic: bool,
    db: Option<&String>,
) -> CliResult<serde_json::Value> {
    let mut conn = ctx::open_db(db)?;
    let ws = ctx::resolve_workspace(&conn, workspace.as_ref())?;
    let extra: Option<analysis_store::AnalyzeExtraInput> = match input_json {
        Some(path) => {
            let value = jsonio::read_json(&path)?;
            Some(
                serde_json::from_value(value)
                    .map_err(|e| CliError::invalid(format!("analyze --input-json 非法: {e}")))?,
            )
        }
        None => None,
    };
    let grain_auto = matches!(grain, cli::AnalyzeGrain::Auto);
    let grain_explicit = match grain {
        cli::AnalyzeGrain::Auto => None,
        cli::AnalyzeGrain::Day => Some("day".into()),
        cli::AnalyzeGrain::Week => Some("week".into()),
        cli::AnalyzeGrain::Month => Some("month".into()),
        cli::AnalyzeGrain::Quarter => Some("quarter".into()),
    };
    let profile = sources_store::resolve_source_profile(source_profile.as_deref(), generic)?;
    analysis_store::run_analyze_with_profile(
        &mut conn,
        ws.workspace_id,
        &dir,
        &from,
        &to,
        grain_auto,
        grain_explicit,
        policy,
        recalculate,
        extra,
        profile,
    )
}

pub fn analysis(action: cli::AnalysisAction, db: Option<&String>) -> CliResult<serde_json::Value> {
    match action {
        cli::AnalysisAction::Show {
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
            let (current, _) = dsheng_core::repository::analysis_runs::current_revision(
                &conn,
                &ws.workspace_id.to_string(),
                &run,
            )
            .map_err(CliError::from)?;
            analysis_store::analysis_run_view(
                &conn,
                ws.workspace_id,
                &run,
                revision.unwrap_or(current),
            )
        }
    }
}

// 保持 InvalidArgument 的 grain 值域已由 clap 枚举保证；此处引用错误码避免未用告警。
#[allow(dead_code)]
fn unused_code_anchor() -> CliErrorCode {
    CliErrorCode::DataValidationFailed
}
