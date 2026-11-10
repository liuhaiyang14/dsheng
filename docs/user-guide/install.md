# 安装与首次启动

当前发布版本为 0.1.13：[下载 macOS arm64 DMG](https://github.com/liuhaiyang14/dsheng/releases/download/v0.1.13/dsheng_0.1.13_macos_arm64.dmg)。该安装包未签名，首次打开前请阅读下方的安全提示。

## 平台与产物

| 平台 | 架构 | 安装包 | 便携包 |
| --- | --- | --- | --- |
| macOS 13+ | Apple Silicon（arm64） | `.dmg`（内含 `.app`） | `.zip` 解压运行 |
| macOS 13+ | Intel（x64） | `.dmg`（分架构） | `.zip` 解压运行 |
| Windows 10 22H2 / Windows 11 | x64 | `.msi` | `.zip` 解压运行 |

注意：Windows 只提供 `.msi` 安装包，不提供 `.exe` 安装器。安装包与便携包含同一桌面
程序、运行资源、同版本 CLI 与数据库迁移，**不含**用户数据库、经营历史或 Provider
配置。请按 CPU 架构选择对应包；版本号与 SHA-256 见随包发布的版本清单。

## unsigned 首次启动（标准流程，不提供绕过）

当前发布的应用未做开发者签名与公证。首次启动时操作系统会提示「无法验证开发者」，
这是预期现象。请按系统**正常安全流程**操作（工具不提供、也不建议任何绕过系统安全
检查的方法）：

### macOS（Gatekeeper）

1. 双击 `.dmg`，将 `dsheng.app` 拖入「应用程序」文件夹（或直接从便携 ZIP 解压）。
2. 首次打开若弹出「无法打开」，打开「系统设置 → 隐私与安全性」，下拉到「安全性」，
   找到关于 dsheng 的提示，点击「仍要打开」，按提示确认。
3. 也可在「应用程序」中按住 Control 键点击 dsheng，选择「打开」→「打开」。
4. 若你的 Mac 由公司管理且策略完全阻止运行 unsigned 应用，则本工具在该设备不可用；
   请与设备管理员确认，不要尝试禁用 Gatekeeper。

### macOS 弹出「已损坏，无法打开」怎么办

这**不是应用真的损坏**。如果你是通过微信、邮件或浏览器下载方式获取安装包，
macOS 会自动在文件上打隔离标记（quarantine），导致未签名应用被系统判为"已损坏"。

解决方法：安装完成后，打开「终端」（Terminal），粘贴以下命令并回车：

```bash
xattr -cr /Applications/dsheng.app
```

然后重新打开应用即可。如仍弹出「无法验证开发者」，按上方标准流程操作。

> **提示**：使用 U 盘或移动硬盘直接拷贝安装包到对方电脑不会触发此问题。

### Windows（SmartScreen）

1. 运行 `.msi` 完成安装（或解压便携 ZIP 到任意用户目录）。
2. 首次启动若出现 SmartScreen 蓝色窗口「Windows 已保护你的电脑」，点击「更多信息」，
   再点击「仍要运行」。
3. 若组策略/企业管控禁止运行，则按提示处理；不要降低系统安全策略。

建议在首次启动后核对下载包的 SHA-256 与版本清单一致，再开始导入数据。

## 数据库位置（固定用户数据目录）

首次启动时，工具在当前用户数据目录创建 SQLite 数据库（桌面端与 CLI 共用同一文件）：

- macOS：`~/Library/Application Support/com.dsheng.desktop/dsheng.sqlite3`
- Windows：`%LOCALAPPDATA%\com.dsheng.desktop\dsheng.sqlite3`（按系统当前用户解析
  Known Folder；解析失败会明确报错，不会退回安装目录）

同目录下还有数据库的 WAL/SHM 临时文件与 `backups/` 备份目录。

要点：

- **便携 ≠ 数据放在程序旁边**：「便携版」指免安装解压运行，数据库仍在上述固定用户
  数据目录，不会在 ZIP 解压目录新建第二个库；换台电脑运行就是另一个空库。
- 升级/覆盖安装/卸载**不删除**该用户数据库；想彻底清理需手动删除上述目录（见下文）。
- 数据库属于当前用户账户；多账户各自独立。

## WebView2（仅 Windows 桌面端）

Windows 桌面端界面依赖系统 WebView2 运行时（Windows 11 一般已内置）。若缺失，启动前
的本地检查会提示「缺少 WebView2 运行时」并给出微软官方下载入口，安装后重启应用即可；
工具不会静默联网代装。CLI 与 WebView2 无关。详细处置见
[troubleshooting.md](troubleshooting.md)。

## 卸载与数据保留

- macOS：删除 `dsheng.app`；Windows：「设置 → 应用」卸载，或删除便携目录。
- 卸载**不会**删除用户数据目录中的数据库与备份。确认不再需要数据后，可手动删除：
  - macOS：`~/Library/Application Support/com.dsheng.desktop/`
  - Windows：`%LOCALAPPDATA%\com.dsheng.desktop\`
- 已导出到其他位置的导出包/文件不受卸载影响；API Key 随数据库一并删除。

## 下一步

- 配置模型 Provider（可选）：[provider-setup.md](provider-setup.md)
- 建立企业与周期：[workspace-period.md](workspace-period.md)
- 首次全流程走查：[checklist.md](checklist.md)
