//! 证据守门：模型返回的 evidence_refs 逐条复核；三条路径各自整条丢弃候选（不是降权）。
//! ①不存在的行 ID ②窗口外的行 ID ③未映射实体(unbound)的行。

use crate::ai::conclusion::{Conclusion, Verdict};
use crate::ai::ports::{EvidenceGate, GateDecision};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct GateContext {
    /// 本窗口合法 ID 白名单
    pub window_ids: HashSet<String>,
    /// 全局存在的 ID 集合（用于区分"窗口外"与"不存在"）
    pub all_ids: HashSet<String>,
    /// 未映射实体的行 ID
    pub unbound_ids: HashSet<String>,
    /// 证据充分性下限
    pub min_refs: usize,
}

impl Default for GateContext {
    fn default() -> Self {
        Self { window_ids: HashSet::new(), all_ids: HashSet::new(), unbound_ids: HashSet::new(), min_refs: 1 }
    }
}

#[derive(Debug, Clone)]
pub struct ConcreteEvidenceGate {
    pub ctx: GateContext,
}

impl EvidenceGate for ConcreteEvidenceGate {
    fn check(&self, c: &Conclusion) -> GateDecision {
        check_conclusion(c, &self.ctx)
    }
}

/// 逐条复核；任一条违规即整条丢弃，返回可区分的原因码。
pub fn check_conclusion(c: &Conclusion, ctx: &GateContext) -> GateDecision {
    if c.evidence_refs.is_empty() {
        return GateDecision::Drop { reason_code: "EVIDENCE_MISSING".into() };
    }
    for r in &c.evidence_refs {
        if ctx.unbound_ids.contains(r) {
            return GateDecision::Drop { reason_code: "UNBOUND_REF".into() };
        }
        if !ctx.window_ids.contains(r) {
            let code = if ctx.all_ids.contains(r) { "OUTSIDE_WINDOW" } else { "NONEXISTENT_REF" };
            return GateDecision::Drop { reason_code: code.into() };
        }
    }
    // 证据充分性下限
    let valid = c.evidence_refs.iter()
        .filter(|r| ctx.window_ids.contains(r.as_str()) && !ctx.unbound_ids.contains(r.as_str()))
        .count();
    if valid < ctx.min_refs {
        return GateDecision::Drop { reason_code: "INSUFFICIENT_EVIDENCE".into() };
    }
    GateDecision::Pass
}

/// 三条丢弃路径的统计累加器（"模型今天编了几次"可回查）
#[derive(Debug, Clone, Default)]
pub struct GateStats {
    pub nonexistent: u32,
    pub outside_window: u32,
    pub unbound: u32,
    pub insufficient: u32,
    pub passed: u32,
}

impl GateStats {
    pub fn record(&mut self, d: &GateDecision) {
        match d {
            GateDecision::Pass => self.passed += 1,
            GateDecision::Drop { reason_code } => match reason_code.as_str() {
                "NONEXISTENT_REF" => self.nonexistent += 1,
                "OUTSIDE_WINDOW" => self.outside_window += 1,
                "UNBOUND_REF" => self.unbound += 1,
                "INSUFFICIENT_EVIDENCE" | "EVIDENCE_MISSING" => self.insufficient += 1,
                _ => {}
            },
        }
    }
    pub fn total_dropped(&self) -> u32 { self.nonexistent + self.outside_window + self.unbound + self.insufficient }
}

/// 守住状态推进边界：判定一条结论是否可进入 candidate_ranked。
/// 无证据引用的候选整条丢弃；"证据不足" verdict 也不进候选集。
pub fn admissible(c: &Conclusion, gate: &ConcreteEvidenceGate) -> bool {
    if matches!(c.verdict, Verdict::证据不足) { return false; }
    matches!(gate.check(c), GateDecision::Pass)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::conclusion::BaselineStats;

    fn c(refs: Vec<&str>) -> Conclusion {
        Conclusion {
            interface_code: "A".into(), service_name: "S".into(), verdict: Verdict::接口自身劣化,
            confidence: 0.9, evidence_refs: refs.into_iter().map(|s| s.to_string()).collect(),
            reasoning_summary: "".into(), next_check: "".into(), degraded: false,
            baseline_stats: BaselineStats { p95_ms: 0.0, p99_ms: 0.0, degrade_ratio: 0.0, error_rate: 0.0, baseline_samples: 0 },
            request_id: "r".into(),
        }
    }

    fn ctx() -> GateContext {
        GateContext {
            window_ids: ["flow#a1".into(), "log#a2".into()].into_iter().collect(),
            all_ids: ["flow#a1".into(), "log#a2".into(), "flow#a9".into()].into_iter().collect(),
            unbound_ids: ["log#a2".into()].into_iter().collect(),
            min_refs: 1,
        }
    }

    #[test]
    fn nonexistent_ref_drops_whole_candidate() {
        let conclusion = c(vec!["flow#a1", "flow#fake"]);
        assert_eq!(check_conclusion(&conclusion, &ctx()), GateDecision::Drop { reason_code: "NONEXISTENT_REF".into() });
    }

    #[test]
    fn outside_window_ref_drops() {
        let conclusion = c(vec!["flow#a9"]);
        assert_eq!(check_conclusion(&conclusion, &ctx()), GateDecision::Drop { reason_code: "OUTSIDE_WINDOW".into() });
    }

    #[test]
    fn unbound_ref_drops() {
        let conclusion = c(vec!["log#a2"]);
        assert_eq!(check_conclusion(&conclusion, &ctx()), GateDecision::Drop { reason_code: "UNBOUND_REF".into() });
    }

    #[test]
    fn valid_ref_passes() {
        let conclusion = c(vec!["flow#a1"]);
        assert_eq!(check_conclusion(&conclusion, &ctx()), GateDecision::Pass);
    }

    #[test]
    fn insufficient_evidence_drops() {
        let mut cx = ctx();
        cx.min_refs = 2;
        let conclusion = c(vec!["flow#a1"]);
        assert_eq!(check_conclusion(&conclusion, &cx), GateDecision::Drop { reason_code: "INSUFFICIENT_EVIDENCE".into() });
    }

    #[test]
    fn stats_distinguish_three_paths() {
        let mut s = GateStats::default();
        s.record(&GateDecision::Drop { reason_code: "NONEXISTENT_REF".into() });
        s.record(&GateDecision::Drop { reason_code: "OUTSIDE_WINDOW".into() });
        s.record(&GateDecision::Drop { reason_code: "UNBOUND_REF".into() });
        assert_eq!(s.total_dropped(), 3);
        assert_eq!(s.nonexistent, 1);
        assert_eq!(s.outside_window, 1);
        assert_eq!(s.unbound, 1);
    }

    #[test]
    fn tampered_output_not_admissible_and_state_capped() {
        let gate = ConcreteEvidenceGate { ctx: ctx() };
        let tampered = c(vec!["flow#fake"]);
        assert!(!admissible(&tampered, &gate));
        // 状态边界：admissible 只决定能否进 candidate_ranked，不产生 awaiting/ticket
        let good = c(vec!["flow#a1"]);
        assert!(admissible(&good, &gate));
    }
}
