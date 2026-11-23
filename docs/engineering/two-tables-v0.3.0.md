# v0.3.0 两表输出契约与发布准备

## 目的

v0.3.0 将两表导出从“能写出一份工作簿”提升为可回归的固定输出合同。实现以用户提供的两个 XLSX 模板为版式参考，但模板中的业务数据、客户、人员、文件名和工作表内容不进入代码。企业差异停留在 `source_profile`、规范化观察和映射 revision；所有 profile 最终投影到同一个 `two_tables_v1`。

机器结构合同见 [`contracts/v1/two-tables.schema.json`](../../contracts/v1/two-tables.schema.json)。它描述逻辑表格与排版约束，适配器和导出器都不能通过 profile 改写列顺序。

## 固定输出

- `核算表模板.xlsx` 的结果工作表名为 `核算表`。12 列顺序固定为：序号、科目、计算公式 / 取数来源、本月MP、年度计划·当月目标、月度预定、销售额占比(预定)、月度实绩、销售额占比(实绩)、预实差异额、实绩达成预定比、实绩达成年度计划比。
- `推移表模板.xlsx` 的结果工作表名为 `推移表`。14 列顺序固定为：月份、总销售额、采购成本、毛利额、毛利率、经营费用、人工费、结算收益、利润率、单位时间附加值、费用构成比、预定实绩差异额、实绩达成预定比、实绩达成年度计划比。月份按行从旧到新排列在第 5–20 行，累计行固定为第 21 行，标签按实际月份范围生成并以“累计”结尾；不转置为月份列，不插值，不把缺失当零。
- 图表页使用工作表 `图表`，标题和副标题合并，并插入 4 张原生折线图（总销售额、结算收益、利润率、单位时间附加值）。图表只引用同一 analysis revision 的真实月份数值，不从图片或企业模板反推数据。
- 两表和图表的标题区域按模板合并：核算表 `A1:L1/A2:L2`、推移表 `A1:N1/A2:N2`、图表页 `A1:H1/A2:H2`；核算表冻结 `A5`，推移表冻结 `B5`，表头和月份行滚动时保持可读。逻辑合同将标题合并、冻结窗格、表头行和数据起始行作为可检查约束，运行元数据集中保留在隐藏 `_metadata` sheet。
- 工作簿元数据保留 workspace/run/revision/as-of、期间、周期策略、指标语义哈希、source manifest、比较基期和覆盖状态；不写本机绝对路径、Provider Key 或业务样例数据。

当前工程布局基线是第 1 行标题、第 2 行说明、第 3 行留白、第 4 行表头、第 5 行开始数据；运行元数据位于隐藏 `_metadata` sheet。`two-tables.schema.json` 同时约束 `title_merged=true`、核算表冻结 `A5`、推移表冻结 `B5` 和推移表累计行 21；若模板排版需要改变坐标，应先更新合同、模板说明和回归夹具，不在单个 profile 分支中偏移。

## 数据和来源边界

两表生成链路固定为：

```text
source files
  → selected source_profile + version/hash
  → normalized observations/evidence
  → workspace metric definitions + confirmed formulas
  → analysis revision
  → two_tables_v1 exporter
```

`source_profile` 只负责文件/工作表角色、别名、单位/税口径、日期、主明细、去重和来源证据。它不能提供可执行代码、任意 Excel 公式或直接写单元格的逻辑，也不能改变两表列结构。生成的 `核算表`、`推移表`、`差异分析` 和 `图表` 工作表不是 source 输入；扫描到这些结果必须返回 `generated_output_input_not_allowed`。

数值在内部和机器输出保持规范 Decimal 字符串；可视单元格才按金额两位、百分比一位等单位规则展示。`pending`、冲突、缺目标、缺覆盖或不可比结果必须保留空值与 `pending_reasons`/证据引用，不能把未知值写成 0，也不能用当前 profile 猜旧 revision 的来源。

## v0.3.0 验证和发布边界

版本同步检查以下位置必须都是 `0.3.0`：根 workspace、`src-tauri` crate 与 Tauri config、UI package/lock，以及 `App.tsx` 页脚；两个 Cargo lock 只更新本地 `dsheng-*` package 版本，依赖版本不因升版漂移。

建议在实现合并后运行：

```bash
python3 -m json.tool contracts/v1/two-tables.schema.json >/dev/null
python3 tools/validate_contracts.py
cargo check --workspace --offline
cargo check --manifest-path src-tauri/Cargo.toml --offline
npm --prefix src-ui run build
```

导出实现需要补齐两份模板的结构回归：核算表核对 12 列、合并标题和 `A5` 冻结；推移表核对 14 列、月份行 5–20、累计行 21、`B5` 冻结、缺失状态与图表引用；图表页核对 4 张原生折线图。至少两个脱敏 source profile 在相同规范化观察上应逐列生成一致的两表结果。打包前新建 `target/release-dist/v0.3.0/`，保留 `v0.2.1` 目录和清单；v0.3.0 的 manifest 只能由实际打包脚本生成，未构建的平台如实记为 missing。

