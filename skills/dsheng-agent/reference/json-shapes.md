# dsheng 关键 JSON 结构摘录

字段名**精确照抄**以下 schema，抄写时不得改拼/改驼峰：
`contracts/v1/settings.schema.json`、`contracts/v1/read-views.schema.json`、
`contracts/v1/task-edit.schema.json`、`contracts/v1/task-reconcile.schema.json`。
本文件是摘录，不是替代；冲突时以 schema 为准。

**数值规则（全局）**：本地 JSON 中金额/数量/工时/目标/实绩/当前值/基线与 `progress`
全部是规范十进制**字符串**或 `null`——无指数、无千分位、无多余前导/尾随零、负零写
`0`；输入最多 6 位小数、28 位有效数字；`progress` 为 `0`–`100`、最多 2 位小数。
revision/order/计数是 JSON 整数。日期 `YYYY-MM-DD`，时间 UTC ISO-8601。

## 1. envelope（所有 --json 命令的 stdout，单对象）

```json
{ "schema_version": "1", "ok": true, "data": { } }
{ "schema_version": "1", "ok": false,
  "error": { "code": "REVISION_CONFLICT", "message": "…", "field": null, "details": null } }
```

`field` 是可空的 JSON-pointer 或 flag 名；`details` 只含脱敏元数据。

## 2. confirmationPreview（生成类 --dry-run 的 data）

`settings.schema.json#/$defs/confirmationPreview`。required 全集：

```
confirmation_id(uuid)  expires_at(date-time)  workspace_id(uuid)
workflow("task"|"diagnosis")  operation("task_generate"|"diagnosis_generate"|"diagnosis_continue"|"generation_retry")
provider{provider_id, revision, name, base_url, model}
run_id(uuid)  analysis_revision_no(int>=1)  analysis_input_fingerprint(hex64)
report_id(uuid|null)  report_revision_no(int|null)
input_manifest_hash(hex64)  prompt_version  schema_version  context_policy_version(string|null)
source_scope_summary[]{source_id, file_label, sheet(string|null), selected_range, row_count, content_hash(hex64)}
request_bytes(0..1048576)  conservative_input_tokens(int)
budget{context_window_tokens, output_reserve_tokens}  timeouts{connect_seconds, response_seconds, total_seconds}
request_count_maximum(=1)  key_present(=true)
```

向用户展示发送范围时，至少转述 `provider`、`source_scope_summary`、`request_bytes`、
`conservative_input_tokens`、`request_count_maximum`。确认 TTL 600 秒。

注意：默认预算 `context_window_tokens=32768` 对诊断上下文常不足
（`CONTEXT_BUDGET_EXCEEDED`）；需要时经用户确认调大（如 65536，上限 1048576）。

## 3. 分析视图（analyze 输出 / analysis show）

`read-views.schema.json#/$defs/analysisRunView`：

```
workspace_id(uuid)  run_id(uuid)  period{from,to}  current_revision_no(int>=1)
analysis_input_fingerprint(hex64)  grain("day"|"week"|"month"|"quarter")
metrics[]{ …metricRowView }  series[]{ …metricSeriesView }
```

`metricRowView`：

```
metric_id(uuid)  metric_key(^[a-z][a-z0-9_]{0,63}$)  metric_label  unit  direction("up"|"down"|"neutral")
target_value(decimalString|null)  actual_value(decimalString|null)  delta(decimalString|null)   # delta = actual - target，缺任一操作数为 null
mom(comparisonView|null)  yoy(comparisonView|null)
```

`comparisonView`：

```
comparison_type("mom"|"yoy")  label  base_from  base_to
base_value  absolute_change  change_rate        # 均 decimalString|null
availability_status("available"|"partial"|"unavailable")  reason(string|null)
base_refs[]{run_id(uuid), revision_no(int>=1)}  evidence_ids[]
```

`availability_status="unavailable"` 时 `reason` 给出不可比原因（如 `missing_base`）。
**不得把 null/不可比合成趋势或差值。**

`metricSeriesView`：`metric_key`、`metric_label`、`unit`、`buckets[]`、
`actual[]`（与 buckets 对齐，缺桶为 null，不插值）、`plan[]`、`delta[]|null`、
`composition[]{name, value}|null`。

## 4. scan / workspace / provider 视图

`scanSummaryView`：`workspace_id`、`directory_label`、`scanned_at`、
`files[]{file_id, file_label, row_count, sheet_count, mapping_status("mapped"|"pending_review"), note}`、
`total_rows`、`coverage{from, to, granularity}`、`pending_inputs[]`。

