//! HTTP 客户端构造、错误分类与响应体上限（provider-config-delivery.md §1/§3）。
//!
//! - 仅 `POST {base_url}/chat/completions`；URL 校验复用
//!   `contracts::settings::validate_base_url` / `chat_completions_url`
//!   （仅 HTTPS/字面回环 HTTP；禁 userinfo/query/fragment）。
//! - redirect `Policy::none`：3xx 响应直接返回给调用方并按
//!   `REDIRECT_REJECTED` 拒绝（不携 Authorization 改向新 origin）。
//! - 响应体累计上限 2 MiB：`Content-Length` 仅作提前拒绝，实际按解压后
//!   字节流累计，超限终止读取并 `RESPONSE_TOO_LARGE`。
//! - 只读 `choices[0].message.content`；`finish_reason` 为非 stop 显式值 →
//!   `INCOMPLETE_OUTPUT`，缺失/null 放行继续 schema 校验。
//! - 错误分类为纯函数（状态码 / reqwest 错误 → CliErrorCode）。
//!
//! **本模块不发起任何真实网络请求**；`send_once` 仅供主 Agent 在获得
//! 用户授权的真实联调中调用（每 attempt 一条请求、无自动重试）。
//! 测试只覆盖构造/分类/上限纯逻辑（依赖注入，非 Provider 调用）。

use crate::contracts::cli::CliErrorCode;
use crate::contracts::provider::{
    ChatCompletionsRequest, ChatCompletionsResponse, RESPONSE_BODY_MAX_BYTES,
};
use crate::contracts::settings::{chat_completions_url, Budget, Timeouts};
use std::error::Error as _;
use std::io::Read;
use std::time::Duration;

/// 构造 blocking 客户端（rustls TLS；redirect none；connect/response 超时）。
///
/// response 窗口（连接完成 → 完整 body）与 total 预算（preparing → 终态）
/// 由 [`crate::provider::deadline::DeadlineEngine`] 在调用侧执行；
/// 客户端层将单请求上界设为 connect+response（合同组合约束保证
/// total ≥ connect+response，total 由引擎兜底）。
pub fn build_client(
    base_url: &str,
    timeouts: &Timeouts,
    budget: &Budget,
) -> Result<(reqwest::blocking::Client, String), (CliErrorCode, String)> {
    // URL 规则（含 base_url 已是完整路径的提示性拒绝）
    let endpoint = chat_completions_url(base_url).map_err(|reason| {
        (
            CliErrorCode::ProviderUrlInvalid,
            format!("base_url 非法: {reason}"),
        )
    })?;
    crate::contracts::settings::validate_budget_timeouts(budget, timeouts).map_err(|reason| {
        (
            CliErrorCode::InvalidTimeoutBudget,
            format!("超时/预算组合非法: {reason}"),
        )
    })?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(timeouts.connect_seconds as u64))
        .timeout(Duration::from_secs(
            (timeouts.connect_seconds + timeouts.response_seconds) as u64,
        ))
        .build()
        .map_err(|e| {
            (
                CliErrorCode::ProviderUrlInvalid,
                format!("客户端构造失败: {e}"),
            )
        })?;
    Ok((client, endpoint))
}

/// 单次派发（每 attempt 一条；无自动重试）。仅真实联调使用；
/// 本工作包测试不调用此函数。
pub fn send_once(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    api_key: &str,
    request: &ChatCompletionsRequest,
) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
    let body = serde_json::to_vec(request).map_err(|e| {
        (
            CliErrorCode::RequestTooLarge,
            format!("请求序列化失败: {e}"),
        )
    })?;
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .map_err(classify_reqwest_error_pair)?;
    if let Some(len) = response.content_length() {
        if len > RESPONSE_BODY_MAX_BYTES as u64 {
            return Err((
                CliErrorCode::ResponseTooLarge,
                format!("Content-Length {len} 超过 {}", RESPONSE_BODY_MAX_BYTES),
            ));
        }
    }
    let status = response.status();
    if status.is_redirection() {
        return Err((
            CliErrorCode::RedirectRejected,
            "服务端返回重定向，已拒绝（不换 origin）".into(),
        ));
    }
    if !status.is_success() {
        return Err((
            classify_status(status.as_u16()),
            format!("HTTP {}", status.as_u16()),
        ));
    }
    let mut bytes = Vec::new();
    let mut reader = response;
    reader.read_to_end_limited(&mut bytes)?;
    parse_chat_response(&bytes)
}

