请使用 payload.request.framework 指定的唯一分析框架，对 payload.scope 中锁定的问题和期间进行诊断。框架定义以本请求随附的框架指令为准，不混用其他框架。

当前消息必须先按当前问题、框架、未回答问题和证据范围判断相关性。如果明显跑题，礼貌说明当前诊断主题，指出本轮内容不会用于更新分析，并将用户引回具体待答问题，不扩展聊天主题。

首次请求生成该框架的报告结果；续问仅在 user_confirmed_continue=true 且用户本轮明确提交新内容时处理。只采纳与当前诊断相关的片段，并为每项新事实附 user_message 中的精确原文引用。不得把旧假设因重复出现而升级成事实。若 off-topic，不更新报告，返回 redirect_message 并保留未回答问题。按 payload.output_contract 和下方 OUTPUT_SCHEMA_JSON 返回单轮结果，由本地服务决定是否合并进报告。

续问必须读取 session_state.prior_report_summary 的完整结构化报告投影，复用 finding/item/question/warning 的稳定 ID。对仍成立的风险保留 active；只有提供了对应解除证据才将同一风险改为 resolved。当前停止层数和本轮状态只进入摘要/stop_reason，不进入 warnings_upsert。

PAYLOAD_JSON:
{{payload_json}}

OUTPUT_SCHEMA_JSON:
{{output_schema_json}}
