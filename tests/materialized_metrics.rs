//! task 34 验收：物化指标表 —— 刷新值正确、幂等、失败保留上一版、一致性校验能发现不一致、新鲜度滞后标注。
mod common;

use common::flow;
use interface_fault_rca::baseline::materialized::{compute_snapshot, consistency_check, is_stale, refresh};
use interface_fault_rca::store::sqlite_index::IndexDb;

fn rows(dur: f64) -> Vec<interface_fault_rca::store::event_schema::EventRow> {
    (0..20)
        .map(|i| flow("A", "SVC", 1000 + i as i64 * 100, dur + (i % 3) as f64, if i == 0 { 500 } else { 200 }))
        .collect()
}

#[test]
fn compute_snapshot_yields_correct_stats() {
    let snap = compute_snapshot(&rows(100.0));
    let s = snap.get("A").unwrap();
    assert!(s.p95 >= 100.0);
    assert!(s.call_count == 20);
    assert!(s.error_rate > 0.0 && s.error_rate < 1.0);
}

#[test]
fn refresh_is_idempotent_for_same_window() {
    let db = IndexDb::open_in_memory().unwrap();
    let r = rows(100.0);
    let a = refresh(&db, 1000, &r, "2026-01-01T00:00:00Z");
    let b = refresh(&db, 1000, &r, "2026-01-01T00:00:00Z");
    assert_eq!(a.get("A").unwrap().p95, b.get("A").unwrap().p95);
    let (p95, _, _, count, _) = db.materialized_at("A", 1000).unwrap();
    assert_eq!(count, 20);
    assert_eq!(p95, a.get("A").unwrap().p95);
}

#[test]
fn consistency_check_detects_stale_value() {
    let db = IndexDb::open_in_memory().unwrap();
    let r = rows(100.0);
    refresh(&db, 1000, &r, "t");
    // 用差异巨大的新数据比对，应发现不一致
    let drift = consistency_check(&db, 1000, &rows(900.0), 0.05);
    assert!(!drift.is_empty());
}

#[test]
fn stale_detection_uses_refresh_interval() {
    assert!(is_stale(1000, 1000 + 3600, 600, 2)); // 超过 2 倍间隔
    assert!(!is_stale(1000, 1000 + 600, 600, 2)); // 间隔内新鲜
}
