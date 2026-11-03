# 命令行（CLI）与 Agent Skill

桌面端之外，dsheng 提供独立命令行 `dsheng`（与桌面应用同一核心、共用同一用户数据
库），以及给本机 AI Agent 使用的 `dsheng-agent` Skill。

## 安装 CLI

- macOS（Apple Silicon/Intel）与 Windows x64 各有独立 CLI 安装文件：二进制归档 +
  用户级安装辅助脚本（macOS `.sh` / Windows `.ps1`）+ 版本与校验清单。
- 默认安装到用户目录（macOS `~/.local/bin/dsheng`；Windows
  `%LOCALAPPDATA%\Programs\dsheng\dsheng.exe`）。**不会**静默修改系统 PATH、不提权、
  不绕过 Gatekeeper/SmartScreen——首次运行 unsigned 二进制时按
  [install.md](install.md) 的标准流程确认即可。
- 辅助脚本无法运行时，手动解压并用绝对路径调用同样可用。
- 验证安装：

```bash
dsheng --version --json
dsheng --help
```

`--version` 输出应用版本、合同/Schema/Prompt 版本与平台架构，不含用户数据。

## 与桌面端共用数据库

桌面端与 CLI 读写**同一个**用户数据库（位置见 [install.md](install.md)）。CLI 通过
`--db <数据库路径>` 或环境变量 `DSHENG_DB` 指向桌面库；两者同名同义。桌面上建的
企业/分析，CLI 立即可见，反之亦然。并发写入按乐观并发保护（版本冲突会明确报错）。
请保持桌面端与 CLI 使用同一构建版本。

## 基本调用约定

- 企业命令显式 `--workspace <id>`，或先用 `workspace use` 设当前企业。
- `--json` 时 stdout 输出**恰好一个** JSON envelope：
  `{"schema_version":"1","ok":true,"data":{…}}` 或
  `{"schema_version":"1","ok":false,"error":{"code","message","field","details"}}`；
  进度与诊断信息只写 stderr。适合脚本解析。
- 未知 flag、未知输入字段一律拒绝（`INVALID_ARGUMENT`），不会静默忽略。
- 金额/数量/进度等数值在 JSON 中都是十进制字符串（如 `"11500.25"`）。
- 输入 JSON 支持 `--input-json <file>` 或 `-`（stdin）；stdin 专通道（Key、诊断答复、
  更正声明）见各命令说明。

## 退出码族

| 退出码 | 含义 | 典型来源 |
| --- | --- | --- |
| 0 | 成功 | — |
| 2 | 参数/配置/确认/版本冲突 | `INVALID_ARGUMENT`、`WORKSPACE_REQUIRED`、`REVISION_CONFLICT`、`CONFIRMATION_REQUIRED`、`CONFIRMATION_STALE`、发送前取消 `CONFIRMATION_CANCELLED` 等 |
| 3 | 本地数据/数据库校验 | `DATA_VALIDATION_FAILED`、`LOCAL_SAVE_FAILED`、`DELETE_DEPENDENCY_BLOCKED`、迁移/备份/完整性错误等 |
| 4 | 已派发的 Provider 失败 | 网络/超时/401/403/404/429/5xx、Schema/语义校验失败、派发后取消 `CANCELLED`、`INTERRUPTED`；`INPUT_REVISION_CHANGED` 派发后也是 4（派发前 2） |
| 5 | 导出失败 | `EXPORT_FAILED`（含当前版本 PDF 导出明确拒绝） |

完整 69 项错误码见 Skill 参考 `skills/dsheng-agent/reference/errors.md`（与
`contracts/v1/cli-surface.json` 一致）及 [troubleshooting.md](troubleshooting.md)。

## 一个安全的生成流程（CLI）

```bash
# 1) 本地分析（0 次请求）
dsheng analyze --workspace <id> --dir <path> --from 2026-10-01 --to 2026-10-31 --grain auto --json

# 2) 诊断预览（0 次请求；返回 confirmation_id，10 分钟有效）
dsheng diagnosis generate --workspace <id> --run <run-id> --framework 5why \
  --problem "问题表述" --evidence <id>... --dry-run --json

# 3) 你确认预览内容后，才真正发送（两个参数必须同时给）
dsheng diagnosis generate --workspace <id> --run <run-id> --framework 5why \
  --problem "问题表述" --evidence <id>... \
  --confirmation-id <id> --confirm-send --json

# 4) 任务生成同样先 dry-run 再确认；之后本地合并候选
dsheng task generate --workspace <id> --run <run-id> --dry-run --json
dsheng task reconcile --workspace <id> --run <run-id> --input-json r.json --dry-run --json
dsheng task reconcile --workspace <id> --run <run-id> --input-json r_apply.json --apply --json
```

缺 `--confirmation-id` 或 `--confirm-send` 任何一个都不会发送；一次确认最多 1 条
HTTP 请求，无自动重试。

## dsheng-agent Skill（给本机 AI Agent）

Skill 位于仓库 `skills/dsheng-agent/`（可打包为 `skill.zip`），内容：

- `SKILL.md`：Agent 接入指南——定位 `dsheng`、显式 `--workspace`、先只读勘察、生成
  类先 dry-run 并把发送范围展示给你、获你批准后才带确认参数发送、解析 envelope、
  候选合并与导出；含明确禁止事项（不绕过确认门禁、不读 SQLite、不硬编码 Key、
  取消/失败语义）。
- `reference/`：54 条命令速查、关键 JSON 结构摘录（envelope/预览/分析/任务/编辑/
  合并输入）、69 项错误码表。
- `scripts/verify_skill.sh`：本地逻辑演练（合成临时库全流程 dry-run，不触网、不建
  假 Provider）；`scripts/package_skill.sh` 打包 Skill ZIP。

给 Agent 的建议用法：安装独立 CLI 后，把 Skill 目录交给你的 Agent（或在支持
Agent Skills 的客户端里安装 skill.zip），并明确告知「所有发送类操作必须先给我看
预览」。Agent 演练用合成数据；真实发送只使用你配置的真实 Provider，且每次都要经过
你的批准。API Key 不会进入命令行、日志或 Skill 文件。
