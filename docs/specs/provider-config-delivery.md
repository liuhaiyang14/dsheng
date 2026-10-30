# Provider、配置入口与跨平台交付合同 v1

日期：2026-10-30。状态：规划规则定稿；产品实现、平台构建和真实联调未执行。关闭 PR-06/10/12/13 的规格缺口，不授予新的真实 Provider 请求额度。

本文件与 settings.schema.json、cli-surface.json 是配置/命令表面的合同；指标公式及周期算法见 data-period-import.md，任务/版本/删除/数据库迁移见 task-lifecycle.md，诊断上下文见独立上下文规范。本文不重定义这些领域规则。所有配置值是用户运行工具时的输入，不是让实现线程选择产品方法。

## 1. Provider 的身份、路由和保存

1. Provider 按 workspace 隔离，id 为应用分配 UUID。name 去首尾空格后在工作区内唯一。protocol 固定 chat_completions，stream=false。base_url 去末尾斜杠后追加 /chat/completions；若已输入完整 /chat/completions 路径，本地提示填写 Base URL，不重复追加或猜测改写路径。
2. UI 显式选择或 CLI --provider <id> 优先；未指定时只使用 workspace.active_provider_id。没有已确认默认时 PROVIDER_REQUIRED，不取第一条/最近一条、不自动 fallback。删除/不存在/跨企业的配置拒绝发送。
3. 新建配置不自动设默认。用户点击“设为默认”或执行 provider use 才更新 active_provider_id；删除默认配置后置 null，不自动换服务。历史保留当次 provider_id、name、base_url、请求/响应模型名及 revision 快照，不保留 Key。
4. providers.revision 从 1 起。name、路由、model、Key、预算或超时任一实际改变均递增；相同值不增。修改携带 expected_revision；竞争返回 REVISION_CONFLICT，不最后写入覆盖。
5. Key 按已确认产品选择，明文保存在 SQLite providers.api_key。UI 显示掩码；公开 JSON 只有 key_present 布尔值，不输出 Key、前后缀或 Key hash。Key 不进入 manifest、日志、导出或错误。CLI 仅 --api-key-stdin 接收，禁止命令参数或配置 JSON 内含 api_key。
6. provider set --input-json <file> --api-key-stdin 可同时更新公开配置与 Key；公开配置文件和 stdin 分离。--input-json - 与 --api-key-stdin 不可同时占用 stdin。key_action=replace 必须伴随非空秘密通道；keep 可创建暂缺 Key 的配置，发送预览返回 PROVIDER_KEY_MISSING。清空用显式 provider clear-key；不把掩码当作 Key 写回。配置保存/清空不调用模型。
7. URL 仅 HTTPS；HTTP 只允许字面 localhost、127.0.0.1、[::1]。禁止 URL userinfo/query/fragment、非 HTTP scheme；拒绝重定向，不携 Authorization 改向新 origin；不自动禁用 TLS 校验。loopback 能力用于用户实际配置的服务，不构成允许 Mock Provider/验收假 endpoint 的例外。

## 2. 预览确认、一条请求及版本绑定

diagnosis generate/continue、task generate、generation retry 复用同一流程。--dry-run 在本地返回预览。发送须同时有 --confirmation-id <id> 和 --confirm-send，UI 确认按钮提交同一 id。一次操作最多派发 1 条 HTTP 请求，首版不做模型分块/跨块汇总、后台摘要或自动重试。

预览创建本地 confirmation，10 分钟有效；记录摘要及 hash，不持久化完整 prompt/payload/Key。输出包含 Provider name/base_url/model/revision、企业/期间、来源/工作表/选定范围、字节及保守 token 估算、预算、预计最多一次请求和相关版本。

确认绑定 workflow、workspace_id、provider_id/provider_revision、run_id/analysis_revision_no、input_fingerprint、prompt/schema/context_policy 版本、报告 ID/revision（续问或可选引用时）、scope/source manifest hash、本轮输入 hash、预算/参数快照。Key 只通过 provider_revision 间接绑定，不保存其 hash。发送前重建并逐项检查；变动、过期或跨企业返回 CONFIRMATION_STALE，不静默取消引用或切 Provider 后发送。

