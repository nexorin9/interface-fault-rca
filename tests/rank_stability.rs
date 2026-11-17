//! task 36 验收：候选排序 —— 连续排序逐位一致、并列字典序打破、微扰第一名稳定、得分构成可复核。
mod common;

use common::baseline_conclusion;
use interface_fault_rca::baseline::rank::{rank, smooth, ScoreBreakdown};
use interface_fault_rca::config::BaselineConfig;

fn items() -> Vec<interface_fault_rca::baseline::aggregate::BaselineConclusion> {
    vec![
        baseline_conclusion("ESB-B", 3.0, 300.0, vec!["flow#b".into()]),
        baseline_conclusion("ESB-A", 5.0, 500.0, vec!["flow#a".into()]),
        baseline_conclusion("ESB-C", 2.0, 200.0, vec!["flow#c".into()]),
    ]
}

fn codes(v: &[interface_fault_rca::baseline::rank::RankedCandidate]) -> Vec<&str> {
    v.iter().map(|x| x.conclusion.interface_code.as_str()).collect()
}

#[test]
fn repeated_ranking_is_bitwise_identical() {
    let cfg = BaselineConfig::default();
    let r1 = rank(items(), &cfg);
    let r2 = rank(items(), &cfg);
    assert_eq!(codes(&r1), codes(&r2));
    for n in 0..10 {
        assert_eq!(codes(&rank(items(), &cfg)), codes(&r1));
    }
}

#[test]
fn ties_broken_by_interface_code_lexicographically() {
    let cfg = BaselineConfig::default();
    let two = vec![
        baseline_conclusion("Z", 3.0, 300.0, vec![]),
        baseline_conclusion("A", 3.0, 300.0, vec![]),
    ];
    let r = rank(two, &cfg);
    assert_eq!(r[0].conclusion.interface_code, "A");
    assert_eq!(r[1].conclusion.interface_code, "Z");
}

#[test]
fn top_stable_under_small_jitter() {
    let cfg = BaselineConfig::default();
    let base = rank(items(), &cfg);
    let jittered = vec![
        baseline_conclusion("ESB-B", 3.02, 298.0, vec![]),
        baseline_conclusion("ESB-A", 5.05, 495.0, vec![]),
        baseline_conclusion("ESB-C", 2.01, 201.0, vec![]),
    ];
    let jittered = rank(jittered, &cfg);
    assert_eq!(base[0].conclusion.interface_code, jittered[0].conclusion.interface_code);
}

#[test]
fn score_breakdown_is_auditable() {
    let cfg = BaselineConfig::default();
    let r = rank(items(), &cfg);
    let b: &ScoreBreakdown = &r[0].score_breakdown;
    assert!(b.ratio_component > 0.0);
    assert!(r[0].score > 0.0);
    // 分项之和等于总分（浮点容差）
    let sum = b.latency_component + b.ratio_component + b.error_component + b.host_component;
    assert!((sum - r[0].score).abs() < 1e-9);
}

#[test]
fn smooth_suppresses_single_spike() {
    let s = smooth(&[10.0, 10.0, 1000.0, 10.0, 10.0]);
    assert!(s[2] < 1000.0);
    assert_eq!(s[0], 10.0); // 明显差异项不改变
}
