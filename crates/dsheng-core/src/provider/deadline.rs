//! Deadline 引擎（provider-config-delivery.md §3 超时预算、主计划 §14）。
//!
//! 默认 15/180/240 秒（connect/response/total）；可配范围与组合约束由
//! `contracts::settings::validate_budget_timeouts` 把关（本模块不重复）。
//! 语义：
//! - connect：DNS/TCP/TLS 共用，自 preparing 起算；
//! - response：**连接完成后**开始发请求至完整 body；等待期的零星数据
//!   事件**不重置**任何 deadline；
//! - total：preparing 至终态，含解析/校验/保存。
//! - 每个 deadline 至多触发一次 `timed_out`，无自动重试；终态后停止检查。
//!
//! 时钟注入（用例 57）：生产用系统时钟，测试用 [`FakeClock`]。
//! 纯逻辑层：不发起网络请求。

use crate::contracts::settings::Timeouts;

/// 触发的 deadline 种类 → CliErrorCode：Connect/Response/Total Timeout。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeadlineKind {
    Connect,
    Response,
    Total,
}

impl DeadlineKind {
    pub fn error_code(self) -> crate::contracts::cli::CliErrorCode {
        use crate::contracts::cli::CliErrorCode;
        match self {
            DeadlineKind::Connect => CliErrorCode::ConnectTimeout,
            DeadlineKind::Response => CliErrorCode::ResponseTimeout,
            DeadlineKind::Total => CliErrorCode::TotalTimeout,
        }
    }
}

/// 单调毫秒时钟抽象（用例 57：时钟注入）。
pub trait MonotonicClock {
    fn now_millis(&self) -> i64;
}

/// 系统时钟（生产；基于 `SystemTime`）。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl MonotonicClock for SystemClock {
    fn now_millis(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default()
    }
}

/// 测试时钟（纯逻辑依赖注入）。
#[derive(Debug, Clone)]
pub struct FakeClock {
    now: i64,
}

impl FakeClock {
    pub fn new(start_millis: i64) -> Self {
        Self { now: start_millis }
    }
    pub fn advance(&mut self, millis: i64) {
        self.now += millis;
    }
}

impl MonotonicClock for FakeClock {
    fn now_millis(&self) -> i64 {
        self.now
    }
}

/// Deadline 引擎状态机。
#[derive(Debug, Clone)]
pub struct DeadlineEngine {
    timeouts: Timeouts,
    started_at: Option<i64>,
    connect_done_at: Option<i64>,
    response_done: bool,
    connect_fired: bool,
    response_fired: bool,
    total_fired: bool,
    finished: bool,
}

impl DeadlineEngine {
    pub fn new(timeouts: Timeouts) -> Self {
        Self {
            timeouts,
            started_at: None,
            connect_done_at: None,
            response_done: false,
            connect_fired: false,
            response_fired: false,
            total_fired: false,
            finished: false,
        }
    }

    /// preparing 开始。
    pub fn begin(&mut self, now_millis: i64) {
        self.started_at = Some(now_millis);
    }

    /// 连接完成（DNS/TCP/TLS 结束，开始发请求）：response deadline 自此起算。
    pub fn on_connect_complete(&mut self, now_millis: i64) {
        if self.connect_done_at.is_none() {
            self.connect_done_at = Some(now_millis);
        }
    }

    /// 等待期零星数据事件：不重置任何 deadline。
    pub fn on_data(&mut self) {
        // 故意空实现：零星数据不推迟 response/total deadline
    }

    /// 完整 body 已到达：response 窗口结束，此后只剩 total 兜底
    /// （total 覆盖解析/校验/保存）。
    pub fn on_response_complete(&mut self) {
        self.response_done = true;
    }

    /// 终态：停止检查。
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// 检查当前时刻；返回本轮新触发的 deadline（每个至多一次）。
    ///
    /// 优先级：connect（若未完成，报告更具体的成因）→ response → total。
    pub fn check(&mut self, now_millis: i64) -> Option<DeadlineKind> {
        if self.finished {
            return None;
        }
        let started = self.started_at?;
        let total_ms = self.timeouts.total_seconds as i64 * 1_000;
        let connect_ms = self.timeouts.connect_seconds as i64 * 1_000;
        let response_ms = self.timeouts.response_seconds as i64 * 1_000;

        if self.connect_done_at.is_none() && !self.connect_fired {
            if now_millis - started > connect_ms {
                self.connect_fired = true;
                return Some(DeadlineKind::Connect);
            }
        } else if let Some(connect_done) = self.connect_done_at {
            if !self.response_done
                && !self.response_fired
                && now_millis - connect_done > response_ms
            {
                self.response_fired = true;
                return Some(DeadlineKind::Response);
            }
        }
        if !self.total_fired && now_millis - started > total_ms {
            self.total_fired = true;
            return Some(DeadlineKind::Total);
        }
        None
    }

