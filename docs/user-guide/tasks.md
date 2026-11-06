# 任务单：生成、合并、步骤与进度

## 手动新建与修改

选择已有数据的期间后，点击任务单页的「新增任务」，即可直接保存一项正式任务，
不需要配置 Provider 或生成诊断报告。新建时选择「本期任务」或「下期计划」，期间
按该次分析保存的企业周期计算；填写任务名称和至少一项行动，可补充负责人、目标、
日期、完成标准等信息。填写数值时需写明数值依据。新任务从未开始、进度 0 开始，
不会自动产生 AI 评价。

点击列表中的任务打开详情，可以修改任务、增加行动项，并为行动项填写负责人、
起止日期和验收标准；已有行动项可以编辑、删除或调整顺序，至少保留一项。人工新增
与 AI 生成后已应用的任务使用相同的编辑入口，保存修改不调用模型。

修改任务文字和目标不会重置已有执行进度；自动进度模式下，增删行动项或改变完成
状态会按行动项重新计算进度。重新打开已完成事项时会提示确认。内容修改后，原有
SMART 评价标为待复核，不会把旧评价当成对新内容的认可。

CLI 也可本地新建，输入结构见 `contracts/v1/task-create.schema.json`：

```bash
dsheng task create --workspace <id> --input-json manual-task.json --json
```

## 显式生成（先预览、逐次确认）

任务单由你显式发起（可引用一份同版本的诊断报告），进入页面或生成报告**不会**自动
触发任务生成；任务生成也不会发起根因追问。

任务数量和每项任务的步骤数量按经营问题、证据与执行需要决定，不再限制为一项任务、
一个步骤。重要的独立事项分别列出，同一目标拆成必要行动；不要求每类都有任务，也不
固定生成若干条来凑数。缺少依据的负责人、金额和日期仍需留空或待确认。

```bash
dsheng task generate --workspace <id> --run <run-id> --dry-run --json
dsheng task generate --workspace <id> --run <run-id> --diagnosis-report <report-id> --dry-run --json
```

- 与诊断同一条确认门禁：`--dry-run` 预览 → 你批准 → `--confirmation-id <id>
  --confirm-send` 才发送；最多 1 次请求，无自动重试。
- 只有你选中的诊断报告会作为参考，带入其已保存的框架、摘要、事实、假设和相关证据；
  假设仍是假设，不会当作已核实事实。报告必须与当前分析 revision/指纹一致；报告或
  证据已过期、不再属于本次来源范围时会提示重选或取消引用，不会静默发送不匹配内容。
  无报告或取消引用后，仍可依据本期数据生成任务。
- 发送预览会列出本次包含的历史任务名称和数量，说明将发送当前执行记录；同时展示所选报告的框架、摘要与
  引用数量，便于确认发送范围。
- 生成结果是一份**候选（candidate）**，不会直接改动你的任务列表。
- 若生成后页面异常或关闭，可在同一企业、同一期间点击「读取已生成结果」，取回与当前分析版本一致的待应用候选。此操作只读取本地数据库，不重新调用模型；已应用、已丢弃或分析已变更的结果不会被恢复。

## 第一期与后续期间

第一期没有相关历史任务是正常情况，直接依据本期经营数据制定任务。手动新建后直接
保存为正式任务；AI 生成候选后需要你确认应用，才会成为后续可检视的正式任务。
仅生成但未应用的候选不会算作执行记录。

进入后续期间时，程序会读取同一企业在当期需要检视的已应用任务，以及更早期间仍未
完成的任务，不带入未来期间任务。你需要在任务详情中维护实际进度、阻塞原因和复盘
记录，模型才能参考这些已保存的信息。记录为「已完成」不等于经营目标已达成；没有
记录的未完成原因不会被自动补造，也不会仅因有旧任务就重复创建一份。

## 候选合并（reconcile）：new / update / replace / ignore

首次为该分析生成且本期没有任何任务时，可直接创建；否则（包括显式重生成）一律进入
候选合并：你在「生成建议」与现有任务之间逐项选择，工具不做标题/指标/模型键的自动
匹配。

```bash
dsheng task reconcile --workspace <id> --run <run-id> --input-json reconcile.json --dry-run --json
```

四种动作：

| 动作 | 结果 |
| --- | --- |
| `new` | 用候选内容创建新任务（新 UUID），新步骤一律「未开始」 |
| `update` | 你指定现有任务与步骤，仅 `take_fields` 白名单内字段用候选值覆盖；**未列字段绝不覆盖**；进度/状态不受候选影响 |
| `replace` | 指定旧任务并填理由：创建新任务，旧任务转为 superseded（只读保留历史） |
| `ignore` | 忽略该候选项（选择仍被记录） |

