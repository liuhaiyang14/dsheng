请根据下面的经营数据和任务制定规则，生成目标期间的任务单。只使用 payload 中已提供的信息和证据。经营问题诊断报告是可选参考，不能把其中未核实的推断当作事实。

按每个指标的 direction、target_constraint_scope 和候选的可比/冲突/证据状态执行内置目标规则，不向费用/耗时套收入下限。先明确改善目标，再按业务依据拆解行动步骤。对总体目标及拆解后的子目标逐项执行 SMART 检查。遇到同一指标的来源冲突时保留两边证据并标记 needs_review，不要替用户选定结果。责任人、行动节点、完成标准和证据引用按输出结构填写。缺少依据的字段使用 null/空数组并加入 needs_review，不要猜测或补齐。

优先按 payload.output_contract 和下方 OUTPUT_SCHEMA_JSON 返回单个 JSON 对象，不要输出 Markdown、解释性前言、诊断报告或 5WHY/5W/5W2H 追问。若 Provider 外层包装不同，由适配层抽取；抽取后的对象仍必须通过输出合同。

本次只需返回最小但完整的合同对象：除非输入明确要求多个事项，最多一个任务、一个步骤；所有无依据字段使用 null/空数组。首选从 `{` 开始到 `}` 结束的 JSON；如果外层格式不同，适配层仍会提取字段并执行合同校验。

PAYLOAD_JSON:
{{payload_json}}

OUTPUT_SCHEMA_JSON:
{{output_schema_json}}
