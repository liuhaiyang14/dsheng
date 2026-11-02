//! 离线 JSON Schema 校验层（Draft 2020-12 + format 断言）。
//!
//! 合同：主计划 §8.2（"user 附完整输入合同与输出 schema；提取后的对象
//! 通过输出合同/schema 校验后才能保存"）、prompts/README.md「离线Schema」：
//! 构建时嵌入输入、输出与报告 Schema，按各自 `$id` 建立离线注册表，
//! 跨资源引用用注册 ID，不依赖仓库路径且**不联网取 Schema**。
//!
//! - 注册表覆盖 `contracts/v1` 与 `prompts/v1/schema` 下全部带 `$id` 的
//!   `.schema.json`；retriever 走 `offline()`——注册表缺失的引用直接报错，
//!   绝不回源网络。
//! - `should_validate_formats(true)` 启用 format 断言（uuid / date /
//!   date-time 为 jsonschema 内建实现）。
//! - 引用、quote、ID、状态与合并域等语义不变量不由本层负责，
//!   见 generation 上下文/候选校验模块。
//!
//! 本模块为纯逻辑层：不做 Provider 调用、不读运行时文件
//! （全部 schema 来自构建期 `include_str!` 嵌入）。

use crate::contracts::prompts::{DIAGNOSIS_INPUT_SCHEMA, PROMPT_RESOURCES, TASK_INPUT_SCHEMA};
use crate::contracts::provider::Framework;
use jsonschema::{Draft, Registry, Resource, Validator};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// contracts/v1 下未由 contracts::prompts 嵌入的其余合同 schema
/// （settings / task-edit / task-reconcile；只读引用，不修改合同文件）。
const SETTINGS_SCHEMA: &str = include_str!("../../../../contracts/v1/settings.schema.json");
const TASK_EDIT_SCHEMA: &str = include_str!("../../../../contracts/v1/task-edit.schema.json");
const TASK_RECONCILE_SCHEMA: &str =
    include_str!("../../../../contracts/v1/task-reconcile.schema.json");

/// 输出合同校验错误（`json指针: 消息`，对齐 Python 工具的
/// `e.json_path: e.message` 报告格式；最多保留前 10 条）。
pub type SchemaErrors = Vec<String>;

/// 进程级单例：全部离线校验器（构建期一次编译）。
pub struct ContractValidators {
    /// 任务输出 schema2 根对象（$id: task-output-v2）。
    pub task_output: Validator,
    /// 任务子 schema（`#/$defs/task`，用于 C 方案裸任务判定）。
    pub task_sub: Validator,
    turn_5why: Validator,
    turn_5w: Validator,
    turn_5w2h: Validator,
    /// 诊断报告 schema3（$id: diagnosis-report-v3）。
    pub report: Validator,
    /// 模型输入合同（$id: dsheng-task-input-v2）。
    pub task_input: Validator,
    /// 模型输入合同（$id: dsheng-diagnosis-input-v2）。
    pub diagnosis_input: Validator,
}

static VALIDATORS: OnceLock<ContractValidators> = OnceLock::new();

/// 获取离线合同校验器单例（首次调用编译全部 schema；失败即 panic，
/// 因为嵌入资源与 versions.json 哈希一致性已由构建期测试守卫）。
pub fn contract_validators() -> &'static ContractValidators {
    VALIDATORS.get_or_init(build_validators)
}

fn parse(schema_text: &str) -> serde_json::Value {
    serde_json::from_str(schema_text).expect("嵌入 schema 必须是合法 JSON")
}

