//! task 40 验收：可审计调用元数据 —— 字段齐全、不含 prompt/响应全文、候选按 request_id 回查、降级历史可回查。
use interface_fault_rca::ai::audit_meta::{build_meta, digest, meta_is_clean, DbAuditSink};
use interface_fault_rca::ai::ports::AuditSink;
use interface_fault_rca::store::sqlite_index::IndexDb;

#[test]
fn metadata_fields_are_populated() {
    let m = build_meta("req-1", "model-x", "http://host:8000/v1/chat?token=secret",
        "prompt body SECRET_TEXT", "response body SECRET_RESP", 42, 100, true, 1);
    assert_eq!(m.request_id, "req-1");
    assert_eq!(m.model, "model-x");
    assert_eq!(m.token_count, 42);
    assert_eq!(m.elapsed_ms, 100);
    assert!(m.degraded);
    assert_eq!(m.retries, 1);
}

#[test]
fn audit_excludes_prompt_and_response_body() {
    let m = build_meta("r", "m", "http://h:8000", "SECRET_PROMPT_12345", "SECRET_RESP_67890", 1, 1, false, 0);
    assert!(meta_is_clean(&m, "SECRET_PROMPT_12345"));
    assert!(meta_is_clean(&m, "SECRET_RESP_67890"));
    // host 只保留 host 部分，不含路径与凭据
    assert!(!m.host.contains("token"));
}

#[test]
fn candidate_can_lookup_call_by_request_id() {
    let db = IndexDb::open_in_memory().unwrap();
    let m = build_meta("req-lookup", "m", "http://h", "in", "out", 5, 10, false, 0);
    DbAuditSink { db: &db }.write(&m);
    let found = db.model_call_by_request("req-lookup").unwrap();
    assert_eq!(found.0, "m");
    assert!(!found.2); // degraded=false
    assert_eq!(db.model_call_count(), 1);
}

#[test]
fn degradation_history_is_queryable() {
    let db = IndexDb::open_in_memory().unwrap();
    for i in 0..3 {
        let m = build_meta(&format!("r{i}"), "m", "http://h", "in", "out", 1, 1, i != 0, 0);
        DbAuditSink { db: &db }.write(&m);
    }
    assert_eq!(db.model_call_count(), 3);
    assert!(db.model_call_by_request("r1").unwrap().2);
    assert!(!db.model_call_by_request("r0").unwrap().2);
}

#[test]
fn digest_is_stable_and_hex() {
    let d = digest("same-input");
    assert_eq!(d, digest("same-input"));
    assert_eq!(d.len(), 64);
    assert_ne!(d, digest("other"));
}
