//! task 2 验收：幂等键唯一性 —— 同批 1000 行（含 50 行重复）连续写两次后总行数不变且 __tsid 唯一。
mod common;

use common::flow;
use interface_fault_rca::store::sqlite_index::IndexDb;
use interface_fault_rca::store::tsid::compute_tsid;
use std::collections::BTreeSet;

#[test]
fn tsid_is_stable_for_same_inputs() {
    let a = compute_tsid("esb_audit", 1000, "CODE", "SVC", "flow");
    let b = compute_tsid("esb_audit", 1000, "CODE", "SVC", "flow");
    assert_eq!(a, b);
    assert_eq!(a.len(), 64); // sha256 十六进制
}

#[test]
fn thousand_rows_with_fifty_duplicates_yield_950_unique_tsid() {
    let mut ids = BTreeSet::new();
    let mut ts_list = vec![];
    for i in 0..950 {
        ts_list.push(2000 + i as i64); // 950 个唯一 ts
    }
    for i in 0..50 {
        ts_list.push(2000 + i as i64); // 50 行重复前 50 个 ts
    }
    for ts in ts_list {
        let id = compute_tsid("esb_audit", ts, "CODE", "SVC", "flow");
        ids.insert(id);
    }
    assert_eq!(ids.len(), 950);
}

#[test]
fn repeated_mark_tsid_seen_is_idempotent() {
    let db = IndexDb::open_in_memory().unwrap();
    let tsid = compute_tsid("esb_audit", 1000, "CODE", "SVC", "flow");
    let first = db.mark_tsid_seen(&tsid, "esb_audit", "2026-01-01T00:00:00Z").unwrap();
    let second = db.mark_tsid_seen(&tsid, "esb_audit", "2026-01-01T00:00:00Z").unwrap();
    assert!(first);   // 首次写入返回 true
    assert!(!second); // 重复写入返回 false
    assert_eq!(db.distinct_tsid_count(), 1);
}

#[test]
fn two_rounds_of_same_batch_do_not_increase_rows() {
    let db = IndexDb::open_in_memory().unwrap();
    let rows: Vec<_> = (0..100).map(|i| flow("CODE", "SVC", 3000 + i, 100.0 + i as f64, 200)).collect();
    let mut count_before = 0;
    for r in &rows {
        if db.mark_tsid_seen(&r.tsid, r.source(), "t").unwrap() { count_before += 1; }
    }
    assert_eq!(count_before, 100);
    let mut count_after = 0;
    for r in &rows {
        if db.mark_tsid_seen(&r.tsid, r.source(), "t").unwrap() { count_after += 1; }
    }
    assert_eq!(count_after, 0);
    assert_eq!(db.distinct_tsid_count(), 100);
}
