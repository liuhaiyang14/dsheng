# UNSIGNED 首次启动安全说明

dsheng 0.1.0 桌面包与独立 CLI 均**未签名、未公证**（unsigned）。这是当前
产品阶段的既定状态：不购买证书、不做公证、不接入任何签名服务。本文按
目标系统实际行为说明首次启动会看到什么、如何正常放行，以及哪些事我们
**不做**。

声明（先读）：

- 本产品**不提供、不建议**任何绕过系统安全检查的方法或脚本（包括移除
  quarantine 属性、关闭 Gatekeeper/SmartScreen、改执行策略等）。
- “按提示放行”是操作系统给用户的正常选择路径，不是绕过。
- **受管（企业 MDM/组策略）设备可能直接拒绝未签名软件，且管理员禁用
  “仍要打开/仍要运行”入口。此时本产品在该设备不可用，属预期行为**，
  不是缺陷，也没有官方支持通道。
- 本文不声称任何“认证/审核通过”。
- 明文 API Key 风险：Provider API Key 以明文保存在本机 SQLite
  （`providers.api_key`）。能读取当前用户数据库文件的人/程序即可读取
  Key。这在使用手册与发布说明中必须如实告知。

## macOS（实测基线：macOS 26.7.1 arm64，2026-11-03）

实测事实（本机构建产物直接检验）：

- `codesign -dv dsheng.app` → `Signature=adhoc`（`flags=0x20002(adhoc,
  linker-signed)`）。即：二进制带有 arm64 必需的 ad-hoc 签名，但**没有
  Developer ID、没有公证**。独立 CLI 二进制同为 adhoc linker-signed。
- `spctl --assess --type execute dsheng.app` → **rejected**（exit 1）。
  也就是说 Gatekeeper 评估不通过：凡带“下载隔离属性”（quarantine）的
  副本都会被拦。

行为与处理：

1. **本机自己构建/解压自己打包的副本**：无隔离属性，双击/终端直接运行
   （开发与验收即此场景）。
2. **从网络下载的 .dmg/.zip**（浏览器下载会自动加隔离属性）：
   - 首次双击 `dsheng.app` → 弹出 Gatekeeper 提示
     （“无法打开'dsheng'，因为无法验证开发者”/“Apple 无法检查其是否
     含恶意软件”一类文案）。
   - 点“完成/好”关闭弹窗 → 打开 **系统设置 → 隐私与安全性**，向下滚动
     到“安全性”区域，会看到关于 dsheng 被拦的记录与 **“仍要打开”**
     按钮 → 点击并用 Touch ID/密码确认 → 再回 Finder 双击启动。
   - 新版 macOS 已移除“右键→打开”的放行入口；以系统设置路径为准。
3. **独立 CLI（终端运行）**：首次执行下载来的 `dsheng` 时，终端会打印
   `killed: 9` 或 Gatekeeper 拒绝信息。处理方式同上（系统设置 → 隐私与
   安全性 → 仍要打开），或换用“安装辅助脚本复制后的副本按提示放行”。
   我们不移除、也不建议任何人对二进制执行
   `xattr -d com.apple.quarantine` —— 那是绕过，不是放行。
4. 企业 MDM 可完全禁用上述入口（`syspolicy` 策略），此时无法运行。

## Windows 10 22H2 / Windows 11（SmartScreen；待真机走查）

> 本节按微软文档化行为编写，尚未在 Windows 真机实测；发布前须按
> RELEASE-CHECKLIST.md 在两版本真机走查后修订（“实测”标注留空处）。

- 桌面 `.msi` / 便携 `.zip`、独立 CLI 均未签名：首次运行（或 SmartScreen
  对“未知发布者”的程序判定）可能出现蓝色弹窗
  **“Windows 已保护你的电脑”**。
- 正常放行路径：弹窗中点 **“更多信息”** → **“仍要运行”**。
- `dsheng.exe`（CLI）从 PowerShell/CMD 运行时，SmartScreen 拦截多见于
  安装器场景；命令行二进制一般直接运行，若被“标记为从网络下载”，
  Windows 可能提示由 Defender 检查——按系统提示操作即可。
- 我们**不**提供 `Unblock-File` 脚步、**不**调用
  `Set-ExecutionPolicy`、**不**关闭 SmartScreen/Defender；安装辅助
  `windows-user-install.ps1` 仅做用户目录复制（被策略拦截时按 README
  手动复制，等效）。
- 组策略（如 `SmartScreen` 强制“阻止”、AppLocker/WDAC）可能拒绝运行，
  且无“仍要运行”入口：受管设备上不可用，属预期。
- 桌面运行还需 **WebView2 Evergreen 运行时**（微软官方组件，非本项目
  分发）。缺失时应用启动前本地检查并提示“缺少 WebView2 运行时”与
  微软官方下载入口（`https://developer.microsoft.com/microsoft-edge/webview2/`）；
  本产品不静默下载、不内嵌安装器（`webviewInstallMode=skip`）。CLI 不
  依赖 WebView2。

## 校验下载完整性（放行之前）

对下载文件先核对 SHA-256（值见 `docs/release/manifest-v<版本>.json`）：

```sh
# macOS
shasum -a 256 dsheng_<版本>_macos_arm64.dmg
# Windows PowerShell
Get-FileHash -Algorithm SHA256 .\dsheng_<版本>_windows_x64.msi
```

哈希不一致即停止，不要放行。
