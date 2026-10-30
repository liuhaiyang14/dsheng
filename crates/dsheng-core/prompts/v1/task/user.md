请根据下面的经营数据和任务制定规则，生成目标期间的任务单。只使用 payload 中已提供的信息和证据。经营问题诊断报告是可选参考，不能把其中未核实的推断当作事实。

先明确改善目标，再按业务依据拆解行动步骤。对总体目标及拆解后的子目标逐项执行 SMART 检查。遇到同一指标的来源冲突时保留两边证据并标记 needs_review，不要替用户选定结果。责任人、行动节点、完成标准和证据引用按输出结构填写。缺少依据的字段使用 null/空数组并加入 needs_review，不要猜测或补齐。

严格按 payload.output_contract 和下方 OUTPUT_SCHEMA_JSON 返回单个 JSON 对象，不要输出 Markdown、解释性前言、诊断报告或 5WHY/5W/5W2H 追问。

PAYLOAD_JSON:
{{payload_json}}

OUTPUT_SCHEMA_JSON:
{{output_schema_json}}