/// 全部本地 `$id` schema 的 (id, 文档) 清单。
fn local_schemas() -> BTreeMap<String, serde_json::Value> {
    let mut schemas = BTreeMap::new();
    for (name, content) in PROMPT_RESOURCES {
        if name.starts_with("schema/") {
            schemas.insert(schema_id_of(name).to_string(), parse(content));
        }
    }
    schemas.insert("dsheng-task-input-v2".into(), parse(TASK_INPUT_SCHEMA));
    schemas.insert(
        "dsheng-diagnosis-input-v2".into(),
        parse(DIAGNOSIS_INPUT_SCHEMA),
    );
    schemas.insert(
        "dsheng-settings-operations-v1".into(),
        parse(SETTINGS_SCHEMA),
    );
    schemas.insert("dsheng-task-edit-v1".into(), parse(TASK_EDIT_SCHEMA));
    schemas.insert(
        "dsheng-task-reconcile-v1".into(),
        parse(TASK_RECONCILE_SCHEMA),
    );
    schemas
}

/// 从资源文件名推导注册 $id（schema/*.schema.json 的 $id 与文件名对应，
/// 冻结清单见 versions.json current_resource_hashes）。
fn schema_id_of(resource_name: &str) -> &'static str {
    match resource_name {
        "schema/task-output.schema.json" => "task-output-v2",
        "schema/diagnosis-turn-5why.schema.json" => "diagnosis-turn-5why-v3",
        "schema/diagnosis-turn-5w.schema.json" => "diagnosis-turn-5w-v3",
        "schema/diagnosis-turn-5w2h.schema.json" => "diagnosis-turn-5w2h-v3",
        "schema/diagnosis-report.schema.json" => "diagnosis-report-v3",
        _ => unreachable!("未知 schema 资源 {resource_name}"),
    }
}

fn build_registry(schemas: &BTreeMap<String, serde_json::Value>) -> Registry<'static> {
    Registry::new()
        .draft(Draft::Draft202012)
        .extend(
            schemas
                .iter()
                .map(|(id, value)| (id.as_str(), Resource::from_contents(value.clone()))),
        )
        .expect("本地 $id schema 注册失败（URI 非法）")
        .prepare()
        .expect("本地 $id schema 索引失败（$id 冲突或非法）")
}

fn build_validators() -> ContractValidators {
    let schemas = local_schemas();
    let registry = build_registry(&schemas);
    let build = |id: &str| -> Validator {
        let schema = schemas
            .get(id)
            .unwrap_or_else(|| panic!("本地 schema 清单缺少 {id}"));
        jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .offline()
            .with_registry(&registry)
            .build(schema)
            .unwrap_or_else(|e| panic!("schema {id} 编译失败: {e}"))
    };

    // 任务子 schema：与 Python 参考实现 task_subvalidator 相同的构造方式
    // （复用根 $defs，顶层 $ref 指向 #/$defs/task）。
    let task_sub_schema = serde_json::json!({
        "$defs": schemas
            .get("task-output-v2")
            .and_then(|s| s.get("$defs").cloned())
            .unwrap_or_default(),
        "$ref": "#/$defs/task",
    });
    let registry_for_sub = build_registry(&schemas);
    let task_sub = jsonschema::options()
        .with_draft(Draft::Draft202012)
        .should_validate_formats(true)
        .offline()
        .with_registry(&registry_for_sub)
        .build(&task_sub_schema)
        .expect("任务子 schema 编译失败");

    ContractValidators {
        task_output: build("task-output-v2"),
        task_sub,
        turn_5why: build("diagnosis-turn-5why-v3"),
        turn_5w: build("diagnosis-turn-5w-v3"),
        turn_5w2h: build("diagnosis-turn-5w2h-v3"),
        report: build("diagnosis-report-v3"),
        task_input: build("dsheng-task-input-v2"),
        diagnosis_input: build("dsheng-diagnosis-input-v2"),
    }
}

impl ContractValidators {
    /// 按框架取诊断 turn schema3 校验器。
    pub fn turn(&self, framework: Framework) -> &Validator {
        match framework {
            Framework::Why5 => &self.turn_5why,
            Framework::W5 => &self.turn_5w,
            Framework::W5h2 => &self.turn_5w2h,
        }
    }