消费 confirmation 与创建 attempt 原子完成；重复消费返回已建立 attempt ID/状态，不派发第二条。失败/取消/超时/中断的手动重试需新预览、新 confirmation；成功 attempt 不可重试。确认过期不影响已发送请求本身的超时预算。

发送时内存使用已确认 Provider 快照，不跟随 GUI/CLI 设置变化。派发后 Provider 配置变化不重发、不换路由；原请求可以完成并显示原 provider_revision。分析/报告版本变更则按 task-lifecycle 合同拒绝提交当前结果。更新 Key 不写入 attempt/历史快照。任何失败保护上一有效结果。

## 3. 请求和响应预算

| 项目 | 默认/硬上限 | 用户可配范围 | 行为 |
| --- | ---: | --- | --- |
| 最终 HTTP JSON body UTF-8 | 1,048,576 字节 | 固定硬上限 | 含 model/messages 和完整 schema；超限本地 REQUEST_TOO_LARGE，request_count=0 |
| 完整 HTTP 响应 body | 2,097,152 字节 | 固定硬上限 | 对解压后字节流累计；Content-Length 仅作提前拒绝；超限终止读取并 RESPONSE_TOO_LARGE |
| 单轮 user_message | 8,000 Unicode scalar values | 固定硬上限 | 不按 UTF-16 单元计；超限 USER_MESSAGE_TOO_LARGE，不截断 |
| context_window_tokens | 32,768 | 整数 8,192–1,048,576 | 本地预算，不是已识别模型能力承诺 |
| output_reserve_tokens | 8,192 | 整数 1,024–65,536，且 <= context/2 | 非法组合拒绝保存，不自动缩小 |
| connect_timeout_seconds | 15 秒 | 整数 1–60 | DNS/TCP/TLS共用 |
| response_timeout_seconds | 180 秒 | 整数 30–600 | 连接完成开始发请求至完整body；零星数据不重置 |
| total_timeout_seconds | 240 秒 | 整数 60–900，且 >= connect+response | preparing至终态，含解析/校验/保存 |

保守输入估算固定为最终 system 和 user 的 content 字符串 UTF-8 字节数之和 + 256 消息开销。conservative_input_tokens + output_reserve_tokens > context_window_tokens 时本地 CONTEXT_BUDGET_EXCEEDED，不发请求。这是本工具的保守预算估算，不保证任意 tokenizer/隐藏包装的绝对上界；服务仍报上下文不足时按 CONTEXT_REJECTED 失败并要求缩范围。不得调用远端 tokenizer/模型估算，也不声称估算就是账单 token。

预留 output tokens 不添加到通用 wire：请求体仍只用 model/messages，不默认传 max_tokens/厂商参数。因此输出长度以实际 body 硬上限和校验兜底。finish_reason 为 length/tool_calls/content_filter 等显式字符串且非 stop 值时拒绝为 INCOMPLETE_OUTPUT；字段缺失或 null 允许继续严格正文/Schema校验，不能仅凭缺失断言失败。

超限一律停下并展示大小/预算，用户缩文件/工作表/行范围后重新预览。若已锁定诊断范围变动，冻结旧 session、新建报告；不得静默丢行、删风险/ID或裁剪输入。无隐藏总结或多请求归并。最多一次不代表成功保证。

## 4. 配置作用域与 GUI/CLI

企业数据命令显式 --workspace，或使用用户通过 workspace use 选择的 app_preferences.active_workspace_id；两者都无则 WORKSPACE_REQUIRED。对象 ID 必须归属已选工作区，不按对象偷偷切企业。长任务开始时快照工作区，不跟随后续切换。

