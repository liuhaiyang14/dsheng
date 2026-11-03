# dsheng CLI 命令速查（54 条，语法照抄 contracts/v1/cli-surface.json）

- 全局：`--json` 输出单 envelope（stdout 单对象，进度/诊断仅 stderr）；`--db <路径>` 指定数据库（默认 `DSHENG_DB` 或平台用户数据目录）。未知 flag 与未知输入字段一律拒绝（`INVALID_ARGUMENT`），不会静默忽略。
- 生成类默认值：`request_maximum=1`、无自动重试、非流式；`--dry-run` 仅本地预览；发送须 `--confirmation-id <id>` 与 `--confirm-send` 成对出现，确认 TTL 600 秒。
- API Key 只经 `--api-key-stdin`（stdin）接收；`--input-json -` 与其不能同时占用 stdin。
- 别名注记：`map` 是 `mapping show` 的已废弃只读别名（无隐式变更）；`task update --progress/--status/--note` 是规范化到同一 task-edit 合同的 CLI sugar（仍需 expected revision，无独立校验路径）。

## 只读（read，Provider 调用上限 0）

| 命令 | 语法（合同原文） | 输入/输出契约 | 备注 |
| --- | --- | --- | --- |
| `workspace list` | `dsheng workspace list --json` | 出：`contracts/v1/settings.schema.json#/$defs/workspaceView (array for list)` |  |
| `workspace period list` | `dsheng workspace period list --workspace <id> --json` | 出：`contracts/v1/settings.schema.json#/$defs/periodPolicyView array inside data.policies` |  |
| `provider list` | `dsheng provider list --workspace <id> --json` | 出：`contracts/v1/settings.schema.json#/$defs/providerView array inside data.providers` | Public fields and key_present only; no Key or Key hash |
| `scan` | `dsheng scan --workspace <id> --dir <path> --json` | 入：`docs/specs/data-period-import.md`<br>出：`contracts/v1/read-views.schema.json#/$defs/scanSummaryView` |  |
| `mapping show` | `dsheng mapping show --workspace <id> --run <id> --json` | 入：`docs/specs/data-period-import.md`<br>出：`contracts/v1/read-views.schema.json#/$defs/fieldMappingView` |  |
| `manual-input list` | `dsheng manual-input list --workspace <id> --period-key <key> --json` | 入：`docs/specs/data-period-import.md` |  |
| `metric list` | `dsheng metric list --workspace <id> --json` | 入：`contracts/v1/metric-catalog.json` |  |
| `source-conflict list` | `dsheng source-conflict list --workspace <id> --run <id> --json` | 入：`docs/specs/data-period-import.md` |  |
| `analysis show` | `dsheng analysis show --workspace <id> --run <id> [--revision <n>] --json` | 出：`contracts/v1/read-views.schema.json#/$defs/analysisRunView` | reads a saved immutable revision without recalculation; latest current revision when --revision omitted<br>added by approved additive contract extension 2026-11-02 |
| `diagnosis list` | `dsheng diagnosis list --workspace <id> --run <id> --json` | 出：`contracts/v1/read-views.schema.json#/$defs/diagnosisReportSummaryView` |  |
| `diagnosis show` | `dsheng diagnosis show --workspace <id> --report <id> [--revision <n>] --json` | 出：`crates/dsheng-core/prompts/v1/schema/diagnosis-report.schema.json (report_revision snapshot object)` |  |
| `generation status` | `dsheng generation status --workspace <id> --attempt <id> --json` | 出：`attempt state object: attempt_id/session_id/operation/request_stage/request_count/terminal_state/error_code (descriptive, envelope-wrapped)` |  |
| `task list` | `dsheng task list --workspace <id> [--run <id>] --json` | 出：`contracts/v1/read-views.schema.json#/$defs/taskView` |  |
| `task show` | `dsheng task show --workspace <id> --task <id> --json` | 出：`contracts/v1/read-views.schema.json#/$defs/taskView` |  |
| `task timeline` | `dsheng task timeline --workspace <id> --from <date> --to <date> --grain <day|week|month|quarter> --json` | 出：`contracts/v1/read-views.schema.json#/$defs/timelineEntryView` |  |
| `history list` | `dsheng history list --workspace <id>  --json` | 入：`docs/specs/task-lifecycle.md`<br>出：`contracts/v1/read-views.schema.json#/$defs/historyEntryView` |  |
| `history show` | `dsheng history show --workspace <id> --run <id> [--revision <n>] --json` | 入：`docs/specs/task-lifecycle.md`<br>出：`contracts/v1/read-views.schema.json#/$defs/historyDetailView` |  |
| `history revisions` | `dsheng history revisions --workspace <id> --run <id> --json` | 入：`docs/specs/task-lifecycle.md` |  |
| `history compare` | `dsheng history compare --workspace <id> --run <id> [--revision <n>] --type <mom|yoy> --json` | 入：`docs/specs/task-lifecycle.md` |  |
| `version` | `dsheng --version --json` | — | app version, schema version, contract versions, platform, architecture; no user data |
| `help` | `dsheng --help` | — |  |

