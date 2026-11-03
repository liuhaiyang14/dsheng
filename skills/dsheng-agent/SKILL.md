---
name: dsheng-agent
description: dsheng「两表一单」本地经营分析工具的 Agent 接入技能：定位 dsheng CLI、显式选择 workspace、本地只读命令、生成类命令的双门禁确认流程（dry-run → 用户批准 → --confirmation-id + --confirm-send）、任务候选 reconcile 与导出。任何 Provider 请求都必须先向用户展示发送范围并获批准。
---

# dsheng Agent 接入技能（SKILL）

本技能指导本地 Agent 安全使用 `dsheng` CLI（合同版本 1，54 条命令）。唯一事实来源是
`contracts/v1/cli-surface.json`；JSON 结构见 `reference/json-shapes.md`，命令速查见
`reference/cli-commands.md`，错误码见 `reference/errors.md`。与文档冲突时以机器合同为准。

## 0. 定位 dsheng 与数据库

按以下顺序定位可执行文件（找到第一个即用）：

1. 环境变量 `DSHENG_BIN` 指定的路径；
2. `PATH` 上的 `dsheng`；
3. 仓库内 `target/release/dsheng`、`target/debug/dsheng`（相对本技能目录为
   `../../target/...`）。

定位后先运行 `dsheng --version --json` 确认可用（输出 app/schema/prompt 版本与平台，
不含用户数据）。

数据库定位（优先级从高到低）：`--db <路径>` → 环境变量 `DSHENG_DB` → 平台用户数据目录
（macOS `~/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3`；Windows
`%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3`）。桌面应用与 CLI 共用同一数据库：
Agent 操作用户现有数据时应通过 `--db`/`DSHENG_DB` 显式指向桌面库；演练或测试必须用
`mktemp` 临时库，绝不触碰用户真实数据目录。

## 1. 核心约定

- **envelope**：`--json` 时 stdout 恰好一个 JSON 对象——成功
  `{"schema_version":"1","ok":true,"data":{...}}`；失败
  `{"schema_version":"1","ok":false,"error":{"code","message","field","details"}}`。
  进度/诊断只走 stderr。解析 stdout 前先检查退出码，再解析 envelope。
- **退出码族**：`0` 成功；`2` 参数/配置/确认/版本冲突；`3` 本地数据/数据库校验；
  `4` 已派发的 Provider 失败（`INPUT_REVISION_CHANGED` 派发前为 2、派发后为 4）；`5` 导出失败。
- **workspace**：企业数据命令必须显式 `--workspace <uuid>`，或使用用户经
  `workspace use` 确认的当前企业；两者皆无报 `WORKSPACE_REQUIRED`。绝不从对象 ID 猜企业，
  绝不静默切换。
- **数值**：所有金额/数量/工时/进度在本地 JSON 中都是规范十进制**字符串**（如
  `"11500.25"`、`"40"`），不是 JSON number。revision/order/计数是整数。
- **乐观并发**：本地写入普遍要求 `expected_revision`；冲突返回 `REVISION_CONFLICT`，
  应重新读取当前值后重试整个操作，不重放旧 patch。

## 2. 标准工作流

### 步骤 A：准备与只读勘察（0 次 Provider 调用）

```bash
dsheng --version --json                                   # 版本/合同确认
dsheng workspace list --json                              # 现有企业
dsheng scan --workspace <id> --dir <path> --json          # 扫描目录内 xlsx/xls/csv
dsheng analyze --workspace <id> --dir <path> \
  --from 2026-10-01 --to 2026-10-31 --grain auto --json   # 本地计算，产生 run + revision
dsheng analysis show --workspace <id> --run <run-id> --json   # 只读查看已存快照
dsheng diagnosis list --workspace <id> --run <run-id> --json
dsheng task list --workspace <id> [--run <run-id>] --json
dsheng task show --workspace <id> --task <task-id> --json
dsheng history list --workspace <id> --json
```

`analyze` 是本地计算（无 Provider）。输出 `analysisRunView`：`run_id`、
`current_revision_no`、`analysis_input_fingerprint`、`metrics`（含 `mom`/`yoy` 比较对象）、
`series`。后续生成、续问、reconcile 都要绑定这组 run/revision/指纹。

### 步骤 B：生成类命令——双门禁（每操作最多 1 次 Provider 调用）

适用于 `diagnosis generate`、`diagnosis continue`、`task generate`、`generation retry`。

1. **先 dry-run**：加 `--dry-run`（或不带确认 flag）执行，返回 `confirmationPreview`
   （`data.confirmation_id`，10 分钟有效）。
2. **向用户展示发送范围**：Provider 名称/base_url/model/revision、企业/期间、来源与
   选定范围（`source_scope_summary`）、请求字节数与保守 token 估算（`request_bytes`、
   `conservative_input_tokens`）、预算/超时、`request_count_maximum=1`。
