# 工作包任务书：CLI 与导出（启动前按当时 core API 补接口细节）

启动条件：集成轮 1 完成（数据/Provider/UI 已合并、核心服务接口稳定）。

## 所有权
- `crates/dsheng-cli/**`、`src-tauri/src/export/**`、`templates/blank/**`、本模块测试。
- 不负责：核心业务逻辑重复实现、修改 Provider 确认规则、提示词、UI。

## 交付范围（对应用例 4-7、11、16-17、21、28、30-33、67 的 CLI 部分）
1. 53 条命令全量实现（contracts/v1/cli-surface.json 逐条：workspace/provider/period/
   scan/mapping/manual-input/metric/source-conflict/analyze/diagnosis/task/generation/
   history/export/template/version/help）；`--json` stdout 单 envelope、进度走 stderr、
   退出码按错误目录；未知 flag/未知输入字段拒绝。
2. 双门禁：`--dry-run` → confirmation_id（10 分钟单次）；发送必须
   `--confirmation-id <id> --confirm-send` 同时具备；Key 仅 `--api-key-stdin`，
   与 `--input-json -` 互斥。
3. 导出：业务包 ZIP（三工作簿+结构化 JSON）、单工作簿（核算/推移/任务）、
   会议版 10 页 md/json、图表 png/csv/xlsx；模板包 import-template-v1.zip
   （三空白 XLSX+字段说明，布局按 data-period-import §5 逐格）。
   公式注入防护（=+-@ 开头写文本）；API Key 永不进导出。
4. XLSX 写入：XLSX 用 rust_xlsxwriter（或合同允许的等价库），>15 位有效数写
   decimal 文本并标 number_storage=decimal_text；任务 sheet 名固定
   `任务单检视`/`任务单计划`/`任务依据`，完整期间放第 3 行；task_id/step_id 技术列。
5. 依赖注入测试确认门禁（未确认不调用 client 接口）；合成夹具往返
   （模板→填合成值→导入→导出→核对）。

## 测试命令
cargo fmt/clippy 0 警告；cargo test --workspace；CLI 冒烟脚本
（合成库、合成目录，断言 envelope/退出码）。