- 每个候选任务必须恰好选择一次；一个旧任务不能被多个候选写入/替代；update 的候选
  步骤同样要逐个 new/update/ignore。未被提及的旧任务/步骤保持原样。
- `--dry-run` 返回 diff 与 `apply_token`（10 分钟）；确认后把 token 写入输入 JSON
  顶层，再 `--apply`：
  ```bash
  dsheng task reconcile --workspace <id> --run <run-id> --input-json reconcile_apply.json --apply --json
  ```
- `update` 的 `take_fields` 若包含你人工编辑过的字段，界面会高亮「将覆盖你的修改」；
  空候选（模型建议 0 条任务）也是有效结果，不会被解释为「删除全部任务」。

## 字段与身份

- 任务/步骤的永久身份是本地分配的 UUID；模型输出的 task_key/step_id 只在单份候选内
  有效，不作为身份。
- 类别为固定四类：revenue（销售最大化）/cost（费用最小化）/efficiency（时间最短化）/
  other（其他事项）。除 other 外必须绑定本工作区有效指标；模型给出未知类别时整份
  候选被拒（`TASK_CATEGORY_UNRECOGNIZED`），不会默认成 other。
- 数值（目标/预定/实绩/当前值）是十进制字符串；人工修改数值必须写 `source_note`
  （调整依据），它不是模型事实。

## 编辑任务与步骤

```bash
dsheng task list --workspace <id> [--run <id>] --json
dsheng task show --workspace <id> --task <id> --json
dsheng task update --workspace <id> --task <id> --patch-json edit.json --json
dsheng task step update --workspace <id> --step <id> --patch-json step.json --json
dsheng task step add --workspace <id> --task <id> --input-json step.json --json
dsheng task step delete --workspace <id> --step <id> --expected-revision <n> --json
dsheng task step move --workspace <id> --step <id> --before <step-id> --expected-revision <n> --json
```

- 所有写入都要 `expected_task_revision`（乐观并发）；冲突报 `REVISION_CONFLICT`，
  重读当前值后整体重试，不重放旧 patch。带 `mutation_id` 的重复提交返回原结果，
  不重复创建。
- 内容编辑（标题/数值/日期/类别/单位等）与进度推进是**两种操作**；内容编辑不允许
  直接写 status/progress。数值类改动需 `source_note`；步骤日期必须在任务期间内
  （`STEP_OUTSIDE_TASK_PERIOD`）；任务至少保留一条活跃步骤（`AT_LEAST_ONE_STEP`）。
- 别名说明：CLI 的 `task update --progress/--status/--note` 是同一编辑合同的简写
  （仍需 expected revision），正式脚本建议直接用 `--patch-json`。

## 进度与状态规则

- 步骤状态：not_started / in_progress / completed / blocked（blocked 必填理由）。
- **自动模式**：进度=已完成步骤数/总数（四舍五入两位）；状态按优先级推导——
  有阻塞（任务级理由或任一步骤 blocked）→ blocked；全部完成 → completed+100；
  有进行中/完成 → in_progress；全部未开始 → not_started+0。真实未全完成但四舍五入
  恰到 100 时封顶显示 99.99 并保留计数。
- **手动模式**：你给 0–100 的总体进度并必须写 `adjustment_reason`（解释与步骤比率
  的差异）。约束：not_started 仅 0；completed 仅 100；in_progress 为 0–99.99。
  下一次步骤状态/增删变化会自动切回自动模式并记 `progress_override_reset` 事件。
- 解除阻塞需显式提交非 blocked 状态；仍有 blocked 步骤时报
  `STEP_STILL_BLOCKED`，不会替你绕过。
- 完成/重开：从非 completed → completed 写完成时间；重开清空完成时间并追加 reopened
  事件，旧完成事件保留。被 replace 的旧任务（superseded）只读，不再推进。
- 「已延期」是派生提示（今天 > 截止日且未完成），不是独立状态；未设截止日不判延期。

## SMART 校验状态

生成时模型输出会附 SMART 评价（S/M/A/R/T 各维度 met / needs_confirmation /
not_applicable），详情中分别展示为「通过」「待确认」「不适用」，总目标与子目标的评价逐项保留。
之后你修改了目标语义字段（目标值、指标、单位、日期、步骤结构等），
状态转为 **stale**，界面提示「内容已改待复核」并显示旧评价对应的任务版本。评价不可用时只在该区域提示，任务信息和步骤仍可查看与修改。工具不会静默重新调用
模型复评——需要新评价就重新生成候选。

## 时间线

```bash
dsheng task timeline --workspace <id> --from 2026-11-01 --to 2026-11-30 --grain day --json
```

按日/周/月/季与日期范围查看计划开始、截止、完成与进度事件；未设置日期的事件不参与
日期排序，工具不会伪造排序日期。
