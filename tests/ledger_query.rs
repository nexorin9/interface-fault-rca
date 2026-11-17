//! task 42 验收：台账查询四维度 + 交班摘要含 Top-N 与待确认项 + 引用可回查 + 不含患者明细。
use interface_fault_rca::store::sqlite_index::{IndexDb, LedgerStatus};
use interface_fault_rca::writeback::ledger::{query, LedgerQuery};
use interface_fault_rca::writeback::report::{export_handover, handover_text, refs_in_ledger};

fn seed(db: &IndexDb) -> Vec<i64> {
    let a = db.insert_candidate("ESB-A", "SVC-A", 100, 200, "接口自身劣化", 0.9,
        &["flow#a1".into()], "req1", "v1", false, "", "k1", "t").unwrap();
    let b = db.insert_candidate("ESB-B", "SVC-B", 100, 200, "厂商返回异常", 0.7,
        &["log#b1".into()], "req2", "v1", true, "", "k2", "t").unwrap();
    vec![a, b]
}

#[test]
fn query_by_four_dimensions() {
    let db = IndexDb::open_in_memory().unwrap();
    let ids = seed(&db);
    db.confirm(ids[1], "op", "t").unwrap();
    assert_eq!(query(&db, &LedgerQuery::ByInterface("ESB-A".into())).len(), 1);
    assert_eq!(query(&db, &LedgerQuery::ByWindow(0, 1000)).len(), 2);
    assert_eq!(query(&db, &LedgerQuery::ByStatus(LedgerStatus::CandidateRanked)).len(), 1);
    assert_eq!(query(&db, &LedgerQuery::ByStatus(LedgerStatus::AwaitingConfirmation)).len(), 1);
    assert_eq!(query(&db, &LedgerQuery::All).len(), 2);
    // 工单号维度（先建单）
    db.writeback_ticket(ids[1], "T-42").unwrap();
    assert_eq!(query(&db, &LedgerQuery::ByTicketNo("T-42".into())).len(), 1);
}

#[test]
fn handover_contains_top_n_and_pending() {
    let db = IndexDb::open_in_memory().unwrap();
    seed(&db);
    let entries = db.ledger_all();
    let text = handover_text(100, 200, &entries, true);
    assert!(text.contains("Top-2"));
    assert!(text.contains("ESB-A"));
    assert!(text.contains("待确认"));
    assert!(text.contains("规则基线结论")); // degraded 标注
}

#[test]
fn handover_refs_all_in_ledger() {
    let db = IndexDb::open_in_memory().unwrap();
    seed(&db);
    let entries = db.ledger_all();
    let text = handover_text(100, 200, &entries, false);
    assert!(refs_in_ledger(&text, &entries));
    // 加入一个白名单外引用应被识别
    let tampered = format!("{text} 证据行 flow#ghost。");
    assert!(!refs_in_ledger(&tampered, &entries));
}

#[test]
fn export_handover_writes_two_files() {
    let db = IndexDb::open_in_memory().unwrap();
    seed(&db);
    let entries = db.ledger_all();
    let dir = std::env::temp_dir().join(format!("ift-handover-{}", std::process::id()));
    let (txt, md) = export_handover(&dir, 100, 200, &entries, false);
    assert!(std::path::Path::new(&txt).exists());
    assert!(std::path::Path::new(&md).exists());
    assert!(!std::fs::read_to_string(&txt).unwrap().is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn handover_has_no_patient_identifiers() {
    let db = IndexDb::open_in_memory().unwrap();
    seed(&db);
    let text = handover_text(100, 200, &db.ledger_all(), false);
    // 摘要只涉及接口维度，不含住院号/患者化名等拟真标识
    assert!(!text.contains("住院号"));
    assert!(!text.contains("患者"));
}
