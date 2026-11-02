# 独立只读复审清单（每轮集成后，P0/P1=0 才可合并）

复审 agent 使用：只读，不改代码。对合并候选（分支/提交区间）逐项核查，
按 P0（阻塞发布）/P1（必须修复）/P2（记录）分级输出。

## 安全与凭据
- [ ] 全库无 API Key、无 ~/.zshrc 引用、无本机绝对路径硬编码（/Users/、C:\）
- [ ] Provider 公开视图仅 key_present；错误/日志/导出不出现 Key 或前后缀
- [ ] git 历史无业务 XLSX/PPTX/SQLite 文件；fixtures 全合成且文件内有合成声明

## 冻结合同完整性
- [ ] contracts/v1/**、docs/specs/**、prompts/**、migrations/0001_init.sql 与基线逐字节一致（git diff 基线哈希）
- [ ] cargo test 中提示词哈希守卫测试通过（改一个字即失败）
- [ ] validate_contracts.py PASS

## C 方案适配层（MUST）
- [ ] 提取实现与 tools/extract_provider_json.py --policy adapt 行为一致：严格优先、
      根形状强标记仅 schema_version/tasks、唯一裸任务包装、多候选拒绝
- [ ] envelope_locally_reconstructed 在包装路径必然持久化（存储类型/字段/测试断言）

## 门禁与状态机
- [ ] 未确认不派发：dry-run → confirmation_id → --confirm-send 双门禁纯逻辑测试存在
- [ ] 每动作 ≤1 HTTP 请求；无自动重试路径；终态不可回退；迟到事件不覆盖有效结果
- [ ] 任务进度/状态转换与 task-lifecycle §5 逐条一致（含 99.99 上限、block 优先、
      manual 重置、reopened 事件）
- [ ] 同期重做只追加 revision；历史快照不被 live 推进改写

## 数值与隔离
- [ ] 业务数值无 f64 中转：decimal 字符串端到端（SQLite/JSON/模型边界词法保留）
- [ ] 工作区隔离：跨 workspace ID 全路径拒绝（服务层+测试）
- [ ] 图表/导出不把缺失当零、不插值

## 测试与构建
- [ ] cargo fmt/clippy 0 警告；cargo test 全绿；DC 56 + TS 34+50 执行数与合同一致
- [ ] 测试模块标明层次（纯逻辑/依赖注入/组件）
- [ ] 无 Mock Provider/假 endpoint/录制回放/伪造模型历史

输出格式：逐项 结论+证据（文件:行/测试名/命令输出）＋P0/P1/P2 列表＋总体判定。
