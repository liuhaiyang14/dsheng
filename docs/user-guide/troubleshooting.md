# 排障手册

## 快速定位

1. 看退出码族：`2` 参数/配置/确认/版本冲突（多为用法问题）；`3` 本地数据/数据库；
   `4` 已派发的 Provider 失败；`5` 导出失败。
2. 看 envelope 里的 `error.code` 与 `field`（指向出错的 flag 或字段）；`details` 只有
   脱敏元数据。
3. 完整 69 项错误码对照表见 `skills/dsheng-agent/reference/errors.md`（与
   `contracts/v1/cli-surface.json` 完全一致）。

## 高频错误码 → 处置

| 错误码（退出码） | 场景 | 处置 |
| --- | --- | --- |
| `WORKSPACE_REQUIRED`（2） | 未指定企业 | 加 `--workspace <id>` 或先 `workspace use` |
| `WORKSPACE_MISMATCH`（2） | 对象不属于当前企业 | 核对 ID 归属；不要试图换企业复用同一 ID |
| `REVISION_CONFLICT`（2） | 乐观并发冲突 | 重读当前 revision/快照后整体重试；不重放旧 patch |
| `PROVIDER_REQUIRED`（2） | 无默认 Provider | 显式 `--provider <id>` 或 `provider use` 设默认 |
| `PROVIDER_KEY_MISSING`（2） | 配置无 Key | `provider set --api-key-stdin` 补 Key 后再预览 |
| `CONFIRMATION_REQUIRED`（2） | 未先预览 / 伪确认号 | 先 `--dry-run` 拿 `confirmation_id`；reconcile 先 dry-run 拿 `apply_token` |
| `CONFIRMATION_STALE`（2） | 预览后配置/数据变了，或超 10 分钟 | 重新 dry-run；不要复用旧确认 |
| `CONTEXT_BUDGET_EXCEEDED`（2） | 保守 token 超预算 | 缩小发送范围，或经确认调大 Provider 的 `context_window_tokens`（诊断上下文较大，默认 32,768 常不够，建议 65,536 起） |
| `REQUEST_TOO_LARGE`（2） | 请求体超 1MiB | 缩小文件/工作表/行范围后重新预览；工具不分块、不截断 |
| `USER_MESSAGE_TOO_LARGE`（2） | 单轮输入超 8000 字符 | 缩短问题/答复文本 |
| `STALE_DIAGNOSIS_REFERENCE`（2） | 任务生成引用了旧版诊断报告 | 重做分析后重新生成报告，或取消引用 |
| `INPUT_REVISION_CHANGED`（2/4） | 等待期间分析/报告/输入被重做 | 结果未提交、旧结果保留；重新生成并重新确认 |
| `AUTH_FAILED`（4） | Provider 401 | Key 可能失效；经 `--api-key-stdin` 更新后重试 |
| `RATE_LIMITED` / `QUOTA_EXHAUSTED`（4） | 限流/额度耗尽 | 等待或处理额度；工具不自动换 Provider、不自动重试 |
| `CONTEXT_REJECTED`（4） | 服务端判上下文超长 | 缩小范围或调大预算后重新预览 |
| `INCOMPLETE_OUTPUT` / `PROTOCOL_ERROR` / `INVALID_JSON` / `SCHEMA_VALIDATION_FAILED` / `SEMANTIC_VALIDATION_FAILED`（4） | 模型返回不合规 | 结果被拒绝且未落库，旧结果不变；重新预览后重试 |
| `EXPORT_FAILED`（5） | 导出失败 | 检查输出路径/磁盘；注意当前版本 `diagnosis export --format pdf` 明确拒绝（仅 md/json） |
| `DELETE_DEPENDENCY_BLOCKED`（3） | 删除期间有存活依赖 | 先处理活动请求/未完成任务/跨期引用，再重新预览删除 |
| `TASK_CATEGORY_METRIC_MISMATCH`（3） | 非 other 类别未绑有效指标 | 补/改 `metric_id` 为本工作区有效指标 UUID |
| `AT_LEAST_ONE_STEP`（3） | 删光步骤 | 保留/新增至少一条活跃步骤 |
| `STEP_OUTSIDE_TASK_PERIOD`（3） | 步骤日期超出任务期间 | 先改任务整体日期，或在同一事务调整步骤 |
| `STATE_PROGRESS_MISMATCH`（3） | 手动进度与状态矛盾 | 按 not_started=0 / in_progress<100 / completed=100 修正 |

## 数据库：备份、恢复与升级

**位置**见 [install.md](install.md)。要点：

- **升级前自动备份**：应用检测到需要迁移时，会先用 SQLite 一致性备份在
  `backups/pre-migration-<版本>-<时间>.sqlite` 建备份（不是直接拷贝主库文件），备份
  校验通过才迁移；失败则保持原库可被旧版本打开。**备份包含明文 API Key 与全部本地
  历史**，请同等保密，不要外传。
- **手动备份建议**：退出所有 dsheng 实例后，整目录复制用户数据目录（含
  `dsheng.sqlite3` 与 `backups/`）；或使用具备 SQLite 一致性备份能力的工具。直接在
  运行中拷贝单个主库文件可能得到含 WAL 的不一致副本。
- **恢复**是显式本地操作：退出所有实例 → 核对备份版本与完整性 → 确认覆盖 → 工具
  先为现库再做一次保留备份，然后原子替换并校验。恢复失败（`DB_RESTORE_FAILED`）时
  现库与备份都保留。恢复不会触碰源数据文件，也不发起请求。
- **升级兼容**：旧客户端打开新版数据库会被拒绝（`DB_SCHEMA_UNSUPPORTED`），提示升级
  对应客户端；不自动降级、不自动改写。迁移期间要求所有 GUI/CLI 实例退出；占用时报
  `DB_MIGRATION_BUSY`，不会强杀其他进程。
- **完整性异常**（`DB_INTEGRITY_FAILED`，如断电后）：停止普通写入，保留全部文件，
  从已验证备份手工恢复；不要继续写或尝试「修复」。
- `history delete` 只删当前数据库中的记录，不清理备份与既有导出。

## WebView2 缺失（仅 Windows 桌面端）

症状：启动前本地检查提示「缺少 WebView2 运行时」。处置：点击提示中的微软官方下载
入口（或自行访问微软 WebView2 下载页）安装 Evergreen 运行时，然后重启 dsheng。工具
不会静默联网代装。CLI 不依赖 WebView2。若企业环境限制安装，请联系管理员。

## 模型生成相关

- **不发送就失败**：检查确认参数是否成对（`--confirmation-id` + `--confirm-send`）、
  确认是否 10 分钟内、预览后是否改过配置/数据。
- **一次没成功**：没有自动重试；用 `generation retry --dry-run` 重新预览并再次确认。
- **取消**：发送前直接放弃即可；发送后 `generation cancel` 只保证本地停止等待，
  **不保证远端停止计算/计费**。
- **重装/换机**：卸载与覆盖安装不删数据；换机需自行迁移用户数据目录（注意其中含
  明文 Key）。

## 仍解决不了

- 收集：命令原文（隐去 Key）、退出码、envelope 中的 `error.code/message/field`、
  `dsheng --version --json` 输出。
- 不要把数据库文件、导出包或含 Key 的内容发给任何人。
