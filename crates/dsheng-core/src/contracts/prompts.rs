//! 冻结提示词资源嵌入与版本/哈希管理（prompts/v1/versions.json）。
//!
//! 提示词正文按用户批准版本管理（2026-11-06 授权新增中文展示偏好）；
//! 业务规则与输出形状沿用 2026-11-02 冻结版本，不在组装层临时追加指令；
//! 每次构建/测试都校验嵌入资源 SHA-256 与 versions.json 的
//! current_resource_hashes 一致；发现不一致即构建失败。
//!
//! 组装顺序（主计划 §8.2，禁止混拼）：
//! - task：system = task/system.md；user = task/user.md + 完整输入合同 + 输出 schema。
//! - diagnosis：system = diagnosis/system.md + 单个 framework 文件；
//!   user = diagnosis/user.md + 完整输入合同 + 所选 turn schema。

use crate::contracts::provider::{Framework, Workflow};
use sha2::{Digest, Sha256};

/// 资源文件名 → 嵌入内容。键与 versions.json current_resource_hashes 完全一致。
pub const PROMPT_RESOURCES: &[(&str, &str)] = &[
    (
        "diagnosis/framework-5w.md",
        include_str!("../../prompts/v1/diagnosis/framework-5w.md"),
    ),
    (
        "diagnosis/framework-5w2h.md",
        include_str!("../../prompts/v1/diagnosis/framework-5w2h.md"),
    ),
    (
        "diagnosis/framework-5why.md",
        include_str!("../../prompts/v1/diagnosis/framework-5why.md"),
    ),
    (
        "diagnosis/system.md",
        include_str!("../../prompts/v1/diagnosis/system.md"),
    ),
    (
        "diagnosis/user.md",
        include_str!("../../prompts/v1/diagnosis/user.md"),
    ),
    (
        "schema/diagnosis-report.schema.json",
        include_str!("../../prompts/v1/schema/diagnosis-report.schema.json"),
    ),
    (
        "schema/diagnosis-turn-5w.schema.json",
        include_str!("../../prompts/v1/schema/diagnosis-turn-5w.schema.json"),
    ),
    (
        "schema/diagnosis-turn-5w2h.schema.json",
        include_str!("../../prompts/v1/schema/diagnosis-turn-5w2h.schema.json"),
    ),
    (
        "schema/diagnosis-turn-5why.schema.json",
        include_str!("../../prompts/v1/schema/diagnosis-turn-5why.schema.json"),
    ),
    (
        "schema/task-output.schema.json",
        include_str!("../../prompts/v1/schema/task-output.schema.json"),
    ),
    (
        "task/system.md",
        include_str!("../../prompts/v1/task/system.md"),
    ),
    (
        "task/user.md",
        include_str!("../../prompts/v1/task/user.md"),
    ),
];

/// versions.json（嵌入快照；当前接受状态由其 current_acceptance.state 描述）。
pub const VERSIONS_JSON: &str = include_str!("../../prompts/v1/versions.json");

/// 输入合同（模型输入 schema，版本 2）。
pub const TASK_INPUT_SCHEMA: &str = include_str!("../../../../contracts/v1/task-input.schema.json");
pub const DIAGNOSIS_INPUT_SCHEMA: &str =
    include_str!("../../../../contracts/v1/diagnosis-input.schema.json");

pub fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    let out = hasher.finalize();
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// versions.json 的 current_resource_hashes 视图。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct VersionsDoc {
    current_resource_hashes: std::collections::BTreeMap<String, String>,
    current_input_contract_hashes: std::collections::BTreeMap<String, String>,
}

/// 校验全部嵌入资源与输入合同哈希。任何不一致返回 (键, 期望, 实际) 列表。
pub fn verify_frozen_hashes() -> Result<(), Vec<(String, String, String)>> {
    let doc: VersionsDoc =
        serde_json::from_str(VERSIONS_JSON).expect("内置 versions.json 必须可解析");
    let mut failures = Vec::new();
    for (key, content) in PROMPT_RESOURCES {
        let actual = sha256_hex(content);
        match doc.current_resource_hashes.get(*key) {
            Some(expected) if *expected == actual => {}
            Some(expected) => failures.push((key.to_string(), expected.clone(), actual)),
            None => failures.push((key.to_string(), "<missing>".into(), actual)),
        }
    }
    for (key, content) in [
        (
            "contracts/v1/diagnosis-input.schema.json",
            DIAGNOSIS_INPUT_SCHEMA,
        ),
        ("contracts/v1/task-input.schema.json", TASK_INPUT_SCHEMA),
    ] {
        let actual = sha256_hex(content);
        match doc.current_input_contract_hashes.get(key) {
            Some(expected) if *expected == actual => {}
            Some(expected) => failures.push((key.to_string(), expected.clone(), actual)),
            None => failures.push((key.to_string(), "<missing>".into(), actual)),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures)
    }
}

