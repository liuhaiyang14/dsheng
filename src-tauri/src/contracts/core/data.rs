//! 数据源/经营分析 command 实现（UI scan/fieldMappings/analyze；
//! CLI scan/mapping show/analyze——本地计算，无 Provider）。

use super::{analyze_grain_of, dispatch, envelope, mapping_cmd, req_str};
use crate::contracts::env::DesktopEnv;
use dsheng_cli::cli as args;
use serde_json::Value;

/// dsheng scan（directory_label = UI 数据源目录标签）。
pub fn scan(env: &DesktopEnv, workspace_id: String, directory_label: String) -> Value {
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
        dsheng_cli::store::sources::scan_summary_desktop(
            &mut conn,
            workspace.workspace_id,
            &directory_label,
        )
    })());
    __log(&format!(
        "exit ok={:?} {:?}",
        __out.get("ok").and_then(|v| v.as_bool()),
        __t0.elapsed()
    ));
    __out
}

/// dsheng mapping show。
pub fn mapping_show(env: &DesktopEnv, workspace_id: String, run_id: String) -> Value {
    envelope(dispatch(
        env,
        mapping_cmd(args::MappingAction::Show {
            workspace: Some(workspace_id),
            run: run_id,
        }),
    ))
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
        let db = env.db_arg()?;
        let conn = dsheng_cli::ctx::open_db(Some(&db))?;
        let resolved = dsheng_cli::ctx::resolve_workspace(&conn, workspace.as_ref())?;
        let policy_version = dsheng_cli::store::sources::validate_desktop_analysis_period(
            &conn,
            resolved.workspace_id,
            &from,
            &to,
            &grain_name,
        )?;
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
            },
        )
    })())
}

/// dsheng template download（--package import；空白三工作簿＋字段说明）。
pub fn template_download(env: &DesktopEnv, out_path: String) -> Value {
    envelope(dispatch(
        env,
        args::Command::Template {
            action: args::TemplateAction::Download {
                package: "import".into(),
                out: out_path,
            },
        },
    ))
}
