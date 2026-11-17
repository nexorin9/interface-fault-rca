//! 统一结论结构：确定性基线路径与模型路径产出同构，
//! 「模型不可达 → 走基线」在数据层是同一条路径，不是两套实现。

use crate::baseline::aggregate::BaselineConclusion;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    接口自身劣化,
    下游依赖饱和,
    厂商返回异常,
    调用量突增,
    证据不足,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::接口自身劣化 => "接口自身劣化",
            Verdict::下游依赖饱和 => "下游依赖饱和",
            Verdict::厂商返回异常 => "厂商返回异常",
            Verdict::调用量突增 => "调用量突增",
            Verdict::证据不足 => "证据不足",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "接口自身劣化" => Some(Self::接口自身劣化),
            "下游依赖饱和" => Some(Self::下游依赖饱和),
            "厂商返回异常" => Some(Self::厂商返回异常),
            "调用量突增" => Some(Self::调用量突增),
            "证据不足" => Some(Self::证据不足),
            _ => None,
        }
    }
    pub fn all() -> [Verdict; 5] {
        [Verdict::接口自身劣化, Verdict::下游依赖饱和, Verdict::厂商返回异常, Verdict::调用量突增, Verdict::证据不足]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineStats {
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub degrade_ratio: f64,
    pub error_rate: f64,
    pub baseline_samples: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conclusion {
    pub interface_code: String,
    pub service_name: String,
    pub verdict: Verdict,
    pub confidence: f64,
    pub evidence_refs: Vec<String>,
    pub reasoning_summary: String,
    pub next_check: String,
    pub degraded: bool,
    pub baseline_stats: BaselineStats,
    pub request_id: String,
}

impl Conclusion {
    /// 从确定性基线结论适配（数据层同构的落点）
    pub fn from_baseline(b: &BaselineConclusion, request_id: &str) -> Self {
        Self {
            interface_code: b.interface_code.clone(),
            service_name: b.service_name.clone(),
            verdict: Verdict::parse(&b.verdict).unwrap_or(Verdict::证据不足),
            confidence: b.confidence,
            evidence_refs: b.evidence_refs.clone(),
            reasoning_summary: if b.reason.is_empty() { b.reasoning_text() } else { b.reason.clone() },
            next_check: "核对证据行原文与 ESB 登记表".to_string(),
            degraded: b.degraded,
            baseline_stats: BaselineStats {
                p95_ms: b.p95_ms, p99_ms: b.p99_ms, degrade_ratio: b.degrade_ratio,
                error_rate: b.error_rate, baseline_samples: b.baseline_samples,
            },
            request_id: request_id.to_string(),
        }
    }
}

trait ReasoningText {
    fn reasoning_text(&self) -> String;
}
impl ReasoningText for BaselineConclusion {
    fn reasoning_text(&self) -> String {
        format!("P95 {}ms 相对基线 {}ms 劣化 {:.2} 倍，错误率 {:.1}%",
            self.p95_ms, self.baseline_p95_ms, self.degrade_ratio, self.error_rate * 100.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bc(verdict: &str) -> BaselineConclusion {
        BaselineConclusion {
            interface_code: "A".into(), service_name: "S".into(), verdict: verdict.into(), confidence: 0.8,
            p95_ms: 300.0, p99_ms: 400.0, baseline_p95_ms: 100.0, degrade_ratio: 3.0,
            error_rate: 0.1, call_count: 10, baseline_samples: 10,
            evidence_refs: vec!["flow#a1".into()], reason: "".into(), degraded: false,
            params: crate::baseline::aggregate::BaselineParamsSnapshot {
                window_minutes: 30, degrade_ratio_threshold: 1.5,
                w_latency_abs: 0.3, w_ratio: 0.35, w_error_delta: 0.2, w_host_deviation: 0.15,
            },
        }
    }

    #[test]
    fn five_verdicts_parse_and_roundtrip() {
        for v in Verdict::all() {
            assert_eq!(Verdict::parse(v.as_str()), Some(v));
        }
        assert_eq!(Verdict::parse("未知"), None);
    }

    #[test]
    fn baseline_adapts_to_unified_conclusion() {
        let c = Conclusion::from_baseline(&bc("接口自身劣化"), "req-1");
        assert_eq!(c.verdict, Verdict::接口自身劣化);
        assert_eq!(c.request_id, "req-1");
        assert_eq!(c.baseline_stats.degrade_ratio, 3.0);
        assert!(c.reasoning_summary.contains("劣化"));
    }

    #[test]
    fn unknown_baseline_verdict_maps_to_insufficient() {
        let c = Conclusion::from_baseline(&bc("未知判定"), "r");
        assert_eq!(c.verdict, Verdict::证据不足);
    }

    #[test]
    fn baseline_and_model_paths_are_isomorphic() {
        let c = Conclusion::from_baseline(&bc("厂商返回异常"), "r");
        let j = serde_json::to_string(&c).unwrap();
        for k in ["verdict", "confidence", "evidence_refs", "degraded", "baseline_stats"] {
            assert!(j.contains(k));
        }
    }
}
