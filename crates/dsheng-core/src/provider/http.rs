//! HTTP 客户端构造、取消与响应读取。
//!
//! 2026-11-05 用户决策：生产路径 [`send_once_cancellable`] 不施加连接、
//! 响应、空闲、总时长或响应大小上限；等待模型完成/报错或用户停止。
//! 旧 blocking / 限额纯函数保留用于兼容历史测试，不再被生产派发调用。
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
    ChatCompletionsRequest, ChatCompletionsResponse, Choice, ResponseMessage,
    REQUEST_BODY_MAX_BYTES, RESPONSE_BODY_MAX_BYTES, STREAMING_BODY_MAX_BYTES,
};
use crate::contracts::settings::{chat_completions_url, Budget, Timeouts};
use crate::provider::deadline::{MonotonicClock, SystemClock};
use crate::provider::stream::SseDeltaParser;
use std::error::Error as _;
use std::io::Read;
use std::time::Duration;
use std::{future::Future, pin::Pin, time::Instant};

type TransportResult<T> = Result<T, (CliErrorCode, String)>;

fn cancelled() -> (CliErrorCode, String) {
    (
        CliErrorCode::Cancelled,
        "已停止本次生成；远端计费由模型服务决定".into(),
    )
}

/// 等待单个网络 future，同时观察取消。取消后 future 被 drop，立即停止本地
/// 连接/响应读取；100ms 只决定取消检查频率，不是输出超时或闲置终止条件。
async fn cancellable_wait<F: Future>(
    future: F,
    should_cancel: &dyn Fn() -> bool,
) -> TransportResult<F::Output> {
    if should_cancel() {
        return Err(cancelled());
    }
    tokio::pin!(future);
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if should_cancel() { return Err(cancelled()); }
            }
            value = &mut future => {
                if should_cancel() { return Err(cancelled()); }
                return Ok(value);
            }
        }
    }
}

type BodyChunk<'a> = Pin<Box<dyn Future<Output = TransportResult<Option<Vec<u8>>>> + 'a>>;

/// 本地字节流接口：生产实现为 reqwest body，测试只注入内存字节及等待 future。
trait ResponseBody {
    fn next_chunk(&mut self) -> BodyChunk<'_>;
}

impl ResponseBody for reqwest::Response {
    fn next_chunk(&mut self) -> BodyChunk<'_> {
        Box::pin(async move {
            self.chunk()
                .await
                .map(|chunk| chunk.map(|bytes| bytes.to_vec()))
                .map_err(classify_reqwest_error_pair)
        })
    }
}

async fn read_response_cancellable(
    body: &mut impl ResponseBody,
    stream: bool,
    started_at: Instant,
    on_chunk: &mut StreamProgress<'_>,
    should_cancel: &dyn Fn() -> bool,
) -> TransportResult<ChatCompletionsResponse> {
    let mut parser = SseDeltaParser::new();
    let mut bytes = Vec::new();
    let mut total_bytes = 0u64;
    let mut chunk_seq = 0u64;
    loop {
        let chunk = cancellable_wait(body.next_chunk(), should_cancel).await??;
        let Some(chunk) = chunk else {
            break;
        };
        total_bytes = total_bytes.saturating_add(chunk.len() as u64);
        chunk_seq = chunk_seq.saturating_add(1);
        if stream {
            parser.feed(&chunk);
        } else {
            bytes.extend_from_slice(&chunk);
        }
        on_chunk(
            chunk_seq,
            total_bytes,
            started_at.elapsed().as_millis().min(i64::MAX as u128) as i64,
        );
        if should_cancel() {
            return Err(cancelled());
        }
        if stream && parser.is_terminal() {
            break;
        }
    }
    if stream {
        let outcome = parser.finish();
        if let Some(message) = parser.provider_error() {
            return Err((CliErrorCode::ProviderRejectedUnknown, message.to_owned()));
        }
        stream_result_to_response(StreamingReadResult {
            content: outcome.content,
            finish_reason: outcome.finish_reason,
            total_bytes,
            chunk_count: chunk_seq,
            skipped_invalid_lines: outcome.skipped_invalid_lines,
        })
    } else {
        // 非流式成功状态也可能包装 Provider error，而不是 choices。
        if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(error) = value.get("error").filter(|error| !error.is_null()) {
                let message = error
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| error.as_str())
                    .unwrap_or("模型服务返回错误");
                return Err((CliErrorCode::ProviderRejectedUnknown, message.to_owned()));
            }
        }
        parse_chat_response(&bytes)
    }
}

