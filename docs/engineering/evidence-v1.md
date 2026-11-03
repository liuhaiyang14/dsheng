# 验收用例证据归档 v1（71 用例全量，2026-11-03）

层级标注：PT=纯合同/纯逻辑（自动测试）、LV=真实模型层（versions.json 冻结证据）、
E2E=产品端到端（命令层/同库往返/真实产物）。手工跨平台步骤以可复现清单＋脚本归档
（主计划 §12.2 基线 #8 允许）；目标机未执行项如实标注"待目标机"，不冒充已执行。

测试套件规模（终态）：core+cli 306、src-tauri 20、src-ui 57（含首启引导套件）；preflight PASS；
validate_contracts PASS；manifest-v0.1.0 四类产物 5 项＋安装辅助 2 项（SHA-256 在档）。

| 用例 | 层级 | 状态 | 证据 |
| --- | --- | --- | --- |
| 1 | E2E(导入) | 已归档 | wp_data_import.rs（合成 xlsx/csv 全结构断言）；真实两表一单.xlsx 本机核对为主 Agent 手工项（不入库） |
| 2 | PT | 已归档 | wp_data_dc_cases.rs D01–D28 计算类算例（引擎与独立期望一致） |
| 3 | PT | 已归档 | wp_data_import.rs 无日期汇总→未提供目标，不合成差异 |
| 4 | E2E | 已归档 | 用例 40 双向（wp_tauri_roundtrip.rs）＋CLI/UI 共用 core（复审轮1/2 确认无重复实现） |
| 5 | PT | 已归档 | wp_cli_s3_gate.rs preview 字段/confirmation_id；任务无诊断框架参数（clap 面） |
| 6 | PT | 已归档 | wp_provider_confirm.rs（TTL/篡改 STALE/重复消费/原子）＋wp_cli_s3_gate.rs 拒绝路径 |
| 7 | PT | 已归档 | wp_provider_assemble.rs（1MiB/8000 scalars/预算）＋wp_provider_http.rs（2MiB 上限） |
| 8 | E2E | 已归档 | wp_data_task_cases.rs TS-01…（任务隔离）＋src-ui task.test.tsx |
| 9 | E2E | 已归档 | wp_data_migration.rs/SQLite 持久化；TS-39 重启可读 |
| 10 | PT | 已归档 | wp_provider_schema.rs 三框架 schema 封闭字段＋SMART（S/M/A/R/T）校验 |
| 11 | PT | 已归档 | wp_cli_s4_export.rs 导出无 Key＋=+-@ 防护断言 |
| 12 | E2E(平台) | 清单就绪 | macOS arm64 本机装包/启动实测（UNSIGNED.md）；Windows 待目标机（RELEASE-CHECKLIST.md） |
| 13 | PT | 已归档 | wp_cli_s4_export.rs 任务三 sheet 结构/字段/技术列 |
| 14 | E2E(导入) | 已归档 | wp_data_import.rs 适配器锚点（核算/推移/任务/通用＋未知列保留） |
| 15 | PT | 已归档 | wp_data_dc_cases.rs P01–P28 期间算例（月→季/文件级不拆） |
| 16 | PT | 已归档 | wp_data_task_cases.rs 全部状态/舍入/重开/阻塞＋CLI/UI 同一状态机（复审确认） |
| 17 | PT | 已归档 | wp_cli_s4_export.rs bundle 内容边界断言 |
| 18 | E2E(平台) | 清单就绪 | macos.sh 产物断言（CLI+migration+资源齐全）；解压启动清单待目标机 |
| 19 | E2E | 已归档 | wp_data_migration.rs 同期重做追加 revision/run 保留＋TS-24/26 |
| 20 | E2E | 已归档 | history show/快照只读（wp_cli_s2_data.rs；源离线可查=快照不依赖文件） |
| 21 | PT | 已归档 | wp_cli_s4_export.rs 模板包三空白 XLSX＋字段说明、无样例数据 |
| 22 | PT | 已归档 | wp_data_dc_cases.rs 比较算例＋reason 优先级（COMPARISON_REASON_PRIORITY 断言） |
| 23 | E2E(UI) | 已归档 | src-ui analysis.test.tsx（缺桶 null 断线、不可比原因）；PNG 导出为 CLI 命令（chart_png） |
| 24 | E2E(UI) | 已归档 | src-ui task.test.tsx 时间线排序/缺日期"未设置" |
| 25 | PT | 已归档 | wp_cli_s2_data.rs 双工作区隔离；月转日策略版本并存（wp_data_migration.rs） |
| 26 | PT | 已归档 | wp_data_dc_cases.rs 策略边界算例＋run 快照含策略版本 |
| 27 | PT | 已归档 | wp_data_migration.rs 回算新 revision/旧结果保留 |
| 28 | PT | 已归档 | WORKSPACE_MISMATCH 全路径（wp_cli_s2_data.rs；复审轮2 第8项） |
| 29 | PT | 已归档 | 生效日区间版本化（wp_data_migration.rs；拒绝重叠） |
| 30 | PT | 已归档 | preflight.sh（正负样例验证：真实文件/凭据样式/绝对路径全 CATCH）PASS |
| 31 | M | 流程在档 | 主 Agent 受控流程（本仓无真实业务文件——.gitignore＋preflight 双保险） |
| 32 | PT | 已归档 | SC-01…34（wp_data_task_cases.rs schema 组）＋TS 语义组＋UI 往返 |
| 33 | PT | 已归档 | reconcile 全流程（apply_token/幂等/墓碑/白名单）wp_data_task_cases.rs＋wp_cli_s2_data.rs |
| 34 | E2E(UI) | 已归档 | src-ui shell.test.tsx 空档案切换＋api.test.ts 隔离 |
| 35 | E2E(平台) | 清单就绪 | manifest-v0.1.0（macOS arm64 present；x64/Windows 待目标机，coverage 如实 missing） |
| 36 | PT | 已归档 | verify_skill.sh 20 步 57 断言全绿（全 dry-run，0 attempt） |
| 37 | E2E(平台) | 清单就绪 | cli_dist.sh 产物＋mac 安装演练通过；Windows .ps1 模板就绪待目标机 |
| 38 | M | 清单就绪 | docs/user-guide/checklist.md 八组走查＋100 条命令一致性检查通过 |
| 39 | PT | 已归档 | version_manifest.py --verify＋manifest-v0.1.0（5 产物 SHA-256） |
| 40 | E2E | 已归档 | wp_tauri_roundtrip.rs 双向＋三向冲突透传＋幂等重放（6 用例） |
| 41 | PT | 已归档 | 生成面枚举=4 操作（clap/gate）；本地命令 0 attempt（verify_skill.sh 总断言） |
| 42 | PT+LV | 已归档 | versions.json 冻结＋contracts::prompts 哈希守卫（frozen_resources_match_hashes） |
| 43 | PT | 已归档 | wp_provider_assemble.rs 组装含选定数据/预算；CLI 版 wp_cli_s3_gate.rs preview |
| 44 | LV | 已归档 | 完整复测批三框架逐层链/七维覆盖（versions.json；历史事实保留） |
| 45 | LV | 已归档 | LV-05 级证据（补充新事实才续问；来源 hash 锁定）＋工程侧续问门禁 PT |
| 46 | PT | 已归档 | 报告独立存取/导出（wp_cli_s2_data.rs diagnosis list/show；export md/json） |
| 47 | E2E(UI) | 已归档 | src-ui gates.test.tsx 诊断任何事件不触发任务生成 |
| 48 | PT | 已归档 | STALE_DIAGNOSIS_REFERENCE（wp_cli_s3_gate.rs/generation 校验） |
| 49 | PT+LV | 已归档 | LV05/06/07 真实通过＋本地复核（wp_provider_task_candidate.rs 下限/冲突/explicit-only） |
| 50 | LV | 已归档 | versions.json dispatch_accounting 44 次确认、无自动重试 |
| 51 | PT | 已归档 | wp_provider_assemble.rs 四组合隔离（未选框架/另一工作流不进请求） |
| 52 | PT | 已归档 | wp_provider_schema.rs＋wp_provider_task_candidate.rs（引用域/SMART/类别） |
| 53 | PT | 已归档 | wp_provider_context.rs（重发锁定/append-restate/孤立答复/round 规则） |
| 54 | LV | 已归档 | 主计划 §16＋versions.json＋VALIDATION.md 原样保留（复审逐字节校验） |
| 55 | PT | 已归档 | --problem 必填/focus 归属/证据关联（wp_cli_s3_gate.rs＋tauri 拒绝路径） |
| 56 | PT | 已归档 | deadline 引擎（wp_provider_http.rs 用例57 时钟注入）＋UI 呈现（store） |
| 57 | PT | 已归档 | 同上 deadline 边界值/零星事件不重置/一次 timed_out |
| 58 | PT | 已归档 | 错误分类 22+ 状态码＋reqwest 错误映射（wp_provider_http.rs） |
| 59 | PT | 已归档 | 重试新确认/旧终态不改（wp_provider_confirm.rs can_retry） |
| 60 | E2E | 已归档 | 迟到门禁（wp_corefix_cli_attempt_commit.rs：不覆盖旧结果/终态锁定） |
| 61 | PT | 已归档 | 单活动 attempt/重复消费返回已有（wp_provider_confirm.rs） |
| 62 | LV | 待授权 | 真实请求可观察证据须用户新授权批次（当前 0 授权剩余；不冒充） |
| 63 | PT | 已归档 | wp_provider_context.rs＋wp_provider_schema.rs 输入校验 |
| 64 | PT | 已归档 | warning 不可变 requirement/解除规则/复发新 ID（schema3＋context） |
| 65 | PT | 已归档 | DC 56 全过（wp_data_dc_cases.rs assert 56 条） |
| 66 | PT | 已归档 | SC 34＋TS 50 全过（wp_data_task_cases.rs 断言计数锁定） |
| 67 | PT | 已归档 | PC 路由/确认/容量（wp_cli_s3_gate.rs＋wp_provider_confirm.rs） |
| 68 | PT | 已归档 | 输入投影跨字段（wp_provider_task_candidate.rs INPUT_CONTRACT_INVALID 族） |
| 69 | LV | 已归档 | 2026-11-02 完整复测批受影响矩阵全部通过＋用户冻结 |
| 70 | E2E(平台) | 部分归档 | identifier/路径/备份/恢复/迁移 PT 全过；macOS arm64 产物实测；WebView2/目标机待 |
| 71 | PT | 已归档 | 轮 1/2/3 独立复审记录（本文件＋acceptance-map.md） |

未决项（如实声明，不阻塞归档）：
- 用例 12/18/35/37/70 的 Windows 与 macOS x64 部分：清单/脚本/CI 就绪，
  待目标机执行（coverage manifest 如实记 missing）。
- 用例 62：真实 Provider E2E 证据需用户逐批授权（合同要求；当前批次授权余 0）。
- 用例 31：示例工作簿仅主 Agent 本机受控使用（流程在档）。
