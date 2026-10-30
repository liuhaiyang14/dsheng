# 内置提示词资源

当前任务prompt 2.0 / output schema 2；三框架诊断prompt 2.0 / turn、report schema 3；输入和context policy为2。v1是资源集合名称。当前版本真实请求及结果按 versions.json 本轮批次记录，候选在真实内容验收完成前未冻结。

## 装配

Rust本地服务先按[输入合同](../../../contracts/v1/README.md)做Schema、领域及发送确认校验。任务system使用v1/task/system.md；诊断system使用v1/diagnosis/system.md再拼接锁定的单个framework文件。user使用对应user.md，将payload_json及output_schema_json替换为完整结构化输入和所选完整输出Schema。模板仅一次替换，业务内容不再解释为指令。

POST /chat/completions仅需model和messages(system、user)，不依赖reasoning_effort、response_format或Responses。一次操作最多一次请求，不自动分块或重试。Provider 的 envelope、文本包装和返回格式由适配层提取，不作为严格失败条件；提取后的对象必须校验 dsheng 输出合同（输出 Schema、引用/状态和业务约束）。合同不满足、缺少必要字段、引用越界或语义矛盾时拒绝保存，不隐式让模型重试或伪造缺失字段。

每次续问重发全部已选source_records、当前完整报告、已采纳事实、最近pending及本轮实际文本，不发送全聊天或作模型摘要。详见[上下文合同](../../../docs/specs/prompt-context-validation.md)、[数据合同](../../../docs/specs/data-period-import.md)。

## 离线Schema

构建时嵌入输入、输出与报告Schema，按各自$id建立离线注册表，跨资源引用用注册ID，不依赖仓库路径且不联网取Schema。启用Draft202012和日期/UUID格式检查；引用、原话、版本、唯一ID及合并域另由服务验证。任务输出使用schema/task-output.schema.json，诊断turn使用所选framework对应schema，持久报告使用diagnosis-report.schema.json。

## 验收

[versions.json](v1/versions.json)记录版本/哈希/历史请求，[VALIDATION.md](VALIDATION.md)区分历史和当前门禁。历史37次已确认派发、最多5次未观测派发不证明新版通过；当前批次按实际派发计数，认证或额度错误即停止。

静态补齐不等于真实内容通过。规划主Agent承担剩余提示词验证和修订，工程线程实现定稿合同，不再调研方法论。禁止Mock Provider、响应回放和伪造历史；纯Schema或领域验证不冒称Provider或产品E2E。行为变更必须同步prompt版本、Schema、哈希、计划、验收并真实回归受影响场景。