## 本地写（local_write，Provider 调用上限 0）

| 命令 | 语法（合同原文） | 输入/输出契约 | 备注 |
| --- | --- | --- | --- |
| `workspace create` | `dsheng workspace create --input-json <file|-> --json` | 入：`contracts/v1/settings.schema.json#/$defs/workspace_create`<br>出：`contracts/v1/settings.schema.json#/$defs/workspaceView (array for list)` |  |
| `workspace update` | `dsheng workspace update --workspace <id> --input-json <file|-> --json` | 入：`contracts/v1/settings.schema.json#/$defs/workspace_update`<br>出：`contracts/v1/settings.schema.json#/$defs/workspaceView (array for list)` |  |
| `workspace use` | `dsheng workspace use --workspace <id> --json` | 入：`contracts/v1/settings.schema.json#/$defs/workspace_use` |  |
| `workspace period set` | `dsheng workspace period set --workspace <id> --input-json <file|-> --json` | 入：`contracts/v1/settings.schema.json#/$defs/period_policy_set` | cadence/effective_from explicit; week_start_day defaults ISO1, fiscal_year_start_month 1, timezone Asia/Shanghai before schema validation; output all applied defaults |
| `provider set` | `dsheng provider set --workspace <id> --input-json <file|-> [--api-key-stdin] --json` | 入：`contracts/v1/settings.schema.json#/$defs/provider_upsert` | stdin cannot be used by both flags; key_action replace requires api-key-stdin; keep is default only in CLI sugar; no model connection test |
| `provider use` | `dsheng provider use --workspace <id> --provider <id|none> --expected-revision <workspace-revision> --json` | 入：`contracts/v1/settings.schema.json#/$defs/provider_use` |  |
| `provider clear-key` | `dsheng provider clear-key --workspace <id> --provider <id> --expected-revision <provider-revision> --confirm-local --json` | 入：`contracts/v1/settings.schema.json#/$defs/provider_clear_key` |  |
| `provider delete` | `dsheng provider delete --workspace <id> --provider <id> --expected-revision <provider-revision> --confirm-local --json` | 入：`contracts/v1/settings.schema.json#/$defs/provider_delete` | preview dependency count first; refuse active attempts; deleting default clears active_provider_id |
| `mapping set` | `dsheng mapping set --workspace <id> --run <id> --input-json <file|-> --json` | 入：`docs/specs/data-period-import.md` | atomic versioned draft; explicit expected_revision; analysis recalculation required |
| `manual-input set` | `dsheng manual-input set --workspace <id> --period-key <key> --input-json <file|-> --json` | 入：`docs/specs/data-period-import.md` | source_note and expected_revision required; decimal strings remain decimal; no implicit reanalysis |
| `metric set` | `dsheng metric set --workspace <id> --input-json <file|-> --json` | 入：`contracts/v1/metric-catalog.json` | versioned definition with expected_revision/effective_from; only permitted AST; no code execution |
| `source-conflict resolve` | `dsheng source-conflict resolve --workspace <id> --run <id> --input-json <file|-> --json` | 入：`docs/specs/data-period-import.md` | selected source evidence IDs, reason and expected_revision required; originals never overwritten |
| `analyze` | `dsheng analyze --workspace <id> --dir <path> --from <date> --to <date> --grain <auto|day|week|month|quarter> [--policy <id>] [--recalculate] [--input-json <file|->] --json` | 入：`docs/specs/data-period-import.md`<br>出：`contracts/v1/read-views.schema.json#/$defs/analysisRunView` | no Provider; explicit policy required for boundary-crossing range; preserve historical snapshots |
| `diagnosis revise` | `dsheng diagnosis revise --workspace <id> --session <id> --fact <id> --replacement-stdin --json` | 入：`replacement input; freezes old session and prepares a new report; no Provider request` | requires the user's full replacement statement on stdin<br>does not mutate or retract the accepted fact in the original report<br>new report must pass normal preview and confirmation before any request |
| `diagnosis pending cancel` | `dsheng diagnosis pending cancel --workspace <id> --session <id> --turn <id> --expected-revision <n> --confirm-local --json` | — | clears only the matching pending clarification; preserves its turn history and does not alter the report |
| `generation cancel` | `dsheng generation cancel --workspace <id> --attempt <id> --json` | — | only local cancellation; cannot guarantee remote billing stopped |
| `task update` | `dsheng task update --workspace <id> --task <id> --patch-json <file|-> --json` | 入：`contracts/v1/task-edit.schema.json` |  |
| `task step update` | `dsheng task step update --workspace <id> --step <id> --patch-json <file|-> --json` | 入：`contracts/v1/task-edit.schema.json` |  |
| `task step add` | `dsheng task step add --workspace <id> --task <id> --input-json <file|-> --json` | 入：`contracts/v1/task-edit.schema.json` |  |
| `task step delete` | `dsheng task step delete --workspace <id> --step <id> --expected-revision <n> --json` | 入：`docs/specs/task-lifecycle.md` |  |
| `task step move` | `dsheng task step move --workspace <id> --step <id> --before <step-id> --expected-revision <n> --json` | 入：`docs/specs/task-lifecycle.md` |  |
| `task reconcile` | `dsheng task reconcile --workspace <id> --run <id> --input-json <file|-> (--dry-run | --apply) --json` | 入：`contracts/v1/task-reconcile.schema.json`<br>出：`contracts/v1/read-views.schema.json#/$defs/reconcilePreviewView (--dry-run only; --apply returns applied result summary)` | Schema includes candidate identity, expected revisions and apply_token; --dry-run returns diff/apply_token; --apply requires token; no model identity matching |
| `history delete` | `dsheng history delete --workspace <id> --run <id> (--dry-run | --confirm-delete <delete_preview_id>) --json` | 入：`docs/specs/task-lifecycle.md` | dependency-aware physical delete; separate delete preview is not Provider confirmation |

