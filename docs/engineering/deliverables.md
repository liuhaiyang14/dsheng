# 四类交付物清单与验收负责人（Phase 0 基线 #9）

依据主计划 §10.3、§11 与 provider-config-delivery.md §6。版本号统一取
`dsheng-core` crate 版本（`cargo workspace.package.version`）；发布时四类产物
必须在同一版本清单中关联版本号与 SHA-256。

| # | 交付物 | 内容清单 | 不包含 | 平台/架构 | 生产工作包 | 验收 |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 桌面安装包 | macOS `.app`→`.dmg`＋便携 `.zip`；Windows `.msi`＋便携 `.zip`（不交 `.exe` 安装器）。含桌面 UI、运行资源、同版本 `dsheng` CLI、SQLite migration、unsigned 说明 | 用户数据库、分析历史、Provider 配置、API Key、业务文件 | macOS 13+ arm64/x64；Windows 10 22H2+/11 x64 | REL | 用例 12/18/35（M） |
| 2 | Agent 接入 Skill | `skills/dsheng-agent/SKILL.md`＋CLI 命令/JSON Schema 参考＋验证脚本；可打包 ZIP | 可执行文件、凭据、真实业务样例、绕过确认门禁的逻辑 | 纯文本（依赖已安装 CLI） | DOC | 用例 36（S） |
| 3 | 独立 CLI 安装文件 | `dsheng` 二进制（macOS arm64/x64、Windows x64）＋`.sh`/`.ps1` 用户级安装辅助＋版本/校验清单 | 桌面 UI、用户数据库、Provider 配置、业务数据 | 同上三 target | REL | 用例 37（M） |
| 4 | 中文使用手册 | `docs/user-guide/*.md`＋分发 PDF：安装/unsigned、Provider 与明文 Key 风险、企业周期、导入模板、日周月季分析、三框架、任务编辑/进度、历史、导出、CLI/Skill、排障 | 真实业务数据、有效 Key、未实测步骤 | 平台无关 | DOC | 用例 38（M） |

共同约束：
- 应用 identifier 固定 `com.dsheng.desktop`；用户库路径见交付合同 §5；
  便携=免安装解压运行，仍用同一用户数据目录。
- 桌面包与独立 CLI 必须同一构建版本产出，不得分叉实现。
- Skill 依赖独立 CLI，不负责安装/更新桌面应用。
- 经营数据导出包（业务 ZIP）与软件交付物分开验收（用例 17）。
- 发布产物含业务 XLSX/PPTX、SQLite、Key 或本机绝对路径即拒绝（用例 30/39）。
- GitHub Release 与二进制发布须产品验收通过＋用户显式授权（主计划 §12.5）。