3. **获得用户明确批准后**才发送：同一命令追加
   `--confirmation-id <id> --confirm-send`。两个 flag 必须同时出现，缺任何一个都不会发送
   （`INVALID_ARGUMENT`），id 不存在/过期/快照漂移也不发送（`CONFIRMATION_REQUIRED` /
   `CONFIRMATION_STALE`），且不产生任何 HTTP 请求。
4. **解析结果 envelope**：任务生成成功返回
   `data.candidate_generation_id`（候选，`state="candidate"`）；诊断返回
   `session_id`/`turn_id`/`turn_state`。失败时上一有效结果不受影响；手动重试用
   `generation retry` 且必须重新 dry-run 取新 confirmation。

用户拒绝或未答复 = 不发送。把预览原样（脱敏后）转述给用户即可，不要替用户做发送决定。

### 步骤 C：任务候选 reconcile（0 次 Provider 调用）

`task generate` 只产生候选，不会改写现有任务。应用候选分两步：

```bash
dsheng task reconcile --workspace <id> --run <run-id> \
  --input-json reconcile.json --dry-run --json    # 校验 + diff + apply_token（10 分钟）
# 用户确认 diff 后：
dsheng task reconcile --workspace <id> --run <run-id> \
  --input-json reconcile_apply.json --apply --json # 输入 JSON 顶层加 apply_token
```

- `mappings` 必须给每个候选任务恰好一次 `new`/`update`/`replace`/`ignore`；
  `update`/`replace` 还要在 `expected_tasks` 里精确声明旧任务及其 revision。
- `--apply` 的输入 JSON 需在 dry-run 输出中拿到 `apply_token` 后写入**顶层字段**
  `apply_token`（cli-surface 注记；`task-reconcile.schema.json` 本身不含该字段）。
- 逐项向用户展示将 `new`/`update`/`replace`/`ignore` 哪些任务；`update` 的
  `take_fields` 是覆盖白名单，包含人工修改过的字段时必须明确提示“将覆盖你的修改”。

### 步骤 D：任务推进与导出（0 次 Provider 调用）

```bash
dsheng task update --workspace <id> --task <task-id> --patch-json edit.json --json
dsheng task step add|update|delete|move ...            # 均需 expected revision
dsheng task timeline --workspace <id> --from <date> --to <date> --grain day --json
dsheng export --workspace <id> --run <run-id> --bundle business --out out.zip
dsheng export --workspace <id> --run <run-id> --workbook task --out tasks.xlsx
dsheng export --workspace <id> --run <run-id> --meeting --format md --out meeting.md
dsheng diagnosis export --workspace <id> --report <id> --format md --out report.md
```

编辑输入结构（content/progress/period_link 三种 operation）见
`reference/json-shapes.md`；数值改动必须带 `source_note`。导出不调用模型。

## 3. 明确禁止事项

- **不绕过确认门禁**：任何生成类命令不得在没有用户批准的情况下携带
  `--confirm-send`；不得伪造、猜测或复用 `confirmation_id`；不得在用户拒绝后重试发送。
  一次确认最多一条 HTTP 请求，没有自动重试。
- **不读 SQLite**：一切数据访问都经 CLI 命令；不直接打开/查询数据库文件。
- **不硬编码 Key**：API Key 只经 `provider set --api-key-stdin`（stdin）进入；
  绝不把 Key 写进命令行参数、JSON、日志、Skill 文件或对话输出。公开输出只有
  `key_present` 布尔值。示例中只使用占位符（如 `placeholder-key-not-real`）。
- **不跨 workspace**：不把一个企业的 ID 用到另一个企业；对象归属不符是
  `WORKSPACE_MISMATCH`，不是“切换后重试”。
- **取消/失败语义**：发送前取消是本地操作（`CONFIRMATION_CANCELLED`，退出码 2）；
  派发后取消用 `generation cancel`（`CANCELLED`，退出码 4，不保证远端停止计费）。
  失败/取消/超时/中断后重试必须新预览、新确认；成功 attempt 不可重试。
  诊断“停止”就是不再续问，不会自动补发请求。
- **不伪装模型输出**：本技能的演练/验证只用合成数据与本地逻辑（见
  `scripts/verify_skill.sh`），不构造 Mock Provider、假 endpoint 或伪造响应。
- **不动真实数据目录**：演练一律 `mktemp` 临时库 + `--db` 指向。

## 4. 验证与打包

```bash
bash skills/dsheng-agent/scripts/verify_skill.sh   # 本地逻辑演练（全部 dry-run，不触网）
bash skills/dsheng-agent/scripts/package_skill.sh  # 打包 skill.zip（zip -r）
```

`verify_skill.sh` 是**本地逻辑演练层，非 Provider 调用**：用合成临时库走
workspace create → provider set（占位 Key）→ analyze → diagnosis generate --dry-run →
task generate --dry-run → task reconcile --dry-run/apply（合成候选夹具）→ task update →
export → history list，并断言 envelope/退出码/门禁拒绝路径。