/// 所有生成共用的生产网络入口。每 attempt 一次 POST，无自动重试。
///
/// 必须从普通同步线程或 spawn_blocking 调用。current-thread runtime 在当前
/// 线程轮询，因此取消检查允许借用非 Send 的 SQLite 连接。异步请求和 body
/// 读取均可被取消打断，首响应前也不例外。请求输入预算仍由原校验链处理；
/// 未发送 max_tokens 等输出截断参数，未设置任何本地响应时长/字节上限。
pub fn send_once_cancellable(
    base_url: &str,
    api_key: &str,
    request: &ChatCompletionsRequest,
    stream: bool,
    on_chunk: &mut StreamProgress<'_>,
    should_cancel: &dyn Fn() -> bool,
) -> TransportResult<ChatCompletionsResponse> {
    if should_cancel() {
        return Err(cancelled());
    }
    let endpoint = chat_completions_url(base_url).map_err(|reason| {
        (
            CliErrorCode::ProviderUrlInvalid,
            format!("base_url 非法: {reason}"),
        )
    })?;
    let body = wire_body_with_stream(request, stream)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            (
                CliErrorCode::NetworkError,
                format!("网络运行时启动失败: {error}"),
            )
        })?;
    runtime.block_on(async {
        // reqwest async Client 的 connect/read/total timeout 默认均为 None。
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(classify_reqwest_error_pair)?;
        let started_at = Instant::now();
        let accept = if stream {
            "text/event-stream"
        } else {
            "application/json"
        };
        let send = client
            .post(endpoint)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .header("Accept", accept)
            .body(body)
            .send();
        let mut response = cancellable_wait(send, should_cancel)
            .await?
            .map_err(classify_reqwest_error_pair)?;
        let status = response.status();
        if !status.is_success() {
            return Err((
                classify_status(status.as_u16()),
                format!("HTTP {}", status.as_u16()),
            ));
        }
        // Content-Length 是传输元数据，不是输出限额；按实际 body 完整读取。
        read_response_cancellable(&mut response, stream, started_at, on_chunk, should_cancel).await
    })
}

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

/// 构造流式 blocking 客户端（rustls TLS；redirect none；connect＋逐读空闲超时）。
///
/// 与非流式 [`build_client`] 的差别（provider-config-delivery.md §3 增量）：
/// - **相邻数据块空闲上限** = response_seconds。reqwest blocking 0.12.28
///   无 `ClientBuilder::read_timeout`（仅 async ClientBuilder 提供；已核实
///   源码 blocking/client.rs），socket 层等效方案：blocking `.timeout(d)` 的
///   实现是"每次 `read()` 单独的 park 超时窗口"
///   （blocking/response.rs: `wait::timeout(self.body_mut().read(buf), timeout)`，
///   deadline 在每次 read 调用重算）——即正文的逐块空闲窗口 = d；窗口内
///   任何数据到达即正常返回。空闲触发时 read 返回错误链含
///   `error::TimedOut` 的 io 错误（`reqwest::Error::is_timeout()` 为真），
///   由 [`read_streaming_loop`] 归类 RESPONSE_TIMEOUT。
///   已知取舍（报告注明）：同一窗口也作用于 send 阶段（connect＋响应头），
///   即"连接→首字节"整体（而非仅空闲段）受 response_seconds 约束，比合同
///   （connect 单独计）略严；总时长不受影响（由 deadline 循环兜底）。
/// - **不设请求级总 timeout**：总时长由调用侧以 total deadline 循环控制
///   （每次 read 前检查，到即 `timed_out`；见 [`send_once_streaming`]）。
pub fn build_streaming_client(
    base_url: &str,
    timeouts: &Timeouts,
    budget: &Budget,
) -> Result<(reqwest::blocking::Client, String), (CliErrorCode, String)> {
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
        .timeout(Duration::from_secs(timeouts.response_seconds as u64))
        .build()
        .map_err(|e| {
            (
                CliErrorCode::ProviderUrlInvalid,
                format!("流式客户端构造失败: {e}"),
            )
        })?;
    Ok((client, endpoint))
}