/// 读取并累计响应体，上限 2 MiB（超限终止读取 → RESPONSE_TOO_LARGE）。
///
/// 纯函数（接受任意 `Read`，测试注入内存流）。
pub fn read_body_limited<R: Read>(mut reader: R) -> Result<Vec<u8>, (CliErrorCode, String)> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 16 * 1_024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > RESPONSE_BODY_MAX_BYTES {
                    return Err((
                        CliErrorCode::ResponseTooLarge,
                        format!("响应体超过 {}", RESPONSE_BODY_MAX_BYTES),
                    ));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(out),
            Err(e) => return Err((CliErrorCode::ProtocolError, format!("响应体读取失败: {e}"))),
        }
    }
}

/// Content-Length 提前拒绝（仅作提前；真实上限按流累计）。
pub fn content_length_precheck(content_length: Option<u64>) -> Result<(), CliErrorCode> {
    if content_length.is_some_and(|len| len > RESPONSE_BODY_MAX_BYTES as u64) {
        return Err(CliErrorCode::ResponseTooLarge);
    }
    Ok(())
}

/// 解析 Chat Completions 响应：只读 choices[0].message.content；
/// finish_reason 缺失/null 放行，非 stop 显式值 → INCOMPLETE_OUTPUT。
pub fn parse_chat_response(body: &[u8]) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
    let parsed: ChatCompletionsResponse = serde_json::from_slice(body)
        .map_err(|e| (CliErrorCode::InvalidJson, format!("响应 JSON 无效: {e}")))?;
    let first = parsed
        .choices
        .first()
        .ok_or_else(|| (CliErrorCode::ProtocolError, "choices 为空".to_string()))?;
    // 正文缺失/非字符串 → PROTOCOL_ERROR
    if first.message.content.is_none() {
        return Err((
            CliErrorCode::ProtocolError,
            "choices[0].message.content 缺失".into(),
        ));
    }
    if let Some(reason) = first.finish_reason.as_deref() {
        if reason != "stop" {
            return Err((
                CliErrorCode::IncompleteOutput,
                format!("finish_reason={reason}（非 stop）"),
            ));
        }
    }
    Ok(parsed)
}

/// HTTP 状态码 → CliErrorCode（3xx 拒绝重定向；402 配额；401/403/404/429/5xx）。
pub fn classify_status(status: u16) -> CliErrorCode {
    match status {
        300..=399 => CliErrorCode::RedirectRejected,
        401 => CliErrorCode::AuthFailed,
        402 => CliErrorCode::QuotaExhausted,
        403 => CliErrorCode::AccessDenied,
        404 => CliErrorCode::EndpointOrModelNotFound,
        429 => CliErrorCode::RateLimited,
        400..=499 => CliErrorCode::ProviderRejectedUnknown,
        500..=599 => CliErrorCode::ProviderServerError,
        _ => CliErrorCode::ProtocolError,
    }
}

