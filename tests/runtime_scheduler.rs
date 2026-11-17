//! task 18 验收：双 runtime 调度 —— 模型通道饱和不阻塞采集、权重上限生效、重复启动被拒、三类可观测计数。
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::runtime::{backoff_sequence, Admission, DualRuntime, Scheduler, WorkloadClass};

#[test]
fn model_channel_saturation_does_not_block_ingest() {
    let s = Scheduler::new();
    s.set_weight(WorkloadClass::Model, 1).unwrap();
    // 模型通道打满
    assert_eq!(s.admit(WorkloadClass::Model), Admission::Granted);
    assert_eq!(s.admit(WorkloadClass::Model), Admission::Queued);
    // 采集通道仍可入队（独立无锁计数）
    assert_eq!(s.admit(WorkloadClass::Ingest), Admission::Granted);
    assert_eq!(s.admit(WorkloadClass::Ingest), Admission::Granted);
}

#[test]
fn weight_limit_enforced() {
    let s = Scheduler::new();
    s.set_weight(WorkloadClass::Query, 2).unwrap();
    assert_eq!(s.limit_of(WorkloadClass::Query), 2);
    assert_eq!(s.admit(WorkloadClass::Query), Admission::Granted);
    assert_eq!(s.admit(WorkloadClass::Query), Admission::Granted);
    assert_eq!(s.admit(WorkloadClass::Query), Admission::Queued);
    assert_eq!(s.metrics_of(WorkloadClass::Query).queued, 1);
    s.release(WorkloadClass::Query);
    assert_eq!(s.admit(WorkloadClass::Query), Admission::Granted);
}

#[test]
fn duplicate_start_rejected() {
    let s = Scheduler::new();
    assert!(s.try_start_task("collect").is_ok());
    let e = s.try_start_task("collect").unwrap_err();
    assert_eq!(e.code, ErrorCode::RepeatedStart);
    assert!(s.is_running("collect"));
    s.stop_task("collect");
    assert!(!s.is_running("collect"));
    assert!(s.try_start_task("collect").is_ok());
}

#[test]
fn observable_counters_exposed() {
    let s = Scheduler::new();
    s.admit(WorkloadClass::Ingest);
    s.record_wait(WorkloadClass::Ingest, 25);
    let m = s.metrics_of(WorkloadClass::Ingest);
    assert_eq!(m.active, 1);
    assert_eq!(m.admission_wait_ms_total, 25);
    assert_eq!(s.total_admission_wait_ms(), 25);
}

#[test]
fn zero_weight_rejected() {
    let s = Scheduler::new();
    assert!(s.set_weight(WorkloadClass::Model, 0).is_err());
}

#[test]
fn dual_runtime_creates_two_channels() {
    let rt = DualRuntime::new(2, 2).unwrap();
    let _ = &rt.ingest;
    let _ = &rt.interact;
}

#[test]
fn backoff_is_exponential() {
    let b = backoff_sequence(100, 4);
    assert_eq!(b.len(), 4);
    assert_eq!(b[0].as_millis(), 100);
    assert_eq!(b[1].as_millis(), 200);
    assert_eq!(b[3].as_millis(), 800);
}
