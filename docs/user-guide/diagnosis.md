# 经营问题诊断（5W / 5WHY / 5W2H）

诊断是**显式发起、逐次确认**的模型功能：你选框架、写清问题、勾选证据，工具组装
上下文并给出发送预览；你确认后才发生 1 次真实请求。诊断结果不会自动生成任务单，
也不会在你停止后继续追问。

## 三种框架怎么选

| 框架 | 结构 | 适合的问题 |
| --- | --- | --- |
| **5WHY** | 逐层追问「为什么」，形成因果链 | 已定位到现象、想沿因果往下挖根因（如「本月销售额为何低于预定」） |
| **5W** | What/Why/Where/When/Who 五维各一项 | 问题面较宽、先要全面摸清发生范围与涉及对象 |
| **5W2H** | What/Why/Who/When/Where/How/How much | 既要定位又要行动量纲（含费用/数量）的问题，接近改进行动设计 |

三者的报告字段、JSON 与导出内容不同，互不复用；选错框架就新建一份新报告，不存在
「切换框架刷新同一份报告」。

## 生成一份诊断

```bash
dsheng diagnosis generate --workspace <id> --run <run-id> --framework 5why \
  --problem "本月销售额低于预定，请定位主要原因" \
  --focus-metric <metric-id> --evidence <id>... --dry-run --json
```

- `--problem` 必填；`--focus-metric` 可选（必须是本 run 的指标）；`--evidence` 至少
  一个，且必须是本次分析关联的证据（来源单元格或手工输入），**不允许发送与分析
  无关的证据**。
- 先 `--dry-run`（或省略确认参数）得到预览：Provider/模型、期间、来源范围、请求
  大小与 token 估算、「最多 1 次请求」、`confirmation_id`（10 分钟有效）。
- 你批准后，同一命令改为携带 `--confirmation-id <id> --confirm-send` 才真正发送；
  两个参数缺一不发送。任何配置/数据在确认后发生变化都会使确认失效
  （`CONFIRMATION_STALE`），需重新预览。
- **证据不足时报告会如实停在证据支持的层级**，并提出具体的补充问题；工具不虚构
  事实，诊断结果里不含任务对象。

查看/列出/导出报告（本地只读，0 次请求）：

```bash
dsheng diagnosis list --workspace <id> --run <run-id> --json
dsheng diagnosis show --workspace <id> --report <id> [--revision <n>] --json
dsheng diagnosis export --workspace <id> --report <id> --format md --out report.md
dsheng diagnosis export --workspace <id> --report <id> --format json --out report.json
```

如实说明：`--format pdf` 在当前版本明确拒绝（`EXPORT_FAILED`，提示「PDF 导出未实现，
当前构建仅 md/json」）；需要 PDF 请先导出 md/json 再自行转换。查看/导出报告不会创建
任务，也不调用 Provider。

## 续问（多轮追问）

对同一会话继续提问/补充事实（答复经 stdin 传入，同样先预览后确认）：

```bash
printf '新补充的事实与你的答复' | dsheng diagnosis continue --workspace <id> \
  --session <session-id> --answers-stdin --dry-run --json
```

- 续问**不得改变**原问题、期间、框架、来源清单或选定范围；模型仅返回发生变化的项，
  通过稳定 ID 合并进报告（append-only）。
- 若模型认为需要澄清，本轮进入「待澄清」状态：你可以
  - **澄清重述**：再次 `diagnosis continue` 给出完整、明确的澄清表述。孤立的
    「是/确认」不能替代完整澄清（`CLARIFICATION_RESTATEMENT_REQUIRED`）；
  - **放弃该轮**：`diagnosis pending cancel` 清除这条待澄清项（保留轮次历史，不改报告）：
    ```bash
    dsheng diagnosis pending cancel --workspace <id> --session <id> --turn <turn-id> \
      --expected-revision <n> --confirm-local --json
    ```
- **停止＝不续问**：你不再发起续问，会话就停在当前报告；不存在自动追问或后台补发。

## 更正已采纳的事实（revise）

发现报告采纳的事实有误时，不要在续问里「顺手改」；用更正流程：

```bash
printf '完整更正声明：正确的事实与依据' | dsheng diagnosis revise --workspace <id> \
  --session <id> --fact <fact-id> --replacement-stdin --json
```

`revise` 是本地操作：**冻结旧会话**、旧报告只读保留，并准备一份新报告；新报告仍须走
正常的预览与确认才会发起请求。更正不会改写旧报告的历史内容。

## 警告（warnings）与 resolution_requirement

报告中的风险/不确定项以警告形式记录，每条有稳定 `warning_id` 与 active/resolved
状态：新一轮可新增警告或按 ID 更新；已 resolved 的警告只读，复发会生成新 ID。

警告带 `resolution_requirement`（解除依据要求），规则：

- 要求 `source_record` 的警告，必须以**文件来源证据**落证解除；手工输入/口述材料
  （user_report）不能当作文件事实的独立书证。
- 要求 `user_confirmation` 的警告，由你明确确认后解除。
- 工具不会因为新一轮生成就自动「冲掉」旧警告；未满足解除要求的警告保持 active。

## 失败、重试与取消

- 发送后失败（网络/超时/模型返回不合规等）：旧有效报告保持不变；手动重试用
  `generation retry`，**必须重新预览、重新确认**（新 confirmation），旧 attempt 记录
  保留、不可重试成功项。
- 取消：发送前取消是本地终态；发出后可用 `generation cancel`（不保证远端停止计算/
  计费）。查询状态用 `generation status`。
- 一次操作上限 1 条请求；没有自动重试、没有分块或隐藏摘要。详见
  [troubleshooting.md](troubleshooting.md)。

```bash
dsheng generation status --workspace <id> --attempt <attempt-id> --json
dsheng generation cancel --workspace <id> --attempt <attempt-id> --json
dsheng generation retry --workspace <id> --attempt <attempt-id> --dry-run --json
```
