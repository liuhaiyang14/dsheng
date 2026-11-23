# v1 机器可读合同

这些 Schema、目录和合成算例是当前产品的机器可读输入与输出约束。目录 v1 是集合名称，具体版本取文件内字段；应用实现、CLI 导出和回归测试已覆盖其中的固定行为。

| 文件 | 用途 | 规则来源 |
| --- | --- | --- |
| metric-catalog.json、data-period-cases.json | 指标与数据/周期合成算例 | data-period-import.md |
| task-input.schema.json、diagnosis-input.schema.json | 模型输入版本2 | 数据合同、prompt-context-validation.md |
| task-edit.schema.json、task-reconcile.schema.json、task-state-cases.json | 编辑、候选采纳、任务状态 | task-lifecycle.md |
| settings.schema.json、cli-surface.json | 配置、命令及错误码 | provider-config-delivery.md |
| read-views.schema.json | 读侧命令输出视图（2026-11-02 用户批准的增量扩展；含 analysis show 命令） | task-lifecycle.md §7、data-period-import.md §1.4 |
| two-tables.schema.json | 固定 `two_tables_v1` 两表输出结构、列顺序、期间列方向、标题合并/冻结和图表行 | data-period-import.md §4.4–§5、[two-tables-v0.3.0.md](../../docs/engineering/two-tables-v0.3.0.md) |

规则来源在[docs/specs](../../docs/specs/)。主计划组织范围/分工/验收，专项合同定义唯一行为；发现冲突由规划主Agent修复，不留给实现者自由选择。

本地金额/数量/工时及进度遵守各自Decimal字符串合同，模型number用Decimal-aware序列化和解析，不经float。Schema按$id离线注册，输入引用diagnosis-report-v3，不访问网络。完整Draft202012与format校验后，还要检查日期顺序、ID/引用归属、指纹、计算及状态机。

`two-tables.schema.json` 描述的是分析 revision 的固定结果投影，不是 source 输入 schema。所有 source profile 必须投影到同一 `two_tables_v1`；企业文件名、工作表名、客户、人员和业务样例值不得写入 schema 或导出分支。核算表固定 12 列；推移表固定 14 列，期间（月）按行从旧到新排列，月份行 5–20，累计行 21；图表页固定 4 张原生折线图。列名、工作表名、标题合并和冻结窗格均为固定布局。缺失、待补、冲突和不可比值保留 `null`/`pending` 与原因，禁止补零或造趋势。

静态校验入口tools/validate_contracts.py仅解析合同及合成正反用例，不访问Provider或业务文件，不实现应用。领域算例是已定义预期，必须区分独立复算与未来产品执行。Provider真实内容依单独授权和内容矩阵验收，禁止将合成对象冒充模型响应。
