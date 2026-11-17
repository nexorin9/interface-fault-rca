//! task 8 验收：证据窗口 —— 跨源对齐、列裁剪、越界行 ID 拒绝、超限降级，扫描统计被记录。
mod common;

use common::{flow, log, metric};
use interface_fault_rca::config::QueryLimits;
use interface_fault_rca::query::window::{apply_shift, assemble_from_rows, estimate_clock_shift};
use interface_fault_rca::query::rowid::{check_ref, RefViolation, RowIdIndex};
use interface_fault_rca::query::pushdown::{check_limits, degrade_top_n, guard};
use std::collections::HashSet;

#[test]
fn cross_source_window_aligns_by_interface_code() {
    let rows = vec![
        flow("ESB-A", "SVC", 1000, 100.0, 200),
        log("ESB-A", "SVC", 1100, "ERROR", "boom"),
        metric("ESB-A", "host-1", 1200, "cpu", 0.8),
        flow("ESB-A", "SVC", 9000, 100.0, 200), // 窗口外
    ];
    let w = assemble_from_rows(rows, 1000, 2000).unwrap();
    assert_eq!(w.rows.len(), 3);
    assert!(w.allowed_row_ids.len() == 3);
    assert!(!w.allowed_row_ids.contains("flow#ESB-A@9000"));
}

#[test]
fn out_of_window_and_nonexistent_refs_rejected() {
    let rows = vec![flow("A", "SVC", 1000, 100.0, 200)];
    let idx = RowIdIndex::new(&rows);
    let window = idx.allowed_ids();
    let mut all = window.clone();
    all.insert("flow#outside".to_string());
    // 窗口内合法
    assert!(check_ref("flow#A", &idx, &HashSet::new(), &all).is_none());
    // 窗口外（存在）
    assert_eq!(check_ref("flow#outside", &idx, &HashSet::new(), &all), Some(RefViolation::OutsideWindow));
    // 不存在
    assert_eq!(check_ref("flow#ghost", &idx, &HashSet::new(), &all), Some(RefViolation::Nonexistent));
    // unbound
    let mut unbound = HashSet::new();
    unbound.insert("flow#A".to_string());
    assert_eq!(check_ref("flow#A", &idx, &unbound, &all), Some(RefViolation::Unbound));
}

#[test]
fn over_limit_degrades_to_top_n_with_notice() {
    let mut limits = QueryLimits::default();
    limits.max_scan_rows = 3;
    limits.top_n = 2;
    let rows: Vec<_> = (0..10).map(|i| flow("A", "SVC", 1000 + i, 100.0, 200)).collect();
    let check = check_limits(rows.len(), 100, &limits);
    assert!(!check.within);
    assert!(check.reason.contains("超限"));
    let deg = degrade_top_n(&rows, &limits, &check.reason);
    assert_eq!(deg.top_rows.len(), 2);
    assert!(deg.notice.contains("窗口过大"));
    // guard 走降级分支
    assert!(guard(rows, 100, &limits).is_err());
}

#[test]
fn within_limit_returns_normally() {
    let limits = QueryLimits::default();
    let rows = vec![flow("A", "SVC", 1000, 100.0, 200)];
    assert!(check_limits(rows.len(), 100, &limits).within);
    assert!(guard(rows.clone(), 100, &limits).is_ok());
}

#[test]
fn clock_shift_estimates_and_applies() {
    let flows = vec![flow("A", "SVC", 1000, 100.0, 200), flow("A", "SVC", 2000, 100.0, 200)];
    let logs = vec![
        log("A", "SVC", 1100, "ERROR", "e1"),
        log("A", "SVC", 2100, "ERROR", "e2"),
        log("A", "SVC", 3050, "ERROR", "e3"),
        log("A", "SVC", 4050, "ERROR", "e4"),
        log("A", "SVC", 5100, "ERROR", "e5"),
    ];
    let shift = estimate_clock_shift(&flows, &logs, 5);
    assert!(shift.is_some());
    let mut logs_mut = logs.clone();
    apply_shift(&mut logs_mut, shift.unwrap());
    assert!(logs_mut[0].fields.contains_key("clock_shift_ms"));
    // 样本不足不校正
    assert!(estimate_clock_shift(&flows, &logs[..2], 5).is_none());
}
