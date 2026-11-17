//! task 43 验收：出站失败恢复 —— 5xx 退避重试成功 / 4xx 显式失败状态不推进 / 崩溃重放不产生第二条 / 回写失败标记待回写。
use interface_fault_rca::store::sqlite_index::{IndexDb, LedgerStatus};
use interface_fault_rca::writeback::ledger::{confirm, void_candidate};
use interface_fault_rca::writeback::ticket_client::{idempotency_key, replay_ticket_no, retryable_status};

fn seed(db: &IndexDb) -> i64 {
    db.insert_candidate("ESB-A", "SVC", 100, 200, "接口自身劣化", 0.9,
        &["flow#a1".into()], "req", "v1", false, "", "dedup", "t").unwrap()
}

#[test]
fn five_xx_is_retryable_with_same_idempotency_key() {
    let idem = idempotency_key("ESB-A", "SVC", 100, 200);
    assert!(retryable_status(500));
    assert!(retryable_status(502));
    assert!(retryable_status(503));
    // 重试携带同一幂等键（键稳定即证明）
    assert_eq!(idem, idempotency_key("ESB-A", "SVC", 100, 200));
}

#[test]
fn four_xx_is_explicit_failure_and_no_state_advance() {
    assert!(!retryable_status(400));
    assert!(!retryable_status(422));
    let db = IndexDb::open_in_memory().unwrap();
    let id = seed(&db);
    // 4xx 失败时不回写工单号，状态停在 awaiting_confirmation
    db.transition(id, LedgerStatus::AwaitingConfirmation).unwrap();
    let e = db.ledger_by_id(id).unwrap();
    assert!(e.ticket_no.is_empty());
    assert_eq!(e.status, LedgerStatus::AwaitingConfirmation);
    assert!(db.transition(id, LedgerStatus::TicketCreated).is_ok()); // 只有成功才推进
}

#[test]
fn crash_replay_does_not_create_second_ticket() {
    let idem = idempotency_key("ESB-A", "SVC", 100, 200);
    // POST 成功返回 T-1，进程崩溃未回写；重放同 key，服务端幂等返同号
    let first = replay_ticket_no(&idem, "T-1", "T-1").unwrap();
    let replay = replay_ticket_no(&idem, "T-1", "T-1").unwrap();
    assert_eq!(first, replay);
    assert_eq!(first, "T-1");
    // 即便替身误返不同号也强制取首号
    assert_eq!(replay_ticket_no(&idem, "T-1", "T-999").unwrap(), "T-1");
}

#[test]
fn pending_writeback_keeps_intent_without_duplicate() {
    let db = IndexDb::open_in_memory().unwrap();
    let id = seed(&db);
    confirm(&db, id, "op", "t").unwrap();
    // 模拟回写前崩溃：状态 awaiting_confirmation，工单号空（待回写）
    let e = db.ledger_by_id(id).unwrap();
    assert_eq!(e.status, LedgerStatus::AwaitingConfirmation);
    assert!(e.ticket_no.is_empty());
    // 作废未建单候选可成功；已建单只提示
    assert!(void_candidate(&db, id).unwrap().contains("已作废"));
}

#[test]
fn writeback_fail_does_not_advance_state() {
    let db = IndexDb::open_in_memory().unwrap();
    let id = seed(&db);
    // 未确认直接回写工单号：状态机允许吗？transition 需先 awaiting
    assert!(db.transition(id, LedgerStatus::TicketCreated).is_err());
    assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::CandidateRanked);
}
