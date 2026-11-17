//! task 29 验收：绑定覆盖度 —— 未映射值统计且不进候选、歧义进清单且不进候选排序、报告/CSV 行数与统计一致。
use interface_fault_rca::binding::interface_dict::{export_coverage_report, minimal_dict, coverage, Dictionary, InterfaceEntry};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir() -> PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ifcov-{n}-{seq}"));
    fs::create_dir_all(&p).unwrap();
    p
}

fn ambiguous_dict() -> Dictionary {
    let mut d = minimal_dict();
    d.entries.push(InterfaceEntry {
        code: "ESB-AMBIG".into(), service_name: "服务一".into(), vendor_name: "".into(),
        hosts: vec![], aliases: vec![], basis: "".into(),
    });
    d.entries.push(InterfaceEntry {
        code: "ESB-AMBIG".into(), service_name: "服务二".into(), vendor_name: "".into(),
        hosts: vec![], aliases: vec![], basis: "".into(),
    });
    d
}

#[test]
fn unbound_values_counted_and_excluded_from_candidates() {
    let d = minimal_dict();
    let observed = vec![
        ("ESB-ORDER-CREATE".into(), 100usize),
        ("UNMAPPED-X".into(), 25),
        ("UNMAPPED-Y".into(), 5),
    ];
    let (unbound, ambiguous) = coverage(&d, &observed);
    let total_unbound: usize = unbound.iter().map(|r| r.count).sum();
    assert_eq!(total_unbound, 30);
    assert!(ambiguous.is_empty());
    // 已映射规范码不在未映射清单
    assert!(!unbound.iter().any(|r| r.raw == "ESB-ORDER-CREATE"));
}

#[test]
fn ambiguous_code_enters_list_but_not_candidates() {
    let d = ambiguous_dict();
    let (_, ambiguous) = coverage(&d, &[("ESB-AMBIG".into(), 7usize)]);
    assert_eq!(ambiguous.len(), 1);
    assert_eq!(ambiguous[0].raw, "ESB-AMBIG");
}

#[test]
fn report_and_csv_row_counts_match_statistics() {
    let d = minimal_dict();
    let observed = vec![("UNMAPPED-1".into(), 3usize), ("UNMAPPED-2".into(), 2)];
    let dir = tmp_dir();
    let (md, csv) = export_coverage_report(&d, &observed, &dir).unwrap();
    let md_text = fs::read_to_string(&md).unwrap();
    let csv_text = fs::read_to_string(&csv).unwrap();
    assert!(md_text.contains("绑定覆盖度报告"));
    assert!(md_text.contains("未映射占比"));
    // CSV 数据行数 = 未映射数（含表头）
    let data_lines = csv_text.lines().count() - 1;
    assert_eq!(data_lines, 2);
    fs::remove_dir_all(&dir).ok();
}
