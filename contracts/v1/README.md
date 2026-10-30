# 规划阶段机器可读合同

这些Schema、目录和合成算例是工程输入，产品尚未实现。目录v1是集合名称，具体版本取文件内字段。

| 文件 | 用途 | 规则来源 |
| --- | --- | --- |
| metric-catalog.json、data-period-cases.json | 指标与数据/周期合成算例 | data-period-import.md |
| task-input.schema.json、diagnosis-input.schema.json | 模型输入版本2 | 数据合同、prompt-context-validation.md |
| task-edit.schema.json、task-reconcile.schema.json、task-state-cases.json | 编辑、候选采纳、任务状态 | task-lifecycle.md |
| settings.schema.json、cli-surface.json | 配置、命令及错误码 | provider-config-delivery.md |

规则来源在[docs/specs](../../docs/specs/)。主计划组织范围/分工/验收，专项合同定义唯一行为；发现冲突由规划主Agent修复，不留给实现者自由选择。

本地金额/数量/工时及进度遵守各自Decimal字符串合同，模型number用Decimal-aware序列化和解析，不经float。Schema按$id离线注册，输入引用diagnosis-report-v3，不访问网络。完整Draft202012与format校验后，还要检查日期顺序、ID/引用归属、指纹、计算及状态机。

静态校验入口tools/validate_contracts.py仅解析合同及合成正反用例，不访问Provider或业务文件，不实现应用。领域算例是已定义预期，必须区分独立复算与未来产品执行。Provider真实内容依单独授权和内容矩阵验收，禁止将合成对象冒充模型响应。
