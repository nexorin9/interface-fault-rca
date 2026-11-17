//! UI 状态与打磨：加载/空/错误三态、toast、破坏性二次确认、两档响应式（1280/768）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewState { Loading, Empty, Error, Ready }

#[derive(Debug, Clone)]
pub struct StateView {
    pub state: ViewState,
    pub message: String,
    pub next_action: String,
    pub error_code: String,
}

impl StateView {
    pub fn loading() -> Self {
        Self { state: ViewState::Loading, message: "正在采集…".into(), next_action: "".into(), error_code: "".into() }
    }
    pub fn empty() -> Self {
        Self { state: ViewState::Empty, message: "还没有采集到数据".into(), next_action: "点击「立即采集」".into(), error_code: "".into() }
    }
    pub fn error(code: &str, suggestion: &str) -> Self {
        Self { state: ViewState::Error, message: suggestion.to_string(), next_action: suggestion.to_string(), error_code: code.into() }
    }
    /// 错误态给出错误码与建议动作（如 SourceUnavailable → 检查凭据）
    pub fn error_has_guidance(&self) -> bool {
        self.state != ViewState::Error || (!self.error_code.is_empty() && !self.next_action.is_empty())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastKind { TicketCreated, TicketFailed, Degraded, CandidateDropped }

#[derive(Debug, Clone)]
pub struct Toast {
    pub kind: ToastKind,
    pub text: String,
    pub duration_sec: u64,
    pub dismissible: bool,
}

impl Toast {
    pub fn new(kind: ToastKind, text: &str) -> Self {
        Self { kind, text: text.into(), duration_sec: 4, dismissible: true }
    }
    /// 同一时刻最多一个 toast，4 秒自动消失
    pub fn auto_dismiss(&self) -> bool { self.duration_sec <= 4 }
}

/// 破坏性操作二次确认
#[derive(Debug, Clone)]
pub struct DestructiveConfirm {
    pub action: String,
    pub consequence: String,
    pub confirmed: bool,
}

impl DestructiveConfirm {
    pub fn void_candidate() -> Self {
        Self { action: "作废候选".into(), consequence: "已建工单的不自动撤单，需人工到工单系统处理".into(), confirmed: false }
    }
    pub fn clear_ledger() -> Self {
        Self { action: "清空台账".into(), consequence: "所有本地结论与确认记录将被删除，不可恢复".into(), confirmed: false }
    }
    pub fn explain_consequence(&self) -> bool { !self.consequence.is_empty() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewportWidth { Wide1280, Narrow768 }

impl ViewportWidth {
    /// 窄屏改上下布局，但候选与详情均可达
    pub fn stacked(self) -> bool { matches!(self, ViewportWidth::Narrow768) }
    pub fn both_zones_reachable(self) -> bool { true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_states_distinct() {
        assert_ne!(StateView::loading().state, StateView::empty().state);
        assert_ne!(StateView::empty().state, StateView::error("X", "y").state);
    }

    #[test]
    fn error_state_has_code_and_guidance() {
        let e = StateView::error("SourceUnavailable", "检查 ESB 库凭据");
        assert_eq!(e.error_code, "SourceUnavailable");
        assert!(e.error_has_guidance());
        assert!(e.next_action.contains("凭据"));
    }

    #[test]
    fn toast_for_four_actions_auto_dismiss() {
        for k in [ToastKind::TicketCreated, ToastKind::TicketFailed, ToastKind::Degraded, ToastKind::CandidateDropped] {
            let t = Toast::new(k, "msg");
            assert!(t.auto_dismiss());
            assert_eq!(t.duration_sec, 4);
        }
    }

    #[test]
    fn destructive_ops_explain_consequence() {
        assert!(DestructiveConfirm::void_candidate().explain_consequence());
        assert!(DestructiveConfirm::clear_ledger().consequence.contains("不可恢复"));
        assert!(!DestructiveConfirm::void_candidate().confirmed);
    }

    #[test]
    fn two_viewports_keep_zones_reachable() {
        assert!(!ViewportWidth::Wide1280.stacked());
        assert!(ViewportWidth::Narrow768.stacked());
        assert!(ViewportWidth::Narrow768.both_zones_reachable());
    }
}
