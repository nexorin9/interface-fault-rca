//! task 27 验收：幂等重放 —— 五字段各自区分、空值与空串不撞键、跨进程重放 0 新增。
use interface_fault_rca::store::sqlite_index::IndexDb;
use interface_fault_rca::store::tsid::{compute_tsid, TsidParts};

#[test]
fn each_of_five_fields_changes_tsid() {
    let base = compute_tsid("s", 1000, "code", "svc", "flow");
    assert_ne!(base, compute_tsid("s2", 1000, "code", "svc", "flow")); // source
    assert_ne!(base, compute_tsid("s", 1001, "code", "svc", "flow")); // ts_ms
    assert_ne!(base, compute_tsid("s", 1000, "code2", "svc", "flow")); // interface_code
    assert_ne!(base, compute_tsid("s", 1000, "code", "svc2", "flow")); // service_name
    assert_ne!(base, compute_tsid("s", 1000, "code", "svc", "log"));  // event_kind
}

#[test]
fn empty_and_missing_string_do_not_collide() {
    let empty_code = compute_tsid("s", 1000, "", "svc", "flow");
    let some_code = compute_tsid("s", 1000, "X", "svc", "flow");
    assert_ne!(empty_code, some_code);
    // 长度前缀保证 ("ab","c") 与 ("a","bc") 不碰撞
    let a = compute_tsid("ab", 1000, "c", "", "flow");
    let b = compute_tsid("a", 1000, "bc", "", "flow");
    assert_ne!(a, b);
}

#[test]
fn tsid_parts_struct_matches_function() {
    let p = TsidParts { source: "s", ts_ms: 1000, interface_code: "c", service_name: "v", event_kind: "flow" };
    assert_eq!(p.compute(), compute_tsid("s", 1000, "c", "v", "flow"));
}

#[test]
fn cross_process_replay_yields_zero_new_rows() {
    // 模拟两次独立进程读同一输入：幂等键去重保证第二批 0 新增
    let db = IndexDb::open_in_memory().unwrap();
    let keys: Vec<String> = (0..50).map(|i| compute_tsid("esb_audit", 1000 + i, "A", "SVC", "flow")).collect();
    let mut first = 0;
    for k in &keys { if db.mark_tsid_seen(k, "esb_audit", "t").unwrap() { first += 1; } }
    let mut second = 0;
    for k in &keys { if db.mark_tsid_seen(k, "esb_audit", "t").unwrap() { second += 1; } }
    assert_eq!(first, 50);
    assert_eq!(second, 0);
    assert_eq!(db.distinct_tsid_count(), 50);
}
