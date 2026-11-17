//! 归因编排主线：证据窗口 → PromptBuilder → ModelGateway → EvidenceGate → Conclusion。
//! 模型路径与基线路径产出一律经 EvidenceGate 复核后才进候选集；
//! 编排最多推进到 candidate_ranked，awaiting_confirmation 只能人工触发，ticket_created 只能真实 POST 成功触发。

use crate::ai::conclusion::{Conclusion, Verdict};
use crate::ai::evidence_gate::{ConcreteEvidenceGate, GateContext, GateStats, admissible};
use crate::ai::ports::{EvidenceGate, GateDecision, ModelGateway, ModelRequest, PromptBuilder};
use crate::store::sqlite_index::{IndexDb, LedgerStatus};

pub struct AttributionConfig {
    pub use_model: bool,
    pub request_id: String,
}

#[derive(Debug, Clone)]
pub struct AttributionResult {
    pub candidates: Vec<Conclusion>,
    /// 被丢弃的候选与丢弃原因码（模型编造引用 / 证据越界 / 证据不足 / 判定为证据不足）。
    /// 丢弃是「整条丢弃」，不是降权，因此必须可回查为什么丢。
    pub dropped: Vec<DroppedCandidate>,
    pub gate_stats: GateStats,
    pub degraded: bool,
    pub degrade_reason: String,
    /// 编排产出的状态上限（永远是 candidate_ranked，除非调用方另行人工确认）
    pub capped_status: LedgerStatus,
}

/// 被丢弃的候选（留痕用：值班人要能回答「为什么这条没给我」）
#[derive(Debug, Clone)]
pub struct DroppedCandidate {
    pub interface_code: String,
    pub service_name: String,
    pub verdict: String,
    pub reason_code: String,
    pub reason: String,
    pub evidence_refs: Vec<String>,
}

/// 编排主入口：model_gateway 不可用时走 baseline_conclusions（同构），置 degraded。
pub fn orchestrate<G: ModelGateway + ?Sized, B: PromptBuilder>(
    cfg: &AttributionConfig,
    baseline_conclusions: &[Conclusion],
    gateway: &G,
    builder: &B,
    gate: &ConcreteEvidenceGate,
) -> AttributionResult {
    let mut stats = GateStats::default();
    let mut degraded = false;
    let mut degrade_reason = String::new();

    let model_conclusions: Vec<Conclusion> = if cfg.use_model && gateway.available() {
        let req: ModelRequest = builder.build(&gate.ctx.window_ids.iter().cloned().collect::<Vec<_>>());
        match gateway.call(&req) {
            Ok(resp) => {
                degraded = resp.degraded;
                resp.conclusions
            }
            Err(e) => {
                degraded = true;
                degrade_reason = format!("模型不可用: {}", e.code.as_str());
                vec![]
            }
        }
    } else {
        degraded = true;
        degrade_reason = "模型未配置或不可用".to_string();
        vec![]
    };

    // 优先模型路径结论；模型路径为空/降级时用基线结论（同构，同一出口）
    let source: Vec<Conclusion> = if model_conclusions.is_empty() {
        let mut b = baseline_conclusions.to_vec();
        for c in &mut b { c.degraded = true; c.request_id = cfg.request_id.clone(); }
        b
    } else { model_conclusions };

    let mut candidates = vec![];
    let mut dropped = vec![];
    for c in source {
        let decision = gate.check(&c);
        stats.record(&decision);
        let drop_code = match &decision {
            GateDecision::Drop { reason_code } => Some(reason_code.clone()),
            GateDecision::Pass => {
                // 守门放行，但判定本身是「证据不足」→ 同样整条丢弃，并留原因码
                if admissible(&c, gate) { None } else { Some("VERDICT_INSUFFICIENT".to_string()) }
            }
        };
        match drop_code {
            None => candidates.push(c),
            Some(reason_code) => dropped.push(DroppedCandidate {
                interface_code: c.interface_code.clone(),
                service_name: c.service_name.clone(),
                verdict: c.verdict.as_str().to_string(),
                reason: c.reasoning_summary.clone(),
                reason_code,
                evidence_refs: c.evidence_refs.clone(),
            }),
        }
    }
    // "证据不足" verdict 不进候选
    candidates.retain(|c| !matches!(c.verdict, Verdict::证据不足));

    AttributionResult {
        candidates, dropped, gate_stats: stats, degraded, degrade_reason,
        capped_status: LedgerStatus::CandidateRanked,
    }
}

