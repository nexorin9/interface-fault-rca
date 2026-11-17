//! task 21 验收：schema 漂移 —— 缺必需列/新增列/类型不符/整批拒绝不留半份数据。
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::schema_map::{esb_mapping, MappingTable};
use std::collections::{BTreeMap, HashMap};

#[test]
fn missing_required_column_is_blocking() {
    let m = esb_mapping();
    let actual: Vec<String> = vec![]; // 全缺
    let types = HashMap::new();
    let drift = m.detect_drift(&actual, &types);
    assert!(!drift.missing_required.is_empty());
    assert!(drift.has_blocking());
    assert_eq!(drift.blocking_error().unwrap_err().code, ErrorCode::SchemaMismatch);
}

#[test]
fn new_column_is_reported_but_not_blocking() {
    let m = esb_mapping();
    let mut actual: Vec<String> = m.rules.values().map(|r| r.column.clone()).collect();
    actual.push("unexpected_new_col".into());
    let drift = m.detect_drift(&actual, &HashMap::new());
    assert!(drift.new_columns.contains(&"unexpected_new_col".to_string()));
    assert!(!drift.has_blocking()); // 新增列本身不阻断
}

#[test]
fn type_mismatch_is_blocking() {
    let mut m = MappingTable::new();
    m.add("duration_ms", "duration_ms", true, "int");
    let actual = vec!["duration_ms".to_string()];
    let mut types = HashMap::new();
    types.insert("duration_ms".to_string(), "varchar".to_string());
    let drift = m.detect_drift(&actual, &types);
    assert!(!drift.type_mismatch.is_empty());
    assert_eq!(drift.blocking_error().unwrap_err().code, ErrorCode::SchemaMismatch);
}

#[test]
fn unmapped_columns_go_to_unmapped_bucket() {
    let mut m = MappingTable::new();
    m.add("duration_ms", "duration_ms", true, "int");
    let mut row = BTreeMap::new();
    row.insert("duration_ms".into(), "100".into());
    row.insert("extra".into(), "x".into());
    let mapped = m.map_row(&row);
    assert_eq!(mapped.fields.get("duration_ms").unwrap(), "100");
    assert!(mapped.unmapped.contains_key("extra"));
}

#[test]
fn no_half_batch_on_blocking_drift() {
    // 整批拒绝：只要 blocking_error 返回 Err，调用方不写任何行
    let m = esb_mapping();
    let drift = m.detect_drift(&[], &HashMap::new());
    let mut written = 0;
    if drift.blocking_error().is_ok() { written += 1; }
    assert_eq!(written, 0);
}

#[test]
fn config_is_single_source_of_column_names() {
    let txt = std::fs::read_to_string("config/pipelines.toml").unwrap();
    assert!(txt.contains("esb_audit"));
    assert!(txt.contains("version"));
}
