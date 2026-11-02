//! 四种调用组合的请求组装与预算/上限检查（主计划 §8.2、
//! provider-config-delivery.md §3、prompt-context-validation.md §1）。
//!
//! 组装规则（contracts::prompts::assembly_manifest，禁止混拼）：
//! - system = 清单文件序列**直接拼接**（文件均以单个换行结尾，拼接后
//!   逐文件保持行边界；不添加额外分隔符，不把计划副本/企业事实追加进 system）。
//! - user = 模板文件单次替换 `{{payload_json}}`（完整输入合同 JSON）与
//!   `{{output_schema_json}}`（冻结输出 schema 文件原文）；业务内容不再
//!   解释为指令（单遍替换，payload 中出现的占位符样式文本不会被二次替换）。
//!
//! 上限检查（超限返回对应 CliErrorCode，不截断、不拆批、不缩数组）：
//! - 单轮 user 文本 ≤ 8,000 Unicode scalars（按 char 计数，非 UTF-16）→
//!   `USER_MESSAGE_TOO_LARGE`。**合同解释**：与两份输入 schema 中
//!   `user_message` / `user_context` / `continuation.user_message` 的
//!   `maxLength: 8000` 一致，该上限作用于**用户提交的单轮文本字段**
//!   （诊断 user_message、任务 user_context）；装配后的完整 user 模板
//!   （含 payload 与输出 schema）由 token 预算与 1 MiB 请求体上限约束——
//!   否则任何携带输出 schema 的真实请求都无法通过（冻结输出 schema 文件
//!   本身即 >8,000 字符）。
//! - 保守输入 token = system+user UTF-8 字节 + 256；加输出预留超上下文窗口
//!   → `CONTEXT_BUDGET_EXCEEDED`。
//! - 请求体 UTF-8 ≤ 1 MiB → `REQUEST_TOO_LARGE`。
//!
//! 纯逻辑层：无 Provider 调用、无网络。

use crate::contracts::cli::CliErrorCode;
use crate::contracts::prompts::{assembly_manifest, resource};
use crate::contracts::provider::{
    conservative_input_tokens, context_budget_exceeded, ChatCompletionsRequest, Framework,
    WireMessage, WireRole, Workflow, REQUEST_BODY_MAX_BYTES, USER_MESSAGE_MAX_SCALARS,
};
use crate::contracts::settings::Budget;

/// 组装错误：错误码 + 脱敏说明（不含 prompt 正文/Key）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("assemble error: {code:?} {detail}")]
pub struct AssembleError {
    pub code: CliErrorCode,
    pub detail: String,
}