    /// 校验并返回错误列表（空 = 通过；最多保留前 10 条）。
    pub fn errors_of(validator: &Validator, instance: &serde_json::Value) -> SchemaErrors {
        let mut errors: SchemaErrors = validator
            .iter_errors(instance)
            .map(|e| format!("{}: {}", e.instance_path(), e))
            .collect();
        errors.truncate(10);
        errors
    }
}

/// C 方案适配层所需的两个合同谓词（任务输出 schema2 根对象 / 裸任务子 schema）。
pub struct TaskOutputChecks;

impl crate::provider::extract::ExtractionChecks for TaskOutputChecks {
    fn root_contract_passes(&self, obj: &serde_json::Value) -> bool {
        contract_validators().task_output.is_valid(obj)
    }
    fn task_subschema_passes(&self, obj: &serde_json::Value) -> bool {
        contract_validators().task_sub.is_valid(obj)
    }
}

fn check(validator: &Validator, obj: &serde_json::Value) -> Result<(), SchemaErrors> {
    let errors = ContractValidators::errors_of(validator, obj);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// 校验任务输出根对象（schema2）。
pub fn validate_task_output(obj: &serde_json::Value) -> Result<(), SchemaErrors> {
    check(&contract_validators().task_output, obj)
}

/// 校验诊断单轮输出（所选框架 turn schema3）。
pub fn validate_diagnosis_turn(
    framework: Framework,
    obj: &serde_json::Value,
) -> Result<(), SchemaErrors> {
    check(contract_validators().turn(framework), obj)
}

/// 校验诊断报告对象（schema3）。
pub fn validate_diagnosis_report(obj: &serde_json::Value) -> Result<(), SchemaErrors> {
    check(&contract_validators().report, obj)
}

/// 校验任务输入合同投影（模型输入 v2）。
pub fn validate_task_input(obj: &serde_json::Value) -> Result<(), SchemaErrors> {
    check(&contract_validators().task_input, obj)
}

/// 校验诊断输入合同投影（模型输入 v2）。
pub fn validate_diagnosis_input(obj: &serde_json::Value) -> Result<(), SchemaErrors> {
    check(&contract_validators().diagnosis_input, obj)
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试：离线注册表与 format 断言（非 Provider 调用）。
    use super::*;

    #[test]
    fn all_local_schemas_compile_offline() {
        // 全部 9 个 $id schema 可编译；跨资源 $ref（diagnosis-input →
        // diagnosis-report-v3）在注册表内解析，不走网络
        let _ = contract_validators();
    }

    #[test]
    fn offline_registry_rejects_unknown_ref() {
        // 注册表检索函数直接报错而非回源（离线 retriever）
        let schema = serde_json::json!({"$ref": "https://example.com/nonexistent.schema.json"});
        let registry = build_registry(&local_schemas());
        let result = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .offline()
            .with_registry(&registry)
            .build(&schema);
        assert!(result.is_err(), "未知 $ref 必须编译失败，不得联网解析");
    }

    #[test]
    fn uuid_format_asserted_on_task_subschema() {
        let v = contract_validators();
        let mut task = full_task();
        assert!(v.task_sub.is_valid(&task), "完整任务对象应通过子 schema");
        task["metric_id"] = serde_json::json!("not-a-uuid");
        assert!(
            !v.task_sub.is_valid(&task),
            "uuid format 必须断言（非法 metric_id 拒绝）"
        );
    }

    /// 合成完整任务对象（与 tests/wp_provider_extract.rs 同构）。
    fn full_task() -> serde_json::Value {
        serde_json::json!({
            "task_key": "candidate-task-1",
            "title": "提升10月合成销售额",
            "summary": "摘要",
            "category": "revenue",
            "metric_id": "00000000-0000-4000-8000-000000000020",
            "source_period": {"from": "2026-09-01", "to": "2026-09-30"},
            "target_period": {"from": "2026-10-01", "to": "2026-10-31"},
            "baseline": {"value": 100.25, "unit": "元"},
            "target_value": 100,
            "unit": "元",
            "owner": null,
            "planned_start": "2026-10-01",
            "due_date": "2026-10-31",
            "steps": [{
                "step_id": "candidate-step-1",
                "order": 1,
                "action": "与负责人确认措施",
                "owner": null,
                "start_date": null,
                "due_date": null,
                "done_criteria": "清单确认",
                "evidence_ids": ["cell:00000000-0000-4000-8000-000000000010:数据:B2"],
                "status": "not_started"
            }],
            "resource_need": null,
            "success_criteria": "不低于100元",
            "evidence_ids": ["cell:00000000-0000-4000-8000-000000000010:数据:B2"],
            "assumptions": [],
            "needs_review": ["步骤执行日期待确认"],
            "smart_check": {
                "overall_goal": {
                    "goal_text": "目标",
                    "criteria": {
                        "S": {"status": "met", "rationale": "r", "evidence_ids": []},
                        "M": {"status": "met", "rationale": "r", "evidence_ids": []},
                        "A": {"status": "needs_confirmation", "rationale": "r", "evidence_ids": []},
                        "R": {"status": "met", "rationale": "r", "evidence_ids": []},
                        "T": {"status": "met", "rationale": "r", "evidence_ids": []}
                    }
                },
                "subgoals": []
            }
        })
    }

    #[test]
    fn date_format_rejects_impossible_calendar_dates() {
        let v = contract_validators();
        // 报告 schema period.from/to 是 date format
        let bad = serde_json::json!({
            "schema_version": "3", "report_id": "r", "generation_session_id": "s",
            "workspace_id": "w", "run_id": "u", "framework": "5why",
            "scope": {"problem_statement": "p", "focus_metric_ids": []},
            "period": {"from": "2026-02-30", "to": "2026-03-01", "period_policy_version": "v1"},
            "revision_no": 1, "summary": "", "source_manifest": [], "findings": [],
            "facts_and_hypotheses": [], "evidence_refs": [], "clarification_questions": [],
            "warnings": [], "stop_reason": null, "can_continue": true,
            "created_at": "2026-11-02T00:00:00Z", "updated_at": "2026-11-02T00:00:00Z",
            "analysis_revision_no": 1,
            "analysis_input_fingerprint": "a".repeat(64)
        });
        let errors = ContractValidators::errors_of(&v.report, &bad).join("; ");
        assert!(
            errors.contains("date"),
            "2026-02-30 非真实日历日期: {errors}"
        );
    }

    #[test]
    fn datetime_format_asserted() {
        let v = contract_validators();
        let mut bad = minimal_report();
        bad["created_at"] = serde_json::json!("2026-11-02 00:00:00"); // 非 RFC3339
        let errors = ContractValidators::errors_of(&v.report, &bad).join("; ");
        assert!(
            errors.contains("date-time"),
            "date-time format 必须断言: {errors}"
        );
    }

    fn minimal_report() -> serde_json::Value {
        serde_json::json!({
            "schema_version": "3", "report_id": "r", "generation_session_id": "s",
            "workspace_id": "w", "run_id": "u", "framework": "5why",
            "scope": {"problem_statement": "p", "focus_metric_ids": []},
            "period": {"from": "2026-10-01", "to": "2026-10-31", "period_policy_version": "v1"},
            "revision_no": 1, "summary": "", "source_manifest": [], "findings": [],
            "facts_and_hypotheses": [], "evidence_refs": [], "clarification_questions": [],
            "warnings": [], "stop_reason": null, "can_continue": true,
            "created_at": "2026-11-02T00:00:00Z", "updated_at": "2026-11-02T00:00:00Z",
            "analysis_revision_no": 1,
            "analysis_input_fingerprint": "a".repeat(64)
        })
    }
}
