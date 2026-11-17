//! task 39 验收：错误五分类映射、仅连接与 5xx 重试、其余不重试、降级产出候选带 degraded、不推进状态。
mod common;

use common::{conclusion, flow};
use interface_fault_rca::ai::attribution::{orchestrate, AttributionConfig};
use interface_fault_rca::ai::conclusion::Verdict;
use interface_fault_rca::ai::errors::{backoff_ms, classify_http, decide_retries, should_degrade, ModelErrorKind};
use interface_fault_rca::ai::evidence_gate::{ConcreteEvidenceGate, GateContext};
use interface_fault_rca::ai::ports::{ModelGateway, ModelRequest, ModelResponse, PromptBuilder};
use interface_fault_rca::error::{AppError, ErrorCode, Result};
use interface_fault_rca::store::sqlite_index::LedgerStatus;
use std::collections::HashSet;

#[test]
fn five_error_kinds_map_to_error_codes() {
    for k in [ModelErrorKind::Connection, ModelErrorKind::Timeout, ModelErrorKind::Http5xx,
              ModelErrorKind::Http4xx, ModelErrorKind::JsonParse, ModelErrorKind::SchemaValidation] {
        assert!(should_degrade(k));
        let _ = k.error_code();
    }
    assert_eq!(classify_http(503), ModelErrorKind::Http5xx);
    assert_eq!(classify_http(400), ModelErrorKind::Http4xx);
}

#[test]
fn only_connection_and_5xx_retry_within_cap() {
    assert_eq!(decide_retries(ModelErrorKind::Connection, 2), 2);
    assert_eq!(decide_retries(ModelErrorKind::Http5xx, 2), 2);
    assert_eq!(decide_retries(ModelErrorKind::Timeout, 2), 0);
    assert_eq!(decide_retries(ModelErrorKind::Http4xx, 2), 0);
    assert_eq!(decide_retries(ModelErrorKind::JsonParse, 2), 0);
    assert_eq!(decide_retries(ModelErrorKind::SchemaValidation, 2), 0);
    let b = backoff_ms(100, 3);
    assert_eq!(b, vec![100, 200, 400]);
}

struct Unavailable;
impl ModelGateway for Unavailable {
    fn available(&self) -> bool { true }
    fn call(&self, _: &ModelRequest) -> Result<ModelResponse> {
        Err(AppError::code_only(ErrorCode::ModelUnavailable))
    }
}
struct B;
impl PromptBuilder for B {
    fn build(&self, allowed: &[String]) -> ModelRequest {
        ModelRequest { body_json: "{}".into(), allowed_row_ids: allowed.to_vec(), request_id: "r".into() }
    }
}

#[test]
fn degradation_still_yields_candidate_with_degraded_flag() {
    let row = flow("ESB-ORDER-CREATE", "SVC", 1000, 300.0, 200);
    let baseline = vec![conclusion(vec!["flow#ESB-ORDER-CREATE"], Verdict::接口自身劣化)];
    let gate = ConcreteEvidenceGate { ctx: GateContext {
        window_ids: [row.row_id.clone()].into_iter().collect(),
        all_ids: [row.row_id.clone()].into_iter().collect(),
        unbound_ids: HashSet::new(), min_refs: 1,
    }};
    let result = orchestrate(
        &AttributionConfig { use_model: true, request_id: "r".into() },
        &baseline, &Unavailable, &B, &gate);
    assert!(result.degraded);
    assert!(!result.candidates.is_empty());
    assert!(result.candidates.iter().all(|c| c.degraded));
    // 状态不推进
    assert_eq!(result.capped_status, LedgerStatus::CandidateRanked);
}
