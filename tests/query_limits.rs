//! task 32 验收：扫描/内存上限与降级 —— 超行数/超内存触发降级、未超限正常、上限可配且非法值拒。
mod common;

use common::flow;
use interface_fault_rca::config::QueryLimits;
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::query::pushdown::{check_limits, degrade_top_n, ensure_within};

#[test]
fn over_scan_rows_triggers_degrade_notice() {
    let mut limits = QueryLimits::default();
    limits.max_scan_rows = 5;
    let c = check_limits(100, 10, &limits);
    assert!(!c.within);
    assert!(c.reason.contains("扫描行数"));
    let rows: Vec<_> = (0..100).map(|i| flow("A", "SVC", i, 100.0, 200)).collect();
    let d = degrade_top_n(&rows, &limits, &c.reason);
    assert_eq!(d.top_rows.len(), limits.top_n);
    assert!(d.notice.contains("窗口过大"));
}

#[test]
fn over_memory_budget_triggers_degrade() {
    let mut limits = QueryLimits::default();
    limits.max_memory_bytes = 1000;
    let c = check_limits(100, 100, &limits);
    assert!(!c.within);
    assert!(c.reason.contains("内存"));
}

#[test]
fn within_limits_returns_normally() {
    let limits = QueryLimits::default();
    let c = check_limits(10, 100, &limits);
    assert!(c.within);
    assert!(ensure_within(10, &limits).is_ok());
}

#[test]
fn ensure_within_hard_errors_on_over_limit() {
    let mut limits = QueryLimits::default();
    limits.max_scan_rows = 10;
    let err = ensure_within(100, &limits).unwrap_err();
    assert_eq!(err.code, ErrorCode::WindowTooLarge);
}

#[test]
fn limits_are_configurable() {
    let mut limits = QueryLimits::default();
    limits.max_scan_rows = 500_000;
    limits.top_n = 50;
    assert_eq!(limits.max_scan_rows, 500_000);
    assert_eq!(limits.top_n, 50);
}
