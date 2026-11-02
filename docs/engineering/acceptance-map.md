# 验收用例 → 工作包归属映射（Phase 0 基线）

依据主计划 §15（用例 1–71）与 §12.3（工作包所有权）。自动化级别：

- **A（自动）**：CI 可重复执行（cargo test / vitest / 静态检查）。
- **S（半自动）**：本机脚本 + 人工触发构建/安装步骤。
- **M（手工）**：可复现手工清单（跨平台安装、unsigned 提示等）。

责任方缩写：MAIN=主 Agent；DATA=数据/SQLite 工作包；PROV=Provider/提示词工作包；
UI=UI 工作包；CLI=CLI/导出工作包；DOC=Skill 与手册工作包；REL=发布工作包。
证据层级标注：纯合同（PT）/ 真实模型（LV）/ 产品 E2E（E2E），互不可替代。

| 用例 | 责任 | 级别 | 证据/实现位置 |
| --- | --- | --- | --- |
| 1 导入两表一单.xlsx 真实覆盖期 | DATA+MAIN | M | 本机受控核对；合成断言入 DATA 测试 |
| 2 公式缓存空时指标与独立计算一致 | DATA | A | DC 算例 D01–D28 执行 |
| 3 无日期汇总文件不生成合成差异 | DATA | A | import 测试 |
| 4 CLI/UI 指标 JSON 一致 | CLI+UI+MAIN | A | 集成测试（同一 core 结果断言） |
| 5 dry-run 预览字段与 confirmation_id | PROV+CLI | A | 纯逻辑（settings confirmationPreview） |
| 6 确认缺失/过期/失效不发请求 | PROV+CLI | A | 纯逻辑依赖注入 |
| 7 1MiB/2MiB/8000 scalars/预算超限拒绝 | PROV | A | 纯逻辑（contracts::provider 常量） |
| 8 任务间状态互不影响 | UI+DATA | A | 状态机测试 TS 用例 |
| 9 重启后历史/任务/事件可读 | DATA | A | SQLite 持久化测试 |
| 10 三框架报告差异 + 任务 SMART | PROV+UI | A+M | schema 纯逻辑 + UI 走查 |
| 11 导出无 Key、公式注入防护 | CLI | A | 导出测试 |
| 12 两平台包启动 + unsigned 说明 | REL | M | 干净环境清单 |
| 13 三任务工作表结构与字段 | CLI+DATA | A | 模板/导出往返测试 |
| 14 真实模板适配器锚点识别 | DATA | M+A | 本机核对 + 合成夹具 |
| 15 月份键→月/季推移；文件级不拆趋势 | DATA | A | P 算例 |
| 16 步骤/进度/舍入按 TS 状态表 | DATA+CLI | A | TS-01…TS-50 |
| 17 导出 ZIP 内容边界 | CLI | A | 打包清单断言 |
| 18 便携 ZIP 含程序/CLI/migration | REL | M+S | 包检查测试 |
| 19 重做仅追加 revision、保留 run | DATA | A | run/revision 测试 |
| 20 历史期间查询/源离线可查 | DATA | A | snapshot 测试 |
| 21 模板包三类工作簿空白 | CLI | A | 模板生成测试 |
| 22 环比/同比与不可比原因 | DATA | A | P 算例 + reason 优先级 |
| 23 图表筛选/缩放/证据/PNG | UI | A+M | vitest + 手册走查 |
| 24 时间线按日期筛选、缺日期不排序 | UI+DATA | A | timeline 排序测试 |
| 25 双工作区隔离；月转日历史保留 | DATA | A | workspace 隔离测试 |
| 26 周起始/财年/时区影响边界 | DATA | A | 策略版本测试 |
| 27 策略变更不跨口径；回算新 revision | DATA | A | 回算测试 |
| 28 跨工作区完全隔离；CLI 拒绝错误 workspace | DATA+CLI | A | WORKSPACE_MISMATCH 测试 |
| 29 生效日切换后新旧策略并存 | DATA | A | policy 区间测试 |
| 30 预发布检查拒绝真实文件/密钥 | REL+MAIN | S | secrets scan 脚本 |
| 31 示例工作簿仅本机验收 | MAIN | M | 受控清单 |
| 32 task-edit 合同字段/校验/往返 | DATA+UI | A | SC-01…SC-34 + UI |
| 33 候选身份/显式 reconcile | DATA+CLI | A | TS 用例 + reconcile 测试 |
| 34 空白档案切换空状态 | UI | A+M | vitest |
| 35 三架构四产物清单 | REL | M | 发布清单 |
| 36 Skill 可验证流程 | DOC | S | 本地逻辑演练脚本 |
| 37 独立 CLI 安装可运行 | REL | M | 干净环境 |
| 38 中文手册一致性 | DOC | M | 走查清单 |
| 39 Release 清单 SHA-256 | REL | S | 清单生成脚本 |
| 40 桌面↔CLI 共库往返 | MAIN+DATA+UI+CLI | A+E2E | 集成测试（共享数据库路径/并发） |
| 41 仅四类动作可请求 Provider | PROV | A | 调用面枚举测试 |
| 42 版本冻结状态展示/追溯 | PROV+CLI | A | contracts::prompts 断言 |
| 43 payload 含选定数据/预览显示范围 | PROV | A | 组装纯逻辑 |
| 44 三诊断模式结构（真实 Provider） | MAIN | LV | 已由完整复测批达成（versions.json） |
| 45 信息不足 5WHY + 续问门禁 | MAIN | LV | 主批/复测批证据 |
| 46 报告持久化/独立导出 | DATA+CLI | A | report 存取测试 |
| 47 无报告任务生成 + 无自动触发 | PROV+UI | A | UI 事件测试 + payload 纯逻辑 |
| 48 可选诊断引用有效性 | PROV | A | STALE_DIAGNOSIS_REFERENCE 纯逻辑 |
| 49 目标约束方向（下限/explicit-only） | PROV+DATA | A | LV05/LV06/LV07 场景已真实验证；本地复核纯逻辑 |
| 50 真实派发计数/无自动重试 | MAIN | LV | versions.json dispatch_accounting |
| 51 四种组合提示词装配隔离 | PROV | A | assembly_manifest 测试 |
| 52 输出 schema 校验（SMART/框架字段） | PROV | A | schema 纯逻辑 |
| 53 多轮保存/重发锁定/不变量 | PROV+DATA | A | CONTEXT_INVALID 纯逻辑 |
| 54 历史记录原样保留 | MAIN | PT | versions.json/VALIDATION.md 冻结 |
| 55 诊断创建输入校验（--problem 等） | PROV+UI | A | 发送前失败纯逻辑 |
| 56 状态机渲染（阶段/耗时/取消） | PROV+UI+CLI | A | 纯逻辑 + 可控时钟 |
| 57 deadline/配置校验（15/180/240） | PROV | A | 边界值纯逻辑 |
| 58 错误分类/脱敏提示 | PROV+UI+CLI | A | 错误枚举纯函数 |
| 59 手动重试新确认 | PROV | A | attempt 序列纯逻辑 |
| 60 迟到事件不提交/终态不可回退 | DATA+MAIN | A | 事务门禁测试 |
| 61 并发互斥/单活动 attempt | DATA+PROV | A | 互斥测试 |
| 62 真实请求可观察证据 | MAIN | LV | 仅主 Agent 授权批次 |
| 63 diagnosis-input/context 纯逻辑 | PROV | A | 输入 schema 校验 |
| 64 warning 生命周期合同 | PROV+DATA | A | schema3 纯逻辑 |
| 65 全部 DC 算例执行 | DATA | A | data-period-cases.json 驱动 |
| 66 全部 TS 用例执行 | DATA | A | task-state-cases.json 驱动 |
| 67 PC 路由/确认/容量合同 | PROV+CLI | A | PC-01…PC-08 对应纯逻辑 |
| 68 输入投影跨字段校验 | PROV | A | INPUT_CONTRACT_INVALID 纯逻辑 |
| 69 LV 矩阵真实复测 | MAIN | LV | 已达成并冻结（2026-11-02） |
| 70 identifier/备份/恢复/四产物 | REL+DATA | A+M | migration/backup 测试 + 平台清单 |
| 71 独立只读交叉复审 | MAIN | PT | 每轮集成后执行 |

