//! 扫描行数 / 内存上限与降级 Top-N：窗口过大不 OOM。
//! 超限时返回 WindowTooLarge，并给出 Top-N + 显式提示（不静默截断）。

use crate::config::QueryLimits;
use crate::error::{AppError, ErrorCode, Result};
use crate::store::event_schema::EventRow;

#[derive(Debug, Clone)]
pub struct LimitCheck {
    pub estimated_rows: usize,
    pub estimated_bytes: usize,
    pub within: bool,
    pub reason: String,
}

/// 查询执行前估算：命中行数 × 单行估算字节 vs 上限。
pub fn check_limits(estimated_rows: usize, avg_row_bytes: usize, limits: &QueryLimits) -> LimitCheck {
    let bytes = estimated_rows.saturating_mul(avg_row_bytes);
    let rows_ok = estimated_rows <= limits.max_scan_rows;
    let mem_ok = bytes <= limits.max_memory_bytes;
    let within = rows_ok && mem_ok;
    let mut reason = String::new();
    if !rows_ok { reason.push_str(&format!("扫描行数 {}/{} 超限；", estimated_rows, limits.max_scan_rows)); }
    if !mem_ok { reason.push_str(&format!("内存估算 {}B/{}B 超限；", bytes, limits.max_memory_bytes)); }
    LimitCheck { estimated_rows, estimated_bytes: bytes, within, reason }
}

#[derive(Debug, Clone)]
pub struct DowngradedResult {
    pub top_rows: Vec<EventRow>,
    pub notice: String,
}

/// 降级为 Top-N（按 ts_ms 取最近 N 条）+ 窗口过大提示。
pub fn degrade_top_n(rows: &[EventRow], limits: &QueryLimits, reason: &str) -> DowngradedResult {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| std::cmp::Reverse(r.ts_ms));
    let n = limits.top_n.min(sorted.len());
    DowngradedResult {
        top_rows: sorted[..n].to_vec(),
        notice: format!("窗口过大，仅返回最近 {n} 条 Top-N。{reason}"),
    }
}

/// 统一入口：先检查上限，超限则降级返回（不报错给上层），正常则原样返回。
pub fn guard(rows: Vec<EventRow>, avg_row_bytes: usize, limits: &QueryLimits) -> std::result::Result<Vec<EventRow>, DowngradedResult> {
    let check = check_limits(rows.len(), avg_row_bytes, limits);
    if check.within { Ok(rows) } else { Err(degrade_top_n(&rows, limits, &check.reason)) }
}

/// 显式报错形态（供需要硬失败的调用点）
pub fn ensure_within(estimated_rows: usize, limits: &QueryLimits) -> Result<()> {
    if estimated_rows > limits.max_scan_rows {
        Err(AppError::new(ErrorCode::WindowTooLarge,
            format!("扫描行数 {}/{} 超限", estimated_rows, limits.max_scan_rows)))
    } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn row(ts: i64) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), "esb_audit".into());
        tags.insert("event_kind".into(), "flow".into());
        tags.insert("interface_code".into(), "A".into());
        EventRow::new(tags, ts, BTreeMap::new(), format!("flow#a{ts}"), format!("t{ts}")).unwrap()
    }

    #[test]
    fn within_limits_passes() {
        let l = QueryLimits { max_scan_rows: 100, max_memory_bytes: 10000, top_n: 5, min_clock_samples: 5 };
        assert!(check_limits(10, 100, &l).within);
        assert!(ensure_within(10, &l).is_ok());
    }

    #[test]
    fn over_row_limit_degrades_with_notice() {
        let l = QueryLimits { max_scan_rows: 2, max_memory_bytes: 100000, top_n: 2, min_clock_samples: 5 };
        let rows = vec![row(1), row(2), row(3)];
        let d = guard(rows, 1, &l).unwrap_err();
        assert_eq!(d.top_rows.len(), 2);
        assert!(d.notice.contains("窗口过大"));
        assert_eq!(d.top_rows[0].ts_ms, 3);
    }

    #[test]
    fn over_memory_limit_detected() {
        let l = QueryLimits { max_scan_rows: 1000, max_memory_bytes: 100, top_n: 5, min_clock_samples: 5 };
        assert!(!check_limits(10, 100, &l).within);
    }

    #[test]
    fn limits_configurable_and_zero_rejected() {
        let mut l = QueryLimits::default();
        l.max_scan_rows = 1;
        assert!(!check_limits(2, 1, &l).within);
        // 非法 0 在 config.rs 的 parse_range 中拒绝，这里确保逻辑不静默放行
        l.max_scan_rows = 0;
        assert!(!check_limits(1, 1, &l).within);
    }
}
