# 发布与打包（docs/release）

状态：脚本与流程可用；当前已在本机（macOS arm64）产出 macOS arm64 两件
桌面产物并生成版本清单。Windows/x64 产物待 CI/目标机执行（见文末覆盖状态）。

当前版本 0.1.12 的 macOS arm64 安装包：[下载 DMG](https://github.com/liuhaiyang14/dsheng/releases/download/v0.1.12/dsheng_0.1.12_macos_arm64.dmg)；便携版和 SHA-256 见 [版本清单](manifest-v0.1.12.json)。

边界（必须遵守）：

- 不签名、不公证（notarize）、不绕过 Gatekeeper/SmartScreen。
- **GitHub Release / 二进制公开分发须产品验收通过并获用户显式授权**，
  本目录只准备产物与清单，不执行发布。
- 大二进制产物**不入 git**：统一输出到 `target/release-dist/`（已被
  .gitignore 覆盖）；git 中只保留 `docs/release/manifest-v<ver>.json`
  记录文件名/大小/SHA-256。
- 任何包内不得包含用户数据库、Provider 配置/API Key、业务文件
  （脚本自动断言，见各脚本）。

## 四类交付物与命名约定

| 类别 | 文件命名（<ver> = workspace 版本） | 说明 |
| --- | --- | --- |
| 桌面安装包 | `dsheng_<ver>_macos_<arm64\|x64>.dmg`、`dsheng_<ver>_windows_x64.msi` | Tauri bundler 产物 |
| 桌面便携包 | `dsheng_<ver>_<macos\|windows>_<arch>_portable.zip` | 解压运行；仍用用户数据目录 |
| 独立 CLI | `dsheng-cli_<ver>_<macos\|windows>_<arch>.zip` | 含 install/ 用户级安装辅助 |
| Skill ZIP / 手册 | `dsheng-agent-skill_<ver>.zip`、`dsheng-manual_<ver>.zip` | 由对应工作包产出后入清单 |

命名不符的文件无法进入版本清单（`version_manifest.py` 直接报错）。

## 环境（macOS 本机）

- macOS 13+（当前实测机：macOS 26.7.1 arm64）、Xcode Command Line Tools
- rustup stable（`~/.cargo/bin` 在 PATH；脚本会自动补）
- Node ≥ 20（src-ui 构建）、python3 ≥ 3.11（tomllib；清单脚本）
- Tauri CLI：`cargo install tauri-cli` 可选；脚本探测不到时自动用
  `npx --package @tauri-apps/cli@2`（首次联网下载）

## 构建流程（macOS arm64 本机）

```bash
# 1) 桌面：.dmg + 便携 zip（含包内断言；约 3-6 分钟）
scripts/package/macos.sh
#   Intel 交叉（需先 rustup target add x86_64-apple-darwin；未在本机验证）：
#   scripts/package/macos.sh --target x86_64-apple-darwin

# 2) 独立 CLI 归档（含安装辅助与 SHA256SUMS）
scripts/package/cli_dist.sh

# 3) 生成版本清单（写入 docs/release/manifest-v<ver>.json，入 git）
python3 scripts/package/version_manifest.py --dist-dir target/release-dist

# 4) 发布前检查（secrets/业务文件扫描 + 清单哈希核对）
scripts/package/preflight.sh --dist-dir target/release-dist
```

产物输出：`target/release-dist/`。增量重跑可加 `--skip-ui-build
--skip-tauri-build --skip-cli-build`（macos.sh）/ `--skip-build`
（cli_dist.sh）复用缓存。

脚本内置断言（失败即中止，不产出半成品包）：

- macos.sh：Info.plist（identifier=com.dsheng.desktop、版本一致、
  LSMinimumSystemVersion=13.0）、主可执行存在、CLI `--version` 的
  app_version/identifier 与 workspace 版本一致、migration 清单与
  `crates/dsheng-core/src/repository/mod.rs` 的 `MIGRATIONS`
  `include_str!` 嵌入清单一致、便携 zip 内含 app/CLI/migrations、
  无 `.sqlite*/.db*/.key/.pem/.env/.xlsx/.pptx` 等禁止文件、
  DMG `hdiutil imageinfo` 校验。
- cli_dist.sh：二进制版本/identifier、归档成员（二进制+安装辅助+
  SHA256SUMS）、无禁止文件。

## 版本清单（manifest）

`docs/release/manifest-v<ver>.json`（入 git）：

- 硬校验：根 `Cargo.toml` `[workspace.package].version` 与
  `src-tauri/tauri.conf.json` `version` 一致，否则报错；
  `src-tauri/Cargo.toml`、`src-ui/package.json` 版本不一致时告警
  （`--strict` 升级为报错）。
- `artifacts[]`：四类产物 × 平台 × 架构 × 文件 × size × SHA-256
  （zip 附内容摘要：条目数/顶层成员/是否含 CLI 与 migrations）。
- `coverage[]`：期望产物矩阵逐项 present/missing——**如实记录**，
  未构建的 Windows/x64 标 missing，不虚构。
- `components`：`install/` 安装辅助与 `migrations/*.sql` 的哈希
  （与 repository 嵌入清单核对）+ db_schema_version。
- `signing.signed=false`：unsigned 声明内嵌。

核对既有清单（发布前必跑；preflight 已自动调用）：

```bash
python3 scripts/package/version_manifest.py --verify docs/release/manifest-v<ver>.json --dist-dir target/release-dist
```

产物文件被移动后，把所在目录经 `--dist-dir` 传入即可（可多次）。

## 数据目录语义（便携 ≠ 数据在程序旁）

安装版、便携版、独立 CLI 共用同一用户库（首启自动建库/迁移）：

- macOS：`~/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3`
- Windows：`%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3`

删除/移动程序目录不影响用户数据；包内绝无用户库。

## 首次启动安全提示

见 [UNSIGNED.md](UNSIGNED.md)（按 macOS 26 本机实测行为编写；
Windows 部分按系统文档行为编写、待真机走查）。

## 验收与发布

- 四产物手工验收步骤：[RELEASE-CHECKLIST.md](RELEASE-CHECKLIST.md)
  （用例 12/18/35/37/39/70）。
- 全部产物产出并 preflight PASS 后，由用户决定是否执行 GitHub Release；
  本工作包不执行发布、不持有发布凭据。

## 当前覆盖状态（随 manifest 同步更新）

| 产物 | 状态 |
| --- | --- |
| dsheng_0.1.12_macos_arm64.dmg | 本机已产出（2026-11-10） |
| dsheng_0.1.12_macos_arm64_portable.zip | 本机已产出（2026-11-10） |
| dsheng-cli_0.1.12_macos_arm64.zip | 尚未产出 |
| macOS x64 两件 | 脚本支持 `--target x86_64-apple-darwin`；未执行（需 rust target + Intel 验证机） |
| Windows x64 msi/portable/cli | 打包在 Windows 宿主执行（脚本/CI 就绪）；未执行 |
| Skill ZIP / 手册 | 归属其他工作包；manifest coverage 记 missing |

已知缺口（报告不改配置，详见工作包完成报告）：

1. `tauri.conf.json` 无 `bundle.icon`，.app/.dmg 无应用图标。
2. `beforeBuildCommand: npm --prefix ../src-ui run build` 的相对路径
   在 tauri CLI（于仓库根执行钩子）下解析错位；打包脚本用 `--config`
   覆盖绕开，仓库配置本身建议主 Agent 修正。
