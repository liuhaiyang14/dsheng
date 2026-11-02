//! S5 HTTP 层集成测试（纯逻辑层：构造/分类/上限；非 Provider 调用、
//! **不发起任何真实网络请求**，包括对 localhost）。
//!
//! 覆盖用例 58 的错误分类清单（状态码、finish_reason、正文缺失、
//! 响应体上限、reqwest 错误状态回退）与用例 57 的 deadline 引擎
//! 时钟注入（lib 单测 provider::deadline）。

use dsheng_core::contracts::cli::CliErrorCode;
use dsheng_core::contracts::settings::{Budget, Timeouts};
use dsheng_core::provider::http::{
    build_client, classify_reqwest_error, classify_status, content_length_precheck,
    parse_chat_response, read_body_limited,
};
use std::io::Cursor;

const TIMEOUTS: Timeouts = Timeouts {
    connect_seconds: 15,
    response_seconds: 180,
    total_seconds: 240,
};
const BUDGET: Budget = Budget {
    context_window_tokens: 32_768,
    output_reserve_tokens: 8_192,
};

#[test]
fn client_construction_rules() {
    // 仅 HTTPS；HTTP 仅字面回环；完整路径提示；禁止 userinfo/query
    assert!(build_client("https://api.example.com/v1", &TIMEOUTS, &BUDGET).is_ok());
    assert!(build_client("http://localhost:9100", &TIMEOUTS, &BUDGET).is_ok());
    assert!(build_client("http://127.0.0.1:9100/base", &TIMEOUTS, &BUDGET).is_ok());
    // 合同缺口（已记录，不改合同）：contracts::settings::validate_base_url 的
    // host 拆分按 ':' 取首段，"[::1]" 被拆成 "["，IPv6 字面回环被误拒——
    // 以冻结实现行为为准，见完成报告"合同缺口"。
    let cases = [
        "http://insecure.example.com",
        "http://[::1]:9100",
        "https://api.example.com/v1/chat/completions",
        "https://user:pass@api.example.com",
        "https://api.example.com/v1?q=1",
        "https://api.example.com/v1#frag",
        "ftp://api.example.com",
    ];
    for url in cases {
        let err = build_client(url, &TIMEOUTS, &BUDGET).unwrap_err();
        assert_eq!(err.0, CliErrorCode::ProviderUrlInvalid, "{url}");
    }
    // 预算/超时组合非法
    let bad_timeouts = Timeouts {
        connect_seconds: 15,
        response_seconds: 180,
        total_seconds: 100, // < connect+response
    };
    assert_eq!(
        build_client("https://api.example.com/v1", &bad_timeouts, &BUDGET)
            .unwrap_err()
            .0,
        CliErrorCode::InvalidTimeoutBudget
    );
    let bad_budget = Budget {
        context_window_tokens: 8_192,
        output_reserve_tokens: 8_192, // reserve > context/2
    };
    assert_eq!(
        build_client("https://api.example.com/v1", &TIMEOUTS, &bad_budget)
            .unwrap_err()
            .0,
        CliErrorCode::InvalidTimeoutBudget
    );
}

#[test]
fn error_classification_full_catalog_case58() {
    // 状态码全清单
    let status_cases = [
        (301u16, CliErrorCode::RedirectRejected),
        (302, CliErrorCode::RedirectRejected),
        (304, CliErrorCode::RedirectRejected),
        (307, CliErrorCode::RedirectRejected),
        (308, CliErrorCode::RedirectRejected),
        (400, CliErrorCode::ProviderRejectedUnknown),
        (401, CliErrorCode::AuthFailed),
        (402, CliErrorCode::QuotaExhausted),
        (403, CliErrorCode::AccessDenied),
        (404, CliErrorCode::EndpointOrModelNotFound),
        (405, CliErrorCode::ProviderRejectedUnknown),
        (409, CliErrorCode::ProviderRejectedUnknown),
        (413, CliErrorCode::ProviderRejectedUnknown),
        (422, CliErrorCode::ProviderRejectedUnknown),
        (429, CliErrorCode::RateLimited),
        (451, CliErrorCode::ProviderRejectedUnknown),
        (500, CliErrorCode::ProviderServerError),
        (501, CliErrorCode::ProviderServerError),
        (502, CliErrorCode::ProviderServerError),
        (503, CliErrorCode::ProviderServerError),
        (504, CliErrorCode::ProviderServerError),
        (529, CliErrorCode::ProviderServerError),
    ];
    for (status, expected) in status_cases {
        assert_eq!(classify_status(status), expected, "HTTP {status}");
    }

    // 正文/协议类
    let invalid_json: &[u8] = b"<html>gateway error</html>";
    assert_eq!(
        parse_chat_response(invalid_json).unwrap_err().0,
        CliErrorCode::InvalidJson
    );
    let empty_choices = br#"{"choices":[]}"#;
    assert_eq!(
        parse_chat_response(empty_choices).unwrap_err().0,
        CliErrorCode::ProtocolError
    );
    let missing_content = br#"{"choices":[{"message":{}}]}"#;
    assert_eq!(
        parse_chat_response(missing_content).unwrap_err().0,
        CliErrorCode::ProtocolError
    );
    // finish_reason 显式非 stop 值全类
    for reason in ["length", "tool_calls", "content_filter", "function_call"] {
        let body = format!(
            r#"{{"choices":[{{"message":{{"content":"x"}},"finish_reason":"{reason}"}}]}}"#
        );
        let err = parse_chat_response(body.as_bytes()).unwrap_err();
        assert_eq!(
            err.0,
            CliErrorCode::IncompleteOutput,
            "finish_reason={reason}"
        );
    }
    // finish_reason 缺失/null 放行（继续 schema 校验，不凭缺失断言失败）
    assert!(parse_chat_response(br#"{"choices":[{"message":{"content":"x"}}]}"#).is_ok());
    assert!(parse_chat_response(
        br#"{"choices":[{"message":{"content":"x"},"finish_reason":null}]}"#
    )
    .is_ok());

    // 响应体上限：累计流 + Content-Length 提前拒绝
    assert_eq!(
        read_body_limited(Cursor::new(vec![b'a'; 2_097_153]))
            .unwrap_err()
            .0,
        CliErrorCode::ResponseTooLarge
    );
    assert!(read_body_limited(Cursor::new(vec![b'a'; 2_097_152])).is_ok());
    assert_eq!(
        content_length_precheck(Some(3_000_000)).unwrap_err(),
        CliErrorCode::ResponseTooLarge
    );
}

#[test]
fn reqwest_error_status_fallback_classification() {
    // 无网络构造 reqwest::blocking::Response（http::Response 转换），
    // error_for_status 产生带状态码的 reqwest::Error，验证分类回退路径。
    for (status, expected) in [
        (401u16, CliErrorCode::AuthFailed),
        (403, CliErrorCode::AccessDenied),
        (404, CliErrorCode::EndpointOrModelNotFound),
        (429, CliErrorCode::RateLimited),
        (500, CliErrorCode::ProviderServerError),
    ] {
        let raw = http::Response::builder()
            .status(status)
            .body(Vec::new())
            .unwrap();
        let response = reqwest::blocking::Response::from(raw);
        let err = response.error_for_status().unwrap_err();
        assert_eq!(classify_reqwest_error(&err), expected, "HTTP {status}");
    }
}
