# 新用户走查清单（可复现手工验收）

按顺序逐项执行并勾选；每步都写明**预期结果**，不符即停并按
[troubleshooting.md](troubleshooting.md) 排查。建议首次使用完整走一遍（约 20–30 分钟）。
CLI 步骤均可用 `--json` 观察 envelope；`<id>` 代指实际 UUID。

## A. 安装与首启（对应 [install.md](install.md)）

- [ ] A1 按平台下载安装包/便携包，核对版本清单中的 SHA-256 一致。
- [ ] A2 首次启动出现 unsigned 提示，按标准流程放行：
      macOS「系统设置 → 隐私与安全性 → 仍要打开」；Windows SmartScreen「更多信息 →
      仍要运行」。未出现提示也正常（策略差异）。
- [ ] A3 启动后确认用户数据目录已创建：
      macOS `~/Library/Application Support/com.dsheng.desktop/`；
      Windows `%LOCALAPPDATA%\com.dsheng.desktop\`。
- [ ] A4（Windows 未装 WebView2 的机器）看到「缺少 WebView2 运行时」提示与官方下载
      入口；安装运行时后重启可进入。
- [ ] A5 CLI（若安装）：`dsheng --version --json` 退出码 0，`ok=true`；`dsheng --help`
      正常显示。

## B. 企业与周期（[workspace-period.md](workspace-period.md)）

- [ ] B1 新建企业「演示企业」，周期=月度、生效日=当年 1 月 1 日；确认默认
      时区 `Asia/Shanghai`、周一起始、财年 1 月被明确展示。
      CLI：`dsheng workspace create --input-json ws.json --json` → `data.workspace.workspace_id`。
- [ ] B2 `dsheng workspace list --json` 能看到该企业；`dsheng workspace use --workspace <id> --json`
      设为当前企业。
- [ ] B3 再建第二个企业「演练企业」，确认两边列表互不可见对方数据（隔离验证）。

## C. Provider（可选；[provider-setup.md](provider-setup.md)）

- [ ] C1 阅读「API Key 明文保存」风险说明；确认接受后才继续。
- [ ] C2 新建 Provider（名称/HTTPS base_url/model），Key 仅经
      `printf '…' | dsheng provider set --workspace <id> --input-json provider.json --api-key-stdin --json`
      录入；输出只有 `key_present: true`，无 Key 内容。
- [ ] C3 `dsheng provider list --workspace <id> --json` 只见掩码/`key_present`；
      `provider use` 设为默认。确认保存过程没有任何模型请求（无连接测试）。

## D. 导入与分析（[import-and-templates.md](import-and-templates.md) / [analysis.md](analysis.md)）

- [ ] D1 `dsheng template download --package source --profile <profile-id> --out source-template.zip`，
      解包见当前 profile 的空白原始 source 模板；另用 `--package output --contract two_tables_v1`
      验证固定两表输出模板。两表输出模板不作为输入。
- [ ] D2 按 source profile 模板第 7 行起填入 1–2 行合成原始业务数据（不要使用真实客户数据做走查）。
- [ ] D3 `dsheng scan --workspace <id> --dir <目录> --source-profile <profile-id> --json`：文件
      `mapping_status="mapped"`；扩展名不支持的文件只出现在 `pending_inputs`。
- [ ] D4 （若有表头不在别名表）`dsheng mapping show --workspace <id> --run <id> --json`
      查看未映射列并 `mapping set` 补齐。
- [ ] D5 `dsheng analyze --workspace <id> --dir <目录> --from 2026-10-01 --to 2026-10-31 --grain auto --json`：
      退出码 0；记录 `run_id`/`current_revision_no`；`metrics[0].actual_value` 为十进制
      字符串原值；无基期时 `mom.availability_status="unavailable"` 且给出原因。
- [ ] D6 `dsheng analysis show --workspace <id> --run <id> --json` 与 D5 一致；
      重复执行 D5 后 `current_revision_no` +1（重做追加 revision）。
- [ ] D7 （可选）`dsheng manual-input set …` 补一个目标值（带 `source_note`），重跑
      D5 后新 revision 生效；旧 revision 仍可查。

## E. 诊断（可选；[diagnosis.md](diagnosis.md)）

- [ ] E1 `dsheng diagnosis generate --workspace <id> --run <id> --framework 5why --problem "演示问题" --evidence <id> --dry-run --json`：
      返回 `confirmation_id`、`request_count_maximum=1`、来源范围与 token 估算；
      此时**未发生请求**（预览即止）。
- [ ] E2 门禁负向验证（不消耗额度）：
      - 只加 `--confirm-send` → 退出码 2 `INVALID_ARGUMENT`；
      - 带伪 `--confirmation-id … --confirm-send` → 退出码 2 `CONFIRMATION_REQUIRED`。
- [ ] E3 你批准预览后，携带 `--confirmation-id <id> --confirm-send` 真实发送一次；
      成功后 `diagnosis list/show` 可见报告；导出
      `dsheng diagnosis export --workspace <id> --report <id> --format md --out report.md`。
      尝试 `--format pdf` 应得到明确拒绝提示（当前仅 md/json）。
- [ ] E4 （若报告要求澄清）用 `diagnosis continue --answers-stdin` 给完整澄清重述，
      先 dry-run 后确认；或 `diagnosis pending cancel` 放弃该轮。

## F. 任务单（[tasks.md](tasks.md)）

- [ ] F1 `dsheng task generate --workspace <id> --run <id> --dry-run --json` 得到预览；
      批准后确认发送，得到 `candidate_generation_id`（候选，未落库为任务）。
- [ ] F2 `dsheng task reconcile --workspace <id> --run <id> --input-json r.json --dry-run --json`
      得到 diff 与 `apply_token`；把 token 写入输入 JSON 顶层后
      `--apply`，`data.new_task_ids` 出现新任务。
- [ ] F3 `dsheng task show --workspace <id> --task <id> --json`：初始
      `status="not_started"`、步骤未开始；SMART 为 `generated`。
- [ ] F4 `dsheng task update --workspace <id> --task <id> --patch-json progress.json --json`
      手动推进 40%（含 `adjustment_reason`）；`task show` 中 `progress="40"`、状态
      `in_progress`；改内容字段后 SMART 变 `stale`。
- [ ] F5 `dsheng task step add …` 加一条步骤、`task step move …` 调整顺序；尝试删光
      步骤应被 `AT_LEAST_ONE_STEP` 拒绝。
- [ ] F6 `dsheng task timeline --workspace <id> --from <起> --to <止> --grain day --json`
      能看到计划开始/截止条目。

## G. 历史与导出（[history-and-export.md](history-and-export.md)）

- [ ] G1 `dsheng history list --workspace <id> --json` 列出期间；`history revisions`
      ≥2；`history show --revision 1` 返回旧版数字（未被 D6/D7 重算改写）。
- [ ] G2 `dsheng history compare --workspace <id> --run <id> --type mom --json`（无基期
      时显示不可比原因而非 0）。
- [ ] G3 `dsheng export --workspace <id> --run <id> --bundle business --out bundle.zip`：
      解包见三份 XLSX + 结构化 JSON + metadata；**检查包内无 API Key、无本机绝对路径**。
- [ ] G4 `dsheng export --workspace <id> --run <id> --workbook task --out tasks.xlsx` 与
      `--meeting --format md --out meeting.md`（10 页，缺项如实标注）。
- [ ] G5 公式注入检查：在任务标题填 `=SUM(1+1)` 类文本后重导出，用表格软件打开
      确认显示为文本而非公式。
- [ ] G6 删除演练：`dsheng history delete --workspace <id> --run <id> --dry-run --json`
      得到 `delete_preview_id` 与清单；若该期间有可推进任务应先报
      `DELETE_DEPENDENCY_BLOCKED`（预期行为）；无依赖期间用
      `--confirm-delete <id>` 删除后 `history list` 不再出现。

## H. 收尾

- [ ] H1 确认未在任何命令行/日志/笔记中留下 API Key 明文。
- [ ] H2 （可选）整目录备份用户数据目录一份（含明文 Key，妥善保管）。
- [ ] H3 卸载演练（可选）：卸载应用后确认用户数据目录仍在；重新安装后数据可读。

全项通过即完成一次「本地分析 → 任务导出」的最小闭环验收（对应验收用例 38 的手册
走查要求）。
