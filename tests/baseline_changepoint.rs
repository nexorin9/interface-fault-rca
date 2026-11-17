//! task 9 验收：四形态变更点检测 —— 阶梯劣化/尖刺/缓慢爬升/基线自身抖动互不混淆。
use interface_fault_rca::baseline::changepoint::{detect, ChangeShape};

#[test]
fn step_degradation_detected() {
    // 前半段稳定在 100，后半段跃升到 200 并保持
    let vals = vec![100.0, 105.0, 98.0, 102.0, 300.0, 310.0, 295.0, 305.0];
    let cp = detect(&vals, 100.0);
    assert_eq!(cp.shape, ChangeShape::StepDegradation, "detail={}", cp.detail);
    assert!(cp.confidence >= 0.5);
}

#[test]
fn spike_detected_and_not_confused_with_step() {
    let vals = vec![100.0, 102.0, 98.0, 101.0, 99.0, 800.0, 100.0, 97.0];
    let cp = detect(&vals, 100.0);
    assert_eq!(cp.shape, ChangeShape::Spike, "detail={}", cp.detail);
    assert_ne!(cp.shape, ChangeShape::StepDegradation);
}

#[test]
fn slow_ramp_detected() {
    let vals = vec![100.0, 115.0, 130.0, 145.0, 160.0, 175.0, 190.0, 205.0];
    let cp = detect(&vals, 100.0);
    assert_eq!(cp.shape, ChangeShape::SlowRamp, "detail={}", cp.detail);
}

#[test]
fn baseline_jitter_detected() {
    let vals = vec![80.0, 130.0, 70.0, 140.0, 75.0, 135.0, 65.0, 145.0];
    let cp = detect(&vals, 105.0);
    assert_eq!(cp.shape, ChangeShape::BaselineJitter, "detail={}", cp.detail);
}

#[test]
fn four_shapes_are_distinct_labels() {
    use std::collections::BTreeSet;
    let labels: BTreeSet<_> = [
        ChangeShape::StepDegradation.as_str(),
        ChangeShape::Spike.as_str(),
        ChangeShape::SlowRamp.as_str(),
        ChangeShape::BaselineJitter.as_str(),
    ].into_iter().collect();
    assert_eq!(labels.len(), 4);
}