| 设置/动作 | GUI 入口 | CLI | 版本/生效 |
| --- | --- | --- | --- |
| 当前企业 | 顶部企业切换 | workspace use --workspace | 当前用户偏好；不分析、不发请求 |
| 企业创建/名称 | 系统设置→企业档案 | workspace create/update --input-json | workspace revision；名称变更不改历史快照 |
| 周期/时区/周始/财年 | 系统设置→核算周期 | workspace period set --input-json | 新 period_policy 版本；effective_from 必填，不重释旧历史 |
| Provider/Key | Provider设置 | provider set --input-json [--api-key-stdin] | 同工作区providers.revision；无模型连接测试 |
| 默认Provider | Provider设置→设为默认 | provider use --provider --expected-revision | expected_revision为workspace配置revision |
| 清Key/删除Provider | Provider设置→清空/删除 | provider clear-key/delete --expected-revision --confirm-local | 删除活动attempt使用的配置拒绝；默认删除后null，历史快照保留 |
| Token/超时预算 | Provider设置→请求预算 | provider set --input-json | Provider同revision；仅新预览，不改执行中attempt |
| 字段映射 | 数据源→映射 | mapping show/set --run --input-json | 版本化草稿；重算才进入新analysis revision |
| 手工输入 | 经营分析→待补输入 | manual-input list/set --period-key --input-json | workspace+period+metric/input；必填source_note，重算生效 |
| 指标/核算口径 | 系统设置→核算口径 | metric list/set --input-json | metric-catalog同字段；expected_revision/effective_from必填，新版本 |
| 来源冲突 | 数据源→冲突 | source-conflict list/resolve --run --input-json | 明确选择source evidence并写reason；不改原值，重算生效 |
| 历史回算 | 历史→指定口径重算 | analyze --policy --recalculate --input-json | 本地预览后计算；按period_key生成新revision或记录 |
| 模板 | 数据源→下载模板 | template download --package import --out | 空白三类工作簿和字段说明 |

新建企业默认 Asia/Shanghai、week_start_day=1（ISO周一）、fiscal_year_start_month=1；UI展示并确认，CLI省略时明确输出采用默认，不依赖宿主时区。cadence与首次effective_from必填。后续改时区必须创建period_policy，不直接修改历史；workspace.timezone_id仅新策略默认/当前显示缓存。period_policy包含timezone_id，历史run用快照。公式/映射/输入/冲突合同由data-period-import定义，配置值可由企业填写，不由开发者猜测。

规范化输入时只补本文声明的默认值；JSON Schema的default注解本身不会填值，Rust适配层填充后统一校验并返回实际采用值。Schema完成类型和有限枚举校验；Rust补 total>=connect+response、reserve<=context/2、IANA时区存在、日历日期真实、workspace归属、引用存在和effective区间规则。写入前校验，失败无部分保存；UI/CLI共用核心。输出不回显秘密。

## 5. 数据目录和版本兼容

application identifier 固定 com.dsheng.desktop。GUI、随包CLI和独立CLI共用核心路径解析，不依赖cwd/解压路径/程序文件名。

- macOS：$HOME/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3。
- Windows：%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3；用系统当前用户Known Folder解析，缺失时报错，不回退cwd。
- WAL/SHM同目录；本地一致性备份在backups/。当前用户级权限；用户库/备份不进入安装、便携、业务导出或Skill包。
- “便携”表示免安装解压运行，仍使用同一用户数据目录，不创建USB旁边的新库。
- schema兼容、迁移独占、一致性备份、失败回滚和删除按task-lifecycle规范；不自动降级，安装/卸载不删除用户库，不提供自动更新器。

## 6. 平台目标与四类产物

下表为首版产品目标和待执行验收，不是已构建/已实测声明。依赖不支持目标时发布Agent给证据交主Agent变更合同，不能静默提高门槛。

