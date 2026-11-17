//! 证据窗口装配：三源按 ts_ms 对齐；跨源按 interface_code 恢复（不在应用层跨库 join）；
//! 产出带 __row_id 的证据集合与合法 ID 白名单；时钟漂移容忍。

use crate::config::QueryLimits;
use crate::error::{AppError, ErrorCode, Result};
use crate::store::dataset::{query_window, ScanStats};
use crate::store::event_schema::EventRow;
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct EvidenceWindow {
    pub rows: Vec<EventRow>,
    pub allowed_row_ids: HashSet<String>,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub stats: ScanStats,
    pub clock_shift_applied: bool,
    pub clock_shift_ms: i64,
}

/// 从 Parquet 数据集装配证据窗口。三源同源表，按 ts_ms 区间取数即完成时间对齐。
pub fn assemble(root: &Path, from_ms: i64, to_ms: i64, limits: &QueryLimits) -> Result<EvidenceWindow> {
    if from_ms > to_ms {
        return Err(AppError::new(ErrorCode::WindowTooLarge, "窗口起点晚于终点"));
    }
    let mut stats = ScanStats::default();
    let mut rows = query_window(root, from_ms, to_ms, &[], &mut stats)?;
    // 扫描上限：超限降级为 Top-N + 提示（在结论层体现，这里保留 Top-N 行）
    if rows.len() > limits.max_scan_rows {
        let mut sorted: Vec<_> = rows.into_iter().collect();
        sorted.sort_by_key(|r| std::cmp::Reverse(r.ts_ms));
        rows = sorted.into_iter().take(limits.top_n).collect();
    }
    let allowed = rows.iter().map(|r| r.row_id.clone()).collect();
    Ok(EvidenceWindow {
        rows, allowed_row_ids: allowed, window_start_ms: from_ms, window_end_ms: to_ms,
        stats, clock_shift_applied: false, clock_shift_ms: 0,
    })
}

/// 从内存行装配（fixture / 测试路径）
pub fn assemble_from_rows(rows: Vec<EventRow>, from_ms: i64, to_ms: i64) -> Result<EvidenceWindow> {
    let in_window: Vec<_> = rows.into_iter().filter(|r| r.ts_ms >= from_ms && r.ts_ms <= to_ms).collect();
    let allowed = in_window.iter().map(|r| r.row_id.clone()).collect();
    Ok(EvidenceWindow {
        rows: in_window, allowed_row_ids: allowed, window_start_ms: from_ms, window_end_ms: to_ms,
        stats: ScanStats::default(), clock_shift_applied: false, clock_shift_ms: 0,
    })
}

/// 跨源时钟偏移估计：取同 interface_code 的 flow 与 log 时间差中位数；样本不足则不校正。
pub fn estimate_clock_shift(flows: &[EventRow], logs: &[EventRow], min_samples: usize) -> Option<i64> {
    use crate::baseline::aggregate::median;
    let mut diffs = vec![];
    for f in flows.iter().filter(|r| r.kind() == "flow") {
        for l in logs.iter().filter(|r| r.kind() == "log" && r.interface_code() == f.interface_code()) {
            diffs.push((l.ts_ms - f.ts_ms) as f64);
            if diffs.len() >= 200 { break; }
        }
    }
    if diffs.len() < min_samples { return None; }
    Some(median(&diffs) as i64)
}

/// 对日志行施加偏移校正
pub fn apply_shift(rows: &mut [EventRow], shift_ms: i64) {
    for r in rows.iter_mut().filter(|r| r.kind() == "log") {
        r.ts_ms = (r.ts_ms as i64).saturating_sub(shift_ms);
        r.fields.insert("clock_shift_ms".into(), shift_ms.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn row(source: &str, kind: &str, code: &str, ts: i64, id: &str) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), source.into());
        tags.insert("event_kind".into(), kind.into());
        tags.insert("interface_code".into(), code.into());
        EventRow::new(tags, ts, BTreeMap::new(), id.into(), compute_tsid(source, ts, code, "", kind)).unwrap()
    }

    #[test]
    fn window_filters_and_whitelists() {
        let rows = vec![row("esb_audit", "flow", "A", 100, "f1"), row("esb_audit", "flow", "A", 200, "f2")];
        let w = assemble_from_rows(rows, 150, 300).unwrap();
        assert_eq!(w.rows.len(), 1);
        assert!(w.allowed_row_ids.contains("f2"));
        assert!(!w.allowed_row_ids.contains("f1"));
    }

    #[test]
    fn invalid_window_errors() {
        assert_eq!(assemble(Path::new("/tmp/none"), 200, 100, &QueryLimits::default()).unwrap_err().code, ErrorCode::WindowTooLarge);
    }

    #[test]
    fn clock_shift_estimated_and_applied() {
        let flows: Vec<_> = (0..10).map(|i| row("esb_audit", "flow", "A", 1000 + i, &format!("f{i}"))).collect();
        let logs: Vec<_> = (0..10).map(|i| row("app_log", "log", "A", 1500 + i, &format!("l{i}"))).collect();
        let shift = estimate_clock_shift(&flows, &logs, 5).unwrap();
        assert_eq!(shift, 500);
        let mut l = logs[0].clone();
        apply_shift(std::slice::from_mut(&mut l), shift);
        assert_eq!(l.ts_ms, 1000);
        assert_eq!(l.fields.get("clock_shift_ms").unwrap(), "500");
    }

    #[test]
    fn insufficient_samples_no_correction() {
        let flows = vec![row("esb_audit", "flow", "A", 1000, "f")];
        let logs = vec![row("app_log", "log", "A", 1500, "l")];
        assert_eq!(estimate_clock_shift(&flows, &logs, 5), None);
    }

    #[test]
    fn cross_midnight_window() {
        let a = chrono::DateTime::parse_from_rfc3339("2026-11-08T23:50:00Z").unwrap().timestamp_millis();
        let b = chrono::DateTime::parse_from_rfc3339("2026-11-09T00:10:00Z").unwrap().timestamp_millis();
        let w = assemble_from_rows(vec![row("esb_audit", "flow", "A", a, "f1"), row("esb_audit", "flow", "A", b, "f2")], a, b).unwrap();
        assert_eq!(w.rows.len(), 2);
    }
}