/// 组装最终 HTTP 请求体：stream=false＝原样 model/messages；
/// stream=true＝追加 `"stream": true`（§1 增量；wire 仅增此一键）。
pub fn wire_body_with_stream(
    request: &ChatCompletionsRequest,
    stream: bool,
) -> Result<Vec<u8>, (CliErrorCode, String)> {
    let body = if stream {
        let mut value = serde_json::to_value(request).map_err(|e| {
            (
                CliErrorCode::RequestTooLarge,
                format!("请求序列化失败: {e}"),
            )
        })?;
        if let Some(obj) = value.as_object_mut() {
            obj.insert("stream".into(), serde_json::Value::Bool(true));
        }
        serde_json::to_vec(&value)
    } else {
        serde_json::to_vec(request)
    }
    .map_err(|e| {
        (
            CliErrorCode::RequestTooLarge,
            format!("请求序列化失败: {e}"),
        )
    })?;
    if body.len() > REQUEST_BODY_MAX_BYTES {
        return Err((
            CliErrorCode::RequestTooLarge,
            format!("请求体 {} 超过 {}", body.len(), REQUEST_BODY_MAX_BYTES),
        ));
    }
    Ok(body)
}

/// 流式读取循环累计结果（喂给 [`stream_result_to_response`] 转非流式响应）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingReadResult {
    /// 全部 delta.content 拼接后的完整正文。
    pub content: String,
    /// finish_reason 终值（最后非 null；缺省 None）。
    pub finish_reason: Option<String>,
    /// 累计接收字节（含 SSE 封装/事件行；守 2 MiB 上限）。
    pub total_bytes: u64,
    /// 读取块数（read() 返回次数；progress 回调 chunk_seq 与之对应）。
    pub chunk_count: u64,
    /// JSON 无效被跳过的 data 行数（SseDeltaParser 终止上报）。
    pub skipped_invalid_lines: usize,
}

/// 流式读取进度回调参数：(chunk_seq, total_bytes, elapsed_ms)。每 chunk 触发。
pub type StreamProgress<'a> = dyn FnMut(u64, u64, i64) + 'a;

