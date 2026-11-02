//! 生成会话：提示词组装、输入投影、输出契约校验、诊断会话/轮次（Provider 工作包所有权）。
//!
//! 合同：docs/specs/prompt-context-validation.md（context_policy_version=2、
//! 澄清恢复、更正与矛盾、警告证据合同）、主计划 §8.2–§8.4、
//! contracts/v1/task-input.schema.json、diagnosis-input.schema.json。
//!
//! 硬约束：提示词正文已冻结（见 contracts::prompts），不得修改/重设计；
//! 发现契约缺口报告主 Agent。禁止 Mock Provider/伪造模型响应。

pub mod assemble;
pub mod input;
