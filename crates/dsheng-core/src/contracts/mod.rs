//! 冻结合同的 Rust 类型基线（Phase 0，主 Agent 所有权）。
//!
//! 本模块只做两件事：
//! 1. 把 `contracts/v1/**`、四份规格与 `prompts/v1/versions.json` 中已冻结的
//!    字段/枚举/不变量转成 Rust 类型——不重新选择任何业务规则；
//! 2. 提供跨工作包共享的校验函数（Decimal 规范字符串、unit signature、
//!    公式 AST 限制、日期区间、超时/预算组合等）。
//!
//! 各工作包在其自有目录内实现业务逻辑，但对外交换必须使用本模块类型。
//! 若发现合同缺口：报告主 Agent 统一更新合同与版本，不得自行改类型语义。

pub mod cli;
pub mod decimal;
pub mod domain;
pub mod evidence;
pub mod metrics;
pub mod prompts;
pub mod provider;
pub mod settings;
pub mod tasks;

pub use decimal::{canonical_decimal, DecimalPolicyError, INPUT_MAX_FRACTION_DIGITS};
pub use domain::{
    AvailabilityStatus, Cadence, ComparisonType, Coverage, CoverageStatus, DateRange, Direction,
    Granularity, MeasureRole, RecordKind, UnitSignature, ValueState,
};
