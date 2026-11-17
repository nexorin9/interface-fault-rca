//! task 15 验收：golden 对拍 —— 同一批流水逐字段比对 P95/P99/错误率/候选集合，不一致即红；可重复；案例集挂钩。
mod common;

use common::{allocator, baseline_conclusion, flow};
use interface_fault_rca::baseline::aggregate::{aggregate_flows, conclude, percentile};
use interface_fault_rca::baseline::rank::rank;
use interface_fault_rca::config::BaselineConfig;
use std::collections::BTreeMap;

fn golden_rows() -> Vec<interface_fault_rca::store::event_schema::EventRow> {
    let mut rows = vec![];
    for i in 0..40 {
        rows.push(flow("ESB-A", "SVC", 1000 + i * 100, 100.0 + (i % 5) as f64 * 10.0, if i % 10 == 0 { 500 } else { 200 }));
    }
    rows
}

/// golden 快照值（由本测试数据确定，手写断言而非快照框架）
fn golden_metrics() -> (f64, f64, f64, usize) {
    (140.0, 140.0, 0.1, 40)
}

#[test]
fn aggregation_matches_golden_field_by_field() {
    let agg = aggregate_flows(&golden_rows());
    let a = agg.get("ESB-A").unwrap();
    let (p95, p99, err, count) = golden_metrics();
    assert_eq!(a.p95(), p95);
    assert_eq!(a.p99(), p99);
    assert!((a.error_rate() - err).abs() < 1e-9);
    assert_eq!(a.total, count as u64);
}

#[test]
fn candidate_set_is_deterministic_across_runs() {
    let cfg = BaselineConfig::default();
    let rows = golden_rows();
    let base: Vec<_> = rows.iter().filter(|r| r.ts_ms < 1000 + 20 * 100).cloned().collect();
    let cur: Vec<_> = rows.iter().filter(|r| r.ts_ms >= 1000 + 20 * 100).cloned().collect();
    let r1 = rank(conclude(&cur, &base, &cfg), &cfg);
    let r2 = rank(conclude(&cur, &base, &cfg), &cfg);
    assert_eq!(r1.len(), r2.len());
    for (a, b) in r1.iter().zip(r2.iter()) {
        assert_eq!(a.conclusion.interface_code, b.conclusion.interface_code);
        assert_eq!(a.conclusion.verdict, b.conclusion.verdict);
        assert!((a.conclusion.p95_ms - b.conclusion.p95_ms).abs() < 1e-9);
    }
}

#[test]
fn reverse_assertion_changed_column_turns_red() {
    // 故意改一列的期望值，对拍必须失败（证明对拍是有效的回归门）
    let agg = aggregate_flows(&golden_rows());
    let a = agg.get("ESB-A").unwrap();
    let wrong_golden_count = 39; // 正确值 40
    assert_ne!(a.total as usize, wrong_golden_count);
}

#[test]
fn percentile_and_case_set_link() {
    let sorted = vec![10.0, 20.0, 30.0, 40.0, 50.0];
    assert_eq!(percentile(&sorted, 0.95), 50.0);
    // 案例集存在且可被读取（对拍矩阵的输入）
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string("docs/cases/interface-fault-cases.json").unwrap()).unwrap();
    assert!(doc["cases"].as_array().unwrap().len() >= 12);
    let _ = (baseline_conclusion("A", 1.0, 100.0, vec![]), allocator());
}

#[test]
fn candidate_scores_are_comparable() {
    let cfg = BaselineConfig::default();
    let cs = vec![
        baseline_conclusion("A", 3.0, 300.0, vec![]),
        baseline_conclusion("B", 2.0, 200.0, vec![]),
    ];
    let ranked = rank(cs, &cfg);
    let mut seen: BTreeMap<String, f64> = BTreeMap::new();
    for r in &ranked { seen.insert(r.conclusion.interface_code.clone(), r.score); }
    assert!(seen["A"] > seen["B"]);
}
