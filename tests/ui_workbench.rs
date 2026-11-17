//! task 44 验收：主工作台 —— 三区结构存在、候选按劣化倍数降序、degraded 时出现降级横幅。
use interface_fault_rca::ui::components::StatusBadge;
use interface_fault_rca::ui::workbench::{CandidateListItem, WorkbenchLayout};

fn item(code: &str, ratio: f64) -> CandidateListItem {
    CandidateListItem { interface_code: code.into(), service_name: "SVC".into(), ratio, badge: StatusBadge::pending() }
}

#[test]
fn three_zones_present() {
    let w = WorkbenchLayout::new(vec![item("A", 3.0)], false);
    assert!(w.three_zones_present());
    assert!(!w.left_candidates.is_empty());
    assert!(w.right_detail.is_some());
}

#[test]
fn candidates_sorted_descending_by_ratio() {
    let w = WorkbenchLayout::new(vec![item("A", 2.0), item("B", 5.0), item("C", 3.5)], false).sorted_by_ratio();
    let codes: Vec<_> = w.left_candidates.iter().map(|c| c.interface_code.as_str()).collect();
    assert_eq!(codes, vec!["B", "C", "A"]);
}

#[test]
fn degrade_banner_shows_when_degraded() {
    let w = WorkbenchLayout::new(vec![item("A", 3.0)], true);
    assert!(w.status_bar.show_degrade_banner());
    assert!(w.status_bar.degrade_banner.contains("规则基线结论"));
}

#[test]
fn no_banner_when_model_available() {
    let w = WorkbenchLayout::new(vec![item("A", 3.0)], false);
    assert!(!w.status_bar.show_degrade_banner());
}

#[test]
fn empty_ledger_still_shows_layout() {
    let w = WorkbenchLayout::new(vec![], false);
    assert!(w.left_candidates.is_empty());
    assert!(w.right_detail.is_none());
    assert!(w.three_zones_present());
}
