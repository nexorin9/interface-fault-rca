//! 主工作台结构：左候选列表 + 右详情区 + 底状态条；数据在测试数据层由台账/查询结果填充。
//! 一屏说清「哪个接口先坏的、劣化多少、证据在哪、模型是否降级」。

use crate::ui::components::{RatioBar, StatusBadge};
use crate::ui::theme::FONT_MONO;

#[derive(Debug, Clone)]
pub struct CandidateListItem {
    pub interface_code: String,
    pub service_name: String,
    pub ratio: f64,
    pub badge: StatusBadge,
}

#[derive(Debug, Clone)]
pub struct StatusBar {
    pub last_collect: String,
    pub event_count: u64,
    pub model_degraded: bool,
    pub degrade_banner: String,
}

impl StatusBar {
    pub fn new(last_collect: &str, event_count: u64, degraded: bool) -> Self {
        Self {
            last_collect: last_collect.to_string(), event_count,
            model_degraded: degraded,
            degrade_banner: if degraded { "模型不可达，以下为规则基线结论".to_string() } else { String::new() },
        }
    }
    pub fn show_degrade_banner(&self) -> bool { self.model_degraded && !self.degrade_banner.is_empty() }
}

/// 工作台布局数据结构（三区）
#[derive(Debug, Clone)]
pub struct WorkbenchLayout {
    pub left_candidates: Vec<CandidateListItem>,
    pub right_detail: Option<String>, // 详情区展示的候选 interface_code
    pub status_bar: StatusBar,
    pub mono_font: &'static str,
}

impl WorkbenchLayout {
    pub fn new(candidates: Vec<CandidateListItem>, degraded: bool) -> Self {
        let detail = candidates.first().map(|c| c.interface_code.clone());
        Self { left_candidates: candidates, right_detail: detail,
            status_bar: StatusBar::new("-", 0, degraded), mono_font: FONT_MONO }
    }

    /// 候选按劣化倍数降序（用等宽字体对齐竖排比对）
    pub fn sorted_by_ratio(mut self) -> Self {
        self.left_candidates.sort_by(|a, b| b.ratio.partial_cmp(&a.ratio)
            .unwrap_or(std::cmp::Ordering::Equal).then(a.interface_code.cmp(&b.interface_code)));
        self
    }

    pub fn ratio_bars(&self, threshold: f64) -> Vec<RatioBar> {
        self.left_candidates.iter().map(|c| RatioBar::new(c.ratio, threshold)).collect()
    }

    pub fn three_zones_present(&self) -> bool {
        // 左列表（可能为空的 Vec 也是存在的区）、右详情（Option）、底状态条
        !self.mono_font.is_empty() && (self.right_detail.is_some() || self.left_candidates.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::components::StatusBadge;

    fn item(code: &str, ratio: f64) -> CandidateListItem {
        CandidateListItem { interface_code: code.into(), service_name: "S".into(), ratio, badge: StatusBadge::pending() }
    }

    #[test]
    fn three_zones_exist() {
        let w = WorkbenchLayout::new(vec![item("A", 3.0)], false);
        assert!(w.three_zones_present());
        assert!(!w.status_bar.last_collect.is_empty() || w.status_bar.event_count == 0);
    }

    #[test]
    fn candidates_sorted_desc_by_ratio() {
        let w = WorkbenchLayout::new(vec![item("A", 2.0), item("B", 5.0), item("C", 3.0)], false).sorted_by_ratio();
        let codes: Vec<_> = w.left_candidates.iter().map(|c| c.interface_code.as_str()).collect();
        assert_eq!(codes, vec!["B", "C", "A"]);
    }

    #[test]
    fn degrade_banner_shows_when_degraded() {
        let w = WorkbenchLayout::new(vec![], true);
        assert!(w.status_bar.show_degrade_banner());
        assert!(w.status_bar.degrade_banner.contains("规则基线结论"));
    }

    #[test]
    fn mono_font_for_code_alignment() {
        assert_eq!(WorkbenchLayout::new(vec![], false).mono_font, FONT_MONO);
    }

    #[test]
    fn empty_ledger_shows_empty_zone_not_crash() {
        let w = WorkbenchLayout::new(vec![], false);
        assert!(w.left_candidates.is_empty());
        assert!(w.right_detail.is_none());
        assert!(w.three_zones_present());
    }
}
