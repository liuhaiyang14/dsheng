# 经营分析（日/周/月/季）

桌面端在系统设置中选择核算周期，页眉仅切换已有数据的期间。月度显示“2026年10月”，日度显示“2026年10月1日”；同一期间只显示最后一次分析结果，不再提供分析版本选择。

在经营分析页展开分析入口，先扫描数据，再从文件实际包含的期间中选择。月度按企业设定的月度起始日归集已有数据（默认1日，只有 1—5 日记录也属于当月）；日度仅计算选定当天，整月汇总不能拆成每日值。不能自由填写起止日期，也不会根据最早、最晚日期补造中间空白日期。重新分析已有期间时先显示覆盖提示，确认后才更新当前结果，取消不会执行分析。

月度起始日可以设为1–31。例如起始日2对应1月2日至2月1日；31对应平年1月31日至2月27日，下一期2月28日至3月30日。短月起点取月末，后续月份仍按原日号定位；闰年2月使用29日。页眉月份按周期起点所在月份命名，详情与重新分析确认显示完整起止日期。

以下 CLI 日期范围与版本参数仍供专业操作使用。


## 运行一次分析

```bash
dsheng analyze --workspace <id> --dir <path> --from 2026-10-01 --to 2026-10-31 --grain auto --json
```

- `analyze` 是**纯本地计算**（0 次 Provider 调用）：读取扫描来源 + 映射 + 手工输入 +
  已解决冲突 + 指标定义 + 周期策略，生成一份不可变的分析 revision。
- `--grain` 可选 `auto|day|week|month|quarter`；`auto` 按数据实际覆盖自动选粒度。
  **粒度只能向上汇总，不能向下拆**：完整日数据可汇总成月，月度数据不会被拆成日/周。
- 分析区间跨周期策略生效边界时必须 `--policy <id>` 显式选一个版本；重算用
  `--recalculate`（见 [workspace-period.md](workspace-period.md)）。
- 每次成功分析都追加一个新 revision（保留全部旧版）；输入完全相同也会留下显式重做
  记录。`run_id` 稳定，`current_revision_no` 前移。

查看已保存的结果（只读，不重算）：

```bash
dsheng analysis show --workspace <id> --run <id> --json              # 当前 revision
dsheng analysis show --workspace <id> --run <id> --revision 1 --json # 指定历史 revision
```

## 输出里有什么

- `metrics`：每个指标一行——`target_value`/`actual_value`/`delta`（=实绩−预定）与
  `mom`（环比）/`yoy`（同比）比较对象。所有数值都是十进制字符串，保留原始精度。
- `series`：推移序列——`buckets`（期间桶）、`actual`/`plan` 与桶对齐，**缺桶为 null
  断线，不插值、不造趋势**；`composition` 为构成（如可解析）。
- `analysis_input_fingerprint`：本次分析全部输入的指纹。诊断/任务生成/候选合并都会
  绑定它；任何输入变化都会让旧绑定失效。

## 同比/环比与「不可比」

每个比较对象固定结构：`base_from/base_to`（基期）、`base_value`、`absolute_change`
（绝对差）、`change_rate`（变化率）、`availability_status` 与 `reason`：

- `available`：基期与本期都有效且口径兼容——绝对差=本期−基期；基期>0 才给变化率。
- `partial`：零基期/负基期——绝对差可算，百分比不可算（不拿 |基期| 硬算）。
- `unavailable`：不可比。`reason` 给出确定原因，例如 `missing_base`（没有可用的历史
  期间）、`definition_mismatch`（指标定义/口径变化）、`coverage_incomplete`（基期覆盖
  不完整）、`calendar_mismatch`（如周同期/闰日无对应日期）等。

**工具不会把不可比悄悄算成 0 或省略**：界面/导出会显示不可比原因；周同比固定标注
「52 周同期」。历史比较绑定当时的基期 run/revision（`base_refs`），日后基期被重做也
不会改写旧分析的数字。

## 图表

桌面端提供趋势、计划/实绩、差异与贡献、同比环比、结构、任务进度与时间线等视图，
可导出 PNG 与同范围数据：

```bash
dsheng export --workspace <id> --run <id> --chart <id> --format png --out chart.png
dsheng export --workspace <id> --run <id> --chart <id> --format csv --out chart.csv
```

图表只画真实期间与有效数据：缺桶断线、不把缺失当零、不生成合成趋势；筛选条件、
单位与基期/不可比原因随图导出。

## 指标口径（metric）

指标按版本化定义计算（`metric set` 新建/修订；`metric list` 查看）：

- 四种语义：`flow`（流量，无重叠完整期间求和）、`stock`（存量，期末或期初+净变动）、
  `ratio`（比率，分子分母汇总后重算）、`configured_derived`（按确认的有限公式 AST
  重算——只允许引用/常量/加减乘除，不允许任意 Excel 公式或代码）。
- 修改口径/公式/单位/方向/比较政策都会创建**新版本**；同定义多业务范围（多货品/
  仓库）需显式选择或确认无重叠聚合，否则本地拒绝（`SCOPE_SELECTION_REQUIRED`）。
- 明细与汇总不重复求和；累计列不与其内部月份重复相加；合计行可作对账但不与明细
  重复计入。

```bash
dsheng metric list --workspace <id> --json
dsheng metric set --workspace <id> --input-json metric.json --json
```
