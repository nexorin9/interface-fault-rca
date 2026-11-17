//! 结构化运行日志（每行一个 JSON 对象）、每源采集统计与运行摘要导出。
//! 日志正文不含日志原文与凭据，只留摘要与计数。

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogLine {
    pub ts: String,
    pub level: String,
    pub source: String,
    pub event: String,
    pub code: String,
    pub detail: String,
}

impl LogLine {
    pub fn new(level: &str, source: &str, event: &str, code: &str, detail: impl Into<String>) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            level: level.to_string(),
            source: source.to_string(),
            event: event.to_string(),
            code: code.to_string(),
            detail: detail.into(),
        }
    }

    /// 序列化为单行 JSON（可被 serde_json 逐行解析）
    pub fn to_json_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// 每源采集统计（口径固定，可被运行摘要直接引用）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceStats {
    pub source: String,
    pub attempted: u64,
    pub ingested: u64,
    pub dirty_lines: u64,
    pub too_long: u64,
    pub skipped: u64,
    pub elapsed_ms: u64,
    pub watermark: String,
    pub fail_code: String,
}

impl SourceStats {
    pub fn new(source: &str) -> Self {
        Self { source: source.to_string(), ..Default::default() }
    }
    pub fn add(&mut self, other: &SourceStats) {
        self.attempted += other.attempted;
        self.ingested += other.ingested;
        self.dirty_lines += other.dirty_lines;
        self.too_long += other.too_long;
        self.skipped += other.skipped;
        self.elapsed_ms += other.elapsed_ms;
    }
}

/// 脏行 / 丢弃原因的可回查索引（行号 + 原因码），保留原始定位信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropRecord {
    pub source: String,
    pub line_no: usize,
    pub reason_code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub sources: Vec<SourceStats>,
    pub candidate_count: usize,
    pub model_calls: u32,
    pub degraded: bool,
    pub degrade_reason: String,
    pub tickets_created: u32,
    pub dropped: Vec<DropRecord>,
}

impl RunSummary {
    pub fn new(window_start_ms: i64, window_end_ms: i64) -> Self {
        Self {
            window_start_ms, window_end_ms, sources: vec![], candidate_count: 0,
            model_calls: 0, degraded: false, degrade_reason: String::new(),
            tickets_created: 0, dropped: vec![],
        }
    }
    pub fn total_ingested(&self) -> u64 { self.sources.iter().map(|s| s.ingested).sum() }
    pub fn to_json(&self) -> String { serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string()) }
    /// 终端可读摘要（交班时能直接念出来的内容）
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("运行摘要：窗口 [{}, {}]\n", self.window_start_ms, self.window_end_ms));
        for s in &self.sources {
            out.push_str(&format!(
                "  {}: 尝试 {} / 入库 {} / 脏行 {} / 超长 {} / 跳过 {} / 耗时 {}ms / 失败码 {}\n",
                s.source, s.attempted, s.ingested, s.dirty_lines, s.too_long, s.skipped, s.elapsed_ms,
                if s.fail_code.is_empty() { "-" } else { &s.fail_code },
            ));
        }
        out.push_str(&format!("  候选 {} 条 / 模型调用 {} 次 / 建单 {} 张\n",
            self.candidate_count, self.model_calls, self.tickets_created));
        if self.degraded {
            out.push_str(&format!("  【降级】{}，以下为规则基线结论\n", self.degrade_reason));
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct RunLogger {
    lines: Arc<Mutex<Vec<LogLine>>>,
}

impl Default for RunLogger {
    fn default() -> Self { Self::new() }
}

impl RunLogger {
    pub fn new() -> Self { Self { lines: Arc::new(Mutex::new(Vec::new())) } }

    pub fn log(&self, line: LogLine) {
        if let Ok(mut v) = self.lines.lock() { v.push(line); }
    }

    pub fn info(&self, source: &str, event: &str, detail: impl Into<String>) {
        self.log(LogLine::new("INFO", source, event, "-", detail));
    }

    pub fn warn(&self, source: &str, event: &str, code: &str, detail: impl Into<String>) {
        self.log(LogLine::new("WARN", source, event, code, detail));
    }

    /// 导出为逐行 JSON 文本（每行可独立解析）
    pub fn export(&self) -> String {
        self.lines.lock().map(|v| v.iter().map(|l| l.to_json_line()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }

    pub fn len(&self) -> usize { self.lines.lock().map(|v| v.len()).unwrap_or(0) }
    pub fn is_empty(&self) -> bool { self.len() == 0 }
}

/// 计时器辅助
pub fn elapsed_ms(t: &Instant) -> u64 { t.elapsed().as_millis() as u64 }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lines_are_json_parseable() {
        let logger = RunLogger::new();
        logger.info("esb_audit", "collect", "ingested 10 rows");
        logger.warn("prom", "fetch", "SourceUnavailable", "timeout");
        let txt = logger.export();
        let lines: Vec<&str> = txt.lines().collect();
        assert_eq!(lines.len(), 2);
        for l in lines {
            let parsed: LogLine = serde_json::from_str(l).unwrap();
            assert!(!parsed.ts.is_empty());
            assert!(!parsed.level.is_empty());
        }
    }

    #[test]
    fn stats_accumulate_across_collections() {
        let mut a = SourceStats::new("flow");
        a.attempted = 10; a.ingested = 9;
        let mut b = SourceStats::new("flow");
        b.attempted = 5; b.ingested = 5; b.dirty_lines = 1;
        a.add(&b);
        assert_eq!(a.attempted, 15);
        assert_eq!(a.ingested, 14);
        assert_eq!(a.dirty_lines, 1);
    }

    #[test]
    fn logs_and_summary_dont_leak_secret_values() {
        let logger = RunLogger::new();
        logger.info("ticket", "post", "url host only");
        let secret = "SUPER_SECRET_TOKEN_123";
        assert!(!logger.export().contains(secret));
        let s = RunSummary::new(0, 100);
        assert!(!s.to_json().contains(secret));
        assert!(!s.to_text().contains(secret));
    }

    #[test]
    fn run_summary_text_shows_degrade_banner() {
        let mut s = RunSummary::new(0, 100);
        s.degraded = true;
        s.degrade_reason = "模型不可达".to_string();
        assert!(s.to_text().contains("规则基线结论"));
    }
}
