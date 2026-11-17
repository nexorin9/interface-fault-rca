//! task 2 验收：统一事件表 schema 校验 —— 非法 tag 键 / 越界 fields 键返回 SchemaMismatch。
mod common;

use common::{flow, log, metric};
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::store::event_schema::{allowed_fields, TAG_WHITELIST};

#[test]
fn tag_whitelist_has_exactly_six_keys() {
    assert_eq!(TAG_WHITELIST.len(), 6);
    for k in TAG_WHITELIST {
        assert!(["source", "event_kind", "interface_code", "service_name", "vendor_name", "host"].contains(&k));
    }
}

#[test]
fn allowed_fields_branch_by_kind() {
    assert!(allowed_fields("flow").is_some());
    assert!(allowed_fields("log").is_some());
    assert!(allowed_fields("metric").is_some());
    assert!(allowed_fields("unknown").is_none());
}

#[test]
fn illegal_tag_key_is_schema_mismatch() {
    let mut r = flow("CODE", "SVC", 1000, 100.0, 200);
    r.tags.insert("illegal_key".into(), "x".into());
    assert_eq!(r.validate().unwrap_err().code, ErrorCode::SchemaMismatch);
}

#[test]
fn out_of_range_field_key_is_schema_mismatch() {
    let mut r = log("CODE", "SVC", 1000, "ERROR", "boom");
    r.fields.insert("not_allowed".into(), "x".into());
    assert_eq!(r.validate().unwrap_err().code, ErrorCode::SchemaMismatch);
}

#[test]
fn valid_flow_log_metric_rows_pass_schema() {
    assert!(flow("CODE", "SVC", 1000, 100.0, 200).validate().is_ok());
    assert!(log("CODE", "SVC", 1000, "INFO", "ok").validate().is_ok());
    assert!(metric("CODE", "host-1", 1000, "cpu", 0.5).validate().is_ok());
}

#[test]
fn evidence_summary_carries_row_id_and_payload() {
    let r = flow("CODE", "SVC", 1000, 123.0, 500);
    let s = r.evidence_summary();
    assert!(s.contains("flow#CODE"));
    assert!(s.contains("dur=123ms"));
    assert!(s.contains("rc=500"));
}