/// 把候选写入台账（candidate_ranked 为止），不带任何出站 POST。
pub fn record_candidates(db: &IndexDb, result: &AttributionResult, ws: i64, we: i64, dict_version: &str, now: &str) -> Vec<i64> {
    let mut ids = vec![];
    for c in &result.candidates {
        let dedup = format!("{}|{}|{}|{}", c.interface_code, ws, we, c.request_id);
        let id = db.insert_candidate(&c.interface_code, &c.service_name, ws, we,
            c.verdict.as_str(), c.confidence, &c.evidence_refs, &c.request_id,
            dict_version, result.degraded, &c.reasoning_summary, &dedup, now).unwrap_or(0);
        if id > 0 { ids.push(id); }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::conclusion::BaselineStats;
    use crate::ai::ports::{AuditMeta, AuditSink, ModelResponse};
    use crate::error::{AppError, ErrorCode, Result};

    fn conclusion(refs: Vec<&str>) -> Conclusion {
        Conclusion {
            interface_code: "A".into(), service_name: "S".into(), verdict: Verdict::接口自身劣化,
            confidence: 0.9, evidence_refs: refs.into_iter().map(|s| s.to_string()).collect(),
            reasoning_summary: "r".into(), next_check: "n".into(), degraded: false,
            baseline_stats: BaselineStats { p95_ms: 300.0, p99_ms: 400.0, degrade_ratio: 3.0, error_rate: 0.1, baseline_samples: 10 },
            request_id: "req".into(),
        }
    }

    struct Gateway { ok: bool }
    impl ModelGateway for Gateway {
        fn available(&self) -> bool { self.ok }
        fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
            if self.ok {
                Ok(ModelResponse { conclusions: vec![conclusion(vec!["flow#a1"])], degraded: false,
                    request_id: req.request_id.clone(), retries: 0 })
            } else { Err(AppError::code_only(ErrorCode::ModelUnavailable)) }
        }
    }
    struct Builder;
    impl PromptBuilder for Builder {
        fn build(&self, allowed: &[String]) -> ModelRequest {
            ModelRequest { body_json: "{}".into(), allowed_row_ids: allowed.to_vec(), request_id: "req".into() }
        }
    }
    struct Sink;
    impl AuditSink for Sink { fn write(&self, _m: &AuditMeta) {} }

    fn gate() -> ConcreteEvidenceGate {
        ConcreteEvidenceGate { ctx: GateContext {
            window_ids: ["flow#a1".into()].into_iter().collect(),
            all_ids: ["flow#a1".into()].into_iter().collect(),
            unbound_ids: std::collections::HashSet::new(), min_refs: 1,
        } }
    }

    #[test]
    fn baseline_and_model_paths_are_isomorphic() {
        let base = vec![conclusion(vec!["flow#a1"])];
        let r = orchestrate(&AttributionConfig { use_model: true, request_id: "req".into() },
            &base, &Gateway { ok: true }, &Builder, &gate());
        assert_eq!(r.candidates.len(), 1);
        assert!(!r.degraded);
    }

    #[test]
    fn model_unavailable_switches_to_baseline_and_sets_degraded() {
        let base = vec![conclusion(vec!["flow#a1"])];
        let r = orchestrate(&AttributionConfig { use_model: true, request_id: "req".into() },
            &base, &Gateway { ok: false }, &Builder, &gate());
        assert!(r.degraded);
        assert!(r.degrade_reason.contains("模型"));
        assert_eq!(r.candidates.len(), 1);
        assert!(r.candidates[0].degraded);
    }

    #[test]
    fn gate_rejected_candidate_not_in_set() {
        let base = vec![conclusion(vec!["flow#fake"])];
        let r = orchestrate(&AttributionConfig { use_model: false, request_id: "req".into() },
            &base, &Gateway { ok: true }, &Builder, &gate());
        assert!(r.candidates.is_empty());
        assert!(r.gate_stats.total_dropped() >= 1);
    }

    #[test]
    fn status_capped_at_candidate_ranked() {
        let base = vec![conclusion(vec!["flow#a1"])];
        let r = orchestrate(&AttributionConfig { use_model: false, request_id: "req".into() },
            &base, &Gateway { ok: true }, &Builder, &gate());
        assert_eq!(r.capped_status, LedgerStatus::CandidateRanked);
        // 台账中绝无 awaiting_confirmation / ticket_created（由编排产生）
        let db = IndexDb::open_in_memory().unwrap();
        let ids = record_candidates(&db, &r, 0, 100, "v1", "now");
        assert_eq!(ids.len(), 1);
        assert_eq!(db.ledger_by_id(ids[0]).unwrap().status, LedgerStatus::CandidateRanked);
        assert!(db.ledger_by_status(LedgerStatus::AwaitingConfirmation).is_empty());
        assert!(db.ledger_by_status(LedgerStatus::TicketCreated).is_empty());
    }
}
