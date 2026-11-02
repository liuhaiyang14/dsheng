# 工作包任务书：发布与跨平台

启动条件：集成构建可运行（桌面 + CLI 同库）。

## 所有权
- `scripts/package/**`、`install/**`、`.github/workflows/**`、`docs/release/**`、
  构建/包检查测试。
- 不负责：签名、绕过安全提示、公开发布执行（须用户授权）、真实数据、业务核心。

## 交付范围（用例 12、18、35、37、39、70）
1. 构建：macOS arm64/x64（.app→.dmg+便携 zip）、Windows x64（.msi+便携 zip，
   不产 .exe 安装器）；桌面包内含同版本 dsheng CLI 与 migration；identifier
   com.dsheng.desktop；MACOSX_DEPLOYMENT_TARGET=13.0；WebView2 Evergreen
   webviewInstallMode=skip + 启动前缺失提示（不静默安装）。
2. 独立 CLI 安装文件：三 target 二进制 + macOS .sh / Windows .ps1 用户级辅助
   （默认不提权、不改 PATH、不绕过 Gatekeeper/SmartScreen）+ 版本/校验清单。
3. 便携语义：解压运行仍用同一用户数据目录（macOS ~/Library/Application
   Support/com.dsheng.desktop；Windows %LOCALAPPDATA%\...）。
4. 清单与校验：统一版本 manifest（四类产物×平台/架构×版本×SHA-256）；
   发布前 secrets scan 拒绝真实 XLSX/PPTX/SQLite/Key/本机绝对路径（用例 30）。
5. CI：跨平台构建矩阵+测试；CI 不调用 Provider。
6. unsigned 说明文档（docs/release/）：按目标 OS 实测后编写，不称"通过认证"。

## 边界
GitHub Release 与二进制发布须产品验收通过+用户显式授权，本包只准备产物与清单。
