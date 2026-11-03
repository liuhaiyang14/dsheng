//! 桌面 command 注册表（S5 联调冒烟基准）。
//!
//! `COMMANDS` 是 UI `src-ui/src/api/tauriClient.ts` 全部 invoke 调用的
//! 权威清单（命令名 + UI 侧 camelCase 参数键；键集含可选参数——UI 以
//! `x: x ?? null` 形式恒定发送）。集成测试将本表与 tauriClient.ts 静态
//! 提取结果逐一比对，并核对本 crate `generate_handler![...]` 注册名，
//! 保证 UI↔壳两侧不漂移。参数名映射：Rust snake_case ↔ UI camelCase
//! （tauri command 默认 lower_camel_case 键）。

/// 单条 command 规格。
pub struct CommandSpec {
    /// Tauri command 名（= Rust fn 名，snake_case）。
    pub name: &'static str,
    /// UI invoke 第二参数对象的键集（camelCase，按 UI 发送顺序）。
    pub args: &'static [&'static str],
}

/// UI DshengClient 全部命令（31 条 + ping 连通性探测在 mod.rs 单列）。
pub const COMMANDS: &[CommandSpec] = &[
    // ---- workspace / period ----
    CommandSpec {
        name: "workspace_list",
        args: &[],
    },
    CommandSpec {
        name: "workspace_create",
        args: &["input"],
    },
    CommandSpec {
        name: "workspace_update",
        args: &["input"],
    },
    CommandSpec {
        name: "workspace_use",
        args: &["input"],
    },
    CommandSpec {
        name: "period_list",
        args: &["workspaceId"],
    },
    CommandSpec {
        name: "period_set",
        args: &["input"],
    },
    // ---- provider ----
    CommandSpec {
        name: "provider_list",
        args: &["workspaceId"],
    },
    CommandSpec {
        name: "provider_set",
        args: &["input", "apiKey"],
    },
    CommandSpec {
        name: "provider_use",
        args: &["input"],
    },
    CommandSpec {
        name: "provider_clear_key",
        args: &["input"],
    },
    CommandSpec {
        name: "provider_delete",
        args: &["input"],
    },
    // ---- 数据源 / 经营分析 ----
    CommandSpec {
        name: "template_download",
        args: &["out_path"],
    },
    CommandSpec {
        name: "scan",
        args: &["workspaceId", "directoryLabel"],
    },
    CommandSpec {
        name: "mapping_show",
        args: &["workspaceId", "runId"],
    },
    CommandSpec {
        name: "analyze",
        args: &["input"],
    },
    // ---- 诊断（双门禁 preview/confirm + 读/导出） ----
    CommandSpec {
        name: "diagnosis_generate_preview",
        args: &["request"],
    },
    CommandSpec {
        name: "diagnosis_generate_confirm",
        args: &["confirmationId"],
    },
    CommandSpec {
        name: "diagnosis_continue_preview",
        args: &["request"],
    },
    CommandSpec {
        name: "diagnosis_continue_confirm",
        args: &["confirmationId"],
    },
    CommandSpec {
        name: "diagnosis_list",
        args: &["workspaceId", "runId"],
    },
    CommandSpec {
        name: "diagnosis_show",
        args: &["workspaceId", "reportId", "revision"],
    },
    CommandSpec {
        name: "diagnosis_export",
        args: &["workspaceId", "reportId", "format", "outPath"],
    },
    // ---- 任务生成 / 合并 ----
    CommandSpec {
        name: "task_generate_preview",
        args: &["request"],
    },
    CommandSpec {
        name: "task_generate_confirm",
        args: &["confirmationId"],
    },
    CommandSpec {
        name: "task_reconcile_dry_run",
        args: &["input"],
    },
    CommandSpec {
        name: "task_reconcile_apply",
        args: &["input", "applyToken"],
    },
    // ---- 任务读写 / 时间线 ----
    CommandSpec {
        name: "task_list",
        args: &["workspaceId", "runId"],
    },
    CommandSpec {
        name: "task_show",
        args: &["workspaceId", "taskId"],
    },
    CommandSpec {
        name: "task_update",
        args: &["input"],
    },
    CommandSpec {
        name: "task_timeline",
        args: &["query"],
    },
    // ---- 历史 ----
    CommandSpec {
        name: "history_list",
        args: &["workspaceId"],
    },
    CommandSpec {
        name: "history_show",
        args: &["workspaceId", "runId", "revision"],
    },
];

/// 注册表一致性自检（编译期数据，运行时单测使用）。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_names_unique() {
        let mut names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "command 名必须唯一");
    }
}
