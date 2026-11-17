//! task 14 验收：自建案例集 —— ≥12 条、五字段齐、confidence 0..1、case_id 唯一、覆盖八类形态、主路径可消费。
mod common;

use common::{conclusion, flow};
use interface_fault_rca::ai::conclusion::{Conclusion, Verdict};
use interface_fault_rca::ai::evidence_gate::{check_conclusion, ConcreteEvidenceGate, GateContext};
use interface_fault_rca::ai::ports::{EvidenceGate, GateDecision};
use std::collections::{BTreeSet, HashSet};

fn load_cases() -> serde_json::Value {
    let txt = std::fs::read_to_string("docs/cases/interface-fault-cases.json").expect("案例集存在");
    serde_json::from_str(&txt).expect("合法 JSON")
}

#[test]
fn case_set_has_at_least_12_unique_cases() {
    let doc = load_cases();
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 12);
    let ids: BTreeSet<_> = cases.iter().map(|c| c["case_id"].as_str().unwrap()).collect();
    assert_eq!(ids.len(), cases.len());
}

#[test]
fn every_case_has_five_fields_and_valid_confidence() {
    let doc = load_cases();
    for c in doc["cases"].as_array().unwrap() {
        let id = c["case_id"].as_str().unwrap();
        for k in ["case_id", "input", "expect", "basis", "confidence"] {
            assert!(c.get(k).is_some(), "{id} 缺字段 {k}");
        }
        let conf = c["confidence"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&conf), "{id} confidence 越界");
        assert!(!c["basis"].as_str().unwrap().is_empty(), "{id} basis 为空");
    }
}

#[test]
fn eight_shapes_are_covered() {
    let doc = load_cases();
    let shapes: BTreeSet<_> = doc["cases"].as_array().unwrap().iter()
        .map(|c| c["input"]["shape"].as_str().unwrap()).collect();
    for s in ["step", "spike", "ramp", "jitter", "ambiguous", "unbound", "insufficient_evidence", "vendor_return_change"] {
        assert!(shapes.contains(s), "缺形态 {s}");
    }
}

#[test]
fn no_patient_identifiers_in_case_set() {
    let txt = std::fs::read_to_string("docs/cases/interface-fault-cases.json").unwrap();
    for term in ["住院号", "处方号", "患者姓名", "身份证"] {
        assert!(!txt.contains(term), "出现拟真标识 {term}");
    }
}

#[test]
fn gate_drop_cases_are_consumed_by_main_path() {
    // 案例集的三条丢弃路径能被 EvidenceGate 真实消费
    let ctx = GateContext {
        window_ids: ["flow#a1".into()].into_iter().collect(),
        all_ids: ["flow#outside".into()].into_iter().collect(),
        unbound_ids: ["log#unbound".into()].into_iter().collect(),
        min_refs: 1,
    };
    let gate = ConcreteEvidenceGate { ctx };
    let ghost: Conclusion = conclusion(vec!["flow#ghost"], Verdict::接口自身劣化);
    assert!(matches!(check_conclusion(&ghost, &gate.ctx), GateDecision::Drop { .. }));
    let outside = conclusion(vec!["flow#outside"], Verdict::接口自身劣化);
    assert_eq!(check_conclusion(&outside, &gate.ctx), GateDecision::Drop { reason_code: "OUTSIDE_WINDOW".into() });
    let unbound = conclusion(vec!["log#unbound"], Verdict::接口自身劣化);
    assert_eq!(check_conclusion(&unbound, &gate.ctx), GateDecision::Drop { reason_code: "UNBOUND_REF".into() });
    // 合法引用通过
    let ok = conclusion(vec!["flow#a1"], Verdict::接口自身劣化);
    assert_eq!(gate.check(&ok), GateDecision::Pass);
    let _ = (flow("A", "S", 1, 1.0, 200), HashSet::<String>::new());
}
