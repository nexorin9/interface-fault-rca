//! task 25 验收：manifest 自描述校验 —— 一致通过 / 列集不符报错指出差异 / 损坏拒绝写入 / 重启续写不重不漏。
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::store::manifest::{actual_columns, Manifest};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp() -> PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ifmf-{n}-{seq}"));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn consistent_schema_passes() {
    let m = Manifest::new();
    assert!(m.validate_against(&actual_columns()).is_ok());
    assert!(m.checksum_ok());
}

#[test]
fn column_mismatch_reports_diff_column() {
    let m = Manifest::new();
    let mut actual = actual_columns();
    actual.remove("__tsid");
    actual.insert("surprise_col".into(), "Utf8".into());
    let e = m.validate_against(&actual).unwrap_err();
    assert_eq!(e.code, ErrorCode::SchemaMismatch);
    assert!(e.detail.contains("__tsid"));
    assert!(e.detail.contains("surprise_col"));
}

#[test]
fn save_load_roundtrip_with_cursor() {
    let dir = tmp();
    let mut m = Manifest::new();
    m.add_source("esb_audit");
    m.add_partition("esb_audit/2026-11-08");
    m.write_cursor.insert("esb_audit".into(), 12345);
    m.save(&dir).unwrap();
    let loaded = Manifest::load(&dir).unwrap();
    assert_eq!(loaded.write_cursor.get("esb_audit"), Some(&12345));
    assert!(loaded.partitions.contains(&"esb_audit/2026-11-08".to_string()));
    assert!(loaded.checksum_ok());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn corrupted_manifest_refuses_load() {
    let dir = tmp();
    std::fs::write(Manifest::path(&dir), "{ broken json").unwrap();
    let e = Manifest::load(&dir).unwrap_err();
    assert_eq!(e.code, ErrorCode::ConfigMissing);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn restart_continues_from_cursor_without_duplicate() {
    let dir = tmp();
    let mut m = Manifest::new();
    m.write_cursor.insert("esb".into(), 100);
    m.save(&dir).unwrap();
    // 模拟重启：从 manifest 恢复游标继续写
    let mut loaded = Manifest::load(&dir).unwrap();
    let cursor = *loaded.write_cursor.get("esb").unwrap();
    assert_eq!(cursor, 100);
    loaded.write_cursor.insert("esb".into(), cursor + 50);
    loaded.save(&dir).unwrap();
    let final_m = Manifest::load(&dir).unwrap();
    assert_eq!(*final_m.write_cursor.get("esb").unwrap(), 150); // 只增不减
    std::fs::remove_dir_all(&dir).ok();
}