    /// 每个 deadline 是否已触发（测试断言"一次"语义用）。
    pub fn fired(&self) -> (bool, bool, bool) {
        (self.connect_fired, self.response_fired, self.total_fired)
    }
}

#[cfg(test)]
mod tests {
    // 纯逻辑/依赖注入层测试（用例 57：时钟注入；非 Provider 调用）。
    use super::*;

    fn defaults() -> Timeouts {
        crate::contracts::settings::DEFAULT_TIMEOUTS
    }

    #[test]
    fn defaults_15_180_240_accepted() {
        let mut clock = FakeClock::new(1_000);
        let mut engine = DeadlineEngine::new(defaults());
        engine.begin(clock.now_millis());
        clock.advance(5_000);
        engine.on_connect_complete(clock.now_millis());
        clock.advance(100_000);
        engine.on_response_complete();
        // 240s 内不触发
        clock.advance(135_000);
        assert_eq!(engine.check(clock.now_millis()), None);
        clock.advance(1);
        assert_eq!(engine.check(clock.now_millis()), Some(DeadlineKind::Total));
        assert_eq!(engine.check(clock.now_millis()), None, "total 只触发一次");
    }

    #[test]
    fn connect_timeout_fires_before_connect_completes() {
        let mut clock = FakeClock::new(0);
        let mut engine = DeadlineEngine::new(defaults());
        engine.begin(clock.now_millis());
        clock.advance(15_000);
        assert_eq!(engine.check(clock.now_millis()), None);
        clock.advance(1);
        assert_eq!(
            engine.check(clock.now_millis()),
            Some(DeadlineKind::Connect)
        );
        assert_eq!(engine.check(clock.now_millis()), None);
    }

    #[test]
    fn response_deadline_anchored_at_connect_completion_and_not_reset_by_data() {
        let mut clock = FakeClock::new(0);
        let mut engine = DeadlineEngine::new(defaults());
        engine.begin(clock.now_millis());
        clock.advance(10_000); // 10s 完成连接
        engine.on_connect_complete(clock.now_millis());
        // 零星数据到达，不重置 response deadline
        for _ in 0..10 {
            clock.advance(17_000);
            engine.on_data();
            assert_eq!(engine.check(clock.now_millis()), None);
        }
        // 连接完成 180s 后触发 response（而不是从最后数据起算）
        clock.advance(11_000);
        assert_eq!(
            engine.check(clock.now_millis()),
            Some(DeadlineKind::Response)
        );
        assert_eq!(engine.check(clock.now_millis()), None);
    }

    #[test]
    fn total_covers_parsing_and_saving_after_body() {
        let mut clock = FakeClock::new(0);
        let mut engine = DeadlineEngine::new(defaults());
        engine.begin(clock.now_millis());
        clock.advance(5_000);
        engine.on_connect_complete(clock.now_millis());
        clock.advance(100_000); // body 完成 @105s，response 窗口结束
        engine.on_response_complete();
        assert_eq!(engine.check(clock.now_millis()), None);
        clock.advance(135_001); // 解析+校验+保存耗时超过 total（240s）
        assert_eq!(engine.check(clock.now_millis()), Some(DeadlineKind::Total));
        // response 不再触发（窗口已在 body 完成时关闭）
        assert_eq!(engine.fired(), (false, false, true));
    }

    #[test]
    fn finish_stops_checks() {
        let mut clock = FakeClock::new(0);
        let mut engine = DeadlineEngine::new(defaults());
        engine.begin(clock.now_millis());
        engine.finish();
        clock.advance(1_000_000);
        assert_eq!(engine.check(clock.now_millis()), None);
    }

    #[test]
    fn kind_to_error_code() {
        assert_eq!(
            DeadlineKind::Connect.error_code(),
            crate::contracts::cli::CliErrorCode::ConnectTimeout
        );
        assert_eq!(
            DeadlineKind::Response.error_code(),
            crate::contracts::cli::CliErrorCode::ResponseTimeout
        );
        assert_eq!(
            DeadlineKind::Total.error_code(),
            crate::contracts::cli::CliErrorCode::TotalTimeout
        );
    }
}
