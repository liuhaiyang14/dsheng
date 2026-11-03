# dsheng 桌面壳（Tauri 2）

业务逻辑全部在 `dsheng-core` / `dsheng-cli` 服务层；本 crate 只做 IPC
command 接线与窗口生命周期。桌面 UI 与 CLI 共享同一 SQLite（用例 40）。

## 桌面启动（开发模式）

两个终端（或用 tmux/并发跑）：

```bash
# 终端 1：UI dev server（tauri.conf.json devUrl=http://localhost:5173）
npm --prefix src-ui run dev

# 终端 2：桌面壳（自动连接 devUrl）
cargo run --manifest-path src-tauri/Cargo.toml
```

发布构建：`npm --prefix src-ui run build && cargo build --manifest-path src-tauri/Cargo.toml --release`。

注意：本 crate 被根 workspace 显式 exclude（`frontendDist` 需前端先构建），
一律用 `--manifest-path src-tauri/Cargo.toml` 单独构建/测试。

## 数据目录（docs/specs/provider-config-delivery.md §5）

- macOS：`~/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3`
- Windows：`%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3`（Known Folder；
  缺失报 `DATA_DIRECTORY_UNAVAILABLE`，不回退 cwd）
- Linux（开发）：`~/.local/share/com.dsheng.desktop/dsheng.sqlite3`

目录创建/打开/版本协商/首启迁移复用 `dsheng_cli::ctx::open_db_at`——与
CLI `--db` 同一实现。**CLI 与桌面共享同一库**：CLI 侧加
`--db <桌面库路径>`（或导出 `DSHENG_DB`，桌面/CLI 同名同义，也用于测试注入）。

## Command 接线

- UI `DshengClient` 全部命令（31 条）注册为 `#[tauri::command(async)]`
  （`src/contracts/commands.rs`；名/参数对齐 `src-ui/src/api/tauriClient.ts`，
  Rust snake_case ↔ UI camelCase）。返回 CliEnvelope JSON 原样（失败也以
  envelope 表达，不 reject invoke）。
- 实现（`src/contracts/core/**`）直调 dsheng-cli 服务层：构造同一 clap
  命令枚举走 `dsheng_cli::cmd::dispatch`；`--input-json` 类输入经临时文件
  走同一读取/校验路径；stdin 专通道（Provider Key、diagnosis continue
  答复）以内联文本直达同一服务函数。壳层无业务逻辑。
- 生成类命令走服务层双门禁：`*_preview` 本地预览并记录 confirmation；
  `*_confirm` 重建请求 + `--confirmation-id/--confirm-send`，由服务层逐项
  重验快照后原子消费派发。confirm 只带 confirmation_id，原始请求存进程内
  备忘（应用重启后需重新预览）；门禁本体全在服务层。
- 命令清单基准：`contracts::COMMANDS`（S5 测试与 tauriClient.ts 静态比对）。

## 测试

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml
```

集成测试：`tests/wp_tauri_roundtrip.rs`（用例 40：CLI↔桌面 command 层
同一 SQLite 双向读写一致、REVISION_CONFLICT 传递、数据目录解析/注入）、
`tests/wp_tauri_client_contract.rs`（S5：command 名/参数形状与
tauriClient.ts 一致、generate_handler 注册齐全）。