/// reqwest 错误 → CliErrorCode：
/// 重定向 / connect+超时 / 读超时 / TLS（rustls 源错误）/ 网络(DNS 等) /
/// body/decode 协议错误。
pub fn classify_reqwest_error(err: &reqwest::Error) -> CliErrorCode {
    if err.is_redirect() {
        return CliErrorCode::RedirectRejected;
    }
    if err.is_timeout() {
        return if err.is_connect() {
            CliErrorCode::ConnectTimeout
        } else {
            CliErrorCode::ResponseTimeout
        };
    }
    if err.is_connect() {
        return if error_chain_mentions_tls(err) {
            CliErrorCode::TlsError
        } else {
            CliErrorCode::NetworkError // DNS/TCP 归网络类
        };
    }
    if err.is_decode() || err.is_body() {
        return CliErrorCode::ProtocolError;
    }
    if err.is_request() {
        return CliErrorCode::NetworkError;
    }
    if let Some(status) = err.status() {
        return classify_status(status.as_u16());
    }
    CliErrorCode::ProviderRejectedUnknown
}

fn classify_reqwest_error_pair(err: reqwest::Error) -> (CliErrorCode, String) {
    (classify_reqwest_error(&err), format!("{err}"))
}

/// 沿 source 链查找 TLS/rustls 特征（错误分类辅助）。
fn error_chain_mentions_tls(err: &reqwest::Error) -> bool {
    let mut source: Option<&dyn std::error::Error> =
        err.source().map(|s| s as &dyn std::error::Error);
    let mut depth = 0;
    while let Some(e) = source {
        let type_name = format!("{e:?}");
        let msg = e.to_string().to_ascii_lowercase();
        if type_name.contains("rustls") || type_name.contains("Tls") || msg.contains("tls") {
            return true;
        }
        source = e.source();
        depth += 1;
        if depth > 8 {
            break;
        }
    }
    false
}

/// 为 [`reqwest::blocking::Response`] 提供的限长读取桥
/// （避免依赖不稳定 trait 方法；语义同 read_body_limited）。
trait ReadToEndLimited {
    fn read_to_end_limited(&mut self, out: &mut Vec<u8>) -> Result<(), (CliErrorCode, String)>;
}

