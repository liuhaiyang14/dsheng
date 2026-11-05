# dsheng（两表一单）工程交接文档

> 生成时间：2026-11-05 14:10 · 生成者：主 Agent（工程实现全程）
> 状态：macOS 0.1.1 可用（哈希 f35ac121）；Windows CI 骨架已推待启用

---

## 一、产品是什么

本地经营分析桌面工具：读取企业 Excel 表格（核算表/推移表），自动识别指标并计算，支持 5WHY/5W/5W2H 诊断报告和任务单生成（调 GLM 大模型），数据全存本地 SQLite。

**技术栈**：Tauri 2 + React + TypeScript + Vite + Rust + SQLite
**仓库**：https://github.com/liuhaiyang14/dsheng（main 分支）

---

## 二、当前可用功能

| 功能 | 状态 | 说明 |
| --- | --- | --- |
| 拖拽/选目录导入 Excel | ✅ | 拖文件夹或点按钮选目录，本地解析 |
| 自动识别核算表/推移表 | ✅ | 核算表→accounting 适配器；推移表（转置布局）→trend 适配器 |
| 一键注册 16 个常用指标 | ✅ | 数据源页"待核对"卡上一键按钮，含真实科目全名别名 |
| 本地指标计算（13 项） | ✅ | 销售额/结算收益/毛利/人工费/付款/开票/收款/附加值等 |
| 经营分析屏（表格+图表） | ✅ | 指标卡/差异/图表/不可比说明（友好中文） |
| 诊断报告生成（5WHY/5W/5W2H） | ✅ | 需配置 Provider；流式实时进度可取消 |
| 任务单生成 | ✅ | 同上；候选生成后需 task reconcile 应用 |
| 模板下载 | ✅ | 三份空白工作簿＋字段说明 |
| 历史查询/版本快照 | ✅ | 不可变 revision，可追溯 |
| CLI（54 命令） | ✅ | 与桌面共用同一 Rust core 和 SQLite |
| Agent Skill | ✅ | skills/dsheng-agent/，本地验证脚本 57 断言全过 |
| 中文使用手册 | ✅ | docs/user-guide/ 11 篇＋PDF |

---

## 三、快速上手（新机器）

```bash
# 1. 安装 Rust（如无）
brew install rustup

# 2. 安装 Node.js ≥20（如无）
brew install node

# 3. 克隆并构建
git clone https://github.com/liuhaiyang14/dsheng.git
cd dsheng

# 4. 安装前端依赖
cd src-ui && npm install && cd ..

# 5. 构建桌面应用
bash scripts/package/macos.sh

# 6. 产物在 target/release-dist/dsheng_0.1.1_macos_arm64.dmg
```

### 日常开发命令

```bash
cargo test --workspace                          # Rust 全量测试（363 个）
cargo clippy --workspace --all-targets          # 静态检查（必须 0 警告）
cd src-ui && npx vitest run                     # 前端测试（64 个）
cd src-ui && npm run build                      # 前端构建
cargo test --manifest-path src-tauri/Cargo.toml # Tauri 壳层测试（14 个）
python3 tools/validate_contracts.py             # 合同校验（必须 PASS）
bash scripts/package/macos.sh                   # 打 macOS dmg
```

---

## 四、架构概览

