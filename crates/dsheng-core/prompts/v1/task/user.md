请根据下面的经营数据和任务制定规则，生成目标期间的任务单。只使用 payload 中已提供的信息和证据。经营问题诊断报告是可选参考，不能把其中未核实的推断当作事实。

按每个指标的 direction、target_constraint_scope 和候选的可比/冲突/证据状态执行内置目标规则，不向费用/耗时套收入下限。先检查全部有依据的重要独立改善事项，分别明确改善目标，再按业务依据拆解每项必要的行动步骤。任务和步骤数量按实际需要决定，不设固定数量、不按分类凑数、不重复同一行动。对总体目标及拆解后的子目标逐项执行 SMART 检查。遇到同一指标的来源冲突时保留两边证据并标记 needs_review，不要替用户选定结果。责任人、行动节点、完成标准和证据引用按输出结构填写。缺少依据的字段使用 null/空数组并加入 needs_review，不要猜测或补齐。

优先按 payload.output_contract 和下方 OUTPUT_SCHEMA_JSON 返回一个 **Schema 2 根 JSON 对象**，不要输出 Markdown、解释性前言、诊断报告或 5WHY/5W/5W2H 追问。根对象必须同时包含 `schema_version: "2"`、字符串 `summary` 和数组 `tasks`；即使只有一个任务，也必须放在 `tasks` 数组中，不能把任务字段直接放在根上。若 Provider 外层包装不同，由适配层抽取这个根对象；抽取后的对象仍必须通过输出合同。

返回覆盖有依据的重要改善事项的完整合同对象；每个独立目标可形成一项任务，每项任务可含多个必要步骤。先检视 business_review.prior_task_outcomes 中的本地执行记录；没有历史记录时直接依据本期数据制定任务，不补造历史。所有无依据字段使用 null/空数组。首选从 `{` 开始到 `}` 结束的 JSON 根对象，形状必须是 `{"schema_version":"2","summary":"...","tasks":[...]}`；没有合格任务时使用 `tasks: []`，不要返回裸任务对象。如果外层格式不同，适配层仍会提取这个根对象并执行合同校验。

PAYLOAD_JSON:
{{payload_json}}

OUTPUT_SCHEMA_JSON:
{{output_schema_json}}

END OF OUTPUT SCHEMA. 下面是一个**完整输出的形状示例**。它只演示根对象、`tasks` 数组与任务对象的嵌套形状，以及无排期依据时步骤日期写 null 并在 needs_review 标注的写法。示例仅展示一个数组元素以说明结构，绝不限定实际任务或步骤数量；实际输出须按本次证据包含全部必要任务和步骤。示例中的数值、日期、指标 ID 和证据 ID 都是合成占位值，必须完全忽略，只能使用本次 payload 中的真实值：

{"schema_version":"2","summary":"基于来源记录生成一项合成销售额改善任务；步骤排期无依据，待确认。","tasks":[{"task_key":"candidate-task-1","title":"提升10月合成销售额","summary":"来源记录显示9月合成销售额为100.25；10月目标不低于100。","category":"revenue","metric_id":"00000000-0000-4000-8000-000000000020","source_period":{"from":"2026-09-01","to":"2026-09-30"},"target_period":{"from":"2026-10-01","to":"2026-10-31"},"baseline":{"value":100.25,"unit":"元"},"target_value":100,"unit":"元","owner":null,"planned_start":"2026-10-01","due_date":"2026-10-31","steps":[{"step_id":"candidate-step-1","order":1,"action":"与负责人确认10月销售改善措施","owner":null,"start_date":null,"due_date":null,"done_criteria":"措施清单获负责人确认","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"],"status":"not_started"}],"resource_need":null,"success_criteria":"10月合成销售额不低于100元","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"],"assumptions":[],"needs_review":["步骤执行日期待确认","目标数值依据待用户确认"],"smart_check":{"overall_goal":{"goal_text":"将2026年10月合成销售额改善至不低于100","criteria":{"S":{"status":"met","rationale":"目标指标与数值明确","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"]},"M":{"status":"met","rationale":"以销售额数值衡量","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"]},"A":{"status":"needs_confirmation","rationale":"改善幅度依据待确认","evidence_ids":[]},"R":{"status":"met","rationale":"与经营重点相关","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"]},"T":{"status":"met","rationale":"目标期间为2026年10月","evidence_ids":["cell:00000000-0000-4000-8000-000000000010:数据:B2"]}}},"subgoals":[]}}]}

注意示例的三个关键形状特征：根上只有 `schema_version`、`summary`、`tasks` 三个键；每个任务的 20 个字段全部位于 `tasks` 数组对应元素内部，一个都没有出现在根上；无排期依据的步骤 `start_date`/`due_date` 为 null 且 needs_review 含"步骤执行日期待确认"。你的输出必须复刻这个嵌套形状，用本次 payload 的真实值替换所有占位值。

Before sending, check the root shape one last time. Return exactly one root object with these three top-level keys:
{"schema_version":"2","summary":"简短总结","tasks":[]}
When a task exists, put the complete task object inside `tasks`; never move task fields to the root. `schema_version` is the output contract version `"2"`, which is separate from this prompt's version.
