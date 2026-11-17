//! task 47 验收：UI 状态打磨 —— 三态独立、toast 四动作、破坏性二次确认、两档响应式可达。
use interface_fault_rca::ui::states::{
    DestructiveConfirm, StateView, Toast, ToastKind, ViewState, ViewportWidth,
};

#[test]
fn three_states_are_distinct() {
    assert_eq!(StateView::loading().state, ViewState::Loading);
    assert_eq!(StateView::empty().state, ViewState::Empty);
    assert_eq!(StateView::error("SourceUnavailable", "检查凭据").state, ViewState::Error);
    assert!(!StateView::loading().message.is_empty());
    assert!(!StateView::empty().next_action.is_empty());
    assert!(StateView::error("SourceUnavailable", "检查 ESB 库凭据").error_has_guidance());
}

#[test]
fn toast_covers_four_actions() {
    for (kind, text) in [
        (ToastKind::TicketCreated, "建单成功 T-1"),
        (ToastKind::TicketFailed, "建单失败"),
        (ToastKind::Degraded, "已降级为规则基线"),
        (ToastKind::CandidateDropped, "候选被丢弃"),
    ] {
        let t = Toast::new(kind, text);
        assert!(!t.text.is_empty());
        assert!(t.auto_dismiss());
    }
}

#[test]
fn destructive_actions_require_second_confirm_with_consequence() {
    let v = DestructiveConfirm::void_candidate();
    assert!(v.explain_consequence());
    assert!(v.consequence.contains("不自动撤单"));
    assert!(!v.confirmed);
    let c = DestructiveConfirm::clear_ledger();
    assert!(c.explain_consequence());
}

#[test]
fn two_viewport_widths_keep_zones_reachable() {
    assert!(!ViewportWidth::Wide1280.stacked());
    assert!(ViewportWidth::Narrow768.stacked());
    assert!(ViewportWidth::Wide1280.both_zones_reachable());
    assert!(ViewportWidth::Narrow768.both_zones_reachable());
}
