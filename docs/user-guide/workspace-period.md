# 企业档案与核算周期

## 企业（workspace）档案

- 一个企业档案 = 一套独立的导入来源、指标、分析历史、任务、诊断报告与 Provider 配置。
  不同企业之间数据完全隔离；对象 ID 归属不符会直接拒绝（`WORKSPACE_MISMATCH`），
  不会「顺便」切到另一个企业。
- 新建企业时必填**核算周期类型**（day/week/month/quarter）与**生效日**；时区默认
  `Asia/Shanghai`、周起始日默认周一（ISO）、财年起始月默认 1 月。CLI 省略这些默认值时
  会明确输出实际采用的默认，不依赖电脑时区。
- 「当前企业」是一个用户偏好：桌面端顶部切换，CLI 用 `workspace use`。显式
  `--workspace <id>` 始终优先；两者都没有则报 `WORKSPACE_REQUIRED`。
- 企业重命名只改名字，不改任何历史快照。

CLI 示例：

```bash
dsheng workspace list --json
dsheng workspace create --input-json ws.json --json        # ws.json 见下
dsheng workspace update --workspace <id> --input-json ws-rename.json --json
dsheng workspace use --workspace <id> --json
```

`ws.json`：

```json
{ "schema_version": "1", "operation": "workspace_create", "name": "示例企业",
  "initial_period_policy": { "cadence": "month", "effective_from": "2026-01-01" } }
```

## 周期策略与版本化

周期策略决定「一期」的边界，包含六项：`cadence`（day/week/month/quarter）、
`month_start_day`（1–31，默认1，仅月度使用）、
`week_start_day`（ISO 1–7，默认 1=周一）、`fiscal_year_start_month`（1–12，默认 1）、
`timezone_id`（IANA 时区，默认 `Asia/Shanghai`）、`effective_from`（生效日，必填）。

关键规则——**策略按生效日版本化，历史不重释**：

- 修改周期不是「改设置」，而是创建一个**新版本**的策略，从 `effective_from` 起对新
  分析生效；此前已完成的分析保留旧策略快照，不会被拆开重算。
- 例如「月度 → 日度」：生效日之后的新分析按日切期；之前的月度结果仍是完整月度。
  同一日期范围想用旧口径重算，需显式选择旧策略重做。
- 分析区间**跨越策略生效边界**时必须显式选择一个策略版本作用于整个区间
  （`--policy <id>`），工具不会自动分段混算。
- 期间用 `workspace + period_key` 唯一定位（含策略版本、粒度、起止与业务范围）；同一
  期间重做是「追加不可变 revision + 更新当前指针」，不会产生重复期间记录。

CLI 示例：

```bash
dsheng workspace period list --workspace <id> --json
dsheng workspace period set --workspace <id> --input-json policy.json --json
```

`policy.json`（把月度策略从 2026-12-01 起改为周度、周日开始）：

```json
{ "schema_version": "1", "operation": "period_policy_set", "workspace_id": "<uuid>",
  "expected_revision": 2,
  "policy": { "cadence": "week", "week_start_day": 7, "fiscal_year_start_month": 1,
              "timezone_id": "Asia/Shanghai", "effective_from": "2026-12-01" } }
```

## 桌面端月度起始日

在系统设置选择月度后，可设置每月起始日1–31。每期包含起止两日，结束日为下期起点的前一天。起始日2表示1月2日至2月1日；起始日31表示平年1月31日至2月27日，下一期2月28日至3月30日。短月取当月最后一天，闰年2月取29日；之后月份继续按原日号锚定，不累计偏移。

桌面扫描与分析明确采用企业当前选择的策略，历史日期重新分析也使用该口径；选择的月份以周期起点所在月命名。页眉仅切换已保存结果，同一月份/日期只显示最后一次分析。已有期间重新分析会先要求确认覆盖；历史数据库版本继续用于内部追溯。

## 历史回算（指定口径重做）

对已完成期间想换口径（或源数据修正后）重算：

```bash
dsheng analyze --workspace <id> --dir <path> --from 2026-10-01 --to 2026-10-31 --grain month --policy <id> --recalculate --json
```

- `--policy <id>` 显式选用哪个策略版本；跨边界区间必须给。
- 回算按同一 `period_key` 生成新 revision（保留旧 revision 可查），或在新口径下生成
  新 period 记录；旧结果与旧任务链接不被覆盖。
- 回算是本地计算，不调用 Provider。

## 日期与周/财年约定

- 业务日期窗口是**闭区间** `[from, to]`；事件时间以 UTC 保存、按企业时区展示，
  不偷用电脑时区。
- 周按 `week_start_day` 起算 7 天；财季按财年起月锚定连续 3 个月，财年按起始年命名。
- 周同比固定「前移 364 天（52 周同期）」，不切 ISO 53 周；闰日等无对应日期时该比较
  置为不可比并给出原因（见 [analysis.md](analysis.md)）。

## 建议做法

1. 首次建企业时就确认好周期类型与时区，减少后期策略切换带来的解释成本。
2. 需要切换口径时，提前规划 `effective_from`（一般选一期起点），保持各版本生效区间
   不重叠。
3. 换新企业做实验（例如测试导入）再切回原企业，是安全的：两边数据互不可见。
