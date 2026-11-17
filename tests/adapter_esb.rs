//! task 4 验收：ESB MySQL 适配器 —— 列名映射生效、DATETIME(3)→毫秒、缺凭据报 SourceUnavailable。
//! 本地协议替身仅作测试数据层，不是最终对接方式。
use interface_fault_rca::config::EsbConfig;
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::esb_mysql::{check_credentials, datetime3_to_ms, map_db_row, page_query_sql};
use std::collections::BTreeMap;

fn empty_cfg() -> EsbConfig {
    EsbConfig { host: "".into(), port: 3306, user: "".into(), password: "".into(), db_name: "".into() }
}

#[test]
fn datetime3_to_ms_normalizes_precision() {
    let ms = datetime3_to_ms("2026-11-08 20:00:00.123").unwrap();
    assert!(ms > 0);
    // 同一秒不同毫秒精度，差应为 123ms 量级内单调递增
    let ms0 = datetime3_to_ms("2026-11-08 20:00:00.000").unwrap();
    assert_eq!(ms - ms0, 123);
    assert!(datetime3_to_ms("not-a-date").is_none());
}

#[test]
fn column_mapping_applies_and_counts_unmapped() {
    let mut row = BTreeMap::new();
    row.insert("iface_code".into(), "ESB-ORDER-CREATE".into());
    row.insert("request_time".into(), "2026-11-08 20:00:00.000".into());
    row.insert("extra_col".into(), "x".into());
    let mapping = [
        ("interface_code", "iface_code", true),
        ("ts_ms", "request_time", true),
    ];
    let mapped = map_db_row(&row, &mapping).unwrap();
    assert_eq!(mapped.canonical.get("interface_code").unwrap(), "ESB-ORDER-CREATE");
    assert!(mapped.unmapped.contains(&"extra_col".to_string()));
    assert!(mapped.ts_ms > 0);
    assert_eq!(mapped.ts_raw, "2026-11-08 20:00:00.000");
}

#[test]
fn missing_required_column_is_schema_mismatch() {
    let row = BTreeMap::new();
    let mapping = [("interface_code", "iface_code", true)];
    let err = map_db_row(&row, &mapping).unwrap_err();
    assert_eq!(err.code, ErrorCode::SchemaMismatch);
}

#[test]
fn missing_credentials_returns_source_unavailable() {
    let err = check_credentials(&empty_cfg()).unwrap_err();
    assert_eq!(err.code, ErrorCode::SourceUnavailable);
}

#[test]
fn credentials_present_passes_check() {
    let cfg = EsbConfig {
        host: "127.0.0.1".into(), port: 3306, user: "ro".into(),
        password: "placeholder".into(), db_name: "audit".into(),
    };
    assert!(check_credentials(&cfg).is_ok());
}

#[test]
fn page_query_sql_is_parameterized() {
    let sql = page_query_sql(&["interface_code", "request_time"]);
    assert!(sql.contains("$1"));
    assert!(sql.contains("$2"));
    assert!(sql.contains("interface_code"));
}
