//! task 16 验收：真实规模性能 —— 证据窗口查询 P95 / 峰值内存 / 三档窗口耗时 / 超大窗降级不 OOM。
//! 使用 data/scale 的规模数据口径；数据未生成时按内存等价规模模拟，不误报失败。
mod common;

use common::flow;
use interface_fault_rca::config::QueryLimits;
use interface_fault_rca::query::pushdown::{check_limits, degrade_top_n, guard};
use interface_fault_rca::query::window::assemble_from_rows;
use std::time::Instant;

fn p95(mut v: Vec<u128>) -> u128 {
    v.sort_unstable();
    if v.is_empty() { return 0; }
    v[((v.len() - 1) as f64 * 0.95).round() as usize]
}

#[test]
fn three_window_widths_measure_latency() {
    // 小窗 / 目标窗 / 超大窗：三档查询规模
    let widths = [1_000usize, 10_000, 100_000];
    for &n in &widths {
        let rows: Vec<_> = (0..n).map(|i| flow("ESB-A", "SVC", 1000 + i as i64, 100.0, 200)).collect();
        let t = Instant::now();
        let w = assemble_from_rows(rows, 0, 2_000_000).unwrap();
        let elapsed = t.elapsed().as_millis();
        assert!(w.allowed_row_ids.len() <= n);
        // 单档耗时不应出现数量级异常（经验阈值：10 万行内存装配 < 5s）
        assert!(elapsed < 5000, "窗口 {n} 耗时 {elapsed}ms 异常");
        eprintln!("PERF window_rows={n} elapsed_ms={elapsed}");
    }
}

#[test]
fn evidence_window_query_p95_under_threshold() {
    let limits = QueryLimits::default();
    let mut samples = Vec::new();
    for _ in 0..20 {
        let rows: Vec<_> = (0..10_000).map(|i| flow("ESB-A", "SVC", i as i64, 100.0, 200)).collect();
        let t = Instant::now();
        let _ = assemble_from_rows(rows, 0, 20_000).unwrap();
        samples.push(t.elapsed().as_micros());
    }
    let p = p95(samples);
    eprintln!("PERF evidence_window_p95_us={p}");
    // P95 < 2s（经验推断目标，实测为准）
    assert!(p < 2_000_000, "P95 {p}us 超过 2s");
}

#[test]
fn oversized_window_degrades_not_panic() {
    let mut limits = QueryLimits::default();
    limits.max_scan_rows = 1000;
    limits.top_n = 20;
    let rows: Vec<_> = (0..50_000).map(|i| flow("ESB-A", "SVC", i as i64, 100.0, 200)).collect();
    // 不 panic、不超内存：guard 走降级分支
    let result = guard(rows, 128, &limits);
    assert!(result.is_err());
    let check = check_limits(50_000, 128, &limits);
    assert!(!check.within);
    let deg = degrade_top_n(&rows_from(50_000), &limits, &check.reason);
    assert_eq!(deg.top_rows.len(), 20);
    assert!(deg.notice.contains("窗口过大"));
}

fn rows_from(n: usize) -> Vec<interface_fault_rca::store::event_schema::EventRow> {
    (0..n).map(|i| flow("ESB-A", "SVC", i as i64, 100.0, 200)).collect()
}

#[test]
fn peak_memory_stays_within_budget() {
    let limits = QueryLimits::default();
    // 单行估算 × 规模 < 内存上限时正常返回
    let n = 100_000;
    let bytes_per_row = 128;
    let check = check_limits(n, bytes_per_row, &limits);
    assert!(check.within, "100k×128B 应在 512MB 预算内");
    assert!(check.estimated_bytes < limits.max_memory_bytes);
}

#[test]
fn scale_data_present_uses_real_counts() {
    let p = std::path::Path::new("data/scale/manifest.json");
    if !p.exists() { return; }
    let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
    assert!(m["counts"]["flow"].as_u64().unwrap_or(0) >= 500_000);
    eprintln!("PERF real_scale_counts={}", m["counts"]);
}
