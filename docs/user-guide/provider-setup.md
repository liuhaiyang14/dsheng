# Provider 与 API Key 配置

诊断（5W/5WHY/5W2H）与任务单生成需要接入一个兼容 OpenAI `chat/completions`
协议的模型 Provider。不使用这两个功能时无需配置，其余功能全部本地运行。

> ## ⚠️ 重要：API Key 以明文保存在本机数据库
>
> 出于已确认的产品设计，API Key **以明文**保存在本机 SQLite 的
> `providers.api_key` 字段中，不做加密或系统密钥链托管。这意味着：
>
> - **能读取你用户数据目录中数据库文件的人/程序，就能读出 Key**（包括同机其他
>   用户级进程、备份文件、云同步盘同步走的那份副本）。
> - 请仅在此电脑受你控制的前提下保存 Key；公用/共用电脑不要保存。
> - 建议为该工具使用**可单独吊销、有额度上限**的 Key；不再使用时用「清空 Key」或
>   到 Provider 侧吊销。
> - 本地备份（含升级前自动备份）同样包含明文 Key，请同等对待。

## 概念与边界

- Provider 按**企业（workspace）**隔离保存：名称、协议、base_url、模型、预算、超时
  与 Key 都属于某一个企业档案。
- 协议固定 `chat_completions`，非流式；`base_url` 去掉末尾斜杠后追加
  `/chat/completions`。若你填的已是完整 `/chat/completions` 地址，工具会提示改为
  只填 Base URL，不会猜测改写。
- **没有「连接测试」按钮**：保存配置不发起任何模型请求。第一次真正校验路由/Key 是否
  可用，发生在你逐次确认发送的那一次真实请求上。
- **新建 Provider 不会自动成为默认**；需显式「设为默认」。没有默认且未显式选择时，
  生成命令直接拒绝（`PROVIDER_REQUIRED`），不会拿「第一条」凑数。
- 删除默认 Provider 后默认位清空，不会自动切换到其他 Provider。

## Key 的输入与显示

- Key 只通过**标准输入**进入：CLI 用 `provider set --input-json <配置文件> --api-key-stdin`，
  Key 从 stdin 读入（`--input-json -` 与 `--api-key-stdin` 不能同时占用 stdin）。
  **Key 永不出现在命令行参数或配置 JSON 里。**
- 界面显示掩码；CLI/JSON 输出只有 `key_present: true/false` 布尔值——不输出 Key、
  前后缀或哈希。Key 不会进入日志、导出文件或错误信息。
- 清空 Key 是独立操作（需确认）；「删除 Provider」也会随配置一并移除。
- 可以先建一个暂无 Key 的配置（`key_action: keep`）；此时发送预览会返回
  `PROVIDER_KEY_MISSING`，提示先补 Key。

## 配置示例（CLI）

新建（Key 经 stdin）：

```bash
dsheng provider set --workspace <id> --input-json provider.json --api-key-stdin --json
```

`provider.json`（数值为默认值示例）：

```json
{ "schema_version": "1", "operation": "provider_upsert", "workspace_id": "<uuid>",
  "provider_id": null, "expected_revision": null,
  "config": {
    "name": "我的模型服务", "protocol": "chat_completions",
    "base_url": "https://api.example.com/v1", "model": "example-model", "stream": false,
    "timeouts": { "connect_seconds": 15, "response_seconds": 180, "total_seconds": 240 },
    "budget": { "context_window_tokens": 65536, "output_reserve_tokens": 8192 } },
  "key_action": "replace" }
```

```bash
printf '你的Key' | dsheng provider set --workspace <id> --input-json provider.json --api-key-stdin --json
```

查看（无 Key 内容）/ 设默认 / 清 Key / 删除：

```bash
dsheng provider list --workspace <id> --json
dsheng provider use --workspace <id> --provider <id|none> --expected-revision <workspace-revision> --json
dsheng provider clear-key --workspace <id> --provider <id> --expected-revision <provider-revision> --confirm-local --json
dsheng provider delete --workspace <id> --provider <id> --expected-revision <provider-revision> --confirm-local --json
```

说明：

- 更新已有 Provider 时 `provider_id`/`expected_revision` 填实际值；版本冲突返回
  `REVISION_CONFLICT`（见 [troubleshooting.md](troubleshooting.md)），重新读取后重试。
- `provider use` 的 `--expected-revision` 是**工作区 config revision**；清 Key/删除的
  `--expected-revision` 是 **Provider 自身 revision**（`provider list` 可查）。
- 删除前会给出依赖预览；存在未终态生成请求时拒绝删除（`PROVIDER_IN_USE`）。

## URL 与预算约束

- 仅允许 HTTPS；HTTP 只接受字面 `localhost`/`127.0.0.1`/`[::1]`。禁止 URL 携带
  userinfo/query/fragment；不跟随重定向；不提供「跳过 TLS 校验」选项。
- 从 0.1.3 起，模型响应不设应用侧输出时长、空闲时长或大小上限。旧超时配置字段
  仅保留兼容；模型完成、返回错误或用户手动停止时结束请求。
- 预算（token）：上下文窗口 8,192–1,048,576（默认 32,768）、输出预留 1,024–65,536
  （默认 8,192），且 预留 ≤ 窗口/2。这是发送前的保守估算，不是输出长度上限。
- 实测提示：诊断任务的输入上下文较大，默认 32,768 窗口可能直接报
  `CONTEXT_BUDGET_EXCEEDED`；此时可调大 `context_window_tokens`（如 65,536）后重试。

## 发送前的逐次确认

每次诊断/任务生成都会先生成本地预览（confirmation）：显示将使用的 Provider
名称/地址/模型/版本、企业与期间、涉及的来源文件与单元格范围、请求字节数与保守
token 估算、上下文预算，以及「最多 1 次请求」。确认单 10 分钟内有效；任何一项配置
或数据在此期间变化，确认即失效（`CONFIRMATION_STALE`），需重新预览。一次确认最多
产生 1 条 HTTP 请求，没有自动重试；失败后重试要重新预览、重新确认。详见
[diagnosis.md](diagnosis.md) 与 [tasks.md](tasks.md)。

确认发送后，同一弹窗展示等待、接收、校验与保存状态、接收量及距上次输出的时间。
点击「停止生成」即可中断本地请求，包括尚未收到第一个输出块的情况。流式输出开启时
可逐块观察输出活动；关闭时服务商可能一次性返回结果，期间显示等待状态并保留停止按钮。
