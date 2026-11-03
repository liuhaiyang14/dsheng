# 历史、版本与导出

## 版本快照：分析 revision 与任务快照

- 每次成功 `analyze` 都追加一份**不可变分析 revision**（含全部输入快照与指纹）；
  重做同期间保留 run_id、revision 前移，旧版永远可查。
- 任务侧另有一层**任务快照**（task snapshot）：每当影响该期间任务的事务发生，就追加
  一份规范化任务快照。历史查看/导出以 `run + analysis revision + task snapshot` 定位；
  选定快照后，后续任务推进不会改变你正在看的这份视图或已生成的导出。
- 当前任务视图（`task list/show`）与历史快照是两回事：历史不是「实时任务数据」。
  源文件暂时不在本机也能查看历史快照（续问/重新生成则仍需重新校验原来源）。

## 查看历史

```bash
dsheng history list --workspace <id> --json                                   # 期间清单
dsheng history revisions --workspace <id> --run <id> --json                    # 该期间全部 revision
dsheng history show --workspace <id> --run <id> --revision 1 --json           # 指定 revision 明细
dsheng history compare --workspace <id> --run <id> --type mom --json          # 环比（或 --type yoy）
```

比较读取冻结的基期 revision：日后基期被重做，不会改写旧分析的同比/环比数字。

## 删除：先预览、再确认，且有边界

删除是**物理删除**该期间及其附属数据（不是归档），分两步：

```bash
dsheng history delete --workspace <id> --run <id> --dry-run --json     # 返回 delete_preview_id 与删除清单
dsheng history delete --workspace <id> --run <id> --confirm-delete <delete_preview_id> --json
```

- 预览 10 分钟有效；期间依赖在预览后发生变化则预览失效（`DELETE_PREVIEW_STALE`），
  重新预览。
- 存在以下情况直接拒绝（`DELETE_DEPENDENCY_BLOCKED`）：有活动生成请求；有仍可推进的
  任务引用该期间；其他期间的比较/证据/报告/生成结果依赖本期间。工具不会为了方便删除
  而自动完成任务或替代任务。
- 删除范围包括该期间的指标/映射/来源元数据、全部 revision、报告、候选与快照；仅属
  该期间且已完成/已被替代且无外部引用的任务一并删除并在预览中逐项计数。
- **不删除**：磁盘原始 XLSX/CSV、Provider 配置、企业档案、其他期间与任务，以及你已
  导出到别处的文件。已有的本地备份与旧导出可能仍含这些记录——删除只针对当前数据库，
  不会追踪清除备份/导出。
- 没有「强制全库清空」参数；要清理整个企业请另行规划。

## 导出

四种导出（均为本地操作，0 次请求，可指定历史 revision）：

```bash
dsheng export --workspace <id> --run <id> --bundle business --out bundle.zip
dsheng export --workspace <id> --run <id> --workbook accounting --out accounting.xlsx
dsheng export --workspace <id> --run <id> --workbook trend --out trend.xlsx
dsheng export --workspace <id> --run <id> --workbook task --out tasks.xlsx
dsheng export --workspace <id> --run <id> --meeting --format md --out meeting.md
dsheng export --workspace <id> --run <id> --meeting --format json --out meeting.json
dsheng export --workspace <id> --run <id> --chart <id> --format png --out chart.png
dsheng export --workspace <id> --run <id> --chart <id> --format csv --out chart.csv
```

### 业务导出包（bundle）

ZIP 内容为三份独立 XLSX（核算表/推移表/任务单，按模板布局含元数据行与技术列
`task_id`/`step_id`）、结构化 JSON（分析/任务/会议投影）与 metadata（workspace/run/
revision、策略、指标 hash、来源清单、基期引用、过滤器）。**不含**应用二进制、API
Key、数据库文件或本机绝对路径。诊断报告单独经 `diagnosis export` 导出（当前仅
md/json，PDF 未实现）。

### 会议版（meeting）

固定 10 页 Markdown/JSON 投影：期间与完整性、目标、核算预实、差异与证据、推移、
同比环比与不可比原因、已确认追赶目标/行动、本期任务检视、下期任务计划、证据与风险。
缺「已确认追赶目标」时如实标注未提供，不会为补页调用模型。

### 公式注入防护

以 `=`、`+`、`-`、`@` 开头的**文本**单元格在 XLSX 导出中写成文本类型、CSV 中做安全
转义，避免被表格软件当作公式执行；真正的数值（包括负数）仍保留数值类型。超出 15 位
有效数字的精确值以十进制文本写出并标记 `decimal_text`。

## 导出的正确姿势

- 给管理层/会议：`--meeting --format md` 或 bundle 里的会议页。
- 给财务复核：`--workbook accounting`（含差异与证据定位）。
- 给任务跟进：`--workbook task`（多步骤任务按 task_id 分组展开行）。
- 存档某次复盘：先 `--revision <n>` 固定版本再导出，避免读到后续重算。