| 目标平台 | 架构/target | 桌面产物 | 独立CLI与验证 |
| --- | --- | --- | --- |
| macOS 13+ | arm64 / aarch64-apple-darwin | .app装入.dmg与便携.zip | 独立归档+.sh；macOS13和发布时可得当前受支持版本真实运行 |
| macOS 13+ | x64 / x86_64-apple-darwin | 分架构.app/.dmg/.zip | 独立归档+.sh；Intel目标运行，不能只交叉编译 |
| Windows 10 22H2、Windows 11 | x64 / x86_64-pc-windows-msvc | .msi与便携.zip，不交.exe安装器 | 独立归档+.ps1；两版本各测WebView2有/无 |

macOS用系统WKWebView；bundle.macOS.minimumSystemVersion和Rust MACOSX_DEPLOYMENT_TARGET均13.0。Windows桌面用WebView2 Evergreen，webviewInstallMode=skip；不嵌入下载器、不偷偷联网安装。未装时由桌面启动前本地检查显示“缺少WebView2运行时”及微软官方下载入口，用户安装后重启。MSI与便携同一预检，CLI不需要WebView2或桌面先启动。

四类交付物：桌面安装/便携包、dsheng-agent Skill ZIP、独立CLI及用户级安装辅助文件、中文Markdown/PDF手册。统一版本manifest及SHA-256。CLI用户级安装目录macOS为$HOME/.local/bin/dsheng、Windows为%LOCALAPPDATA%\Programs\dsheng\dsheng.exe；不静默改PATH，不提权，不执行Gatekeeper/SmartScreen/PowerShell策略绕过。辅助脚本无法运行时提供手动解压/绝对路径调用的正常路径。

unsigned需明确显示；受管系统策略可能拒绝执行，不保证所有企业电脑可用。安全提示说明按目标OS实测后写，不声称“通过认证”。不增加签名服务、云更新、远程同步。Windows10是产品测试目标，不是微软生命周期支持承诺。

### 官方资料核验范围

核验日期2026-10-30；只确认工具字段及运行时分发机制，不代表本产品验证通过：

- https://v2.tauri.app/distribute/windows-installer/ ：WebView2安装模式，首版选择skip。
- https://v2.tauri.app/reference/config/ ：minimumSystemVersion与webviewInstallMode配置；锁依赖后再验对应schema。
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution ：Evergreen独立分发及运行时检查机制。
- https://developer.microsoft.com/en-us/microsoft-edge/webview2/ ：用户安装运行时的微软入口。

## 7. 错误与验收

稳定JSON外层：成功含schema_version=1、ok=true、data；失败含schema_version=1、ok=false、error={code,message,field,details}。details只允许脱敏ID/revision/预算/计数，禁止Key、URL凭据、完整请求/失败响应。具体error catalog在cli-surface.json。成功0，参数/配置/确认/版本冲突2，本地数据/数据库校验3，已派发Provider失败4，导出5；发送前取消2/CONFIRMATION_CANCELLED，派发后取消4/CANCELLED。

待工程验收：

- PC-01：多Provider无默认拒绝、显式优先、跨企业拒绝、删默认不fallback。
- PC-02：预览后Provider/Key/预算/分析/报告变化使确认失效；重复确认最多一个attempt。
- PC-03：字节/token边界与超1、非法reserve/timeout、8,001个Unicode字符、超大响应；发送前失败request_count=0。
- PC-04：协议/正文/schema错误不覆盖旧结果；纯解析校验不冒充真实HTTP，不建Mock Provider。
- PC-05：GUI/CLI配置同库及revision竞争；Key/配置操作零模型请求，JSON/错误不含Key。
- PC-06：三架构四产物版本/hash一致；Windows有无WebView2，缺失不下载；CLI独立。
- PC-07：换cwd/解压位置仍同用户库；重装保留；权限失败明确错误、不fallback。
- PC-08：最多一条请求，无分块/隐藏摘要/自动重试；手动重试需新confirmation。

以上是标准，不标已执行。Provider/配置、UI/CLI、发布工作包分别提供证据，主Agent整合。真实HTTP/内容验收仅在主Agent获得具体调用额度后进行。