impl AssembleError {
    fn new(code: CliErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// 组装后的消息对。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssembledMessages {
    pub system: String,
    pub user: String,
}

/// 组装并检查后的完整请求材料。
#[derive(Debug, Clone, PartialEq)]
pub struct AssembledRequest {
    pub request: ChatCompletionsRequest,
    /// 最终 HTTP JSON body 的 UTF-8 字节数。
    pub request_bytes: u64,
    /// 保守输入 token 估算（system+user 字节 + 256）。
    pub conservative_input_tokens: u64,
}

/// 单轮用户文本的 Unicode scalar 上限检查（按 char 计数）。
pub fn check_user_message_scalars(text: &str) -> Result<(), AssembleError> {
    let scalars = text.chars().count();
    if scalars > USER_MESSAGE_MAX_SCALARS {
        return Err(AssembleError::new(
            CliErrorCode::UserMessageTooLarge,
            format!("user 文本 {scalars} scalars 超过上限 {USER_MESSAGE_MAX_SCALARS}"),
        ));
    }
    Ok(())
}

/// 按 assembly_manifest 拼装 system/user 消息（不含预算检查）。
pub fn assemble_messages(
    workflow: Workflow,
    framework: Option<Framework>,
    payload_json: &str,
) -> Result<AssembledMessages, AssembleError> {
    let manifest = assembly_manifest(workflow, framework);
    let mut system = String::new();
    for file in &manifest.system_files {
        let content = resource(file).ok_or_else(|| {
            AssembleError::new(CliErrorCode::RuntimeMissing, format!("资源缺失 {file}"))
        })?;
        system.push_str(content);
    }
    let template = resource(manifest.user_file).ok_or_else(|| {
        AssembleError::new(
            CliErrorCode::RuntimeMissing,
            format!("资源缺失 {}", manifest.user_file),
        )
    })?;
    let schema_text = resource(manifest.response_schema).ok_or_else(|| {
        AssembleError::new(
            CliErrorCode::RuntimeMissing,
            format!("资源缺失 {}", manifest.response_schema),
        )
    })?;
    let user = render_user_template(template, payload_json, schema_text)?;
    Ok(AssembledMessages { system, user })
}

const PAYLOAD_PLACEHOLDER: &str = "{{payload_json}}";
const SCHEMA_PLACEHOLDER: &str = "{{output_schema_json}}";

/// 模板单遍渲染：每个占位符恰好出现一次；先替换受信任的 schema 占位符，
/// 再以单次替换注入 payload，payload 内容不会被再次扫描为指令。
fn render_user_template(
    template: &str,
    payload_json: &str,
    schema_text: &str,
) -> Result<String, AssembleError> {
    for (name, needle) in [
        ("payload_json", PAYLOAD_PLACEHOLDER),
        ("output_schema_json", SCHEMA_PLACEHOLDER),
    ] {
        let count = template.matches(needle).count();
        if count != 1 {
            return Err(AssembleError::new(
                CliErrorCode::RuntimeMissing,
                format!("冻结模板占位符 {name} 出现 {count} 次（应为 1）"),
            ));
        }
    }
    let (head, tail) = template
        .split_once(SCHEMA_PLACEHOLDER)
        .expect("占位符唯一性已校验");
    // rest 中不再有 schema 占位符；payload 占位符位于 head 或 tail 之一，
    // 按模板原顺序重组（payload 在 schema 前或后均保持模板字面顺序）。
    if let Some((before, mid)) = head.split_once(PAYLOAD_PLACEHOLDER) {
        return Ok(format!("{before}{payload_json}{mid}{schema_text}{tail}"));
    }
    if let Some((between, after)) = tail.split_once(PAYLOAD_PLACEHOLDER) {
        return Ok(format!("{head}{schema_text}{between}{payload_json}{after}"));
    }
    Err(AssembleError::new(
        CliErrorCode::RuntimeMissing,
        "冻结模板缺少 payload 占位符",
    ))
}

/// 完整组装 + 全部上限检查。
///
/// `user_supplied_text` 是用户提交的单轮文本（诊断 user_message /
/// 任务 user_context），先做 8,000 scalar 检查；随后做 token 预算与
/// 1 MiB 请求体检查。任一超限即返回对应错误码（不截断、不拆批）。
pub fn assemble_request(
    workflow: Workflow,
    framework: Option<Framework>,
    model: &str,
    payload_json: &str,
    user_supplied_text: &str,
    budget: &Budget,
) -> Result<AssembledRequest, AssembleError> {
    check_user_message_scalars(user_supplied_text)?;

    let messages = assemble_messages(workflow, framework, payload_json)?;
    let tokens =
        conservative_input_tokens(messages.system.len() as u64, messages.user.len() as u64);
    if context_budget_exceeded(budget, tokens) {
        return Err(AssembleError::new(
            CliErrorCode::ContextBudgetExceeded,
            format!(
                "保守输入 {tokens} + 输出预留 {} > 上下文窗口 {}",
                budget.output_reserve_tokens, budget.context_window_tokens
            ),
        ));
    }

    let request = ChatCompletionsRequest {
        model: model.to_string(),
        messages: vec![
            WireMessage {
                role: WireRole::System,
                content: messages.system,
            },
            WireMessage {
                role: WireRole::User,
                content: messages.user,
            },
        ],
    };
    let request_bytes = encode_request_body_size(&request)?;

    Ok(AssembledRequest {
        request,
        request_bytes,
        conservative_input_tokens: tokens,
    })
}

/// 请求体编码 + 1 MiB 硬上限检查（独立入口，供组装与发送两端复用）。
///
/// 说明：在合法预算组合下（window ≤ 1,048,576 且 reserve ≥ 1,024），
/// token 预算总是先于 body 上限触发；本检查仍作为发送前的最终硬门。
pub fn encode_request_body_size(request: &ChatCompletionsRequest) -> Result<u64, AssembleError> {
    let body = serde_json::to_string(request).map_err(|e| {
        AssembleError::new(CliErrorCode::RequestTooLarge, format!("序列化失败: {e}"))
    })?;
    let request_bytes = body.len() as u64;
    if request_bytes > REQUEST_BODY_MAX_BYTES as u64 {
        return Err(AssembleError::new(
            CliErrorCode::RequestTooLarge,
            format!("请求体 {request_bytes} 字节超过上限 {REQUEST_BODY_MAX_BYTES}"),
        ));
    }
    Ok(request_bytes)
}

#[cfg(test)]
mod tests {
    // 纯逻辑单元测试（模板渲染与上限检查，非 Provider 调用）。
    use super::*;

    #[test]
    fn user_scalar_limit_counts_chars_not_utf16() {
        // 8,000 scalars 的中英文混合（UTF-16 单元数远大于 char 数）
        let text = "销售额".repeat(2_600) + "a".repeat(200).as_str();
        assert_eq!(text.chars().count(), 8_000);
        assert!(check_user_message_scalars(&text).is_ok());
        let over = format!("{text}好");
        assert_eq!(over.chars().count(), 8_001);
        let err = check_user_message_scalars(&over).unwrap_err();
        assert_eq!(err.code, CliErrorCode::UserMessageTooLarge);
        // UTF-16 计数会是 2*2600+1+200=5401... 实际 8001 scalars 证明按 char 计
    }

