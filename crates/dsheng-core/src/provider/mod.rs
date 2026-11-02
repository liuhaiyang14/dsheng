//! Provider 客户端：请求协议、确认门禁、超时/预算、响应提取（Provider 工作包所有权）。
//!
//! 合同：docs/specs/provider-config-delivery.md §2–§3、主计划 §8.1、
//! prompt-context-validation.md §6（C 方案适配层：严格优先抽取＋唯一裸任务对象
//! 本地包装根信封＋envelope_locally_reconstructed 持久化；参考行为
//! tools/extract_provider_json.py --policy adapt）。
//!
//! 硬约束：禁止 Mock Provider、假 endpoint、录制回放、自动重试；
//! 纯逻辑测试可用依赖注入并标明测试层次。

pub mod confirm;
pub mod deadline;
pub mod extract;
pub mod http;
pub mod schema;
