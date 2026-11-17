//! 归因卡：结论句 + 置信度 + 理由摘要 + 下一步建议 + 证据引用清单 + 「需人工确认」显著标注；
//! degraded 为真时明示「规则基线结论（模型不可达）」，不与模型结论混淆。

use crate::ai::conclusion::{Conclusion, Verdict};
use crate::ui::components::StatusBadge;

#[derive(Debug, Clone)]
pub struct AttributionCard {
    pub verdict_text: String,
    pub confidence: f64,
    pub reasoning: String,
    pub next_check: String,
    pub evidence_refs: Vec<String>,
    pub need_confirm: StatusBadge,
    pub degraded_notice: String,
}

impl AttributionCard {
    pub fn from_conclusion(c: &Conclusion) -> Self {
        Self {
            verdict_text: c.verdict.as_str().to_string(),
            confidence: c.confidence,
            reasoning: c.reasoning_summary.clone(),
            next_check: c.next_check.clone(),
            evidence_refs: c.evidence_refs.clone(),
            need_confirm: StatusBadge::pending(),
            degraded_notice: if c.degraded { "规则基线结论（模型不可达）".to_string() } else { String::new() },
        }
    }

    pub fn shows_degraded(&self) -> bool { !self.degraded_notice.is_empty() }
    pub fn marked_for_confirmation(&self) -> bool { self.need_confirm.label == "待确认" }

    /// 引用清单中每条都能定位到证据区（由 EvidencePanel 校验命中）
    pub fn refs_resolvable<F: Fn(&str) -> bool>(&self, locate: F) -> bool {
        self.evidence_refs.iter().all(|r| locate(r))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::conclusion::BaselineStats;

    fn conclusion(degraded: bool) -> Conclusion {
        Conclusion {
            interface_code: "A".into(), service_name: "S".into(), verdict: Verdict::接口自身劣化,
            confidence: 0.9, evidence_refs: vec!["flow#a1".into()], reasoning_summary: "r".into(),
            next_check: "核对原文".into(), degraded,
            baseline_stats: BaselineStats { p95_ms: 0.0, p99_ms: 0.0, degrade_ratio: 0.0, error_rate: 0.0, baseline_samples: 0 },
            request_id: "r".into(),
        }
    }

    #[test]
    fn card_shows_need_confirmation() {
        let card = AttributionCard::from_conclusion(&conclusion(false));
        assert!(card.marked_for_confirmation());
        assert_eq!(card.verdict_text, "接口自身劣化");
    }

    #[test]
    fn degraded_card_explicitly_marked() {
        let card = AttributionCard::from_conclusion(&conclusion(true));
        assert!(card.shows_degraded());
        assert!(card.degraded_notice.contains("模型不可达"));
    }

    #[test]
    fn normal_card_not_marked_degraded() {
        assert!(!AttributionCard::from_conclusion(&conclusion(false)).shows_degraded());
    }

    #[test]
    fn refs_can_locate_evidence() {
        let card = AttributionCard::from_conclusion(&conclusion(false));
        assert!(card.refs_resolvable(|r| r == "flow#a1"));
        assert!(!card.refs_resolvable(|_| false));
    }
}
