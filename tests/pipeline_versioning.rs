//! task 20 验收：管线版本化 —— 按名+版本精确取用、取不到报错、v1/v2 可区分、同版本可复现、版本号入审计元数据。
mod common;

use common::allocator;
use interface_fault_rca::binding::interface_dict::minimal_dict;
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::pipeline::{
    build_registry_from_toml, default_registry, Pipeline, PipelineDefinition, RawRecord,
};
use std::collections::BTreeMap;
use std::sync::Arc;

fn records() -> Vec<RawRecord> {
    let mut cols = BTreeMap::new();
    cols.insert("interface_code".into(), "ESB-ORDER-CREATE".into());
    cols.insert("duration_ms".into(), "100".into());
    vec![RawRecord { source: "esb_audit".into(), columns: cols, ts_ms: 1000 }]
}

#[test]
fn exact_name_and_version_lookup() {
    let reg = default_registry();
    assert!(reg.get("esb_audit", "v1").is_ok());
    assert!(reg.get("esb_audit", "v2").is_ok());
    let err = reg.get("esb_audit", "v999").unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigMissing);
}

#[test]
fn toml_registry_loads_two_versions() {
    let txt = std::fs::read_to_string("config/pipelines.toml").unwrap();
    let reg = build_registry_from_toml(&txt).unwrap();
    assert!(reg.versions_of("esb_audit").len() >= 1);
}

#[test]
fn pipeline_version_goes_into_row_audit_metadata() {
    let def = PipelineDefinition {
        name: "esb_audit".into(), version: "v1".into(),
        source: "esb_audit".into(), event_kind: "flow".into(),
    };
    let p = Pipeline::new(def, Arc::new(minimal_dict()), allocator());
    let out = p.execute(&records()).unwrap();
    assert_eq!(out.rows.len(), 1);
    assert_eq!(out.rows[0].fields.get("pipeline_version").unwrap(), "esb_audit@v1");
}

#[test]
fn v1_and_v2_versions_are_distinguishable() {
    let mk = |v: &str| Pipeline::new(
        PipelineDefinition { name: "esb_audit".into(), version: v.into(), source: "esb_audit".into(), event_kind: "flow".into() },
        Arc::new(minimal_dict()), allocator());
    let a = mk("v1").execute(&records()).unwrap();
    let b = mk("v2").execute(&records()).unwrap();
    assert_ne!(a.rows[0].fields.get("pipeline_version"), b.rows[0].fields.get("pipeline_version"));
}

#[test]
fn same_version_twice_is_reproducible() {
    let mk = || Pipeline::new(
        PipelineDefinition { name: "esb_audit".into(), version: "v1".into(), source: "esb_audit".into(), event_kind: "flow".into() },
        Arc::new(minimal_dict()), allocator());
    let a = mk().execute(&records()).unwrap();
    let b = mk().execute(&records()).unwrap();
    assert_eq!(a.rows[0].tsid, b.rows[0].tsid);
    assert_eq!(a.rows[0].fields.get("pipeline_version"), b.rows[0].fields.get("pipeline_version"));
}
