//! 增量采集调度：每源独立游标、断点续跑、单源故障隔离、指数退避。
//! 一个源失败不拖垮整轮；失败源游标不动，其余源照常推进。

use crate::error::ErrorCode;
use crate::observability::SourceStats;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct CursorState {
    pub last_offset: i64,
    pub last_size: i64,
    pub last_mtime: i64,
}

#[derive(Debug, Clone)]
pub struct SourceRunResult {
    pub source: String,
    pub ok: bool,
    pub ingested: u64,
    pub fail_code: String,
    pub cursor: CursorState,
    pub backoff_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RoundResult {
    pub sources: Vec<SourceRunResult>,
}

impl RoundResult {
    pub fn all_ok(&self) -> bool { self.sources.iter().all(|s| s.ok) }
    pub fn ok_sources(&self) -> Vec<&str> {
        self.sources.iter().filter(|s| s.ok).map(|s| s.source.as_str()).collect()
    }
}

/// 单源故障隔离调度器：对每源单独执行 collect 闭包，失败只记该源。
pub struct IncrementalScheduler {
    cursors: BTreeMap<String, CursorState>,
    failures: BTreeMap<String, u32>,
}

impl Default for IncrementalScheduler {
    fn default() -> Self { Self::new() }
}

impl IncrementalScheduler {
    pub fn new() -> Self { Self { cursors: BTreeMap::new(), failures: BTreeMap::new() } }

    pub fn set_cursor(&mut self, source: &str, c: CursorState) {
        self.cursors.insert(source.to_string(), c);
    }
    pub fn cursor(&self, source: &str) -> CursorState {
        self.cursors.get(source).cloned().unwrap_or_default()
    }

    /// 执行一轮采集：每源独立 collect(当前游标) -> (ingested, new_cursor, Result)。
    /// 失败源游标不变、保留失败码、退避次数递增；其余源继续。
    pub fn run_round<F>(&mut self, sources: &[&str], mut collect: F) -> RoundResult
    where
        F: FnMut(&str, &CursorState) -> std::result::Result<(u64, CursorState), ErrorCode>,
    {
        let mut results = vec![];
        for &src in sources {
            let cur = self.cursor(src);
            match collect(src, &cur) {
                Ok((ingested, new_cursor)) => {
                    self.cursors.insert(src.to_string(), new_cursor.clone());
                    self.failures.remove(src);
                    results.push(SourceRunResult {
                        source: src.to_string(), ok: true, ingested,
                        fail_code: String::new(), cursor: new_cursor, backoff_ms: 0,
                    });
                }
                Err(code) => {
                    let n = self.failures.entry(src.to_string()).or_insert(0);
                    *n += 1;
                    let backoff = 100u64.saturating_mul(1u64 << (*n - 1).min(6));
                    results.push(SourceRunResult {
                        source: src.to_string(), ok: false, ingested: 0,
                        fail_code: code.as_str().to_string(), cursor: cur, backoff_ms: backoff,
                    });
                }
            }
        }
        RoundResult { sources: results }
    }

    pub fn failure_count(&self, source: &str) -> u32 {
        self.failures.get(source).copied().unwrap_or(0)
    }

    /// 把一轮结果并入采集统计（供运行摘要）
    pub fn merge_stats(round: &RoundResult, stats: &mut BTreeMap<String, SourceStats>) {
        for r in &round.sources {
            let e = stats.entry(r.source.clone()).or_insert_with(|| SourceStats::new(&r.source));
            e.ingested += r.ingested;
            if !r.ok { e.fail_code = r.fail_code.clone(); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_source_failure_does_not_stop_others() {
        let mut s = IncrementalScheduler::new();
        let round = s.run_round(&["esb_audit", "app_log", "prom"], |src, _| {
            if src == "esb_audit" { Err(ErrorCode::SourceUnavailable) }
            else { Ok((10, CursorState::default())) }
        });
        assert!(!round.all_ok());
        assert_eq!(round.ok_sources(), vec!["app_log", "prom"]);
        assert_eq!(round.sources[0].fail_code, "SourceUnavailable");
    }

    #[test]
    fn failed_source_cursor_unchanged_and_backoff_grows() {
        let mut s = IncrementalScheduler::new();
        s.set_cursor("prom", CursorState { last_offset: 42, ..Default::default() });
        let r1 = s.run_round(&["prom"], |_, _| Err(ErrorCode::SourceUnavailable));
        assert_eq!(r1.sources[0].cursor.last_offset, 42);
        let r2 = s.run_round(&["prom"], |_, _| Err(ErrorCode::SourceUnavailable));
        assert!(r2.sources[0].backoff_ms > r1.sources[0].backoff_ms);
        assert_eq!(s.failure_count("prom"), 2);
    }

    #[test]
    fn resume_from_cursor_no_duplicate() {
        let mut s = IncrementalScheduler::new();
        let r = s.run_round(&["esb_audit"], |_, cur| {
            Ok((5, CursorState { last_offset: cur.last_offset + 5, ..Default::default() }))
        });
        assert_eq!(r.sources[0].ingested, 5);
        assert_eq!(s.cursor("esb_audit").last_offset, 5);
        // 中断后重启：游标为 5，第二批从 5 起
        let r2 = s.run_round(&["esb_audit"], |_, cur| {
            assert_eq!(cur.last_offset, 5);
            Ok((3, CursorState { last_offset: cur.last_offset + 3, ..Default::default() }))
        });
        assert_eq!(r2.sources[0].ingested, 3);
    }
}
