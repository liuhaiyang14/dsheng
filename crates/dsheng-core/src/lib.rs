//! dsheng 两表一单本地经营分析工具 —— 共享 Rust 核心。
//!
//! 桌面端（Tauri command）与独立 CLI 都链接本 crate；两者不得分别实现业务逻辑。
//! 模块所有权按主计划 12.3 节冻结：
//!
//! | 模块 | 所有权 | 内容 |
//! | --- | --- | --- |
//! | [`contracts`] | 主 Agent | 冻结合同的 Rust 类型、枚举、不变量（Phase 0 基线） |
//! | [`import`] | 数据与 SQLite 工作包 | 扫描、读取、字段映射、模板识别 |
//! | [`analysis`] | 数据与 SQLite 工作包 | 指标计算、周期、比较、聚合 |
//! | [`repository`] | 数据与 SQLite 工作包 | SQLite 持久化、事务、迁移、任务生命周期 |
//! | [`provider`] | Provider 与提示词工作包 | Provider 客户端、确认门禁、C 方案适配层 |
//! | [`generation`] | Provider 与提示词工作包 | 提示词组装、输入投影、输出校验、会话/轮次 |
//!
//! 修改公共接口/合同必须经主 Agent 更新版本并通知所有工作包。
//!
//! 禁止事项（主计划 12.1）：Mock Provider、假 endpoint、录制回放、自动重试、
//! 伪造模型历史；真实模型请求仅由主 Agent 按用户授权执行。

pub mod contracts;

// ---- 工作包模块（Phase 0 冻结骨架；实现由对应工作包填充） ----

/// 数据与 SQLite 工作包：扫描 / 读取 / 字段映射 / 模板识别。
pub mod import;

/// 数据与 SQLite 工作包：公式安全计算 / 日期聚合 / 同比环比。
pub mod analysis;

/// 数据与 SQLite 工作包：SQLite repository、迁移、任务生命周期与版本快照。
pub mod repository;

/// Provider 与提示词工作包：请求协议、确认门禁、超时/预算、响应解析。
pub mod provider;

/// Provider 与提示词工作包：四种调用组合的提示词组装、输入投影、输出契约校验。
pub mod generation;

/// 桌面/CLI 共用的版本信息（`dsheng --version` 契约）。
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 应用 identifier（交付合同固定值）。
pub const APP_IDENTIFIER: &str = "com.dsheng.desktop";
