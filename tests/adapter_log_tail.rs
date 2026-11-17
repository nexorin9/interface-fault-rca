//! task 6 验收：日志适配器 —— UTF-8/GBK 混编、脏行、超长行、三种时间戳别名、游标续读。
//! 本地替身仅作测试数据层。
use interface_fault_rca::ingest::log_tail::{decode, normalize_ts, parse_line, tail_directory, FileCursor};
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(tag: &str) -> std::path::PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("iftail-{tag}-{n}-{seq}"));
    fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn decode_utf8_and_gbk() {
    assert_eq!(decode("hello 中文".as_bytes()), "hello 中文");
    // GBK 编码的 "错误"
    let gbk = encoding_rs::GBK.encode("错误").0;
    assert_eq!(decode(&gbk), "错误");
}

#[test]
fn normalize_three_timestamp_aliases() {
    let sec = normalize_ts("1762636800").unwrap();
    let ms = normalize_ts("1762636800123").unwrap();
    let iso = normalize_ts("2026-11-08T20:00:00Z").unwrap();
    assert_eq!(sec, 1762636800_000);
    assert_eq!(ms, 1762636800123);
    assert!(iso > 0);
    assert!(normalize_ts("").is_none());
}

#[test]
fn parse_line_json_and_text() {
    let j = r#"{"timestamp":"1762636800000","level":"ERROR","message":"boom","interface_code":"ESB-A"}"#;
    let ev = parse_line(j).unwrap();
    assert_eq!(ev.level, "ERROR");
    assert_eq!(ev.interface_code, "ESB-A");
    let t = "1762636800 ERROR interface=ESB-B connection failed";
    let ev2 = parse_line(t).unwrap();
    assert_eq!(ev2.interface_code, "ESB-B");
    assert!(parse_line("NOT_A_LOG_LINE").is_none());
}

#[test]
fn tail_cursor_does_not_reread() {
    let dir = tmp_dir("cursor");
    let f = dir.join("app.log");
    fs::write(&f, "1762636800 ERROR interface=ESB-A first failure\n").unwrap();
    let patterns = vec!["*.log".to_string()];
    let first = tail_directory(&dir, &patterns, &[], 4096).unwrap();
    assert_eq!(first.events.len(), 1);
    assert_eq!(first.cursors.len(), 1);
    let second = tail_directory(&dir, &patterns, &first.cursors, 4096).unwrap();
    assert_eq!(second.events.len(), 0); // 游标续读不重复
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn dirty_and_too_long_lines_are_counted() {
    let dir = tmp_dir("dirty");
    let f = dir.join("app.log");
    let long = "x".repeat(50);
    let content = format!("NOT_A_LOG\n1762636800 ERROR interface=A msg\n{long}\n");
    fs::write(&f, content).unwrap();
    let patterns = vec!["*.log".to_string()];
    let out = tail_directory(&dir, &patterns, &[], 20).unwrap();
    assert!(out.dirty_lines >= 1);
    assert!(out.too_long >= 1);
    let _cur: Vec<FileCursor> = out.cursors;
    fs::remove_dir_all(&dir).ok();
}
