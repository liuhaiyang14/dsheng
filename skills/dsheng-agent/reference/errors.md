# dsheng 错误码参考（69 项，照抄 contracts/v1/cli-surface.json `errors`）

退出码族：`0` 成功；`2` 参数/配置/确认/版本冲突；`3` 本地数据/数据库校验；`4` 已派发 Provider 失败；`5` 导出失败。
`INPUT_REVISION_CHANGED` 特例：派发前退出码 2（`exit_code`），派发后 4（`exit_code_after_dispatch`）。

失败 envelope：`{"schema_version":"1","ok":false,"error":{"code","message","field","details"}}`；`details` 只含脱敏元数据，永不包含 Key/凭据/完整请求。

## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `INVALID_ARGUMENT` | 无效 CLI 或 Schema 输入 （英文原义：Invalid CLI or schema input） | 检查 flag/参数拼写与取值；未知 flag 与未知输入字段都会被拒绝，不会静默忽略。 |
| `WORKSPACE_REQUIRED` | 未显指定且无已选工作区 （英文原义：No explicit or selected workspace） | 加 --workspace <uuid>，或先 workspace use 确认当前企业。 |
| `WORKSPACE_MISMATCH` | 对象不属于所选工作区 （英文原义：Object outside selected workspace） | 确认对象 ID 属于当前企业；不要试图切换企业重试同一 ID。 |
| `REVISION_CONFLICT` | 期望版本不一致 （英文原义：Expected version differs） | 重新读取对象当前 revision/快照后整体重试，不重放旧 patch。 |
| `PROVIDER_REQUIRED` | 无显式/默认 Provider （英文原义：No explicit/default provider） | 加 --provider <id>，或先 provider use 设置工作区默认。 |
| `PROVIDER_KEY_MISSING` | Provider 无 Key （英文原义：Provider has no Key） | provider set --api-key-stdin 写入 Key 后再预览。 |
| `PROVIDER_URL_INVALID` | URL 无效或被禁止 （英文原义：Invalid or forbidden URL） | 仅 HTTPS（loopback 例外）；禁止 userinfo/query/fragment，检查 base_url。 |
| `PROVIDER_IN_USE` | 活动生成请求阻止删除 （英文原义：Active attempt prevents deletion） | 先等 attempt 终态或 generation cancel，再删 Provider。 |
| `CONFIRMATION_REQUIRED` | 需要预览/确认 （英文原义：Preview/confirmation required） | 先 --dry-run 拿 confirmation_id；reconcile --apply 需先 dry-run 拿 apply_token。 |
| `CONFIRMATION_STALE` | 快照变化或令牌过期 （英文原义：Snapshot changed or token expired） | 重新 --dry-run 生成新确认（TTL 10 分钟）；不要复用旧 id。 |
| `CONFIRMATION_CANCELLED` | 请求前已取消 （英文原义：Cancelled before request） | 本地终态；如需发送必须重新预览并再次获用户批准。 |
| `REQUEST_TOO_LARGE` | 最终请求体超 1MiB （英文原义：Final body exceeds 1MiB） | 缩小文件/工作表/行范围后重新预览；不拆分、不截断。 |
| `USER_MESSAGE_TOO_LARGE` | 用户输入超 8000 Unicode 标量 （英文原义：User input exceeds 8000 Unicode scalars） | 缩短问题/答复文本后重试。 |
| `CONTEXT_BUDGET_EXCEEDED` | 超出保守预算 （英文原义：Conservative budget exceeded） | 缩小发送范围，或经用户确认调大 provider 的 context_window_tokens。 |
| `INVALID_TIMEOUT_BUDGET` | 超时边界或总和非法 （英文原义：Timeout bounds or sum invalid） | 满足 total >= connect + response，且各值在允许区间。 |
| `INVALID_CONTEXT_BUDGET` | 预留/上下文约束非法 （英文原义：Reserve/context constraint invalid） | output_reserve_tokens <= context_window_tokens/2。 |
| `RUNTIME_MISSING` | 缺少所需桌面运行时 （英文原义：Required desktop runtime absent） | 按提示安装 WebView2 运行时后重启（仅影响桌面端，CLI 不需要）。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `DATA_VALIDATION_FAILED` | 输入域校验失败 （英文原义：Input domain validation failed） | 按 message/field 修正输入；不猜测改写业务值。 |
| `DATA_DIRECTORY_UNAVAILABLE` | 无法解析/创建/打开用户数据目录 （英文原义：Cannot resolve/create/open user data directory） | 检查目录权限/磁盘；不回退到其他路径。 |
| `LOCAL_SAVE_FAILED` | 事务失败 （英文原义：Transaction failed） | 本地写入整体回滚；修正输入后重试；进度类先核对模式约束。 |
| `SOURCE_CHANGED` | 锁定的来源身份/范围/哈希变化 （英文原义：Locked source identity/range/hash changed） | 源文件被改动；重新选择范围并重新预览，不混用版本。 |
## 退出码 4（Provider/发送后失败）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `NETWORK_ERROR` | 网络或 DNS 错误 （英文原义：Network or DNS error） | 检查网络后由用户决定是否重新预览+确认重试。 |
| `TLS_ERROR` | TLS 校验错误 （英文原义：TLS validation error） | 检查证书/系统时间；不自动禁用校验。 |
| `REDIRECT_REJECTED` | 意外重定向 （英文原义：Unexpected redirect） | 核实 base_url；本工具不跟随携凭据重定向。 |
| `CONNECT_TIMEOUT` | 连接超时 （英文原义：Connection deadline exceeded） | 网络恢复后重新预览+确认重试（不自动重试）。 |
| `RESPONSE_TIMEOUT` | 读取超时 （英文原义：Read deadline exceeded） | 同上；可考虑调大 response_seconds（需用户确认）。 |
| `TOTAL_TIMEOUT` | 总超时 （英文原义：Total deadline exceeded） | 同上；可考虑调大 total_seconds（需用户确认）。 |
| `AUTH_FAILED` | Provider 返回 401 （英文原义：401） | Key 可能失效；由用户经 --api-key-stdin 更新后重试。 |
| `ACCESS_DENIED` | Provider 返回 403 （英文原义：403） | 确认账号权限/区域限制；不自动换 Provider。 |
| `ENDPOINT_OR_MODEL_NOT_FOUND` | Provider 返回 404 （英文原义：404） | 核对 base_url 与 model 名称。 |
| `RATE_LIMITED` | 已知限流 （英文原义：Known rate limit） | 等待后由用户重新确认发送。 |
| `QUOTA_EXHAUSTED` | 已知额度/套餐耗尽 （英文原义：Known quota/package exhaustion） | 由用户处理额度；不自动换 Provider。 |
| `PROVIDER_REJECTED_UNKNOWN` | Provider 未知原因拒绝 （英文原义：Provider rejected with unknown cause） | 查看脱敏 details；由用户决定下一步。 |
| `PROVIDER_SERVER_ERROR` | Provider 5xx （英文原义：5xx） | 稍后由用户重新确认发送。 |
| `CONTEXT_REJECTED` | Provider 拒绝上下文长度 （英文原义：Provider rejected context length） | 缩小发送范围或调大预算后重新预览。 |
| `RESPONSE_TOO_LARGE` | 响应体超 2MiB （英文原义：Response body exceeds 2MiB） | 缩小请求范围；本工具不分块。 |
| `INCOMPLETE_OUTPUT` | 非 stop 的显式结束原因 （英文原义：Non-stop explicit finish reason） | 结果被拒绝且未落库；旧有效结果保留，重新预览后重试。 |
| `PROTOCOL_ERROR` | 缺失或无效 envelope 内容 （英文原义：Missing or invalid envelope content） | 结果被拒绝；不重试同一响应，重新生成。 |
| `INVALID_JSON` | 内容 JSON 解析失败 （英文原义：Content JSON parse failed） | 同上。 |
| `SCHEMA_VALIDATION_FAILED` | 输出 Schema 校验失败 （英文原义：Output schema failed） | 同上；不手工修补模型输出冒充通过。 |
| `SEMANTIC_VALIDATION_FAILED` | 输出域校验失败 （英文原义：Output domain validation failed） | 同上。 |
| `CANCELLED` | 派发后取消 （英文原义：Cancelled after dispatch） | 终态；重试需新预览+新确认；不保证远端停止计费。 |
| `INTERRUPTED` | 宿主实例关闭/崩溃 （英文原义：Owner instance closed/crashed） | 重启后核对 attempt 状态再决定是否重试。 |
## 退出码 5（导出失败）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `EXPORT_FAILED` | 导出未完成 （英文原义：Export could not complete） | 检查 --out 路径/磁盘；diagnosis export pdf 当前明确拒绝（仅 md/json）。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `DB_SCHEMA_UNSUPPORTED` | 客户端不支持数据库版本 （英文原义：Client does not support database version） | 用与数据库匹配的客户端版本；不降级、不盲写。 |
## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `INPUT_REVISION_CHANGED` | 分析/报告/输入 revision 变化（不提交当前结果，不自动重试）；派发后退出码 4 （英文原义：Analysis/report/input revision changed; no commit to current result; no silent automatic retry） | 重新 analyze/生成并重新确认；旧结果保留。注意：派发前退出码 2、派发后 4。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `DB_MIGRATION_BUSY` | 迁移或活动写入者占用 （英文原义：Migration or active writer prevents operation） | 退出其他 GUI/CLI 实例后重试。 |
| `DB_MIGRATION_FAILED` | 迁移失败（原库受保护） （英文原义：Migration failed; original database protected） | 保留现场与备份，按排障手册处理。 |
| `DELETE_DEPENDENCY_BLOCKED` | 存活依赖阻止删除 （英文原义：Deletion blocked by live dependency） | 先处理活动请求/未完成任务/跨 run 引用，再重新预览删除。 |
## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `IDEMPOTENCY_CONFLICT` | 同幂等键不同载荷 （英文原义：Reused idempotency key with different payload） | 换新 mutation_id；勿复用旧 ID 提交不同内容。 |
| `STALE_DIAGNOSIS_REFERENCE` | 诊断引用与分析/报告 revision 不匹配 （英文原义：Optional diagnosis does not match analysis/report revision） | 重新选择当前报告或取消引用后重新确认。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `UNIT_MISMATCH` | 指标/单位不兼容 （英文原义：Metric/unit incompatible） | 单位改动必须与指标定义兼容，不能只改文字。 |
| `STEP_STILL_BLOCKED` | 未解除的阻塞步骤阻止完成 （英文原义：Unresolved blocked step prevents completion） | 先处理 blocked 步骤或其理由。 |
| `DB_BACKUP_FAILED` | 一致性本地备份失败 （英文原义：Consistent local backup failed; do not migrate） | 不执行迁移；按排障手册从既有备份处理。 |
| `DB_INTEGRITY_FAILED` | 数据库完整性检查失败 （英文原义：Database integrity check failed; preserve files and stop） | 保留全部文件并停止写入，从已验证备份手工恢复。 |
| `DB_RESTORE_FAILED` | 显式本地恢复失败 （英文原义：Explicit local restore failed; preserve original files） | 现库与备份均保留；按排障手册重试。 |
## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `CONTEXT_INVALID` | 跨字段/历史上下文不一致 （英文原义：Cross-field or historical context is inconsistent） | 按 message 修正输入组合。 |
| `SCOPE_SELECTION_REQUIRED` | 多业务范围需显式选择 （英文原义：Multiple business scopes require explicit selection） | 先选定一个 scope 或确认无重叠聚合。 |
| `CLARIFICATION_RESTATEMENT_REQUIRED` | 孤立确认不能替代完整澄清 （英文原义：An isolated confirmation cannot replace a full clarification） | 经 diagnosis continue 提供完整澄清重述。 |
| `MUTATION_RESULT_DELETED` | 幂等结果已随业务对象删除 （英文原义：Idempotent mutation result was removed with deleted business objects） | 不复活已删对象；如需重建走正常新建流程。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `TASK_CATEGORY_METRIC_MISMATCH` | 非 other 类别必须绑定有效指标 （英文原义：Non-other category requires a valid metric; other may omit metric） | category 为 revenue/cost/efficiency 时 metric_id 必须是本工作区有效指标 UUID；other 可为 null。 |
| `TASK_CATEGORY_UNRECOGNIZED` | 未知任务类别拒绝整份候选 （英文原义：Unknown task category rejects the entire candidate） | 候选整体被拒；上游修正类别后再生成，不默认 other。 |
## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `DELETE_PREVIEW_STALE` | 删除依赖在预览后变化 （英文原义：Deletion dependencies changed after preview） | 重新 history delete --dry-run 预览。 |
## 退出码 3（本地数据/数据库校验）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `DUPLICATE_CANDIDATE_KEY` | 候选局部任务键重复 （英文原义：Candidate-local task key is duplicated） | 整份候选被拒；重新生成候选。 |
| `DUPLICATE_RECONCILE_TARGET` | 多候选映射同一持久任务 （英文原义：Multiple candidates target one persistent task） | 调整 mappings，一个旧任务只能被一个候选写入/替代。 |
| `STATE_PROGRESS_MISMATCH` | 任务完成状态与进度不一致 （英文原义：Task completion state and progress disagree） | manual 模式：not_started=0、in_progress 0–99.99、completed=100。 |
| `STEP_OUTSIDE_TASK_PERIOD` | 步骤排期超出任务期间 （英文原义：Step schedule is outside the task period） | 先改任务整体日期，或在同一 content 事务一起调整步骤。 |
| `AT_LEAST_ONE_STEP` | 任务须保留至少一个活跃步骤 （英文原义：A task must retain an active step） | 至少保留/新增一条步骤后再提交。 |
## 退出码 2（参数/配置/确认/版本冲突）

| 错误码 | 含义 | 建议处理 |
| --- | --- | --- |
| `INPUT_CONTRACT_INVALID` | 指标/目标/冲突/单位/范围/证据投影不一致 （英文原义：Metric, target, conflict, unit, scope or evidence projection is inconsistent） | 核对 focus-metric、evidence 关联与目标候选结构后重试。 |
