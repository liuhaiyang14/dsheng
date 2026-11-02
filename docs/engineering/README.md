# 工程构建与协作说明（主 Agent 维护）

## 环境
- Rust：rustup stable（brew rustup；PATH 见 ~/.zshrc 的 /opt/homebrew/opt/rustup/bin）。
- Node：>= 20（UI 包在 src-ui/ 自带 package.json）。
- 工具链版本以 Cargo.lock / src-ui/package-lock.json 锁定。

## 常用命令
```bash
cargo fmt --all
cargo clippy --workspace --all-targets   # 必须 0 警告
cargo test --workspace
python3 tools/validate_contracts.py      # 合同改动后必须 PASS
python3 tools/extract_provider_json.py --selftest
cargo run -p dsheng-cli -- --version     # CLI 骨架
```

## 工作树协作模型（主计划 §12）
- 并行工作包在独立 worktree/分支：wp/data-sqlite、wp/provider-prompt、wp/ui。
- 每个工作包只改自己的所有权目录；共享文件（core Cargo.toml）只允许加依赖行。
- 完成报告 → 主 Agent 静态检查/测试/独立只读复审 P0/P1=0 → 按序合并回 main。
- 冻结不可改：contracts/v1/**、prompts/**（哈希守卫测试）、migrations/0001、
  docs/specs/**、tools/**。缺口一律报主 Agent，不改合同。

## 合并顺序（集成轮 1）
1. wp/data-sqlite（核心域+持久化）。
2. wp/provider-prompt（rebase 到合并后 main，解决 core Cargo.toml 单行冲突）。
3. wp/ui（纯 src-ui，无 Rust 冲突）。
4. 全量 cargo test + 独立复审 → P0/P1 清零 → 提交 main → 启动 CLI/导出包。

## 阶段与门禁
- Phase 0（已完成）：合同工程化基线，见 acceptance-map.md / deliverables.md。
- 集成轮 1 后：CLI/导出 → Tauri 壳（src-tauri，主 Agent 所有权）→ 用例 40
  桌面/CLI 同库端到端 → Skill/手册 → 发布与跨平台 → 终验归档。