/// 流式读取循环（纯逻辑；reader/clock 依赖注入，无网络）。
///
/// - `idle_timeout_ms`：相邻数据块空闲上限（= response_seconds）。生产由
///   客户端逐读空闲窗口在 socket 层强制（read 返回超时 io 错误 →
///   RESPONSE_TIMEOUT，见 [`is_read_idle_timeout`]）；本循环再以注入时钟
///   在每块到达时独立兜底（块间隔超限 → 迟到块丢弃并 RESPONSE_TIMEOUT）。
/// - `total_deadline_at_ms`：total 预算绝对时刻（preparing 起算）。
///   **每次 read 前检查**，到即 TOTAL_TIMEOUT（timed_out）。
/// - `max_bytes`：累计字节上限（2 MiB，解压后字节流累计；超即
///   RESPONSE_TOO_LARGE 终止读取）。
/// - `on_chunk`：每块触发 `(chunk_seq, total_bytes, elapsed_ms)`。
/// - 解析器见 [DONE] 提前停止；EOF 收尾由调用方 `parser.finish()`。
pub fn read_streaming_loop<R: Read>(
    mut reader: R,
    clock: &mut dyn MonotonicClock,
    idle_timeout_ms: i64,
    total_deadline_at_ms: i64,
    max_bytes: usize,
    parser: &mut SseDeltaParser,
    on_chunk: &mut StreamProgress,
) -> Result<StreamingReadResult, (CliErrorCode, String)> {
    let started_at = clock.now_millis();
    let mut last_data_at = started_at;
    let mut total_bytes: u64 = 0;
    let mut chunk_seq: u64 = 0;
    let mut buffer = [0u8; 8 * 1_024]; // 8KB 逐块读取
    loop {
        // total deadline：每次 read 前检查，到即 timed_out（阻塞读本身由
        // socket 层逐读窗口兜底，两层共同保证不无限等待）
        if clock.now_millis() >= total_deadline_at_ms {
            return Err((
                CliErrorCode::TotalTimeout,
                format!(
                    "总时长预算耗尽（elapsed {}ms ≥ deadline {}ms）",
                    clock.now_millis() - started_at,
                    total_deadline_at_ms - started_at
                ),
            ));
        }
        match reader.read(&mut buffer) {
            Ok(0) => break, // EOF
            Ok(n) => {
                let arrived_at = clock.now_millis();
                // 到达时再核 total（块到达晚于两个 deadline 时 total 优先：
                // 总预算是硬顶）
                if arrived_at >= total_deadline_at_ms {
                    return Err((
                        CliErrorCode::TotalTimeout,
                        format!(
                            "总时长预算耗尽（elapsed {}ms ≥ deadline {}ms）",
                            arrived_at - started_at,
                            total_deadline_at_ms - started_at
                        ),
                    ));
                }
                // 相邻数据块空闲上限：本块到达时刻与上一块之差超限 →
                // RESPONSE_TIMEOUT（该迟到块丢弃；socket 层在真实网络上
                // 会先于本检查在逐读窗口内切断，此处为独立兜底）
                if arrived_at - last_data_at > idle_timeout_ms {
                    return Err((
                        CliErrorCode::ResponseTimeout,
                        format!(
                            "相邻数据块空闲 {}ms 超过上限 {}ms",
                            arrived_at - last_data_at,
                            idle_timeout_ms
                        ),
                    ));
                }
                total_bytes += n as u64;
                if total_bytes > max_bytes as u64 {
                    return Err((
                        CliErrorCode::ResponseTooLarge,
                        format!("流式响应累计 {total_bytes} 字节超过 {max_bytes}"),
                    ));
                }
                parser.feed(&buffer[..n]);
                chunk_seq += 1;
                last_data_at = arrived_at;
                on_chunk(chunk_seq, total_bytes, arrived_at - started_at);
                if parser.is_done() {
                    break; // [DONE] 后不再读（迟到数据忽略）
                }
            }
            // socket 层空闲超时（生产=reqwest blocking 逐读窗口触发；
            // io::Error(Other) 内嵌 reqwest::Error::is_timeout()）
            Err(e) if is_read_idle_timeout(&e) => {
                return Err((
                    CliErrorCode::ResponseTimeout,
                    format!("等待数据块超时（空闲上限 {idle_timeout_ms}ms）: {e}"),
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                return Err((
                    CliErrorCode::ProtocolError,
                    format!("流式响应读取失败: {e}"),
                ))
            }
        }
    }
    let outcome = parser.finish();
    Ok(StreamingReadResult {
        content: outcome.content,
        finish_reason: outcome.finish_reason,
        total_bytes,
        chunk_count: chunk_seq,
        skipped_invalid_lines: outcome.skipped_invalid_lines,
    })
}

/// read 空闲超时识别：直连 io TimedOut/WouldBlock（socket 层），或
/// reqwest blocking 逐读窗口触发的 `io::Error(Other, reqwest::Error)`
/// （错误源链含 `error::TimedOut` → `is_timeout()` 为真）。
fn is_read_idle_timeout(e: &std::io::Error) -> bool {
    if matches!(
        e.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        return true;
    }
    e.get_ref()
        .and_then(|payload| payload.downcast_ref::<reqwest::Error>())
        .is_some_and(reqwest::Error::is_timeout)
}

/// 流式累计 → 非流式响应（与非流式 send_once 完全相同的正文完整性检查：
/// 正文缺失/空 → PROTOCOL_ERROR；finish_reason 非 stop → INCOMPLETE_OUTPUT；
/// 缺失/null 放行）。
pub fn stream_result_to_response(
    result: StreamingReadResult,
) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
    if result.content.is_empty() {
        return Err((
            CliErrorCode::ProtocolError,
            "choices[0].delta.content 缺失（流式正文为空）".into(),
        ));
    }
    if let Some(reason) = result.finish_reason.as_deref() {
        if reason != "stop" {
            return Err((
                CliErrorCode::IncompleteOutput,
                format!("finish_reason={reason}（非 stop）"),
            ));
        }
    }
    Ok(ChatCompletionsResponse {
        choices: vec![Choice {
            message: ResponseMessage {
                content: Some(result.content),
            },
            finish_reason: result.finish_reason,
        }],
        model: None,
    })
}

/// 单次流式派发（每 attempt 一条；无自动重试）。仅真实联调使用；
/// 本工作包测试不调用此函数的网络路径（读取循环以依赖注入覆盖）。
///
/// - 请求体 = model/messages ＋ `"stream": true`；
/// - 重定向拒绝/Content-Length 提前拒绝/状态码分类与非流式同一逻辑
///   （4xx/5xx 在流开始前走既有分类）；
/// - 逐 8KB 块读取喂 [`SseDeltaParser`]，累计字节守 2 MiB；
/// - `on_chunk` 每 chunk 触发 `(chunk_seq, total_bytes, elapsed_ms)`；
/// - 返回类型与非流式 [`send_once`] 相同（完整正文＋finish_reason），
///   后续提取/校验链零改动。
pub fn send_once_streaming(
    client: &reqwest::blocking::Client,
    endpoint: &str,
    api_key: &str,
    request: &ChatCompletionsRequest,
    timeouts: &Timeouts,
    total_started_at_ms: i64,
    on_chunk: &mut StreamProgress,
) -> Result<ChatCompletionsResponse, (CliErrorCode, String)> {
    let body = wire_body_with_stream(request, true)?;
    let response = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
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
    let total_deadline_at = total_started_at_ms + timeouts.total_seconds as i64 * 1_000;
    let mut clock = SystemClock;
    let mut parser = SseDeltaParser::new();
    let result = read_streaming_loop(
        response,
        &mut clock,
        timeouts.response_seconds as i64 * 1_000,
        total_deadline_at,
        STREAMING_BODY_MAX_BYTES,
        &mut parser,
        on_chunk,
    )?;
    stream_result_to_response(result)
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

    /// 模拟异步字节到达，不打开socket、不模拟Provider或重放模型输出。
    struct AsyncBody {
        chunks: std::collections::VecDeque<(Duration, Vec<u8>)>,
        pending_after_chunks: bool,
        reads: usize,
    }

    impl ResponseBody for AsyncBody {
        fn next_chunk(&mut self) -> BodyChunk<'_> {
            self.reads += 1;
            let next = self.chunks.pop_front();
            let pending = self.pending_after_chunks;
            Box::pin(async move {
                match next {
                    Some((delay, bytes)) => {
                        tokio::time::sleep(delay).await;
                        Ok(Some(bytes))
                    }
                    None if pending => std::future::pending().await,
                    None => Ok(None),
                }
            })
        }
    }

    struct PendingUntilDropped(std::rc::Rc<std::cell::Cell<bool>>);
    impl Future for PendingUntilDropped {
        type Output = ();
        fn poll(self: Pin<&mut Self>, _: &mut std::task::Context<'_>) -> std::task::Poll<()> {
            std::task::Poll::Pending
        }
    }
    impl Drop for PendingUntilDropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn cancellation_drops_a_pending_network_future_before_any_response() {
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        let started = tokio::time::Instant::now();
        let result = cancellable_wait(PendingUntilDropped(dropped.clone()), &|| {
            started.elapsed() >= Duration::from_millis(250)
        })
        .await;
        assert_eq!(result.unwrap_err().0, CliErrorCode::Cancelled);
        assert!(dropped.get(), "取消必须drop未完成的网络future");
        assert!(started.elapsed() <= Duration::from_millis(350));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn pre_cancelled_request_never_polls_the_network_future() {
        let result = cancellable_wait(async { panic!("不能派发已停止请求") }, &|| true).await;
        assert_eq!(result.unwrap_err().0, CliErrorCode::Cancelled);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn receiving_stream_can_be_stopped_while_its_next_chunk_never_arrives() {
        let mut body = AsyncBody {
            chunks: [(Duration::ZERO, sse("part", None))].into(),
            pending_after_chunks: true,
            reads: 0,
        };
        let started = tokio::time::Instant::now();
        let mut progress = Vec::new();
        let result = read_response_cancellable(
            &mut body,
            true,
            Instant::now(),
            &mut |seq, bytes, _| progress.push((seq, bytes)),
            &|| started.elapsed() >= Duration::from_millis(250),
        )
        .await;
        assert_eq!(result.unwrap_err().0, CliErrorCode::Cancelled);
        assert_eq!(progress.len(), 1);
        assert_eq!(body.reads, 2);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn stream_can_outlast_old_total_and_idle_limits_without_any_size_cap() {
        let content = "x".repeat(RESPONSE_BODY_MAX_BYTES + 1);
        let mut body = AsyncBody {
            chunks: [
                (Duration::from_secs(2_000), sse(&content, None)),
                (Duration::from_secs(2_000), sse("tail", Some("stop"))),
            ]
            .into(),
            pending_after_chunks: true,
            reads: 0,
        };
        let started = tokio::time::Instant::now();
        let mut progress = Vec::new();
        let result = read_response_cancellable(
            &mut body,
            true,
            Instant::now(),
            &mut |seq, bytes, _| progress.push((seq, bytes)),
            &|| false,
        )
        .await
        .unwrap();
        assert_eq!(
            result.choices[0].message.content.as_ref().unwrap(),
            &(content + "tail")
        );
        assert_eq!(body.reads, 2, "finish_reason停止，无需等EOF或DONE");
        assert!(started.elapsed() >= Duration::from_secs(4_000));
        assert!(progress.last().unwrap().1 > RESPONSE_BODY_MAX_BYTES as u64);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn non_streaming_large_body_is_complete_and_progress_is_observable() {
        let content = "x".repeat(RESPONSE_BODY_MAX_BYTES + 1);
        let json =
            serde_json::json!({"choices":[{"message":{"content":content},"finish_reason":"stop"}]})
                .to_string();
        let mut body = AsyncBody {
            chunks: [(Duration::ZERO, json.into_bytes())].into(),
            pending_after_chunks: false,
            reads: 0,
        };
        let mut observed = 0;
        let result = read_response_cancellable(
            &mut body,
            false,
            Instant::now(),
            &mut |_, bytes, _| observed = bytes,
            &|| false,
        )
        .await
        .unwrap();
        assert_eq!(
            result.choices[0].message.content.as_deref(),
            Some(content.as_str())
        );
        assert!(observed > RESPONSE_BODY_MAX_BYTES as u64);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn explicit_provider_stream_error_finishes_without_waiting_for_disconnect() {
        for error in [
            "data: {\"error\":{\"message\":\"provider rejected\"}}\n\n",
            "event: error\ndata: {\"message\":\"provider rejected\"}\n\n",
            "event: error\ndata: provider rejected\n\n",
        ] {
            let mut body = AsyncBody {
                chunks: [(Duration::ZERO, error.as_bytes().to_vec())].into(),
                pending_after_chunks: true,
                reads: 0,
            };
            let err = read_response_cancellable(
                &mut body,
                true,
                Instant::now(),
                &mut |_, _, _| {},
                &|| false,
            )
            .await
            .unwrap_err();
            assert_eq!(
                err,
                (
                    CliErrorCode::ProviderRejectedUnknown,
                    "provider rejected".into()
                )
            );
            assert_eq!(body.reads, 1);
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn provider_length_finish_remains_an_explicit_incomplete_output_error() {
        let mut body = AsyncBody {
            chunks: [(Duration::ZERO, sse("partial", Some("length")))].into(),
            pending_after_chunks: true,
            reads: 0,
        };
        let err =
            read_response_cancellable(&mut body, true, Instant::now(), &mut |_, _, _| {}, &|| {
                false
            })
            .await
            .unwrap_err();
        assert_eq!(err.0, CliErrorCode::IncompleteOutput);
        assert_eq!(body.reads, 1);
    }

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

    // ---- 流式（S3）：纯逻辑依赖注入测试（脚本 reader＋伪时钟；无网络） ----

    use crate::provider::stream::SseDeltaParser;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// 伪时钟共享句柄（reader 到达一块数据时推进时间）。
    #[derive(Clone)]
    struct SharedTime(Rc<RefCell<i64>>);
    struct FakeSharedClock(SharedTime);
    impl MonotonicClock for FakeSharedClock {
        fn now_millis(&self) -> i64 {
            *self.0 .0.borrow()
        }
    }

    /// 脚本 reader：每块数据前置一段"等待耗时"（毫秒）后到达；大于 8KB
    /// 读取缓冲的数据按多次 read 返回（同一块只推进一次时间，模拟 socket
    /// 内核缓冲语义）；数据耗尽返回 Ok(0)（EOF）。reads 记录实际读取次数
    /// （断言"超时即停"）。
    struct ScriptedReader {
        chunks: VecDeque<(i64, Vec<u8>)>,
        pending: Vec<u8>,
        pending_pos: usize,
        time: SharedTime,
        reads: Rc<std::cell::Cell<usize>>,
    }
    impl Read for ScriptedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.reads.set(self.reads.get() + 1);
            if self.pending_pos >= self.pending.len() {
                match self.chunks.pop_front() {
                    Some((wait_ms, data)) => {
                        *self.time.0.borrow_mut() += wait_ms;
                        self.pending = data;
                        self.pending_pos = 0;
                    }
                    None => return Ok(0),
                }
            }
            let n = (self.pending.len() - self.pending_pos).min(buf.len());
            buf[..n].copy_from_slice(&self.pending[self.pending_pos..self.pending_pos + n]);
            self.pending_pos += n;
            Ok(n)
        }
    }

    fn sse(text: &str, reason: Option<&str>) -> Vec<u8> {
        let mut line = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{text}\"}}{}",
            reason
                .map(|r| format!(",\"finish_reason\":\"{r}\""))
                .unwrap_or_default()
        );
        line.push_str("}]}\n");
        line.into_bytes()
    }

    #[allow(clippy::type_complexity)]
    fn run_stream(
        chunks: Vec<(i64, Vec<u8>)>,
        idle_ms: i64,
        deadline_at: i64,
        start_at: i64,
    ) -> (
        Result<StreamingReadResult, (CliErrorCode, String)>,
        Vec<(u64, u64, i64)>,
        Rc<std::cell::Cell<usize>>,
    ) {
        let time = SharedTime(Rc::new(RefCell::new(start_at)));
        let reads = Rc::new(std::cell::Cell::new(0usize));
        let reader = ScriptedReader {
            chunks: chunks.into(),
            pending: Vec::new(),
            pending_pos: 0,
            time: time.clone(),
            reads: reads.clone(),
        };
        let mut clock = FakeSharedClock(time);
        let mut parser = SseDeltaParser::new();
        let mut progress: Vec<(u64, u64, i64)> = Vec::new();
        let result = read_streaming_loop(
            reader,
            &mut clock,
            idle_ms,
            deadline_at,
            RESPONSE_BODY_MAX_BYTES,
            &mut parser,
            &mut |seq, bytes, ms| progress.push((seq, bytes, ms)),
        );
        (result, progress, reads)
    }

    #[test]
    fn streaming_wire_body_adds_stream_flag_only() {
        let request = ChatCompletionsRequest {
            model: "m1".into(),
            messages: vec![],
        };
        let plain = wire_body_with_stream(&request, false).unwrap();
        assert_eq!(plain, br#"{"model":"m1","messages":[]}"#);
        let streamed = wire_body_with_stream(&request, true).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&streamed).unwrap();
        assert_eq!(value["stream"], serde_json::json!(true));
        assert_eq!(value["model"], serde_json::json!("m1"));
        assert_eq!(
            value["messages"],
            serde_json::json!([]),
            "wire 仅增 stream 一键，model/messages 原样"
        );
    }

    #[test]
    fn streaming_idle_below_limit_completes() {
        // 相邻 chunk 间隔 < idle 不超时：idle=30s，每块间隔 29.9s
        let chunk_bodies = [
            sse("第一", None),
            sse("第二", None),
            sse("", Some("stop")),
            b"data: [DONE]\n".to_vec(),
        ];
        let total_len: usize = chunk_bodies.iter().map(|c| c.len()).sum();
        let chunks = vec![
            (0, chunk_bodies[0].clone()),
            (29_900, chunk_bodies[1].clone()),
            (29_900, chunk_bodies[2].clone()),
            (29_900, chunk_bodies[3].clone()),
        ];
        let (result, progress, reads) = run_stream(chunks, 30_000, 1_000_000, 0);
        let ok = result.unwrap();
        assert_eq!(ok.content, "第一第二");
        assert_eq!(ok.finish_reason.as_deref(), Some("stop"));
        assert_eq!(ok.chunk_count, 4);
        assert_eq!(ok.total_bytes as usize, total_len);
        assert_eq!(reads.get(), 4, "[DONE] 后不再读取");
        // progress：每 chunk 触发 (seq, bytes, elapsed)
        let c2 = chunk_bodies[0].len() + chunk_bodies[1].len();
        let c3 = c2 + chunk_bodies[2].len();
        assert_eq!(
            progress,
            vec![
                (1, chunk_bodies[0].len() as u64, 0),
                (2, c2 as u64, 29_900),
                (3, c3 as u64, 59_800),
                (4, total_len as u64, 89_700),
            ]
        );
    }

    #[test]
    fn streaming_idle_exceeded_times_out_once() {
        // chunk 间隔 > idle（31s > 30s）→ RESPONSE_TIMEOUT，且只此一次即终止
        let chunks = vec![
            (0, sse("一", None)),
            (31_000, sse("二", None)),
            (0, sse("三", None)),
        ];
        let (result, _progress, reads) = run_stream(chunks, 30_000, 1_000_000, 0);
        let err = result.unwrap_err();
        assert_eq!(err.0, CliErrorCode::ResponseTimeout);
        assert_eq!(reads.get(), 2, "超时后不再读取（只读了前两块）");
    }

    #[test]
    fn streaming_total_deadline_times_out() {
        // total deadline 到即 timed_out：deadline=1000ms，第二块到达时已 2000ms
        let chunks = vec![(0, sse("一", None)), (2_000, sse("二", None))];
        let (result, _progress, _reads) = run_stream(chunks, 60_000, 1_000, 0);
        assert_eq!(result.unwrap_err().0, CliErrorCode::TotalTimeout);
        // total 优先于 idle（同时满足时 total 更早暴露）
        let chunks = vec![(0, sse("一", None)), (120_000, sse("二", None))];
        let (result, _progress, _reads) = run_stream(chunks, 30_000, 1_000, 0);
        assert_eq!(result.unwrap_err().0, CliErrorCode::TotalTimeout);
    }

    #[test]
    fn streaming_body_cap_2mib() {
        // 累计字节超过 2 MiB → RESPONSE_TOO_LARGE（终止读取）
        let big = vec![b'x'; 1_048_576]; // 1 MiB 非法行（计 skipped，不影响上限判定）
        let chunks = vec![(0, big.clone()), (0, big.clone()), (0, big)];
        let (result, _progress, _reads) = run_stream(chunks, 60_000, 10_000_000, 0);
        assert_eq!(result.unwrap_err().0, CliErrorCode::ResponseTooLarge);
    }

    #[test]
    fn streaming_socket_read_timeout_maps_to_response_timeout() {
        // socket 层 read_timeout（reqwest read_timeout 空闲触发）→ RESPONSE_TIMEOUT
        struct TimedOutReader;
        impl Read for TimedOutReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "operation timed out",
                ))
            }
        }
        let mut clock = crate::provider::deadline::FakeClock::new(0);
        let mut parser = SseDeltaParser::new();
        let err = read_streaming_loop(
            TimedOutReader,
            &mut clock,
            30_000,
            1_000_000,
            RESPONSE_BODY_MAX_BYTES,
            &mut parser,
            &mut |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(err.0, CliErrorCode::ResponseTimeout);
    }

    #[test]
    fn stream_result_to_response_mirrors_nonstreaming_checks() {
        let ok = stream_result_to_response(StreamingReadResult {
            content: "{\"k\":1}".into(),
            finish_reason: Some("stop".into()),
            total_bytes: 10,
            chunk_count: 2,
            skipped_invalid_lines: 1,
        })
        .unwrap();
        assert_eq!(ok.choices[0].message.content.as_deref(), Some("{\"k\":1}"));
        assert_eq!(ok.choices[0].finish_reason.as_deref(), Some("stop"));
        // finish_reason=length → INCOMPLETE_OUTPUT（与非流式一致）
        let err = stream_result_to_response(StreamingReadResult {
            content: "x".into(),
            finish_reason: Some("length".into()),
            total_bytes: 1,
            chunk_count: 1,
            skipped_invalid_lines: 0,
        })
        .unwrap_err();
        assert_eq!(err.0, CliErrorCode::IncompleteOutput);
        // 正文缺失/空 → PROTOCOL_ERROR（与非流式 content 缺失一致）
        let err = stream_result_to_response(StreamingReadResult {
            content: String::new(),
            finish_reason: None,
            total_bytes: 5,
            chunk_count: 1,
            skipped_invalid_lines: 0,
        })
        .unwrap_err();
        assert_eq!(err.0, CliErrorCode::ProtocolError);
        // finish_reason 缺失放行
        assert!(stream_result_to_response(StreamingReadResult {
            content: "x".into(),
            finish_reason: None,
            total_bytes: 1,
            chunk_count: 1,
            skipped_invalid_lines: 0,
        })
        .is_ok());
    }

    #[test]
    fn streaming_client_build_rejects_same_url_rules() {
        let timeouts = Timeouts {
            connect_seconds: 15,
            response_seconds: 180,
            total_seconds: 1800,
        };
        let budget = Budget {
            context_window_tokens: 32_768,
            output_reserve_tokens: 8_192,
        };
        let (_, endpoint) =
            build_streaming_client("https://api.example.com/v1/", &timeouts, &budget).unwrap();
        assert_eq!(endpoint, "https://api.example.com/v1/chat/completions");
        // total 可到 1800（增量后组合约束仍由 validate_budget_timeouts 把关）
        let err =
            build_streaming_client("http://insecure.example.com", &timeouts, &budget).unwrap_err();
        assert_eq!(err.0, CliErrorCode::ProviderUrlInvalid);
    }
}
