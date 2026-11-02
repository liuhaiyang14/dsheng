# 工作包任务书：Agent Skill 与中文手册

启动条件：CLI JSON 契约冻结（集成 CLI/导出包后）。

## 所有权
- `skills/dsheng-agent/**`、`docs/user-guide/**`、Skill/手册验证脚本。
- 不负责：修改 CLI 契约、真实业务数据、API Key、桌面安装包。

## 交付范围（用例 36、38）
1. Skill：`skills/dsheng-agent/SKILL.md`（定位 dsheng、显式 --workspace、先 dry-run、
   展示发送范围、获准后 --confirmation-id + --confirm-send、解析 JSON envelope、
   推进任务/步骤、导出）；附 CLI 命令参考与关键 JSON Schema 摘录；
   可打包 Skill ZIP。验证脚本用本地逻辑+合成数据演练全流程（不建假 Provider）。
2. 中文手册 `docs/user-guide/`：安装与 unsigned 首启（Gatekeeper/SmartScreen 正常
   流程）、Provider/API Key（明文存 SQLite 的访问风险必须醒目）、企业与周期、
   导入/模板、日周月季分析、5W/5WHY/5W2H（文案区分用途）、任务编辑/进度、
   历史/版本、导出、CLI/Skill、排障；命令与当前版本实测一致；四类产物选择关系。

## 验收
- Skill 命令演练脚本全部通过（本地逻辑层）。
- 手册走查清单：新用户按步骤完成本地分析与任务导出；链接/目录/平台差异核对。