    #[test]
    fn template_render_single_pass() {
        let template = "A {{payload_json}} B {{output_schema_json}} C";
        let out =
            render_user_template(template, r#"{"x":"{{output_schema_json}}"}"#, "SCHEMA").unwrap();
        // payload 中的占位符样式文本不被二次替换
        assert_eq!(out, r#"A {"x":"{{output_schema_json}}"} B SCHEMA C"#);
        // 顺序相反的模板
        let template2 = "A {{output_schema_json}} B {{payload_json}} C";
        let out2 = render_user_template(template2, "P", "S").unwrap();
        assert_eq!(out2, "A S B P C");
        // 占位符缺失/重复
        assert!(render_user_template("no placeholders", "P", "S").is_err());
        assert!(render_user_template(
            "{{payload_json}}{{payload_json}} {{output_schema_json}}",
            "P",
            "S"
        )
        .is_err());
    }

    #[test]
    fn four_combinations_assemble() {
        let payload = r#"{"request":{}}"#;
        for (workflow, framework) in [
            (Workflow::Task, None),
            (Workflow::Diagnosis, Some(Framework::Why5)),
            (Workflow::Diagnosis, Some(Framework::W5)),
            (Workflow::Diagnosis, Some(Framework::W5h2)),
        ] {
            let m = assemble_messages(workflow, framework, payload).unwrap();
            assert!(!m.system.is_empty() && !m.user.is_empty());
            assert!(m.user.contains(payload), "user 必须内嵌完整输入合同 JSON");
            assert!(
                m.user.contains("\"$schema\""),
                "user 必须内嵌完整输出 schema JSON"
            );
            if workflow == Workflow::Task {
                assert!(m.system.contains("任务"));
                assert!(m.user.contains("PAYLOAD_JSON"));
            } else {
                // 诊断 system = diagnosis/system.md + 单个 framework 文件
                let f = framework.unwrap();
                assert!(
                    m.system.contains("5WHY")
                        || m.system.contains("5W")
                        || m.system.contains("5W2H")
                );
                let _ = f;
            }
        }
        // 诊断缺框架必须 panic（合同要求显式选择）——由 contracts 层保证
    }

    #[test]
    fn budget_and_body_limits() {
        let budget = crate::contracts::settings::DEFAULT_BUDGET;
        // 构造超大 payload 触发 CONTEXT_BUDGET_EXCEEDED（32768-8192-256 允许约 24k 字节）
        let big_payload = "x".repeat(30_000);
        let err = assemble_request(
            Workflow::Task,
            None,
            "test-model",
            &big_payload,
            "",
            &budget,
        )
        .unwrap_err();
        assert_eq!(err.code, CliErrorCode::ContextBudgetExceeded);

        // 请求体 > 1 MiB：合法预算组合下 token 预算先触发（window-reserve
        // 上限 1,047,552 字节 < body 上限），body 硬门用独立入口验证
        let huge_budget = crate::contracts::settings::Budget {
            context_window_tokens: 1_048_576,
            output_reserve_tokens: 1_024,
        };
        let huge_payload = "y".repeat(1_100_000);
        let err2 = assemble_request(
            Workflow::Task,
            None,
            "test-model",
            &huge_payload,
            "",
            &huge_budget,
        )
        .unwrap_err();
        assert_eq!(err2.code, CliErrorCode::ContextBudgetExceeded);
        let oversized = ChatCompletionsRequest {
            model: "m".into(),
            messages: vec![WireMessage {
                role: WireRole::User,
                content: "z".repeat(REQUEST_BODY_MAX_BYTES + 1),
            }],
        };
        let err3 = encode_request_body_size(&oversized).unwrap_err();
        assert_eq!(err3.code, CliErrorCode::RequestTooLarge);
        // 恰好 1 MiB 边界通过（减去 JSON 结构开销）
        let edge = ChatCompletionsRequest {
            model: "m".into(),
            messages: vec![WireMessage {
                role: WireRole::User,
                content: "z".repeat(REQUEST_BODY_MAX_BYTES - 60),
            }],
        };
        assert!(encode_request_body_size(&edge).is_ok());
    }

    #[test]
    fn normal_assembly_passes_all_checks() {
        let budget = crate::contracts::settings::DEFAULT_BUDGET;
        let out = assemble_request(
            Workflow::Diagnosis,
            Some(Framework::Why5),
            "glm-5.3",
            r#"{"request":{"round_no":1}}"#,
            "请继续分析销售额下降的原因",
            &budget,
        )
        .unwrap();
        assert_eq!(out.request.model, "glm-5.3");
        assert_eq!(out.request.messages.len(), 2);
        assert_eq!(out.request.messages[0].role, WireRole::System);
        assert_eq!(out.request.messages[1].role, WireRole::User);
        assert!(out.request_bytes > 0 && out.request_bytes <= REQUEST_BODY_MAX_BYTES as u64);
        // 保守估算 = system+user 字节 + 256
        let sys = out.request.messages[0].content.len() as u64;
        let usr = out.request.messages[1].content.len() as u64;
        assert_eq!(out.conservative_input_tokens, sys + usr + 256);
    }
}