```
src-ui/               React 前端（七屏界面）
  src/api/            合同化客户端层（TauriClient/FixtureClient）
  src/components/     组件（Onboarding/ImportDropzone/GenerationProgressPanel 等）
  src/pages/          页面（Source/Analysis/Diagnosis/Task/History/Provider/SystemSettings）
  src/state/store.tsx 全局状态（工作区切换/数据加载/诊断进度）

src-tauri/            Tauri 2 壳（桌面窗口 + IPC 命令）
  src/contracts/      33 条 Tauri command → dsheng-cli 服务层
  src/export/         导出库（XLSX/CSV/Markdown/JSON/模板/图表）

crates/dsheng-core/   共享 Rust 核心
  src/contracts/      冻结合同的 Rust 类型（Decimal/指标AST/证据投影/CLI错误码等）
  src/import/         导入器（xlsx/csv 读取/表头识别/锚点/去重冲突）
  src/analysis/       计算引擎（指标聚合/同比环比/DC 算例）
  src/repository/     SQLite 持久化（migration 0001-0003/任务生命周期/分析版本）
  src/provider/       Provider 客户端（HTTP/SSE 流式/确认门禁/提取层）
  src/generation/     生成层（提示词组装/输入投影/输出校验/上下文不变量）
  prompts/v1/         冻结提示词资源（2.3/2.0，SHA-256 哈希守卫）

crates/dsheng-cli/    独立 CLI（dsheng 二进制，54 命令）
contracts/v1/         机器可读合同（JSON Schema/指标目录/DC/TS 算例）
docs/specs/           四份实施规格（冻结）
docs/engineering/     工程文档（验收映射/交付清单/复审记录/派发台账）
skills/dsheng-agent/  Agent Skill（SKILL.md + 参考文档 + 验证脚本）
scripts/package/      打包脚本（macos.sh / cli_dist.sh / preflight.sh）
```

---

## 五、关键设计决策（已冻结，勿随意改）

| 决策 | 原因 | 出处 |
| --- | --- | --- |
| Decimal 28 位精度，除法 12 位 half-even | 金额计算不丢精度 | data-period-import.md §1.1 |
| 空单元格 ≠ 0（除非用户显式确认 empty_as_zero） | 防止把缺失当真实数据 | 同上 |
| C 方案适配层（严格提取＋裸任务包装） | glm 有 1/6 概率返回裸对象 | versions.json envelope_policy |
| 提示词 2.3/2.0 正文一个字不改 | 44 次真实验证通过 | prompts/v1/versions.json |
| 确认门禁：dry-run → confirmation_id → confirm-send | 用户逐次批准发送范围 | provider-config-delivery.md §2 |
| 每次生成最多 1 条 HTTP 请求，不自动重试 | 防止意外计费 | 同上 |
| 流式输出可选（stream=true），无字节上限 | 模型自行停止，本地完整接收 | 2026-11-05 用户决策 |
| 总超时 1500 秒（唯一失控保护） | 防真死循环 | 同上 |
| migration 0001-0003（0001 冻结不可改） | schema 演进以增量方式 | repository/migrations/ |
| 合同增量须用户批准后落地 | 冻结资源不自行修改 | 全程执行纪律 |

---

## 六、已验证的真实数据流程（2026-11-04/05）

用真实核算表＋推移表（~/Desktop/ds/）端到端跑通：

1. **扫描**：拖拽/选目录 → 两表均识别（accounting + trend）
2. **一键注册**：16 个指标自动注册（含真实科目别名如"结算收益（附加价值）"）
3. **分析**：13 项真实指标计算（销售额 198,077.14 / 结算收益 -6,732.05 等）
4. **诊断 5WHY**：glm-5.3 流式生成，171 秒完整收流，报告落库（3 层因果链带证据单元格引用）
5. **任务单**：glm-5.3 流式生成，94 秒完整收流，候选落库（SMART 校验/待确认字段正确留空）

---

## 七、Provider 配置（已验证可用）

在桌面 app 的 Provider 设置页填入：

| 字段 | 值 |
| --- | --- |
| Base URL | `https://api.z.ai/api/coding/paas/v4` |
| 模型 | `glm-5.3` |
| API Key | 从 `~/.zshrc` 取 `ZAI_API_KEY` |
| 流式输出 | **开启**（必须） |
| 上下文窗口 | 524288（512K） |
| 输出预留 | 8192 |
| 连接超时 | 15 秒 |
| 响应超时 | 300 秒 |
| 总超时 | 1500 秒 |

> 注意：Base URL 末尾不加 `/chat/completions`（自动拼接）。
> 首个保存的 Provider 自动设为默认。

---

## 八、已知限制与待做事项

