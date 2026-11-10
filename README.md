# dsheng

dsheng 是一款面向小型企业和经营团队的本地经营分析工具。它把数据导入、期间分析、经营问题诊断和任务跟进放在同一个桌面应用中，数据保存在本机。

## 当前版本

当前版本为 **v0.1.12**，提供 macOS 13+ Apple Silicon（arm64）安装包：

- [下载 macOS arm64 DMG](https://github.com/liuhaiyang14/dsheng/releases/download/v0.1.12/dsheng_0.1.12_macos_arm64.dmg)
- [查看 v0.1.12 版本清单与校验值](docs/release/manifest-v0.1.12.json)

安装包未签名。首次启动前请阅读[安装与首次启动说明](docs/user-guide/install.md)和[安全提示](docs/release/UNSIGNED.md)。

## 主要功能

- 导入 Excel、CSV 和本地目录，按企业设置的期间粒度查看数据。
- 在本机完成指标计算、期间切换、历史查询和图表展示。
- 使用五问根因分析、五W问题界定或五W二H行动分析生成经营问题报告。
- 根据经营差异生成任务和行动步骤，并支持人工新增、修改、排序和更新进度。
- 模型交互采用发送前确认、流式展示和手动停止；已接收内容不会在未确认时自动保存。
- 报告、任务和数据保存在本机，不会静默上传业务文件。

## 快速开始

1. 下载上面的 DMG，将 `dsheng.app` 拖入“应用程序”。
2. 启动应用，创建企业档案。
3. 配置模型服务（如需使用 AI 功能），导入数据目录并确认期间。
4. 运行本地分析，再按需生成诊断报告或任务单。

更完整的使用说明见[用户手册目录](docs/user-guide/index.md)。

## 从源码构建

环境要求：Node.js 20+、Rust stable、Python 3.11+，以及 macOS 13+（Apple Silicon 构建）。

```bash
npm --prefix src-ui install
npm --prefix src-ui run build
cargo test --workspace
scripts/package/macos.sh
```

打包产物位于 `target/release-dist/`。发布流程和校验方式见[发布与打包说明](docs/release/README.md)。

## 项目文档

- [用户手册](docs/user-guide/index.md)
- [安装与首次启动](docs/user-guide/install.md)
- [发布与打包](docs/release/README.md)
- [完整实施规格](docs/specs/)
- [机器可读合同与验收算例](contracts/v1/)
- [工程交接记录](HANDOFF.md)

## 隐私与许可

业务文件、分析结果和模型配置默认保存在本机。发送给模型前，应用会显示数据范围并要求确认；API Key 只用于本机配置。

本仓库目前未附带许可证。除非另有书面授权，代码和文档保留所有权利。
