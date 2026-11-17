//! task 7 验收：实体绑定字典 —— 正常映射 / 歧义拒绝 / unbound / host 缺失四类。
use interface_fault_rca::binding::interface_dict::{
    coverage, minimal_dict, Binding, Dictionary, InterfaceEntry,
};
use interface_fault_rca::error::ErrorCode;
use std::collections::BTreeMap;

#[test]
fn exact_and_alias_mapping() {
    let d = minimal_dict();
    assert!(matches!(d.resolve("ESB-ORDER-CREATE").unwrap(), Binding::Bound { .. }));
    assert!(matches!(d.resolve("ORDER_CREATE").unwrap(), Binding::Bound { .. }));
}

#[test]
fn ambiguous_code_returns_error_and_is_reported() {
    let mut d = minimal_dict();
    d.entries.push(InterfaceEntry {
        code: "ESB-ORDER-CREATE".into(), service_name: "另一服务".into(),
        vendor_name: "".into(), hosts: vec![], aliases: vec![], basis: "".into(),
    });
    let err = d.resolve("ESB-ORDER-CREATE").unwrap_err();
    assert_eq!(err.code, ErrorCode::AmbiguousInterface);
    assert!(d.is_ambiguous("ESB-ORDER-CREATE"));
}

#[test]
fn unmapped_value_is_unbound() {
    let d = minimal_dict();
    let b = d.resolve("UNKNOWN-CODE").unwrap();
    assert_eq!(b, Binding::Unbound { raw: "UNKNOWN-CODE".into() });
}

#[test]
fn host_missing_is_not_error() {
    let d = minimal_dict();
    assert_eq!(d.host_from_labels(&BTreeMap::new()), None);
    let labels: BTreeMap<String, String> = [("job".to_string(), "esb-gateway".to_string())].into_iter().collect();
    assert_eq!(d.host_from_labels(&labels), Some("esb-gw-01".into()));
}

#[test]
fn coverage_reports_unbound_and_ambiguous_without_candidate_pollution() {
    let d = minimal_dict();
    let observed = vec![("ESB-ORDER-CREATE".to_string(), 100usize), ("UNKNOWN".to_string(), 5)];
    let (unbound, ambiguous) = coverage(&d, &observed);
    assert!(unbound.iter().any(|r| r.raw.contains("UNKNOWN")));
    assert!(ambiguous.is_empty());
    // 已映射的规范码不出现在 unbound 清单
    assert!(!unbound.iter().any(|r| r.raw.contains("ESB-ORDER-CREATE")));
}

#[test]
fn all_dict_codes_pass_schema_resolution() {
    let d: Dictionary = minimal_dict();
    for e in &d.entries {
        assert!(matches!(d.resolve(&e.code), Ok(Binding::Bound { .. })));
    }
}
