# dsheng 使用手册（中文）

dsheng 是「两表一单」本地经营分析工具：把你现有的**原始业务 source 文件**放在本机
分析，由工具生成核算表、推移表和任务单；按需（可选）接入大模型 Provider 生成经营问题诊断与任务单建议。所有经营数据、
历史记录与 Provider 配置都保存在本机 SQLite 数据库中，工具不内置任何云端账号。

## 本手册的章节

| 章节 | 内容 | 适合谁 |
| --- | --- | --- |
| [install.md](install.md) | 安装、便携版、unsigned 首次启动、数据库位置、卸载 | 所有新用户 |
| [provider-setup.md](provider-setup.md) | Provider 与 API Key 配置、明文保存风险 | 需要诊断/任务生成功能的用户 |
| [workspace-period.md](workspace-period.md) | 企业档案、数据隔离、核算周期策略 | 所有用户 |
| [import-and-templates.md](import-and-templates.md) | 数据扫描、字段映射、手工补录、来源冲突、模板包 | 导入数据的用户 |
| [analysis.md](analysis.md) | 日/周/月/季分析、推移、同比环比与不可比原因、图表 | 所有用户 |
| [diagnosis.md](diagnosis.md) | 5W / 5WHY / 5W2H 三种诊断框架与多轮追问 | 做经营问题诊断的用户 |
| [tasks.md](tasks.md) | 任务单生成、候选合并、步骤、进度与状态规则 | 用任务单跟进的用户 |
| [history-and-export.md](history-and-export.md) | 历史版本、快照、删除边界、导出包 | 所有用户 |
| [cli-and-skill.md](cli-and-skill.md) | 命令行（CLI）与本地 Agent（Skill）用法 | 终端用户 / AI Agent 使用者 |
| [troubleshooting.md](troubleshooting.md) | 常见错误码处置、数据库备份恢复升级、WebView2 | 遇到问题时查阅 |
| [checklist.md](checklist.md) | 新用户从安装到导出的走查清单 | 首次使用按步核对 |

## 四类交付物与选择关系

dsheng 按同一版本、同一核心构建，分发为四类交付物：

| 交付物 | 形态 | 适合 | 不含 |
| --- | --- | --- | --- |
| 桌面应用安装包 | macOS `.dmg`（Apple Silicon/Intel）、Windows x64 `.msi`；另有各平台便携 ZIP | 日常使用图形界面的用户 | 用户数据库、经营历史、Provider 配置、API Key |
| 独立 CLI 安装文件 | `dsheng` 独立二进制 + 用户级安装辅助脚本（macOS `.sh` / Windows `.ps1`）+ 版本/校验清单 | 只用终端，或供本地 Agent 调用 | 桌面 UI、用户数据库、业务数据 |
| Agent 接入 Skill（`dsheng-agent`） | Skill ZIP（说明 + 命令/JSON 参考 + 本地验证脚本） | 让本机 AI Agent 按安全流程代为操作 | 可执行文件、Provider 凭据、真实业务样例 |
| 中文使用手册 | 本目录 Markdown（可转 PDF） | 所有用户 | 真实业务数据、有效 API Key |

选择建议：

- **只用图形界面** → 装「桌面应用安装包」即可；便携 ZIP 免安装解压运行，数据位置相同。
- **用终端或脚本** → 装「独立 CLI」；它与桌面应用共用同一个用户数据库（见
  [cli-and-skill.md](cli-and-skill.md)），两边看到同一份企业与历史。
- **让 AI Agent 代操作** → 先装「独立 CLI」，再给 Agent 安装「dsheng-agent Skill」；
  Skill 会约束 Agent 走「先预览、经你批准再发送」的门禁流程。
- 四类产物版本号与 SHA-256 在同一版本清单中对应；请使用同一版本的桌面端与 CLI，
  混用旧版客户端打开新版数据库会被拒绝（见 [troubleshooting.md](troubleshooting.md)）。

## 两条贯穿全局的规则

1. **生成类功能（诊断/任务生成）每次发送前都要单独确认**：工具先本地给出预览
   （发送给谁、发送哪些数据范围、多大、预计几次请求），你确认后才真正发起 1 次请求；
   不确认就不发送，没有自动重试。不使用这些功能时，工具完全本地运行。
2. **API Key 以明文保存在本机数据库中**（详见
   [provider-setup.md](provider-setup.md) 的风险说明）。能读取你用户数据目录的人就能读
   到 Key；请自行控制本机访问权限。
