//! task 33 验收：查询可观测指标 —— 正常/降级/超限三条路径都产出指标，数值一致，可按接口窗口回查（JSON 可解析）。
use interface_fault_rca::query::metrics::QueryMetrics;

#[test]
fn normal_path_produces_metrics() {
    let mut m = QueryMetrics::new("ESB-A", 1000, 2000);
    m.scanned_rows = 500;
    m.projected_columns = 4;
    m.matched_partitions = vec!["esb_audit/2026-11-08".into()];
    m.elapsed_ms = 42;
    assert_eq!(m.exit_path, "normal");
    assert!(!m.degraded);
    let j: serde_json::Value = serde_json::from_str(&m.to_json()).unwrap();
    assert_eq!(j["scanned_rows"], 500);
    assert_eq!(j["projected_columns"], 4);
    assert_eq!(j["window_start_ms"], 1000);
}

#[test]
fn degraded_path_marks_reason() {
    let mut m = QueryMetrics::new("ESB-A", 0, 100);
    m.mark_degraded("窗口过大降级 Top-N");
    assert!(m.degraded);
    assert_eq!(m.exit_path, "degraded");
    assert!(m.degrade_reason.contains("窗口过大"));
}

#[test]
fn limit_exceeded_path_marks_reason() {
    let mut m = QueryMetrics::new("ESB-A", 0, 100);
    m.mark_limit_exceeded("超扫描行数上限");
    assert!(m.degraded);
    assert_eq!(m.exit_path, "limit_exceeded");
}

#[test]
fn metrics_carry_interface_and_window_for_lookup() {
    let m = QueryMetrics::new("ESB-B", 1111, 2222);
    assert_eq!(m.interface_code, "ESB-B");
    assert_eq!(m.window_start_ms, 1111);
    assert_eq!(m.window_end_ms, 2222);
}
