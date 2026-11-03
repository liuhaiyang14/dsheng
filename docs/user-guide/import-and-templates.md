# 数据导入、字段映射与模板

## 扫描：工具能读什么

```bash
dsheng scan --workspace <id> --dir <path> --json
```

- 支持 `.xlsx` / `.xls` / `.csv`（UTF-8 或带 BOM；乱码 CSV 需按提示转 GB18030，工具
  不猜编码）。其他扩展名（如 `.pptx`）只登记为 `pending_inputs` 提示，不解析、不外发。
- 扫描识别每个文件的工作表角色：**核算表 / 推移表 / 差异分析 / 任务单检视 /
  任务单计划 / 任务依据 / 图表**，依赖确定的结构锚点（工作表名 + 表头别名），不做
  模糊猜测。识别不了的工作表按通用表处理，进入手动映射。
- 输出 `scanSummaryView`：文件列表（`file_label`、`row_count`、`sheet_count`、
  `mapping_status: mapped|pending_review`）、覆盖区间与待处理项。扫描是只读本地操作。

## 字段映射

识别基于**精确别名**（清理首尾空白/全角转半角后逐字匹配），别名表节选：

| 规范字段 | 接受的表头（节选） |
| --- | --- |
| `metric_label`（科目） | 科目、项目、指标、MP名称、MP指标 |
| `planned`（预定） | 月度预定、本月预定、本期预定、预定、计划值、目标值、本期目标、目标 |
| `actual`（实绩） | 月度实绩、本月实绩、本期实绩、实绩、实际值、本期实际 |
| `annual_month_plan` | 年度计划·当月目标、年度当月目标（年度总计划**不会**自动除以 12） |
| `unit` | 单位、计量单位 |
| `period` | 期间、日期、业务日期、月份、季度 |
| `owner` / `due_date` | 责任人、负责人 / 时间节点、截止日期 |
| `category`（任务类别） | 方向、类别、经营方向（exact 映射到 revenue/cost/efficiency/other） |

- 不在别名表中的列进入 `unmapped`，**不丢列**，由你在映射里手动指定。
- 映射是**版本化草稿**：`mapping set` 后需要重新 `analyze` 才进入新的分析 revision；
  直接查旧结果仍是旧映射。
- 表头变化会使旧映射失效，需重新确认。

```bash
dsheng mapping show --workspace <id> --run <id> --json
dsheng mapping set --workspace <id> --run <id> --input-json mapping.json --json
```

## 手工补录（manual-input）

表里缺的数（如临时的目标值）可按期间+指标手工录入，与文件来源走同一套候选/冲突规则，
不会自动覆盖文件值：

```bash
dsheng manual-input list --workspace <id> --period-key <key> --json
dsheng manual-input set --workspace <id> --period-key <key> --input-json input.json --json
```

- 必填 `source_note`（说明数值依据）；数值保持十进制字符串精度，不经浮点转换。
- 修改会递增 `input_revision`；旧快照可追溯。手工补录**不会触发自动重算**，需重新
  `analyze` 生效。

## 来源冲突（source-conflict)

同一指标/角色/期间出现**多个不同值**（两个文件、或文件与手工值）时产生冲突：
该值不参与计算，直到你显式选择一个来源并写明理由。原值永不删除，落选证据保留。

```bash
dsheng source-conflict list --workspace <id> --run <id> --json
dsheng source-conflict resolve --workspace <id> --run <id> --input-json resolve.json --json
```

`resolve.json` 需包含选中的来源证据 ID、理由与 `expected_revision`。解决记录保存观察、
原因、操作者与时间；改选会创建新 revision。**未解决冲突的值完全不得参与目标/指标
计算**（包括收入下限候选）。

## 空白模板包

```bash
dsheng template download --package import --out import-template-v1.zip
```

ZIP 含三份空白 XLSX（`核算表模板.xlsx`、`推移表模板.xlsx`、`任务单模板.xlsx`）与
`字段说明.md`；无任何示例金额、客户或人员数据。模板第 1–4 行是元数据键位（模板
版本、企业/币种/税口径、来源期间、时区/周始/财年/覆盖度），第 6 行列名，第 7 行起
填数。元数据留空时导入预览会要求补齐。

导入建议流程：**下载模板 → 按列名填写 → `scan` 检查 mapping_status → 处理
pending/冲突 → `analyze`**。如果你的表已是既有布局，也可直接扫描：工具按结构锚点
识别，未识别部分进入手动映射，不需要强行套模板。

## 数值与文本约定（导入侧）

- 空单元格是缺失，不补零；`NA`/`N/A`/`-` 保留原文并记缺失；「待补/待确认」进入
  待补状态。0 与负数原样保留。
- 金额单位元/千元/万元按 1/1000/10000 归一；币种与含税口径必须来自明确来源，
  不自动换汇/折税。
- 百分比：只有表头或格式明确为 `%` 才做 0.5→50% 的解释；普通数字 0.5 就是 0.5。
- 文件里以 `=`、`+`、`-`、`@` 开头的文本在导出时按文本处理（防公式注入，见
  [history-and-export.md](history-and-export.md)）。
