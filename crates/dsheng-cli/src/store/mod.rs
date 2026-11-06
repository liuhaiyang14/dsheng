//! CLI 持久化服务（dsheng-core 未提供的 SQLite 接线；core 类型/校验复用）：
//!
//! - `settings`：workspace/period policy/provider 事务与视图（core 缺设置服务）；
//! - `sources`：source_files / mapping 草稿 / manual_inputs / metric 定义 / 冲突；
//! - `analysis`：analyze 编排（core 计算）与读侧视图 / 历史视图；
//! - `generation`：confirmations/generation_sessions/generation_attempts 的
//!   SQLite `GenerationStore`（core 仅提供 trait + 内存实现）；
//! - `diagnosis`：diagnosis_reports/turns 的本地读写（revise/pending cancel）；
//! - `views`：read-views.schema.json 视图构造（taskView 等）。

pub mod analysis;
pub mod diagnosis;
pub mod generation;
pub mod settings;
pub mod sources;
pub mod task_diagnosis_context;
pub mod task_history;
pub mod views;
