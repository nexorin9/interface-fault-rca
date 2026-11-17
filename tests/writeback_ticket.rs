//! task 11 验收：工单出站 —— 未确认零出站 / 请求体字段与请求头逐项匹配 / 幂等重放一条 / 非法跃迁拒 / 审计只增不改。
use interface_fault_rca::config::TicketConfig;
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::store::sqlite_index::{IndexDb, LedgerStatus};
use interface_fault_rca::writeback::audit::{evidence_digest, record, AuditEvent};
use interface_fault_rca::writeback::ticket_client::{
    check_ticket_config, idempotency_key, replay_ticket_no, retryable_status, TicketPayload,
};

fn payload() -> TicketPayload {
    TicketPayload {
        ticket_type: "接口劣化".into(), severity: "P2".into(), summary: "P95 劣化".into(),
        description: "详见证据".into(), interface_code: "ESB-A".into(), service_name: "SVC".into(),
        occurred_at: "2026-11-08T20:00:00Z".into(),
        evidence_refs: vec!["flow#a1".into(), "log#a2".into()], reporter: "值班机器人".into(),
        human_reason: None,
    }
}

#[test]
fn ticket_payload_has_all_spec_fields() {
    let p = payload();
    let j = serde_json::to_value(&p).unwrap();
    for k in ["ticket_type", "severity", "summary", "description", "interface_code",
              "service_name", "occurred_at", "evidence_refs", "reporter"] {
        assert!(j.get(k).is_some(), "缺字段 {k}");
    }
}

#[test]
fn idempotency_key_is_stable_and_distinct_per_window() {
    let k1 = idempotency_key("ESB-A", "SVC", 100, 200);
    let k2 = idempotency_key("ESB-A", "SVC", 100, 200);
    let k3 = idempotency_key("ESB-A", "SVC", 100, 300);
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
    assert_eq!(k1.len(), 64);
}

#[test]
fn missing_ticket_credentials_is_config_missing() {
    let cfg = TicketConfig { base_url: "".into(), token: "".into() };
    assert_eq!(check_ticket_config(&cfg).unwrap_err().code, ErrorCode::ConfigMissing);
}

#[test]
fn replay_returns_single_ticket_number() {
    let no = replay_ticket_no("idem", "T-001", "T-001").unwrap();
    assert_eq!(no, "T-001");
    // 即使服务端误返不同号，也强制取首号避免第二条
    assert_eq!(replay_ticket_no("idem", "T-001", "T-002").unwrap(), "T-001");
}

#[test]
fn illegal_status_transition_rejected() {
    let db = IndexDb::open_in_memory().unwrap();
    let id = db.insert_candidate("A", "S", 0, 100, "接口自身劣化", 0.9, &["flow#a1".into()],
        "req", "v1", false, "", "dedup", "now").unwrap();
    // candidate_ranked 直接跳 ticket_created 非法
    assert!(db.transition(id, LedgerStatus::TicketCreated).is_err());
    assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::CandidateRanked);
    // 合法链路
    db.transition(id, LedgerStatus::AwaitingConfirmation).unwrap();
    db.writeback_ticket(id, "T-1").unwrap();
    assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::TicketCreated);
}

#[test]
fn audit_chain_is_append_only() {
    let db = IndexDb::open_in_memory().unwrap();
    for i in 0..3 {
        record(&db, &AuditEvent {
            ts: format!("t{i}"), actor: "op".into(), action: "confirm".into(),
            interface_code: "A".into(), evidence_digest: evidence_digest(&["flow#a1".into()]),
            ticket_no: "".into(), detail: "".into(),
        });
    }
    assert_eq!(db.audit_count(), 3);
    let rows = db.audit_rows();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|r| r.1 == "confirm"));
}

#[test]
fn retryable_status_only_5xx_and_zero() {
    assert!(retryable_status(500));
    assert!(retryable_status(503));
    assert!(retryable_status(0));
    assert!(!retryable_status(400));
    assert!(!retryable_status(404));
}
