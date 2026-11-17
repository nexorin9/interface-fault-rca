//! 可复用基础组件：状态徽标、劣化倍数条、来源色条、空态占位块、加载骨架条。
//! 每个组件只消费 theme.rs 的 token（色值来自常量，不写死）；状态标记三件齐备（色+字+形）。

use crate::ui::theme::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBadge {
    pub label: String,
    pub color: Rgb,
    pub icon_glyph: &'static str, // 语义符号（非 emoji），与颜色/文字共同表意
}

impl StatusBadge {
    pub fn pending() -> Self {
        Self { label: "待确认".into(), color: pending_color(), icon_glyph: "◆" }
    }
    pub fn degraded() -> Self {
        Self { label: "规则基线".into(), color: LOG_ORANGE, icon_glyph: "▲" }
    }
    pub fn normal() -> Self {
        Self { label: "正常".into(), color: METRIC_GREEN, icon_glyph: "●" }
    }
    /// 状态不靠颜色单独表意：色 + 文字 + 符号三者齐备
    pub fn accessible(&self) -> bool {
        !self.label.is_empty() && !self.icon_glyph.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct RatioBar {
    pub ratio: f64,
    pub threshold: f64,
    pub color: Rgb,
}

impl RatioBar {
    pub fn new(ratio: f64, threshold: f64) -> Self {
        // 语义色表达劣化，不用红绿对立；取琥珀表待确认
        Self { ratio, threshold, color: if ratio > threshold { PENDING_AMBER } else { FLOW_BLUE } }
    }
    pub fn fill_fraction(&self) -> f64 {
        if self.threshold <= 0.0 { 0.0 } else { (self.ratio / (self.threshold * 4.0)).min(1.0) }
    }
    pub fn over_threshold(&self) -> bool { self.ratio > self.threshold }
}

#[derive(Debug, Clone)]
pub struct SourceColorBar {
    pub source_kind: String,
    pub color: Rgb,
}

impl SourceColorBar {
    pub fn new(source_kind: &str) -> Self {
        Self { source_kind: source_kind.to_string(), color: source_color(source_kind).unwrap_or(TEXT_MUTED) }
    }
}

#[derive(Debug, Clone)]
pub struct EmptyState {
    pub title: String,
    pub next_action: String,
}

impl EmptyState {
    pub fn no_data() -> Self {
        Self { title: "还没有采集到数据".into(), next_action: "点击「立即采集」或等待下一次定时采集".into() }
    }
    pub fn accessible(&self) -> bool { !self.title.is_empty() && !self.next_action.is_empty() }
}

#[derive(Debug, Clone)]
pub struct SkeletonBar {
    pub width: f32,
    pub height: f32,
    pub alpha: f32,
}

impl SkeletonBar {
    pub fn loading() -> Self {
        Self { width: 240.0, height: TEXT_BASE, alpha: state_alpha(WidgetState::Loading) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_badge_has_color_text_and_shape() {
        assert!(StatusBadge::pending().accessible());
        assert_eq!(StatusBadge::pending().color, PENDING_AMBER);
    }

    #[test]
    fn ratio_bar_uses_semantic_color_not_red_green() {
        let over = RatioBar::new(3.0, 1.5);
        assert!(over.over_threshold());
        assert_eq!(over.color, PENDING_AMBER);
        let under = RatioBar::new(1.0, 1.5);
        assert_eq!(under.color, FLOW_BLUE);
    }

    #[test]
    fn source_color_bar_consumes_theme_table() {
        assert_eq!(SourceColorBar::new("flow").color, FLOW_BLUE);
        assert_eq!(SourceColorBar::new("log").color, LOG_ORANGE);
        assert_eq!(SourceColorBar::new("metric").color, METRIC_GREEN);
    }

    #[test]
    fn empty_state_has_next_action() {
        assert!(EmptyState::no_data().accessible());
    }

    #[test]
    fn skeleton_uses_loading_alpha_token() {
        assert_eq!(SkeletonBar::loading().alpha, state_alpha(WidgetState::Loading));
    }
}
