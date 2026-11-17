//! 按 interface_code 聚合 P95/P99/错误率/调用量；当前窗口 P95 ÷ 基线中位数得劣化倍数。
//! 用中位数而非均值，避免被离群值带偏。产出 BaselineConclusion，与统一结论结构对齐。

use crate::config::BaselineConfig;
use crate::store::event_schema::EventRow;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone, Default)]
pub struct Aggregate {
    pub interface_code: String,
    pub durations: Vec<f64>,
    pub total: u64,
    pub errors: u64,
}

impl Aggregate {
    pub fn p95(&self) -> f64 { percentile(&self.durations, 0.95) }
    pub fn p99(&self) -> f64 { percentile(&self.durations, 0.99) }
    pub fn error_rate(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.errors as f64 / self.total as f64 }
    }
}

pub fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() { return 0.0; }
    let rank = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

pub fn median(vals: &[f64]) -> f64 {
    if vals.is_empty() { return 0.0; }
    let mut s = vals.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 { s[n / 2] } else { (s[n / 2 - 1] + s[n / 2]) / 2.0 }
}

/// 从事件行聚合（flow 行的 duration_ms / response_code）
pub fn aggregate_flows(rows: &[EventRow]) -> BTreeMap<String, Aggregate> {
    let mut map: BTreeMap<String, Aggregate> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind() == "flow") {
        let e = map.entry(r.interface_code().to_string()).or_insert_with(|| Aggregate {
            interface_code: r.interface_code().to_string(), ..Default::default()
        });
        if let Some(d) = r.fields.get("duration_ms").and_then(|s| s.parse::<f64>().ok()) {
            e.durations.push(d);
        }
        e.total += 1;
        let rc = r.fields.get("response_code").and_then(|s| s.parse::<i64>().ok()).unwrap_or(200);
        if rc >= 500 { e.errors += 1; }
    }
    for e in map.values_mut() { e.durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal)); }
    map
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineConclusion {
    pub interface_code: String,
    pub service_name: String,
    pub verdict: String,
    pub confidence: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub baseline_p95_ms: f64,
    pub degrade_ratio: f64,
    pub error_rate: f64,
    pub call_count: u64,
    pub baseline_samples: usize,
    pub evidence_refs: Vec<String>,
    pub reason: String,
    pub degraded: bool,
    pub params: BaselineParamsSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineParamsSnapshot {
    pub window_minutes: i64,
    pub degrade_ratio_threshold: f64,
    pub w_latency_abs: f64,
    pub w_ratio: f64,
    pub w_error_delta: f64,
    pub w_host_deviation: f64,
}

impl BaselineParamsSnapshot {
    pub fn from(cfg: &BaselineConfig) -> Self {
        Self {
            window_minutes: cfg.window_minutes,
            degrade_ratio_threshold: cfg.degrade_ratio_threshold,
            w_latency_abs: cfg.w_latency_abs, w_ratio: cfg.w_ratio,
            w_error_delta: cfg.w_error_delta, w_host_deviation: cfg.w_host_deviation,
        }
    }
}

/// 判定基线退化三类情形（样本不足 / 无重叠 / 方差过大），返回 "证据不足" 原因。
pub fn baseline_degeneration(baseline_samples: usize, baseline_durations: &[f64], cfg: &BaselineConfig) -> Option<String> {
    if baseline_samples < cfg.min_baseline_samples {
        return Some(format!("基线样本不足（{}<{}）", baseline_samples, cfg.min_baseline_samples));
    }
    if baseline_durations.is_empty() { return Some("基线与目标窗口无重叠".to_string()); }
    let m = median(baseline_durations);
    if m <= 0.0 { return Some("基线中位数为 0，无法计算倍数".to_string()); }
    let mean = baseline_durations.iter().sum::<f64>() / baseline_durations.len() as f64;
    let var = baseline_durations.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / baseline_durations.len() as f64;
    let cv = var.sqrt() / mean;
    if cv > cfg.max_baseline_cv {
        return Some(format!("基线自身抖动过大（CV={cv:.2}>{:.2}）", cfg.max_baseline_cv));
    }
    None
}

/// 由当前窗口 + 基线窗口行集合产出 BaselineConclusion 列表。
#[allow(clippy::too_many_arguments)]
pub fn conclude(
    current_rows: &[EventRow],
    baseline_rows: &[EventRow],
    cfg: &BaselineConfig,
) -> Vec<BaselineConclusion> {
    let current = aggregate_flows(current_rows);
    let baseline = aggregate_flows(baseline_rows);
    let mut out = vec![];
    for (code, agg) in &current {
        let b = baseline.get(code);
        let b_durations = b.map(|x| x.durations.clone()).unwrap_or_default();
        let baseline_p95 = b.map(|x| x.p95()).unwrap_or(0.0);
        let samples = b.map(|x| x.durations.len()).unwrap_or(0);
        let ratio = if baseline_p95 > 0.0 { agg.p95() / baseline_p95 } else { 0.0 };
        let over = if cfg.threshold_inclusive { ratio >= cfg.degrade_ratio_threshold } else { ratio > cfg.degrade_ratio_threshold };
        let verdict = if let Some(reason) = baseline_degeneration(samples, &b_durations, cfg) {
            out.push(BaselineConclusion {
                interface_code: code.clone(), service_name: String::new(),
                verdict: "证据不足".into(), confidence: 0.3,
                p95_ms: agg.p95(), p99_ms: agg.p99(), baseline_p95_ms: baseline_p95,
                degrade_ratio: ratio, error_rate: agg.error_rate(), call_count: agg.total,
                baseline_samples: samples, evidence_refs: vec![],
                reason, degraded: false, params: BaselineParamsSnapshot::from(cfg),
            });
            continue;
        } else if over {
            "接口自身劣化"
        } else { "证据不足" };
        let refs: Vec<String> = current_rows.iter()
            .filter(|r| r.interface_code() == code && r.kind() == "flow")
            .map(|r| r.row_id.clone()).collect();
        out.push(BaselineConclusion {
            interface_code: code.clone(),
            service_name: current_rows.iter().find(|r| r.interface_code() == code)
                .and_then(|r| r.tags.get("service_name")).cloned().unwrap_or_default(),
            verdict: verdict.into(),
            confidence: (0.5 + (ratio / 10.0).min(0.5)).min(1.0),
            p95_ms: agg.p95(), p99_ms: agg.p99(), baseline_p95_ms: baseline_p95,
            degrade_ratio: ratio, error_rate: agg.error_rate(), call_count: agg.total,
            baseline_samples: samples,
            evidence_refs: refs, degraded: false, params: BaselineParamsSnapshot::from(cfg),
            reason: format!("P95 {}ms / 基线 {:.0}ms = {:.2}倍", agg.p95(), baseline_p95, ratio),
        });
    }
    let _: HashMap<String, ()> = HashMap::new();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn flow(code: &str, ts: i64, dur: i64, rc: i64, id: &str) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), "esb_audit".into());
        tags.insert("event_kind".into(), "flow".into());
        tags.insert("interface_code".into(), code.into());
        let mut fields = BTreeMap::new();
        fields.insert("duration_ms".into(), dur.to_string());
        fields.insert("response_code".into(), rc.to_string());
        EventRow::new(tags, ts, fields, id.into(), compute_tsid("esb_audit", ts, code, "", "flow")).unwrap()
    }

    #[test]
    fn percentile_and_median() {
        let v = vec![1.0, 2.0, 3.0, 4.0, 100.0];
        assert_eq!(median(&v), 3.0);
        assert_eq!(percentile(&v, 0.95), 100.0);
    }

    #[test]
    fn aggregate_p95_error_rate() {
        let rows = vec![flow("A", 1, 100, 200, "a1"), flow("A", 2, 200, 500, "a2")];
        let m = aggregate_flows(&rows);
        assert_eq!(m["A"].total, 2);
        assert_eq!(m["A"].errors, 1);
        assert_eq!(m["A"].error_rate(), 0.5);
    }

    #[test]
    fn degrade_ratio_and_verdict() {
        let base: Vec<_> = (0..10).map(|i| flow("A", i, 100, 200, &format!("b{i}"))).collect();
        let cur: Vec<_> = (10..20).map(|i| flow("A", i, 300, 200, &format!("a{i}"))).collect();
        let cfg = BaselineConfig::default();
        let out = conclude(&cur, &base, &cfg);
        assert_eq!(out.len(), 1);
        assert!((out[0].degrade_ratio - 3.0).abs() < 0.01);
        assert_eq!(out[0].verdict, "接口自身劣化");
        assert!(!out[0].evidence_refs.is_empty());
    }

    #[test]
    fn insufficient_baseline_gives_evidence_missing() {
        let base = vec![flow("A", 1, 100, 200, "b1")];
        let cur: Vec<_> = (10..20).map(|i| flow("A", i, 300, 200, &format!("a{i}"))).collect();
        let out = conclude(&cur, &base, &BaselineConfig::default());
        assert_eq!(out[0].verdict, "证据不足");
        assert!(out[0].reason.contains("样本不足"));
    }

    #[test]
    fn conclusion_carries_params_and_samples() {
        let base: Vec<_> = (0..10).map(|i| flow("A", i, 100, 200, &format!("b{i}"))).collect();
        let cur = vec![flow("A", 20, 300, 200, "a1")];
        let out = conclude(&cur, &base, &BaselineConfig::default());
        assert_eq!(out[0].baseline_samples, 10);
        assert_eq!(out[0].params.window_minutes, 30);
    }
}
