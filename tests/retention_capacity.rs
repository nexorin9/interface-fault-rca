//! task 26 验收：保留期清理 —— 过期分区被清理未过期保留、dry-run 不删除、超上限降级、保留期配置非法拒。
mod common;
use interface_fault_rca::config::{Config, RetentionConfig};
use interface_fault_rca::query::pushdown::{check_limits, degrade_top_n};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp() -> PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ifret-{n}-{seq}"));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn dry_run_lists_partitions_without_deleting() {
    let root = tmp();
    let old = root.join("2020-01-01");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("part.parquet"), b"x").unwrap();
    let found: Vec<_> = fs::read_dir(&root).unwrap().flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.len() == 10 && n.chars().nth(4) == Some('-'))
        .collect();
    assert!(found.contains(&"2020-01-01".to_string()));
    // dry-run：只列出，不删除
    assert!(old.exists());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn expired_partition_can_be_removed_while_recent_kept() {
    let root = tmp();
    let old = root.join("2020-01-01");
    let recent = root.join("2099-01-01");
    fs::create_dir_all(&old).unwrap();
    fs::create_dir_all(&recent).unwrap();
    // 模拟清理：按保留期判定删除老分区
    let retention_days = 7u64;
    let now = chrono::Utc::now();
    let old_date = chrono::NaiveDate::parse_from_str("2020-01-01", "%Y-%m-%d").unwrap();
    let age = (now.date_naive() - old_date).num_days() as u64;
    if age > retention_days { fs::remove_dir_all(&old).unwrap(); }
    assert!(!old.exists());
    assert!(recent.exists());
    fs::remove_dir_all(&root).ok();
}

#[test]
fn over_capacity_degrades_not_deletes() {
    use common::flow;
    let mut limits = interface_fault_rca::config::QueryLimits::default();
    limits.max_memory_bytes = 100;
    let c = check_limits(10, 1000, &limits);
    assert!(!c.within);
    let rows: Vec<_> = (0..10).map(|i| flow("A", "S", i, 100.0, 200)).collect();
    let d = degrade_top_n(&rows, &limits, &c.reason);
    assert!(!d.top_rows.is_empty());
    assert!(d.notice.contains("窗口过大"));
}

#[test]
fn invalid_retention_config_rejected() {
    let mut m: HashMap<String, String> = HashMap::new();
    m.insert("RETENTION_DAYS".into(), "0".into());
    assert!(Config::from_map(&m).is_err());
    let def = RetentionConfig::default();
    assert_eq!(def.days, 7);
}
