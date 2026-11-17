//! 集成测试公共构造器：统一事件行、结论、配置的最小可用样本。
//! 只用库公开 API，不复制内部逻辑。

#![allow(dead_code)]

use interface_fault_rca::ai::conclusion::{BaselineStats, Conclusion, Verdict};
use interface_fault_rca::baseline::aggregate::{BaselineConclusion, BaselineParamsSnapshot};
use interface_fault_rca::config::BaselineConfig;
use interface_fault_rca::store::event_schema::EventRow;
use interface_fault_rca::store::tsid::{compute_tsid, RowIdAllocator};
use std::collections::BTreeMap;
use std::sync::Arc;

pub fn tags(source: &str, kind: &str, code: &str, service: &str) -> BTreeMap<String, String> {
    let mut t = BTreeMap::new();
    t.insert("source".into(), source.into());
    t.insert("event_kind".into(), kind.into());
    t.insert("interface_code".into(), code.into());
    t.insert("service_name".into(), service.into());
    t
}

/// 构造一条 flow 行，duration_ms 与 response_code 可指定。
pub fn flow(code: &str, service: &str, ts_ms: i64, duration_ms: f64, rc: i64) -> EventRow {
    let mut f = BTreeMap::new();
    f.insert("duration_ms".into(), format!("{duration_ms}"));
    f.insert("response_code".into(), format!("{rc}"));
    f.insert("pipeline_version".into(), "esb_audit@v1".into());
    EventRow::new(
        tags("esb_audit", "flow", code, service),
        ts_ms,
        f,
        format!("flow#{code}"),
        compute_tsid("esb_audit", ts_ms, code, service, "flow"),
    )
    .unwrap()
}

/// 构造一条 log 行。
pub fn log(code: &str, service: &str, ts_ms: i64, level: &str, message: &str) -> EventRow {
    let mut f = BTreeMap::new();
    f.insert("level".into(), level.into());
    f.insert("message".into(), message.into());
    f.insert("pipeline_version".into(), "app_log@v1".into());
    EventRow::new(
        tags("app_log", "log", code, service),
        ts_ms,
        f,
        format!("log#{code}#{ts_ms}"),
        compute_tsid("app_log", ts_ms, code, service, "log"),
    )
    .unwrap()
}

/// 构造一条 metric 行。
pub fn metric(code: &str, host: &str, ts_ms: i64, name: &str, value: f64) -> EventRow {
    let mut t = tags("prom", "metric", code, "");
    t.insert("host".into(), host.into());
    let mut f = BTreeMap::new();
    f.insert("metric_name".into(), name.into());
    f.insert("value".into(), format!("{value}"));
    EventRow::new(
        t,
        ts_ms,
        f,
        format!("metric#{code}#{ts_ms}"),
        compute_tsid("prom", ts_ms, code, "", "metric"),
    )
    .unwrap()
}

pub fn baseline_params() -> BaselineParamsSnapshot {
    BaselineParamsSnapshot::from(&BaselineConfig::default())
}

pub fn baseline_conclusion(code: &str, ratio: f64, p95: f64, refs: Vec<String>) -> BaselineConclusion {
    BaselineConclusion {
        interface_code: code.into(),
        service_name: "SVC".into(),
        verdict: "接口自身劣化".into(),
        confidence: 0.9,
        p95_ms: p95,
        p99_ms: p95 * 1.2,
        baseline_p95_ms: 100.0,
        degrade_ratio: ratio,
        error_rate: 0.05,
        call_count: 100,
        baseline_samples: 20,
        evidence_refs: refs,
        reason: String::new(),
        degraded: false,
        params: baseline_params(),
    }
}

pub fn conclusion(refs: Vec<&str>, verdict: Verdict) -> Conclusion {
    Conclusion {
        interface_code: "ESB-ORDER-CREATE".into(),
        service_name: "SVC".into(),
        verdict,
        confidence: 0.9,
        evidence_refs: refs.into_iter().map(|s| s.to_string()).collect(),
        reasoning_summary: "P95 相对基线劣化".into(),
        next_check: "核对 ESB 登记表".into(),
        degraded: false,
        baseline_stats: BaselineStats {
            p95_ms: 300.0,
            p99_ms: 400.0,
            degrade_ratio: 3.0,
            error_rate: 0.05,
            baseline_samples: 20,
        },
        request_id: "req-test".into(),
    }
}

pub fn allocator() -> Arc<RowIdAllocator> {
    Arc::new(RowIdAllocator::new())
}