`workspaceView`：`workspace_id`、`name`、`config_revision`、`active_provider_id(uuid|null)`、
`current_period_policy_id(uuid)`、`created_at`、`updated_at`。
（`workspace create` 的 data 为 `{"created":true,"workspace":{…},"initial_period_policy":{…}}`。）

`providerView`：`provider_id`、`workspace_id`、`revision`、`config{…providerPublic}`、
`key_present(bool)`、`created_at`、`updated_at`。**没有 Key/掩码/hash 字段。**

`periodPolicyView`：`period_policy_id`、`workspace_id`、`version`、
`policy{cadence, week_start_day, fiscal_year_start_month, timezone_id, effective_from}`、
`effective_to(date|null)`、`created_at`。

## 5. 任务视图

`taskView`（required 全集，decimal 字段均为字符串）：

```
workspace_id  task_id  task_revision_no  title  summary
category("revenue"|"cost"|"efficiency"|"other")  metric_id(uuid|null)  metric_label
target_value  planned_value  actual_value  current_value        # decimalString|null
unit  mp_label  mp_unit                                    # string|null
owner  planned_start  due_date  resource_need  success_criteria  note  review_note
target_period{from,to}|null
lifecycle("active"|"superseded")  status("not_started"|"in_progress"|"completed"|"blocked")
progress(progressString)  progress_mode("auto"|"manual")
adjustment_reason  blocked_reason  completed_at
smart_check{smart_status("generated"|"stale"), assessed_task_revision, checks[]{dimension(S|M|A|R|T), status("met"|"needs_confirmation"|"not_applicable"), note}}
steps[]{step_id, step_order, action, owner, start_date, due_date, done_criteria, status, blocked_reason, completed_at}
evidence[]{evidence_id, source_label, locator, analysis_revision_no}
events[]{event_id, task_id, task_revision_no, event_seq, kind, reason, actor_source("ui"|"cli"|"import"|"generation"), occurred_at}
origin_run_id(uuid|null)  overdue(bool)
```

`taskCandidateView`（生成候选）：`candidate_generation_id`、`candidate_revision_no`、
`run_id`、`analysis_revision_no`、`state("candidate"|"applied"|"discarded")`、
`envelope_locally_reconstructed`、
`tasks[]{task_key, title, summary, category, target_value, owner, steps[]{step_id, action}}`。
模型 `task_key`/`step_id` 只在候选内有效，**不是**永久身份。

## 6. task-edit 输入（task update / task step * 的 --patch-json / --input-json）

顶层 required：`schema_version("1")`、`workspace_id(uuid)`、`task_id(uuid)`、
`expected_task_revision(int>=1)`、`mutation_id(uuid)`、`operation`。
客户端生成 `mutation_id`（幂等：同 ID 同 digest 重放返回原结果）。

`operation` 三选一：

- `content`：`{"kind":"content","patch":{…},"step_operations":[…],"source_note":"…"}`
  - `patch` 可写：`title`、`summary`、`category`、`metric_id`、`target_value`、
    `planned_value`、`actual_value`、`current_value`、`unit`、`mp_label`、`mp_unit`、
    `owner`、`planned_start`、`due_date`、`resource_need`、`success_criteria`、`note`、
    `review_note`、`target_period{from,to}`。**不可写** status/progress/completed_at/lifecycle。
    数值字段改动必须同时给非空 `source_note`。
  - `step_operations[]`：`{"op":"add","client_step_key","content":{action,owner,start_date,due_date,done_criteria},"before_step_id(uuid|null)}`
    ／ `{"op":"update","step_id","patch":{action|owner|start_date|due_date|done_criteria|status|blocked_reason}}`
    ／ `{"op":"delete","step_id"}` ／ `{"op":"move","step_id","before_step_id(uuid|null)}`。
  - `patch` 与 `step_operations` 至少其一。
- `progress`：`{"kind":"progress","progress_mode":"auto"|"manual","progress":…,"status":…,"adjustment_reason":…,"blocked_reason":…}`
  六字段全 required：manual 时 `progress` 为 0–100 字符串且 `adjustment_reason` 非空；
  auto 时两者必须 `null`（由服务计算）。`status="blocked"` 时 `blocked_reason` 非空，
  其余状态必须 `null`。manual 约束：not_started 仅 0、completed 仅 100、in_progress 0–<100。