> 基线 #8 冻结：本映射随合同演进由主 Agent 维护；新增用例必须先落归属再实现。

## 证据台账（随集成轮更新；主 Agent 维护）

状态枚举：`待工程` → `实现完成`（自动测试绿）→ `已归档`（证据链接落账）。
证据层级三选一标注：PT 纯合同 / LV 真实模型 / E2E 产品端到端，互不可替代。

| 用例 | 层级 | 状态 | 证据 |
| --- | --- | --- | --- |
| 42 | PT+LV | 已归档 | versions.json（2.3/2.0/schema2/3/input2 冻结）＋contracts::prompts 哈希守卫测试 |
| 50 | LV | 已归档 | versions.json dispatch_accounting（44 次确认派发，无自动重试） |
| 54 | PT | 已归档 | 主计划 §16、versions.json、VALIDATION.md 原样保留 |
| 69 | LV | 已归档 | 2026-11-02 完整复测批受影响矩阵全部通过（LV01–LV08），用户批准冻结 |
| 65 | PT | 实现完成 | 夹具接线（contract_fixtures.rs：DC 56 条解析）；算例执行待数据包合并 |
| 66 | PT | 实现完成 | 夹具接线（TS 34+50 条解析）；用例执行待数据包合并 |
| 71 | PT | 进行中 | 集成轮 1 将执行首轮独立只读复审 |
| 34 | E2E(UI) | 实现完成 | src-ui 空档案切换测试（shell.test.tsx）；E2E 待 Tauri 壳 |
| 8-10/23-26/32-34 UI 部分 | E2E(UI) | 实现完成 | src-ui 8 测试文件 54 用例（cf05892）；核心侧待数据包 |
| 合同扩展 | PT | 已归档 | 2026-11-02 用户批准：read-views.schema.json＋analysis show（9e405e6） |

集成待办（UI 字段对齐）：src-ui 的 `diff_value`→`delta`、mom/yoy 简化对象→完整 comparisonView、
taskCandidateView 增 `envelope_locally_reconstructed`——UI 包合并后由主 Agent 安排适配轮。

（未列出的用例均为 `待工程`；每轮合并后由主 Agent 把达成项从上表更新并链接到
具体测试名/提交哈希/复审报告。）
