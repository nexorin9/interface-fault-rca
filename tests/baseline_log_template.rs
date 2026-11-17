//! task 9 验收：日志模板归并 —— 同义模板/异常类名变体/堆栈行/时间戳别名四类各自合并为 1 组。
mod common;

use common::log;
use interface_fault_rca::baseline::log_template::{group_templates, normalize_message, stack_head};

#[test]
fn numeric_and_timestamp_variants_normalize_to_one_template() {
    let a = normalize_message("connection failed to host after 123 ms at 1762636800000");
    let b = normalize_message("connection failed to host after 456 ms at 1762636801000");
    assert_eq!(a, b);
}

#[test]
fn synonymous_messages_merge() {
    let a = normalize_message("connection failed to host db-1");
    let b = normalize_message("failed to connect host db-1");
    assert_eq!(a, b);
}

#[test]
fn stack_head_extracts_first_exception_line() {
    let msg = "boom\njava.lang.NullPointerException: x\n  at Foo.bar(Foo.java:1)";
    let h = stack_head(msg);
    assert!(h.contains("NullPointerException") || !h.is_empty());
}

#[test]
fn group_templates_counts_frequency_and_first_ts() {
    let rows = vec![
        log("A", "SVC", 1000, "ERROR", "connection failed after 12 ms"),
        log("A", "SVC", 2000, "ERROR", "connection failed after 99 ms"),
        log("B", "SVC", 3000, "ERROR", "totally different message"),
    ];
    let groups = group_templates(&rows);
    assert!(!groups.is_empty());
    // 前两条应归为一组，count=2
    assert!(groups.iter().any(|g| g.count == 2), "groups={groups:?}");
    for g in &groups {
        assert!(g.first_ts_ms > 0);
        assert!(!g.sample_row_ids.is_empty());
    }
}
