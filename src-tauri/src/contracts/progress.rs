//! 生成进度事件桥（S4 流式可观测）。
//!
//! 桌面壳把服务层流式进度（`dsheng_cli::store::generation::StreamProgressEvent`）
//! 经 tauri `Emitter` 以 `generation://progress` 事件广播给 UI：
//! payload 绑定 workspace / confirmation / attempt，包含正文字符数及思考增量。
//! UI 订阅本事件展示阶段、正文字符数、耗时和 Provider 返回的思考文本，
//! 并以 payload 中的 attempt_id 调用既有 cancel 语义取消。
//! 网络请求前即产生等待事件，流式和非流式均可观察并手动停止。

use dsheng_cli::store::generation::{ProgressSink, StreamProgressEvent};
use serde_json::{json, Value};

/// 进度事件名（UI `window.__TAURI__.event.listen('generation://progress')`）。
pub const GENERATION_PROGRESS_EVENT: &str = "generation://progress";

/// 事件 payload（纯函数；测试断言形状用）。
pub fn progress_payload(event: &StreamProgressEvent) -> Value {
    json!({
        "attempt_id": event.attempt_id.to_string(),
        "confirmation_id": event.confirmation_id.to_string(),
        "workspace_id": event.workspace_id.to_string(),
        "chunk_seq": event.chunk_seq,
        "bytes": event.bytes,
        "elapsed_ms": event.elapsed_ms,
        "content_chars": event.content_chars,
        "reasoning_delta": event.reasoning_delta,
        "stage": event.stage,
    })
}

/// 构造向全部窗口 emit `generation://progress` 的进度 sink。
///
/// emit 失败（窗口关闭等）静默忽略——进度事件是尽力而为的观测通道，
/// 不影响发送链路本身。
pub fn emitting_sink(app: &tauri::AppHandle) -> ProgressSink {
    let app = app.clone();
    ProgressSink::Callback(Box::new(move |event: StreamProgressEvent| {
        use tauri::Emitter as _;
        let _ = app.emit(GENERATION_PROGRESS_EVENT, progress_payload(&event));
    }))
}