## 生成（generation，双门禁，每操作最多 1 次 Provider 调用）

| 命令 | 语法（合同原文） | 输入/输出契约 | 备注 |
| --- | --- | --- | --- |
| `diagnosis generate` | `dsheng diagnosis generate --workspace <id> --run <id> --framework <5why|5w|5w2h> --problem <text> [--focus-metric <id>] --evidence <id>... [--provider <id>] [--dry-run | --confirmation-id <id> --confirm-send] --json` | 入：`diagnosis input contract in implementation plan and context specification`<br>预览出：`contracts/v1/settings.schema.json#/$defs/confirmationPreview` | no confirmation flags means preview only, no HTTP; task objects forbidden |
| `diagnosis continue` | `dsheng diagnosis continue --workspace <id> --session <id> --answers-stdin [--provider <id>] [--dry-run | --confirmation-id <id> --confirm-send] --json` | 入：`diagnosis continuation contract in context specification`<br>预览出：`contracts/v1/settings.schema.json#/$defs/confirmationPreview` | pending clarification/short answer behavior per context contract; do not auto-loop |
| `task generate` | `dsheng task generate --workspace <id> --run <id> [--diagnosis-report <id>] [--provider <id>] [--dry-run | --confirmation-id <id> --confirm-send] --json` | 入：`task input contract in implementation plan`<br>预览出：`contracts/v1/settings.schema.json#/$defs/confirmationPreview` | optional report requires same analysis revision; local candidate identity/reconciliation per task-lifecycle.md |
| `generation retry` | `dsheng generation retry --workspace <id> --attempt <id> [--provider <id>] [--dry-run | --confirmation-id <id> --confirm-send] --json` | 入：`docs/specs/provider-config-delivery.md`<br>预览出：`contracts/v1/settings.schema.json#/$defs/confirmationPreview` | only terminal unsuccessful attempt; fresh preview/confirmation; max one new request |

## 导出（export，Provider 调用上限 0）

| 命令 | 语法（合同原文） | 输入/输出契约 | 备注 |
| --- | --- | --- | --- |
| `diagnosis export` | `dsheng diagnosis export --workspace <id> --report <id> [--revision <n>] --format <md|json|pdf> --out <path>` | — |  |
| `export bundle` | `dsheng export --workspace <id> --run <id> [--revision <n>] --bundle business --out <path.zip>` | — |  |
| `export workbook` | `dsheng export --workspace <id> --run <id> [--revision <n>] --workbook <accounting|trend|task> --out <path.xlsx>` | — |  |
| `export meeting` | `dsheng export --workspace <id> --run <id> [--revision <n>] --meeting --format <md|json> --out <path>` | — |  |
| `export chart` | `dsheng export --workspace <id> --run <id> [--revision <n>] --chart <id> --format <png|csv|xlsx> --out <path>` | — |  |
| `template download` | `dsheng template download --package import --out <path.zip>` | 入：`docs/specs/data-period-import.md` |  |

## 实现注记（与合同不冲突的当前行为）

- `task reconcile --apply`：dry-run 返回的 `apply_token` 需写入 `--input-json` **顶层**字段 `apply_token`（cli-surface 命令注记；`task-reconcile.schema.json` 的 required 列表未含该字段，属已知合同缺口）。
- `task step add` 的 `--input-json` 当前接受步骤内容对象（`action` 必填；`owner`/`start_date`/`due_date`/`done_criteria`/`before_step_id` 可选），经同一 task-edit 服务路径落库。
- `diagnosis export --format pdf` 当前明确拒绝（`EXPORT_FAILED`，仅支持 `md`/`json`）。
