//! task 45 验收：证据区与归因卡 —— 三段带 __row_id、需人工确认标记、degraded 明示、引用可定位。
mod common;

use common::conclusion;
use interface_fault_rca::ai::conclusion::Verdict;
use interface_fault_rca::ui::attribution_card::AttributionCard;
use interface_fault_rca::ui::evidence::{EvidenceLine, EvidencePanel};

#[test]
fn evidence_panel_has_three_sections_with_row_ids() {
    let lines = vec![
        EvidenceLine::new("flow#a1", "flow", 1000, "rc=200 dur=100ms"),
        EvidenceLine::new("log#a2", "log", 1100, "level=ERROR"),
        EvidenceLine::new("metric#a3", "metric", 1200, "cpu=0.8"),
    ];
    let panel = EvidencePanel::from_lines(lines);
    assert!(panel.three_sections());
    assert!(panel.every_line_has_row_id());
    assert!(panel.locate("flow#a1").is_some());
    assert!(panel.locate("missing").is_none());
}

#[test]
fn attribution_card_marked_for_confirmation() {
    let c = conclusion(vec!["flow#a1"], Verdict::接口自身劣化);
    let card = AttributionCard::from_conclusion(&c);
    assert!(card.marked_for_confirmation());
    assert_eq!(card.verdict_text, "接口自身劣化");
}

#[test]
fn degraded_card_explicitly_shows_degradation() {
    let mut c = conclusion(vec!["flow#a1"], Verdict::接口自身劣化);
    c.degraded = true;
    let card = AttributionCard::from_conclusion(&c);
    assert!(card.shows_degraded());
    assert!(card.degraded_notice.contains("模型不可达"));
    let normal = AttributionCard::from_conclusion(&conclusion(vec!["flow#a1"], Verdict::接口自身劣化));
    assert!(!normal.shows_degraded());
}

#[test]
fn references_resolve_to_evidence_lines() {
    let c = conclusion(vec!["flow#a1", "log#a2"], Verdict::下游依赖饱和);
    let card = AttributionCard::from_conclusion(&c);
    let panel = EvidencePanel::from_lines(vec![
        EvidenceLine::new("flow#a1", "flow", 1000, "x"),
        EvidenceLine::new("log#a2", "log", 1100, "y"),
    ]);
    assert!(card.refs_resolvable(|id| panel.locate(id).is_some()));
    assert!(!card.refs_resolvable(|id| id == "flow#a1")); // 第二条不可定位
}
