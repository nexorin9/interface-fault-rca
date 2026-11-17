//! task 31 验收：证据行 ID —— 重启不重号、反查字段一致、窗口外 ID 被拒、与 __tsid 职责分离。
mod common;

use common::flow;
use interface_fault_rca::query::rowid::{check_ref, persist_counter, restore_allocator, RefViolation, RowIdIndex};
use interface_fault_rca::store::sqlite_index::IndexDb;
use interface_fault_rca::store::tsid::{compute_tsid, RowIdAllocator};
use std::collections::HashSet;

#[test]
fn allocator_ids_survive_restart_without_collision() {
    let a = RowIdAllocator::new();
    assert_eq!(a.next("esb_audit"), "flow#a0001");
    assert_eq!(a.next("esb_audit"), "flow#a0002");
    let snap = a.snapshot();
    // 模拟重启：从快照恢复
    let b = RowIdAllocator::from_snapshot(&snap);
    assert_eq!(b.next("esb_audit"), "flow#a0003");
}

#[test]
fn db_persisted_counter_restores() {
    let db = IndexDb::open_in_memory().unwrap();
    let a = RowIdAllocator::new();
    a.next_for_prefix("flow");
    a.next_for_prefix("flow");
    persist_counter(&db, &a);
    let restored = restore_allocator(&db);
    assert_eq!(restored.next_for_prefix("flow"), "flow#a0003");
}

#[test]
fn id_lookup_is_consistent_with_fields() {
    let r = flow("ESB-A", "SVC", 1000, 100.0, 200);
    let idx = RowIdIndex::new(&[r.clone()]);
    assert!(idx.lookup_consistent(&r.row_id, r.ts_ms, r.interface_code()));
    assert!(!idx.lookup_consistent(&r.row_id, 9999, r.interface_code()));
    assert!(idx.lookup(&r.row_id).is_some());
}

#[test]
fn outside_window_id_rejected() {
    let inside = flow("A", "SVC", 1000, 100.0, 200);
    let idx = RowIdIndex::new(&[inside]);
    let all: HashSet<String> = ["flow#A".into(), "flow#outside".into()].into_iter().collect();
    // all 含但窗口不含 → OutsideWindow；all 也不含 → Nonexistent
    assert_eq!(check_ref("flow#outside", &idx, &HashSet::new(), &all), Some(RefViolation::OutsideWindow));
    assert_eq!(check_ref("flow#ghost", &idx, &HashSet::new(), &all), Some(RefViolation::Nonexistent));
}

#[test]
fn row_id_and_tsid_have_distinct_roles() {
    let r1 = flow("A", "SVC", 1000, 100.0, 200);
    let r2 = flow("A", "SVC", 1000, 100.0, 200);
    // __tsid 幂等：相同逻辑行相同 tsid
    assert_eq!(r1.tsid, r2.tsid);
    assert_eq!(r1.tsid, compute_tsid("esb_audit", 1000, "A", "SVC", "flow"));
    // __row_id 唯一标识：这里由构造器按 code 生成，两者语义不同——一个是哈希、一个是可读 ID
    assert_ne!(r1.tsid.len(), r1.row_id.len());
}
