//! SSE 流式增量解析器（provider-config-delivery.md §1 2026-11-04 增量）。
//!
//! 纯逻辑、无网络：把 chat_completions 流式响应的字节分片（任意切分，
//! 模拟网络到达顺序/边界）解析为事件序列：
//! - [`StreamEvent::Delta`]：`choices[0].delta.content` 字符串增量（逐段）；
//! - [`StreamEvent::Finish`]：`finish_reason` 终值（最后一个非 null 值；
//!   仅在终止时上报一次，避免中间值被上层误用）；
//! - [`StreamEvent::Done`]：`data: [DONE]` 终止（或 [`SseDeltaParser::finish`]
//!   在 EOF 收尾时对残余行补解析）。
//!
//! 兼容性规则：UTF-8 BOM（流首）、空行（SSE 事件分隔）、CRLF/LF 行结束、
//! `data:` 后可选单个空格；非 `data:` 行（event:/id:/:注释）忽略；
//! `usage` 块与无 choices 的合法 JSON 忽略；JSON 无效的 data 行跳过并计数
//! （[`StreamOutcome::skipped_invalid_lines`] 终止时上报）。
//!
//! 字节缓冲按行切分：分片边界切在多字节 UTF-8 字符中间不影响解析
//! （整行字节齐了才转字符串）。

/// 流式解析事件（feed 期间产出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// 一个 data 行带来的正文增量（原样字符串，未拼接）。
    Delta(String),
    /// finish_reason 终值（最后一个非 null）；终止时最多上报一次。
    Finish(String),
    /// 流终止（`data: [DONE]`）。
    Done,
}

/// 流收尾结果（EOF 或 [DONE] 后取）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamOutcome {
    /// 累积拼接后的完整正文（全部 delta.content 依序连接）。
    pub content: String,
    /// finish_reason 终值（最后一个非 null；全程无显式值则 None）。
    pub finish_reason: Option<String>,
    /// JSON 无效被跳过的 data 行数。
    pub skipped_invalid_lines: usize,
    /// 是否见到 `data: [DONE]` 终止标记（EOF 提前结束则为 false）。
    pub done_marker_seen: bool,
}

/// SSE delta 解析器（喂字节分片 → 事件流；纯逻辑）。
#[derive(Debug, Default)]
pub struct SseDeltaParser {
    /// 未凑齐一行的残余字节（分片边界安全：按 \n 切行，行内 UTF-8 完整才转 str）。
    buffer: Vec<u8>,
    content: String,
    finish_reason: Option<String>,
    skipped_invalid_lines: usize,
    done_marker_seen: bool,
    first_line: bool,
}

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