impl ReadToEndLimited for reqwest::blocking::Response {
    fn read_to_end_limited(&mut self, out: &mut Vec<u8>) -> Result<(), (CliErrorCode, String)> {
        let bytes = read_body_limited(self)?;
        *out = bytes;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑测试（构造/分类/上限；非 Provider 调用、无网络请求）。
    use super::*;

    #[test]
    fn client_construction_validates_url_and_budget() {
        let (client, endpoint) = build_client(
            "https://api.example.com/v1/",
            &Timeouts {
                connect_seconds: 15,
                response_seconds: 180,
                total_seconds: 240,
            },
            &Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
        )
        .unwrap();
        let _ = client;
        assert_eq!(endpoint, "https://api.example.com/v1/chat/completions");
        // 回环 HTTP 允许
        let (_, loop_endpoint) = build_client(
            "http://127.0.0.1:8080",
            &Timeouts {
                connect_seconds: 15,
                response_seconds: 180,
                total_seconds: 240,
            },
            &Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
        )
        .unwrap();
        assert_eq!(loop_endpoint, "http://127.0.0.1:8080/chat/completions");
        // 非回环 HTTP 拒绝
        let err = build_client(
            "http://insecure.example.com",
            &Timeouts {
                connect_seconds: 15,
                response_seconds: 180,
                total_seconds: 240,
            },
            &Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
        )
        .unwrap_err();
        assert_eq!(err.0, CliErrorCode::ProviderUrlInvalid);
        // 已是完整路径
        let err2 = build_client(
            "https://api.example.com/v1/chat/completions",
            &Timeouts {
                connect_seconds: 15,
                response_seconds: 180,
                total_seconds: 240,
            },
            &Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
        )
        .unwrap_err();
        assert_eq!(err2.0, CliErrorCode::ProviderUrlInvalid);
        // 非法组合（total < connect+response）
        let err3 = build_client(
            "https://api.example.com/v1",
            &Timeouts {
                connect_seconds: 60,
                response_seconds: 600,
                total_seconds: 240,
            },
            &Budget {
                context_window_tokens: 32_768,
                output_reserve_tokens: 8_192,
            },
        )
        .unwrap_err();
        assert_eq!(err3.0, CliErrorCode::InvalidTimeoutBudget);
    }

    #[test]
    fn body_limit_accumulates_and_rejects() {
        use std::io::Cursor;
        // 恰好 2 MiB 通过
        let ok = vec![b'x'; RESPONSE_BODY_MAX_BYTES];
        assert_eq!(
            read_body_limited(Cursor::new(ok.clone())).unwrap().len(),
            ok.len()
        );
        // 超过一字节拒绝
        let over = vec![b'x'; RESPONSE_BODY_MAX_BYTES + 1];
        let err = read_body_limited(Cursor::new(over)).unwrap_err();
        assert_eq!(err.0, CliErrorCode::ResponseTooLarge);
        // Content-Length 提前拒绝
        assert!(content_length_precheck(None).is_ok());
        assert!(content_length_precheck(Some(2_097_152)).is_ok());
        assert_eq!(
            content_length_precheck(Some(2_097_153)).unwrap_err(),
            CliErrorCode::ResponseTooLarge
        );
    }

    #[test]
    fn response_parse_rules() {
        let ok = br#"{"choices":[{"message":{"content":"{\"schema_version\":\"2\"}"},"finish_reason":"stop"}],"model":"m"}"#;
        let parsed = parse_chat_response(ok).unwrap();
        assert_eq!(
            parsed.choices[0].message.content.as_deref(),
            Some("{\"schema_version\":\"2\"}")
        );
        // finish_reason 缺失 → 放行
        let no_reason = br#"{"choices":[{"message":{"content":"x"}}]}"#;
        assert!(parse_chat_response(no_reason).is_ok());
        // finish_reason null → 放行
        let null_reason = br#"{"choices":[{"message":{"content":"x"},"finish_reason":null}]}"#;
        assert!(parse_chat_response(null_reason).is_ok());
        // finish_reason=length → INCOMPLETE_OUTPUT
        let length = br#"{"choices":[{"message":{"content":"x"},"finish_reason":"length"}]}"#;
        let err = parse_chat_response(length).unwrap_err();
        assert_eq!(err.0, CliErrorCode::IncompleteOutput);
        // JSON 无效
        let bad = b"not json";
        assert_eq!(
            parse_chat_response(bad).unwrap_err().0,
            CliErrorCode::InvalidJson
        );
        // choices 空
        let empty = br#"{"choices":[]}"#;
        assert_eq!(
            parse_chat_response(empty).unwrap_err().0,
            CliErrorCode::ProtocolError
        );
        // 正文缺失
        let no_content = br#"{"choices":[{"message":{"content":null},"finish_reason":"stop"}]}"#;
        assert_eq!(
            parse_chat_response(no_content).unwrap_err().0,
            CliErrorCode::ProtocolError
        );
    }

    #[test]
    fn status_classification_table() {
        let cases = [
            (301u16, CliErrorCode::RedirectRejected),
            (302, CliErrorCode::RedirectRejected),
            (307, CliErrorCode::RedirectRejected),
            (400, CliErrorCode::ProviderRejectedUnknown),
            (401, CliErrorCode::AuthFailed),
            (402, CliErrorCode::QuotaExhausted),
            (403, CliErrorCode::AccessDenied),
            (404, CliErrorCode::EndpointOrModelNotFound),
            (409, CliErrorCode::ProviderRejectedUnknown),
            (422, CliErrorCode::ProviderRejectedUnknown),
            (429, CliErrorCode::RateLimited),
            (500, CliErrorCode::ProviderServerError),
            (502, CliErrorCode::ProviderServerError),
            (503, CliErrorCode::ProviderServerError),
        ];
        for (status, expected) in cases {
            assert_eq!(classify_status(status), expected, "HTTP {status}");
        }
    }
}