/// 当前版本常量（与 versions.json 及本轮用户授权的语言修订一致）。
pub const TASK_PROMPT_VERSION: &str = "2.4";
pub const DIAGNOSIS_PROMPT_VERSION: &str = "2.1";
pub const TASK_OUTPUT_SCHEMA_VERSION: &str = "2";
pub const DIAGNOSIS_SCHEMA_VERSION: &str = "3";
pub const INPUT_CONTRACT_VERSION: &str = "2";
pub const CONTEXT_POLICY_VERSION: &str = "2";

pub fn prompt_version(workflow: Workflow) -> &'static str {
    match workflow {
        Workflow::Task => TASK_PROMPT_VERSION,
        Workflow::Diagnosis => DIAGNOSIS_PROMPT_VERSION,
    }
}

/// 获取资源内容。
pub fn resource(name: &str) -> Option<&'static str> {
    PROMPT_RESOURCES
        .iter()
        .find(|(k, _)| *k == name)
        .map(|(_, v)| *v)
}

/// 一个工作流的提示词组装清单（正文拼接顺序）。
pub struct AssemblyManifest {
    /// system 消息由这些文件按序拼接。
    pub system_files: Vec<&'static str>,
    /// user 模板文件。
    pub user_file: &'static str,
    /// 输出 schema 资源名。
    pub response_schema: &'static str,
    pub prompt_version: &'static str,
    pub schema_version: &'static str,
}

pub fn assembly_manifest(workflow: Workflow, framework: Option<Framework>) -> AssemblyManifest {
    match (workflow, framework) {
        (Workflow::Task, _) => AssemblyManifest {
            system_files: vec!["task/system.md"],
            user_file: "task/user.md",
            response_schema: "schema/task-output.schema.json",
            prompt_version: TASK_PROMPT_VERSION,
            schema_version: TASK_OUTPUT_SCHEMA_VERSION,
        },
        (Workflow::Diagnosis, Some(f)) => {
            let framework_file = match f {
                Framework::Why5 => "diagnosis/framework-5why.md",
                Framework::W5 => "diagnosis/framework-5w.md",
                Framework::W5h2 => "diagnosis/framework-5w2h.md",
            };
            let schema = match f {
                Framework::Why5 => "schema/diagnosis-turn-5why.schema.json",
                Framework::W5 => "schema/diagnosis-turn-5w.schema.json",
                Framework::W5h2 => "schema/diagnosis-turn-5w2h.schema.json",
            };
            AssemblyManifest {
                system_files: vec!["diagnosis/system.md", framework_file],
                user_file: "diagnosis/user.md",
                response_schema: schema,
                prompt_version: DIAGNOSIS_PROMPT_VERSION,
                schema_version: DIAGNOSIS_SCHEMA_VERSION,
            }
        }
        (Workflow::Diagnosis, None) => panic!("diagnosis 必须显式选择框架"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frozen_resources_match_hashes() {
        if let Err(failures) = verify_frozen_hashes() {
            for (key, expected, actual) in failures {
                eprintln!("hash mismatch {key}: expected {expected} actual {actual}");
            }
            panic!("冻结合同资源哈希不一致——禁止修改提示词/合同正文");
        }
    }

    #[test]
    fn assembly_order() {
        let task = assembly_manifest(Workflow::Task, None);
        assert_eq!(task.system_files, vec!["task/system.md"]);
        let d = assembly_manifest(Workflow::Diagnosis, Some(Framework::Why5));
        assert_eq!(
            d.system_files,
            vec!["diagnosis/system.md", "diagnosis/framework-5why.md"]
        );
    }
}