- `period_link`：`{"kind":"period_link","action":"attach"|"update"|"detach","run_id","analysis_revision_no","cycle_role":"review"|"plan","source_period":{from,to},"target_period":{from,to}}`。

## 7. task-reconcile 输入（--dry-run 与 --apply 同一 Schema）

顶层 required：`schema_version("1")`、`workspace_id(uuid)`、`run_id(uuid)`、
`analysis_revision_no(int>=1)`、`input_fingerprint(hex64)`、`candidate_generation_id(uuid)`、
`candidate_revision_no(int>=1)`、`expected_tasks[]{task_id, task_revision_no}`、
`mutation_id(uuid)`、`mappings[]`。`--apply` 时另在**顶层**加 `apply_token`（dry-run 返回）。

`mappings[]` 每候选恰好一条，四选一：

- `{"candidate_task_key":"…","action":"new"}`
- `{"candidate_task_key":"…","action":"ignore"}`
- `{"candidate_task_key":"…","action":"replace","existing_task_id":uuid,"reason":"非空"}`
- `{"candidate_task_key":"…","action":"update","existing_task_id":uuid,"take_fields":[…],
   "step_mappings":[{candidate_step_id,action:"new"}
     |{candidate_step_id,action:"update",existing_step_id,take_fields:[action|owner|start_date|due_date|done_criteria|evidence_ids]}
     |{candidate_step_id,action:"ignore"}],
   "remove_existing_step_ids":[uuid],
   "final_step_order":[{kind:"existing",step_id}|{kind:"new_candidate",candidate_step_id}]}`

`take_fields` 枚举（任务级）：`title`、`summary`、`category`、`metric_id`、`baseline`、
`target_period`、`target_value`、`unit`、`owner`、`planned_start`、`due_date`、
`resource_need`、`success_criteria`、`evidence_ids`、`assumptions`、`needs_review`。
未列字段绝不覆盖；`expected_tasks` 必须精确覆盖全部 update/replace 旧任务。

dry-run 输出 `reconcilePreviewView`：`apply_token(uuid)`、`expires_at`、
`affected[]{task_id, task_revision_no, action("new"|"update"|"replace"|"ignore")}`、
`created_new[]`、`ignored[]`、`replaced[]`。

## 8. 设置操作输入（--input-json）

`workspace_create`：

```json
{ "schema_version": "1", "operation": "workspace_create", "name": "示例企业",
  "initial_period_policy": { "cadence": "month", "effective_from": "2026-01-01" } }
```

（`week_start_day` 默认 1、`fiscal_year_start_month` 默认 1、`timezone_id` 默认
`Asia/Shanghai`；CLI 会补默认并输出实际采用值。）

`period_policy_set`：`{"schema_version","operation":"period_policy_set","workspace_id",
"expected_revision","policy":{cadence, week_start_day, fiscal_year_start_month, timezone_id, effective_from}}`。

`provider_upsert`：

```json
{ "schema_version": "1", "operation": "provider_upsert", "workspace_id": "<uuid>",
  "provider_id": null, "expected_revision": null,
  "config": { "name": "示例", "protocol": "chat_completions", "base_url": "https://…/v1",
    "model": "…", "stream": false,
    "timeouts": { "connect_seconds": 15, "response_seconds": 180, "total_seconds": 240 },
    "budget": { "context_window_tokens": 65536, "output_reserve_tokens": 8192 } },
  "key_action": "replace" }
```

新建 `provider_id`/`expected_revision` 均为 `null`；更新均为实际值。`key_action=replace`
必须配合 `--api-key-stdin`；`keep` 保留原 Key。约束：
`total_seconds >= connect_seconds + response_seconds`、
`output_reserve_tokens <= context_window_tokens/2`、URL 仅 HTTPS（loopback 例外）。
保存/清 Key/设默认**不调用模型**。

## 9. 常用 data 包装键

命令成功 `data` 的第一层包装键：`workspace list→workspaces[]`、
`workspace period list→policies[]`、`provider list→providers[]`、`metric list→metrics[]`、
`task list→tasks[]`、`task timeline→entries[]`、`history list→runs[]`、
`history revisions→revisions[]`、`history compare→comparisons[]`、
`diagnosis list→reports[]`（`diagnosisReportSummaryView`：
`report_id/run_id/analysis_revision_no/framework/problem_statement/report_revision_no/state/stop_reason/can_continue/active_warning_count/…`）。
`analyze`/`analysis show` 直接返回 `analysisRunView`（含 `run_id`）。
