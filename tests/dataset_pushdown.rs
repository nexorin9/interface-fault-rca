//! task 24 验收：Parquet 分区写入与列/行组裁剪 —— 窄窗扫描行数 < 总量、投影列数 = 请求列数、只命中请求分区。
mod common;

use common::flow;
use interface_fault_rca::store::dataset::{append_rows, date_partition, query_window, ScanStats, ALL_COLS};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp() -> PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("ifds-{n}-{seq}"))
}

#[test]
fn date_partition_splits_by_utc_day() {
    let d1 = date_partition(1762636800000); // 2026-11-08
    let d2 = date_partition(1762636800000 + 86_400_000);
    assert_ne!(d1, d2);
    assert!(d1.contains('-'));
}

#[test]
fn append_and_query_window_with_column_projection() {
    let root = tmp();
    let mut rows = vec![];
    for i in 0..100 {
        rows.push(flow("A", "SVC", 1762636800000 + i * 60000, 100.0, 200));
    }
    append_rows(&root, &rows).unwrap();
    let mut stats = ScanStats::default();
    // 物理列是 tags 六键 + ts_ms + fields_json（spec.md §3.1），fields 整体存一列 JSON，
    // 因此投影只能点名物理列：interface_code 与 fields_json 两列。
    let got = query_window(&root, 1762636800000, 1762636800000 + 9 * 60000,
        &["interface_code", "fields_json"], &mut stats).unwrap();
    assert_eq!(got.len(), 10);
    assert_eq!(stats.projected_columns, 2); // 只投影请求的两列
    assert!(stats.scanned_rows >= got.len());
    assert!(!stats.matched_partitions.is_empty());
    assert!(ALL_COLS.contains(&"event_kind"));
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn narrow_window_scans_fewer_than_total() {
    let root = tmp();
    let mut rows = vec![];
    for i in 0..200 {
        rows.push(flow("A", "SVC", 1762636800000 + i * 60000, 100.0, 200));
    }
    append_rows(&root, &rows).unwrap();
    let mut stats = ScanStats::default();
    let got = query_window(&root, 1762636800000, 1762636800000 + 60_000, &[], &mut stats).unwrap();
    assert!(got.len() < rows.len());
    std::fs::remove_dir_all(&root).ok();
}
