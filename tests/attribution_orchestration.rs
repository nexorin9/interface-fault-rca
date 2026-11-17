//! task 10 验收：归因编排 —— 基线与模型路径同构 / 模型不可用切基线置 degraded / 被 gate 拒绝不进候选 / 状态停在 candidate_ranked。
mod common;

use common::{conclusion, flow};
use interface_fault_rca::ai::attribution::{orchestrate, AttributionConfig};
use interface_fault_rca::ai::conclusion::{Conclusion, Verdict};
use interface_fault_rca::ai::evidence_gate::{ConcreteEvidenceGate, GateContext};
use interface_fault_rca::ai::ports::{ModelGateway, ModelRequest, ModelResponse, PromptBuilder};
use interface_fault_rca::error::{AppError, ErrorCode, Result};
use interface_fault_rca::store::sqlite_index::LedgerStatus;
use std::collections::HashSet;

struct ModelOK;
impl ModelGateway for ModelOK {
    fn available(&self) -> bool { true }
    fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse {
            conclusions: vec![conclusion(vec!["flow#a1"], Verdict::下游依赖饱和)],
            degraded: false, request_id: req.request_id.clone(), retries: 0,
        })
    }
}
struct Unavailable;
impl ModelGateway for Unavailable {
    fn available(&self) -> bool { false }
    fn call(&self, _: &ModelRequest) -> Result<ModelResponse> { Err(AppError::code_only(ErrorCode::ModelUnavailable)) }
}
struct B;
impl PromptBuilder for B {
    fn build(&self, allowed: &[String]) -> ModelRequest {
        ModelRequest { body_json: "{}".into(), allowed_row_ids: allowed.to_vec(), request_id: "r".into() }
    }
}

fn gate() -> ConcreteEvidenceGate {
    ConcreteEvidenceGate { ctx: GateContext {
        window_ids: ["flow#a1".into()].into_iter().collect(),
        all_ids: ["flow#a1".into()].into_iter().collect(),
        unbound_ids: HashSet::new(), min_refs: 1,
    }}
}

#[test]
fn baseline_and_model_paths_are_isomorphic() {
    let base = vec![conclusion(vec!["flow#a1"], Verdict::接口自身劣化)];
    let r_model = orchestrate(&AttributionConfig { use_model: true, request_id: "r".into() },
        &base, &ModelOK, &B, &gate());
    let r_base = orchestrate(&AttributionConfig { use_model: false, request_id: "r".into() },
        &base, &Unavailable, &B, &gate());
    // 两条路径产出同为 Conclusion，字段同构
    let j_model = serde_json::to_string(&r_model.candidates[0]).unwrap();
    let j_base = serde_json::to_string(&r_base.candidates[0]).unwrap();
    for k in ["verdict", "confidence", "evidence_refs", "degraded", "baseline_stats"] {
        assert!(j_model.contains(k));
        assert!(j_base.contains(k));
    }
}

#[test]
fn model_unavailable_switches_to_baseline_with_degraded() {
    let base = vec![conclusion(vec!["flow#a1"], Verdict::接口自身劣化)];
    let r = orchestrate(&AttributionConfig { use_model: true, request_id: "r".into() },
        &base, &Unavailable, &B, &gate());
    assert!(r.degraded);
    assert!(r.candidates.iter().all(|c: &Conclusion| c.degraded));
}

#[test]
fn gate_rejected_candidate_not_in_candidate_set() {
    let bad = vec![conclusion(vec!["flow#ghost"], Verdict::接口自身劣化)];
    let r = orchestrate(&AttributionConfig { use_model: true, request_id: "r".into() },
        &bad, &Unavailable, &B, &gate());
    assert!(r.candidates.is_empty());
}

#[test]
fn orchestration_never_advances_past_candidate_ranked() {
    let base = vec![conclusion(vec!["flow#a1"], Verdict::接口自身劣化)];
    for use_model in [true, false] {
        let r = orchestrate(&AttributionConfig { use_model, request_id: "r".into() },
            &base, &ModelOK, &B, &gate());
        assert_eq!(r.capped_status, LedgerStatus::CandidateRanked);
    }
    let _ = flow("A", "S", 1, 1.0, 200);
}
