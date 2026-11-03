# 发布验收清单（RELEASE-CHECKLIST）

四类交付物 × 用例 12/18/35/37/39/70 的手工/半自动验收步骤。发布执行者
（用户或获授权的主 Agent）逐项打勾并存证据（命令输出/截图/清单文件）。
任何一项不过即不得发布。

前置（每轮发布）：

- [ ] `scripts/package/preflight.sh --dist-dir target/release-dist` 输出
      PASS（secrets/业务文件扫描 + 清单核对，用例 30）。
- [ ] `docs/release/manifest-v<ver>.json` 与当前代码版本一致，且
      `version_manifest.py --verify` 通过（用例 39 前半）。
- [ ] 目标产物在 coverage 中均为 present（缺项要么补产、要么在发布说明
      中明示不支持该平台）。

## 用例 12：两平台包启动 + unsigned 说明（桌面安装包）

macOS（13+ arm64；x64 同步骤换 Intel 机）：

- [ ] 干净用户目录（新建测试账号或清掉
      `~/Library/Application Support/com.dsheng.desktop/`）挂载
      `dsheng_<ver>_macos_arm64.dmg`，拖入 /Applications。
- [ ] 首次启动：按 [UNSIGNED.md](UNSIGNED.md) macOS 节走“系统设置 →
      隐私与安全性 → 仍要打开”；记录弹窗实际文案与系统版本。
- [ ] 首启后在 `~/Library/Application Support/com.dsheng.desktop/`
      生成 `dsheng.sqlite3`（schema 版本 = manifest
      `components.db_schema_version`）；窗口正常打开。
- [ ] 卸载（删 .app）后用户数据仍在；重装后数据可读（用例 70 的
      安装/卸载不删库边界）。

Windows（10 22H2 与 11 各一台 x64；Web 有/无 WebView2 四组合）：

- [ ] 无 WebView2 机器：启动 .msi 前的缺失提示出现且给出微软官方入口；
      不自动下载安装；装官方 Evergreen 后重启应用可进入。
- [ ] 安装 `dsheng_<ver>_windows_x64.msi`；首次启动 SmartScreen 按
      “更多信息 → 仍要运行”放行，记录实际文案（回填 UNSIGNED.md）。
- [ ] 首启后在 `%LOCALAPPDATA%\com.dsheng.desktop\` 生成 dsheng.sqlite3；
      卸载 MSI 不删除用户数据。
- [ ] 便携 zip 解压到任意目录运行，行为与安装版一致（同用户库）。

## 用例 18：便携 ZIP 含程序/CLI/migration（半自动，脚本断言 + 抽查）

- [ ] `scripts/package/macos.sh`（Windows 侧打包脚本同要求）全绿跑完
      （内置断言：可执行/Info.plist/CLI 版本/migration 清单/无业务文件）。
- [ ] 抽查解压 `dsheng_<ver>_<plat>_<arch>_portable.zip`：
      - [ ] `dsheng.app`（或 Windows 程序目录）可启动；
      - [ ] `bin/dsheng --version` 的 app_version=\<ver\>、identifier=
            com.dsheng.desktop；
      - [ ] `migrations/MANIFEST.txt` 所列 SQL 哈希与 manifest-v\<ver\>.json
            `components.migrations` 一致；
      - [ ] zip 内无 `.sqlite*/.db*/.key/.pem/.env/.xlsx/.pptx`（脚本已
            断言，人工复核一次清单输出）。

## 用例 35：三架构四产物清单

- [ ] manifest-v\<ver\>.json 的 coverage 中，交付范围内的每个
      （产物, 平台, 架构）组合均 present，且每项 sha256 与本地
      `shasum -a 256` 复核一致（用例 39 后半）。
- [ ] 当前版本实际范围：macOS arm64 已产；macOS x64、Windows x64 按
      §6 矩阵补齐（CI/目标机跑 `scripts/package/macos.sh --target
      x86_64-apple-darwin` 与 Windows 打包）后才可勾选。
- [ ] Skill ZIP / 手册由对应工作包提供后入清单（coverage 转 present）。

## 用例 37：独立 CLI 安装可运行（干净环境）

macOS：

- [ ] 新解压 `dsheng-cli_<ver>_macos_arm64.zip` 至任意目录，绝对路径运行
      `./dsheng --version`、`--help` 正常（0.1.0 已在本机通过）。
- [ ] `install/macos-user-install.sh`：以普通用户执行 → 复制到
      `~/.local/bin/dsheng`、SHA 校验前后通过、`--version` 冒烟通过；
      脚本未改 PATH/未提权/未动 quarantine（代码审查 + 运行输出核对）。
- [ ] 以 root/sudo 运行被拒绝且提示用户级安装。
- [ ] Intel 机重复上述步骤（x64 归档）。

Windows：

- [ ] 新解压 x64 归档，`.\dsheng.exe --version`、`--help` 正常。
- [ ] `install/windows-user-install.ps1` 在默认策略环境执行：复制到
      `%LOCALAPPDATA%\Programs\dsheng\dsheng.exe`，不改 PATH/不提权/
      无 Unblock-File/Set-ExecutionPolicy 调用；被策略拦截时按 README
      手动复制路径可用。
- [ ] 真机走查后回填 UNSIGNED.md Windows 节“实测”标注。

## 用例 39：Release 清单 SHA-256

- [ ] `python3 scripts/package/version_manifest.py --dist-dir
      target/release-dist` 生成最新清单并提交 git。
- [ ] `--verify` 通过；preflight.sh 第 3 节 OK。
- [ ] （发布执行时）对外公布渠道中附上 manifest 文件本体或等价哈希表。

## 用例 70：identifier/备份/恢复/四产物

- [ ] 三个入口（安装版/便携版/独立 CLI）在同一用户下指向同一数据目录
      与同一 SQLite（换 cwd/解压位置不改变库路径）。
- [ ] 桌面与 CLI 同库往返读写一致（自动化：src-tauri
      `tests/wp_tauri_roundtrip.rs` 已覆盖；发布前再跑一次全量
      `cargo test --workspace` + `cargo test --manifest-path
      src-tauri/Cargo.toml`）。
- [ ] manifest `components.migrations` 与 `db_schema_version` 与随包
      `migrations/MANIFEST.txt` 一致（脚本已断言，人工复核一次）。

## 发布执行（须用户显式授权后进行）

- [ ] 以上全部勾选并归档证据。
- [ ] 用户确认发布渠道与范围（GitHub Release 等），授权后手动执行；
      发布说明附：UNSIGNED.md 摘要、manifest 哈希、数据目录位置、
      API Key 明文存储提示。
