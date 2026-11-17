//! task 30 验收：跨源时间对齐 —— 三源毫秒归一、偏移估计校正、样本不足不校正、跨零点、时区归一。
mod common;

use common::{flow, log, metric};
use interface_fault_rca::ingest::log_tail::normalize_ts;
use interface_fault_rca::ingest::esb_mysql::datetime3_to_ms;
use interface_fault_rca::query::window::{apply_shift, assemble_from_rows, estimate_clock_shift};

#[test]
fn three_sources_normalize_to_ms() {
    // 流水 DATETIME(3)
    let flow_ms = datetime3_to_ms("2026-11-08 20:00:00.123").unwrap();
    // Prom unix 秒（与上面两个 2026-11-08T20:00:00Z 同一时刻）
    let prom_ms = normalize_ts("1794168000").unwrap();
    // 日志 ISO8601
    let log_ms = normalize_ts("2026-11-08T20:00:00Z").unwrap();
    assert!(flow_ms > 0 && prom_ms > 0 && log_ms > 0);
    assert_eq!(flow_ms % 1000, 123); // DATETIME(3) 保留毫秒
    assert!(prom_ms.abs_diff(log_ms) < 1000); // 同一时刻的秒级表示
}

#[test]
fn offset_estimate_and_correction_applied() {
    let flows: Vec<_> = (0..6).map(|i| flow("A", "SVC", 1000 + i * 1000, 100.0, 200)).collect();
    // 日志比流水晚 500ms
    let logs: Vec<_> = (0..6).map(|i| log("A", "SVC", 1500 + i * 1000, "ERROR", "e")).collect();
    let shift = estimate_clock_shift(&flows, &logs, 5).unwrap();
    assert_eq!(shift, 500);
    let mut corrected = logs.clone();
    apply_shift(&mut corrected, shift);
    assert_eq!(corrected[0].ts_ms, 1000);
    assert_eq!(corrected[0].fields.get("clock_shift_ms").unwrap(), "500");
}

#[test]
fn insufficient_samples_do_not_correct() {
    let flows = vec![flow("A", "SVC", 1000, 100.0, 200)];
    let logs = vec![log("A", "SVC", 1500, "ERROR", "e")];
    assert!(estimate_clock_shift(&flows, &logs, 5).is_none());
}

#[test]
fn cross_midnight_window_reads_across_partitions() {
    // 23:50 到次日 00:10，毫秒时间戳跨日期分区
    let before = datetime3_to_ms("2026-11-08 23:50:00.000").unwrap();
    let after = datetime3_to_ms("2026-11-09 00:10:00.000").unwrap();
    let rows = vec![
        flow("A", "SVC", before, 100.0, 200),
        flow("A", "SVC", after, 100.0, 200),
    ];
    let w = assemble_from_rows(rows, before, after).unwrap();
    assert_eq!(w.rows.len(), 2);
    assert!(before < after);
}

#[test]
fn timezone_offset_timestamp_normalizes() {
    let ms = normalize_ts("2026-11-08T20:00:00+08:00").unwrap();
    let utc = normalize_ts("2026-11-08T12:00:00Z").unwrap();
    assert_eq!(ms, utc);
}
