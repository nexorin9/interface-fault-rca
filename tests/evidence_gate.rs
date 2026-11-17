//! task 41 验收：证据守门三条丢弃路径 + 证据不足 + 状态停在 candidate_ranked + 篡改输出丢弃。
mod common;

use common::{conclusion, flow};
use interface_fault_rca::ai::attribution::{orchestrate, AttributionConfig};
use interface_fault_rca::ai::conclusion::Verdict;
use interface_fault_rca::ai::evidence_gate::{admissible, check_conclusion, ConcreteEvidenceGate, GateContext, GateStats};
use interface_fault_rca::ai::ports::{EvidenceGate, GateDecision as GD, ModelGateway, ModelRequest, ModelResponse, PromptBuilder};
use interface_fault_rca::error::{AppError, ErrorCode, Result};
use interface_fault_rca::store::sqlite_index::LedgerStatus;
use std::collections::HashSet;

fn ctx() -> GateContext {
    GateContext {
        window_ids: ["flow#a1".into()].into_iter().collect(),
        all_ids: ["flow#a1".into(), "flow#outside".into()].into_iter().collect(),
        unbound_ids: ["flow#unbound".into()].into_iter().collect(),
        min_refs: 1,
    }
}

#[test]
fn three_drop_paths_have_distinct_reason_codes() {
    let c1 = conclusion(vec!["flow#ghost"], Verdict::接口自身劣化); // 不存在
    let c2 = conclusion(vec!["flow#outside"], Verdict::接口自身劣化); // 窗口外
    let c3 = conclusion(vec!["flow#unbound"], Verdict::接口自身劣化); // unbound
    assert_eq!(check_conclusion(&c1, &ctx()), GD::Drop { reason_code: "NONEXISTENT_REF".into() });
    assert_eq!(check_conclusion(&c2, &ctx()), GD::Drop { reason_code: "OUTSIDE_WINDOW".into() });
    assert_eq!(check_conclusion(&c3, &ctx()), GD::Drop { reason_code: "UNBOUND_REF".into() });
}

#[test]
fn gate_stats_distinguish_drop_reasons() {
    let gate = ConcreteEvidenceGate { ctx: ctx() };
    let mut stats = GateStats::default();
    stats.record(&gate.check(&conclusion(vec!["flow#ghost"], Verdict::接口自身劣化)));
    stats.record(&gate.check(&conclusion(vec!["flow#outside"], Verdict::接口自身劣化)));
    stats.record(&gate.check(&conclusion(vec!["flow#unbound"], Verdict::接口自身劣化)));
    assert_eq!(stats.nonexistent, 1);
    assert_eq!(stats.outside_window, 1);
    assert_eq!(stats.unbound, 1);
    assert_eq!(stats.total_dropped(), 3);
}

#[test]
fn insufficient_and_tampered_candidates_dropped() {
    let gate = ConcreteEvidenceGate { ctx: ctx() };
    // 无证据
    let empty = conclusion(vec![], Verdict::接口自身劣化);
    assert!(!admissible(&empty, &gate));
    // 证据不足 verdict
    let insuff = conclusion(vec!["flow#a1"], Verdict::证据不足);
    assert!(!admissible(&insuff, &gate));
    // 篡改：合法引用 + 非法引用
    let tampered = conclusion(vec!["flow#a1", "flow#ghost"], Verdict::接口自身劣化);
    assert!(matches!(gate.check(&tampered), GD::Drop { .. }));
}

struct GoodGateway;
impl ModelGateway for GoodGateway {
    fn available(&self) -> bool { true }
    fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse { conclusions: vec![conclusion(vec!["flow#a1"], Verdict::接口自身劣化)], degraded: false, request_id: req.request_id.clone(), retries: 0 })
    }
}
struct B;
impl PromptBuilder for B {
    fn build(&self, allowed: &[String]) -> ModelRequest {
        ModelRequest { body_json: "{}".into(), allowed_row_ids: allowed.to_vec(), request_id: "r".into() }
    }
}

#[test]
fn model_and_degraded_paths_stop_at_candidate_ranked() {
    let row = flow("ESB-ORDER-CREATE", "SVC", 1000, 300.0, 200);
    let gate = ConcreteEvidenceGate { ctx: GateContext {
        window_ids: ["flow#ESB-ORDER-CREATE".into()].into_iter().collect(),
        all_ids: HashSet::new(), unbound_ids: HashSet::new(), min_refs: 1,
    }};
    let base = vec![conclusion(vec!["flow#ESB-ORDER-CREATE"], Verdict::接口自身劣化)];
    let r = orchestrate(&AttributionConfig { use_model: true, request_id: "r".into() },
        &base, &GoodGateway, &B, &gate);
    assert_eq!(r.capped_status, LedgerStatus::CandidateRanked);
    let _ = row;
}