### 产品功能
- [ ] 任务单 reconcile UI（update/replace 白名单选择界面；CLI 已有）
- [ ] 手工补录界面（CLI 已有 `manual-input set`）
- [ ] 来源冲突处理界面（CLI 已有 `source-conflict resolve`）
- [ ] 历史删除二次确认界面（CLI 已有 `history delete --dry-run`）
- [ ] 诊断 PDF 导出（当前仅 md/json；合同已含 pdf 枚举）
- [ ] 字段映射确认界面（当前待核对仅引导注册，不展示列级映射）
- [ ] Windows 构建启用（CI 骨架已推 .github/workflows/build-windows.yml）
- [ ] 正式应用图标设计（当前为代码生成版）

### 技术债
- [ ] 任务单生成载荷仍含全部 source_records（~290K tokens）；应改为仅发指标汇总
- [ ] Provider 连接测试（轻量 URL 可达检查，不动模型）
- [ ] 诊断续问 GUI 界面（CLI 已有 `diagnosis continue`）
- [ ] macOS x64 交叉构建验证

---

## 九、数据库位置

| 平台 | 路径 |
| --- | --- |
| macOS | `~/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3` |
| Windows | `%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3` |

- 桌面版、便携版、独立 CLI 共用同一个数据库
- Schema 版本：3（migration 0001-0003）
- API Key 明文存储（已确认的产品选择；手册已说明风险）

---

## 十、排障

### 飞行记录仪
所有 GUI 操作的 Rust 侧 scan 进出和 JS 侧异常都写入 `/tmp/dsheng-cmd.log`。
这是最重要的排障工具——"没反应"时先看这个文件。

### 常见问题

| 症状 | 原因 | 解决 |
| --- | --- | --- |
| 打开 dmg 报"已损坏" | 微信等网络渠道下载触发了 quarantine | 终端执行 `xattr -cr /Applications/dsheng.app` |
| "无法验证开发者" | 未签名（正常） | 系统设置→隐私与安全性→仍要打开 |
| 选目录后无反应 | 旧版 bug（已修）；新版有诊断行可看 | 确认版本号 v0.1.1；看诊断行 |
| 诊断报 Provider 未配置 | 每个 workspace 的 Provider 独立 | Provider 设置→保存→自动设默认 |
| 任务单"预算超限" | 上下文窗口太小（默认已是 512K） | Provider 设置→确认上下文 ≥524288 |
| 流式无进度 | Provider 未开流式 | Provider 设置→流式输出→开启 |
| 白屏/崩溃 | JS 异常（看 /tmp/dsheng-cmd.log [js] 行） | 升级最新版；报告 [js] 错误行 |
| 打包卡住 hdiutil busy | Finder 里挂载了旧 dmg | 推出所有 dsheng 挂载后重试 |

---

## 十一、验收证据

- **71 个用例**全部有归属证据：docs/engineering/evidence-v1.md
- **三轮独立复审** P0/P1=0：docs/engineering/acceptance-map.md
- **版本清单＋SHA-256**：docs/release/manifest-v0.1.1.json
- **真实模型派发台账**：docs/engineering/dispatch-ledger.md（9 次派发，批次 C 两发两中）
- **四类交付物清单**：docs/engineering/deliverables.md

---

## 十二、关键文件索引

| 文件 | 用途 |
| --- | --- |
| `contracts/v1/cli-surface.json` | 54 条 CLI 命令＋69 个错误码（唯一事实来源） |
| `contracts/v1/read-views.schema.json` | 读侧输出视图（UI/CLI 共用） |
| `contracts/v1/metric-catalog.json` | 26 个指标定义（含别名/公式/聚合方式） |
| `crates/dsheng-core/prompts/v1/versions.json` | 提示词版本/哈希/冻结状态 |
| `docs/engineering/dispatch-ledger.md` | 真实模型调用逐次记录 |
| `docs/engineering/review-checklist.md` | 独立复审清单（每轮集成后执行） |
| `scripts/package/preflight.sh` | 发布前检查（secrets/大文件/路径扫描） |
| `tools/validate_contracts.py` | 合同校验（改合同后必须 PASS） |
| `HANDOFF.md` | 本文档 |