impl SseDeltaParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            content: String::new(),
            finish_reason: None,
            skipped_invalid_lines: 0,
            done_marker_seen: false,
            first_line: true,
        }
    }

    /// 喂入一段到达字节（任意切分），返回本段解析出的事件（可为空）。
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<StreamEvent> {
        if self.done_marker_seen {
            return Vec::new(); // [DONE] 后迟到数据一律忽略
        }
        let mut events = Vec::new();
        self.buffer.extend_from_slice(chunk);
        while let Some(pos) = self.buffer.iter().position(|&b| b == b'\n') {
            let mut line: Vec<u8> = self.buffer.drain(..=pos).collect();
            line.pop(); // 去掉 \n
            if line.last() == Some(&b'\r') {
                line.pop(); // CRLF 兼容
            }
            self.process_line(&line, &mut events);
            if self.done_marker_seen {
                self.buffer.clear();
                break;
            }
        }
        events
    }

    /// 流结束（EOF）收尾：残余字节作为最后一行补解析，返回累积结果。
    /// 之后 feed 不再产出（迟到的 EOF 后数据不存在）。
    pub fn finish(&mut self) -> StreamOutcome {
        if !self.done_marker_seen && !self.buffer.is_empty() {
            let mut line: Vec<u8> = std::mem::take(&mut self.buffer);
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            // 尾行（可能含最后一段 delta/finish_reason）只并入累积结果，
            // 不再外发事件——收尾调用方只取 outcome。
            let mut events = Vec::new();
            self.process_line(&line, &mut events);
            drop(events);
        }
        self.buffer.clear();
        StreamOutcome {
            content: self.content.clone(),
            finish_reason: self.finish_reason.clone(),
            skipped_invalid_lines: self.skipped_invalid_lines,
            done_marker_seen: self.done_marker_seen,
        }
    }

    /// 是否已见 [DONE]（循环读取可提前停止）。
    pub fn is_done(&self) -> bool {
        self.done_marker_seen
    }

    /// 已累积正文（进度显示用）。
    pub fn accumulated(&self) -> &str {
        &self.content
    }

    /// 解析单个完整行（无行结束符）。
    fn process_line(&mut self, line: &[u8], events: &mut Vec<StreamEvent>) {
        let mut line = line;
        if self.first_line {
            self.first_line = false;
            if line.starts_with(UTF8_BOM) {
                line = &line[UTF8_BOM.len()..];
            }
        }
        if line.is_empty() {
            return; // 空行 = SSE 事件分隔
        }
        let Some(payload) = line.strip_prefix(b"data:") else {
            return; // 非 data: 行（event:/id:/:注释等）忽略
        };
        // data: 后至多剥一个空格（SSE 惯例 "data: x" / "data:x"）
        let payload = if payload.first() == Some(&b' ') {
            &payload[1..]
        } else {
            payload
        };
        if payload == b"[DONE]" {
            self.done_marker_seen = true;
            if let Some(reason) = self.finish_reason.clone() {
                events.push(StreamEvent::Finish(reason));
            }
            events.push(StreamEvent::Done);
            return;
        }
        let parsed: Result<serde_json::Value, _> = serde_json::from_slice(payload);
        let Ok(value) = parsed else {
            self.skipped_invalid_lines += 1;
            return;
        };
        // usage 块 / 无 choices 的合法 JSON：忽略（不计数）
        let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
            return;
        };
        if let Some(delta_text) = choice
            .get("delta")
            .and_then(|d| d.get("content"))
            .and_then(|c| c.as_str())
        {
            self.content.push_str(delta_text);
            events.push(StreamEvent::Delta(delta_text.to_string()));
        }
        // finish_reason 取最后非 null 显式值（后到覆盖先到）
        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            self.finish_reason = Some(reason.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑测试：任意切分喂入、无网络。
    use super::*;

    /// 逐 byte 极端分片喂入。
    fn feed_by_byte(parser: &mut SseDeltaParser, stream: &[u8]) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        for b in stream {
            events.extend(parser.feed(&[*b]));
        }
        events
    }

    /// 固定大小分片喂入（模拟网络 MSS 到达）。
    fn feed_in_chunks(parser: &mut SseDeltaParser, stream: &[u8], size: usize) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        for chunk in stream.chunks(size) {
            events.extend(parser.feed(chunk));
        }
        events
    }

    fn delta_event(text: &str) -> StreamEvent {
        StreamEvent::Delta(text.to_string())
    }

    fn sample_stream() -> String {
        // 多 chunk 文本拼接 + finish_reason（最后一个非 null）+ usage 块 + [DONE]
        let mut s = String::new();
        s.push_str(
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n",
        );
        s.push_str(
            "data: {\"choices\":[{\"delta\":{\"content\":\"第一段·\"},\"finish_reason\":null}]}\n",
        );
        s.push_str(
            "data: {\"choices\":[{\"delta\":{\"content\":\"第二段\"},\"finish_reason\":null}]}\n",
        );
        s.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n");
        s.push_str("data: {\"choices\":[],\"usage\":{\"total_tokens\":42}}\n");
        s.push_str("data: [DONE]\n");
        s
    }

    #[test]
    fn multi_chunk_text_concatenates_and_finish_takes_last_non_null() {
        for size in [1usize, 3, 7, 4096] {
            let mut parser = SseDeltaParser::new();
            let events = feed_in_chunks(&mut parser, sample_stream().as_bytes(), size);
            assert_eq!(
                events,
                vec![
                    delta_event("第一段·"),
                    delta_event("第二段"),
                    StreamEvent::Finish("stop".into()),
                    StreamEvent::Done,
                ],
                "chunk size {size}"
            );
            let outcome = parser.finish();
            assert_eq!(outcome.content, "第一段·第二段");
            assert_eq!(outcome.finish_reason.as_deref(), Some("stop"));
            assert_eq!(outcome.skipped_invalid_lines, 0);
            assert!(outcome.done_marker_seen);
        }
    }

    #[test]
    fn chunk_boundary_inside_multibyte_utf8() {
        // “分析” = 6 字节 UTF-8；逐 1/2/3 字节切分都会切在字符中间
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"经营分析中\"}}]}\n\
                      data: {\"choices\":[{\"delta\":{\"content\":\"国餐饮企\"}}]}\n\
                      data: [DONE]\n";
        for size in [1usize, 2, 3, 5] {
            let mut parser = SseDeltaParser::new();
            feed_in_chunks(&mut parser, stream.as_bytes(), size);
            assert_eq!(parser.finish().content, "经营分析中国餐饮企", "size {size}");
        }
        // 逐字节（最极端）
        let mut parser = SseDeltaParser::new();
        feed_by_byte(&mut parser, stream.as_bytes());
        assert_eq!(parser.finish().content, "经营分析中国餐饮企");
    }

    #[test]
    fn crlf_blank_lines_bom_and_space_variants() {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"\xEF\xBB\xBF"); // 流首 BOM
        bytes.extend_from_slice(b"data:{\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\r\n"); // 无空格 + CRLF
        bytes.extend_from_slice(b"\r\n"); // 空行
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(b": keep-alive comment\n"); // 注释行
        bytes.extend_from_slice(b"event: message\n"); // 非 data 行
        bytes.extend_from_slice(b"data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n");
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(b"data: [DONE]\r\n");
        bytes.extend_from_slice("data: 迟到数据应被忽略\n".as_bytes());
        let mut parser = SseDeltaParser::new();
        let events = feed_by_byte(&mut parser, &bytes);
        assert_eq!(
            events,
            vec![
                delta_event("A"),
                delta_event("B"),
                StreamEvent::Done, // 无 finish_reason → 无 Finish 事件
            ]
        );
        let outcome = parser.finish();
        assert_eq!(outcome.content, "AB");
        assert_eq!(outcome.finish_reason, None, "无 finish_reason 放行");
        assert_eq!(outcome.skipped_invalid_lines, 0);
        assert!(outcome.done_marker_seen);
        assert!(parser.is_done());
    }

    #[test]
    fn invalid_json_lines_skipped_and_counted() {
        let stream = "data: not-json\n\
                      data: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\
                      data: {broken\n\
                      data: [DONE]\n";
        let mut parser = SseDeltaParser::new();
        let events = feed_in_chunks(&mut parser, stream.as_bytes(), 4);
        assert_eq!(events, vec![delta_event("好"), StreamEvent::Done]);
        let outcome = parser.finish();
        assert_eq!(outcome.content, "好");
        assert_eq!(outcome.skipped_invalid_lines, 2, "终止时上报 skipped");
    }

    #[test]
    fn no_finish_reason_passes_through() {
        let stream = "data: {\"choices\":[{\"delta\":{\"content\":\"正文\"}}]}\n\
                      data: [DONE]\n";
        let mut parser = SseDeltaParser::new();
        let events = feed_in_chunks(&mut parser, stream.as_bytes(), 8);
        assert_eq!(events, vec![delta_event("正文"), StreamEvent::Done]);
        assert_eq!(parser.finish().finish_reason, None);
    }

    #[test]
    fn finish_reason_last_non_null_wins() {
        let stream = "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\
                      data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\
                      data: [DONE]\n";
        let mut parser = SseDeltaParser::new();
        let events = feed_in_chunks(&mut parser, stream.as_bytes(), 5);
        assert_eq!(
            events,
            vec![StreamEvent::Finish("stop".into()), StreamEvent::Done],
            "终值取最后一个非 null，且只在终止时上报一次"
        );
        assert_eq!(parser.finish().finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn eof_without_done_marker_still_yields_tail() {
        // 服务端未发 [DONE] 直接关流：残余行在 finish() 补解析
        let stream =
            "data: {\"choices\":[{\"delta\":{\"content\":\"尾段\"},\"finish_reason\":\"stop\"}]}";
        let mut parser = SseDeltaParser::new();
        let events = feed_in_chunks(&mut parser, stream.as_bytes(), 6);
        assert!(events.is_empty(), "无行结束符时 feed 不产出");
        let outcome = parser.finish();
        assert_eq!(outcome.content, "尾段");
        assert_eq!(outcome.finish_reason.as_deref(), Some("stop"));
        assert!(!outcome.done_marker_seen, "EOF 提前结束无 [DONE] 标记");
    }

    #[test]
    fn usage_block_and_role_only_chunks_ignored() {
        let stream = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\
                      data: {\"usage\":{\"prompt_tokens\":1,\"total_tokens\":9}}\n\
                      data: {\"choices\":[{\"delta\":{\"content\":null}}]}\n\
                      data: [DONE]\n";
        let mut parser = SseDeltaParser::new();
        let events = feed_in_chunks(&mut parser, stream.as_bytes(), 9);
        assert_eq!(events, vec![StreamEvent::Done], "无 content 增量不产 Delta");
        let outcome = parser.finish();
        assert_eq!(outcome.content, "");
        assert_eq!(outcome.skipped_invalid_lines, 0, "合法 JSON 不计 skipped");
    }

    #[test]
    fn accumulated_exposes_progress_text() {
        let mut parser = SseDeltaParser::new();
        parser.feed("data: {\"choices\":[{\"delta\":{\"content\":\"进度\"}}]}\n".as_bytes());
        assert_eq!(parser.accumulated(), "进度");
        assert!(!parser.is_done());
    }
}
