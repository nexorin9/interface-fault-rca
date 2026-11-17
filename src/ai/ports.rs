//! M6 四 trait 契约：ModelGateway / PromptBuilder / EvidenceGate / AuditSink。
//! 具体实现由 task 37–41 各自落地；本文件只定义契约，避免两条并行路径。

use crate::ai::conclusion::Conclusion;
use crate::error::Result;

/// 结构化模型调用入参（含可引用行 ID 白名单）
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub body_json: String,
    pub allowed_row_ids: Vec<String>,
    pub request_id: String,
}

/// 结构化模型调用出参
#[derive(Debug, Clone)]
pub struct ModelResponse {
    pub conclusions: Vec<Conclusion>,
    pub degraded: bool,
    pub request_id: String,
    pub retries: u32,
}

/// 单一模型网关（业务层只经此调用，不 import 厂商 SDK）
pub trait ModelGateway {
    fn call(&self, req: &ModelRequest) -> Result<ModelResponse>;
    fn available(&self) -> bool;
}

/// 证据窗口 → 结构化请求体装配
pub trait PromptBuilder {
    fn build(&self, allowed_row_ids: &[String]) -> ModelRequest;
}

/// 证据守门：复核 evidence_refs 是否全部合法
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision { Pass, Drop { reason_code: String } }

pub trait EvidenceGate {
    fn check(&self, c: &Conclusion) -> GateDecision;
}

/// 可审计调用元数据（不落 prompt 全文）
#[derive(Debug, Clone)]
pub struct AuditMeta {
    pub request_id: String,
    pub model: String,
    pub host: String,
    pub input_hash: String,
    pub token_count: u32,
    pub elapsed_ms: u64,
    pub response_hash: String,
    pub degraded: bool,
    pub retries: u32,
}

pub trait AuditSink {
    fn write(&self, meta: &AuditMeta);
}

#[cfg(test)]
mod tests {
    use super::*;

    // 契约可实现性自检：用最小替身实现四个 trait
    struct StubGateway;
    impl ModelGateway for StubGateway {
        fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
            Ok(ModelResponse { conclusions: vec![], degraded: false, request_id: req.request_id.clone(), retries: 0 })
        }
        fn available(&self) -> bool { true }
    }
    struct StubGate;
    impl EvidenceGate for StubGate {
        fn check(&self, _c: &Conclusion) -> GateDecision { GateDecision::Pass }
    }
    struct StubSink;
    impl AuditSink for StubSink {
        fn write(&self, _m: &AuditMeta) {}
    }

    #[test]
    fn four_traits_are_implementable() {
        let req = ModelRequest { body_json: "{}".into(), allowed_row_ids: vec![], request_id: "r".into() };
        let gw = StubGateway;
        assert!(gw.available());
        assert_eq!(gw.call(&req).unwrap().request_id, "r");
        assert!(matches!(StubGate.check(&fake_conclusion()), GateDecision::Pass));
        StubSink.write(&audit());
    }

    fn fake_conclusion() -> Conclusion {
        Conclusion {
            interface_code: "A".into(), service_name: "S".into(), verdict: crate::ai::conclusion::Verdict::接口自身劣化,
            confidence: 0.8, evidence_refs: vec![], reasoning_summary: "".into(), next_check: "".into(),
            degraded: false, baseline_stats: crate::ai::conclusion::BaselineStats {
                p95_ms: 0.0, p99_ms: 0.0, degrade_ratio: 0.0, error_rate: 0.0, baseline_samples: 0,
            }, request_id: "r".into(),
        }
    }
    fn audit() -> AuditMeta {
        AuditMeta { request_id: "r".into(), model: "m".into(), host: "h".into(), input_hash: "i".into(),
            token_count: 1, elapsed_ms: 1, response_hash: "o".into(), degraded: false, retries: 0 }
    }
}
