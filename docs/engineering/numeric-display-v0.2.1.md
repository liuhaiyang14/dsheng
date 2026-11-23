# v0.2.1：数值显示与目标来源修复

本轮修复经营分析、两表与下载文件中的长小数、比例未转百分数、比较基准未说明来源问题。应用版本升级到 `0.2.1`，历史 `v0.2.0` 包保留，新增 `target/release-dist/v0.2.1` 三项测试包。安装由用户决定。

## 验收口径

- 金额显示两位，百分比显示一位，采用既有 half-even 规则；数量和工时最多六位。百分比数据仍按 ratio 保存，显示时转换为百分数。
- 显示格式不回写 SQLite、原始 Decimal、分析快照或机器 JSON/CSV。Excel 可视主表呈现约定精度，精确原值保留在元数据中。`two_tables_v1` 业务表列保持固定。
- `455022.79245283018867924528302` 应显示为 `455,022.79`；`690391.13274336283185840707965` 应显示为 `690,391.13`；`0.944857673584` 的比较变化率应显示为 `94.5%`。
- 本期明确目标、年度当月计划和去年同期参考下限分别标识。收入下限的既有计算规则不因本次显示修复改变；参考下限不冒充已确认目标。
- 旧 revision 的目标来源没有充分持久化证据时显示待确认，不仅因目标值恰等于同期值就推断来源。用户重新分析会生成带来源信息的新 revision，旧 revision 和指纹保持原样。
- 删除没有真实贡献计算支持的“异常信号／按差异贡献排序”说明及固定长度进度条，展示有来源的目标／参考差异。零差异为持平，费用类指标按其改善方向着色。

## 实现入口

- 前端：`src-ui/src/api/format.ts`、`pages/AnalysisScreen.tsx`、`pages/TablesScreen.tsx` 和图表展示；统一十进制字符串格式化，单独保留原精度工具。
- 后端：`crates/dsheng-cli/src/store/analysis.rs`，写入新分析目标来源，并为历史读视图投影本 revision 的单位信息。
- 输出：`src-tauri/src/export/`（CLI 复用），按单位呈现数值与精确元数据。
- 合同：`contracts/v1/read-views.schema.json`，以可选字段兼容旧结果。
- 交互原型：`prototype.html`；执行范围与验收纳入项目实施计划本轮记录。

## 验证

| 检查 | 命令／方式 | 状态 |
| --- | --- | --- |
| Rust 单元及集成 | `cargo test --workspace` | 496 通过、0 失败；1 项真实来源目录测试按既有 opt-in 规则跳过 |
| Tauri 测试 | `cargo test --manifest-path src-tauri/Cargo.toml` | 47 通过 |
| Tauri 编译 | `cargo check --manifest-path src-tauri/Cargo.toml` | 通过 |
| 前端测试及构建 | `cd src-ui && npm test && npm run build` | 27 文件、229 用例通过；构建通过 |
| JSON Schema / contracts | `python3 tools/validate_contracts.py` | PASS |
| 补丁检查 | `git diff --check` | 通过 |
| Excel 展示与原值 | `cargo test -p dsheng-cli --test wp_cli_s4_export`；解包核对数值、格式、原值元数据 | 9 项通过（已计入 Rust 总数） |
| 页面视觉 | 合成数据下核对短金额、百分比、目标来源及布局 | 1280 / 960 / 768 宽度无页面横向溢出；无页面异常 |
| 原型 | JSDOM 目标来源、无虚假排序、期间交互 | 通过 |

本轮验证使用合成数据和临时 SQLite，不运行已安装应用，不调用 Provider，不改用户原始文件或软件数据库。没有只属于旧语义的遗留失败。既有 jsdom canvas/act 提示和 ECharts 构建体积提示不影响测试通过。

回归中发现的两个问题均已修复：旧导出不得回退读取现行指标定义；未完成任务的 99.99% 进度不得舍入成 100%。任务进度因此保留原精度，不套用经营指标百分数的一位显示规则。

## 审计和交付

已完成无上下文 UI 与 Rust/core/export 独立审计。首轮 UI 审计发现一项 P1：任务自填单位被关联指标单位覆盖；修复为按任务自身单位展示，新增不同单位回归。随后新建无上下文 agent 复审当前工作区，结论为无可复现 P0/P1；导出复审 9 项测试通过。没有未处理的 P0/P1。

最终包只包含 DMG、便携 ZIP、CLI ZIP，校验值记录在 `docs/release/manifest-v0.2.1.json`，不将工程日志、截图或业务数据放进交付目录。三项 macOS arm64 产物已生成到 `target/release-dist/v0.2.1/`，版本清单核对和发布前扫描通过；没有安装或启动应用。
