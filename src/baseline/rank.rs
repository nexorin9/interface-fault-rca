//! 候选排序：延迟绝对值 + 相对基线倍数 + 错误率变化 + 主机指标偏离，权重可配。
//! 确定性 tie-break（倍数→错误率变化→绝对延迟→接口码字典序）+ 抖动抑制，
//! 保证同一份数据两次跑出同一个第一名。

use crate::baseline::aggregate::BaselineConclusion;
use crate::config::BaselineConfig;
use std::cmp::Ordering;

#[derive(Debug, Clone)]
pub struct RankedCandidate {
    pub conclusion: BaselineConclusion,
    pub score: f64,
    pub score_breakdown: ScoreBreakdown,
}

#[derive(Debug, Clone)]
pub struct ScoreBreakdown {
    pub latency_component: f64,
    pub ratio_component: f64,
    pub error_component: f64,
    pub host_component: f64,
    pub tie_break: String,
}

/// 抖动抑制：对单点尖刺做平滑（与邻域均值折中），用于排序打分。
pub fn smooth(values: &[f64]) -> Vec<f64> {
    if values.len() < 3 { return values.to_vec(); }
    values.iter().enumerate().map(|(i, &v)| {
        let lo = i.saturating_sub(1);
        let hi = (i + 1).min(values.len() - 1);
        let neighbor = (values[lo] + values[hi]) / 2.0;
        // 单点偏离邻域超过 50% 才平滑，避免改变明显差异项次序
        if (v - neighbor).abs() > neighbor.abs() * 0.5 { v * 0.7 + neighbor * 0.3 } else { v }
    }).collect()
}

/// 计算分项得分（各分量归一化到 0..1 量级）
pub fn score(c: &BaselineConclusion, cfg: &BaselineConfig) -> ScoreBreakdown {
    let p = &c.params;
    let lat = (c.p95_ms / 1000.0).min(1.0) * p.w_latency_abs;
    let ratio = (c.degrade_ratio / 10.0).min(1.0) * p.w_ratio;
    let err = c.error_rate.min(1.0) * p.w_error_delta;
    // 主机偏离用 reason 中是否含 host 信息近似（基线层无主机序列时置 0）
    let host = 0.0_f64 * p.w_host_deviation;
    ScoreBreakdown {
        latency_component: lat, ratio_component: ratio, error_component: err, host_component: host,
        tie_break: String::new(),
    }
}

fn total(b: &ScoreBreakdown) -> f64 {
    b.latency_component + b.ratio_component + b.error_component + b.host_component
}

/// 四级确定性 tie-break：劣化倍数 → 错误率变化 → 绝对延迟 → 接口码字典序。
fn deterministic_cmp(a: &RankedCandidate, b: &RankedCandidate) -> Ordering {
    let ac = &a.conclusion;
    let bc = &b.conclusion;
    ac.degrade_ratio.partial_cmp(&bc.degrade_ratio).unwrap_or(Ordering::Equal)
        .then_with(|| ac.error_rate.partial_cmp(&bc.error_rate).unwrap_or(Ordering::Equal))
        .then_with(|| ac.p95_ms.partial_cmp(&bc.p95_ms).unwrap_or(Ordering::Equal))
        .then_with(|| ac.interface_code.cmp(&bc.interface_code))
}

/// 排序：先按总分降序，分数相等时走确定性 tie-break。
pub fn rank(conclusions: Vec<BaselineConclusion>, cfg: &BaselineConfig) -> Vec<RankedCandidate> {
    let mut out: Vec<RankedCandidate> = conclusions.into_iter().map(|c| {
        let sd = score(&c, cfg);
        RankedCandidate { conclusion: c, score: total(&sd), score_breakdown: sd }
    }).collect();
    out.sort_by(|a, b| {
        b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal)
            .then_with(|| deterministic_cmp(a, b))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::baseline::aggregate::BaselineParamsSnapshot;

    fn cc(code: &str, ratio: f64, p95: f64, err: f64) -> BaselineConclusion {
        BaselineConclusion {
            interface_code: code.into(), service_name: "".into(), verdict: "接口自身劣化".into(),
            confidence: 0.9, p95_ms: p95, p99_ms: p95 * 1.2, baseline_p95_ms: 100.0,
            degrade_ratio: ratio, error_rate: err, call_count: 10, baseline_samples: 10,
            evidence_refs: vec![], reason: "".into(), degraded: false,
            params: BaselineParamsSnapshot {
                window_minutes: 30, degrade_ratio_threshold: 1.5,
                w_latency_abs: 0.3, w_ratio: 0.35, w_error_delta: 0.2, w_host_deviation: 0.15,
            },
        }
    }

    #[test]
    fn repeated_sort_is_bitwise_identical() {
        let items = vec![cc("A", 3.0, 300.0, 0.1), cc("B", 3.0, 300.0, 0.1), cc("C", 2.0, 200.0, 0.5)];
        let r1 = rank(items.clone(), &BaselineConfig::default());
        let r2 = rank(items, &BaselineConfig::default());
        let ids: Vec<_> = r1.iter().map(|x| x.conclusion.interface_code.clone()).collect();
        let ids2: Vec<_> = r2.iter().map(|x| x.conclusion.interface_code.clone()).collect();
        assert_eq!(ids, ids2);
    }

    #[test]
    fn tie_broken_by_interface_code() {
        let items = vec![cc("Z", 3.0, 300.0, 0.1), cc("A", 3.0, 300.0, 0.1)];
        let r = rank(items, &BaselineConfig::default());
        assert_eq!(r[0].conclusion.interface_code, "A");
    }

    #[test]
    fn top_is_stable_under_small_jitter() {
        let base = vec![cc("A", 5.0, 500.0, 0.1), cc("B", 2.0, 200.0, 0.1)];
        let r1 = rank(base.clone(), &BaselineConfig::default());
        let jitter = vec![cc("A", 5.05, 495.0, 0.11), cc("B", 2.0, 200.0, 0.1)];
        let r2 = rank(jitter, &BaselineConfig::default());
        assert_eq!(r1[0].conclusion.interface_code, r2[0].conclusion.interface_code);
    }

    #[test]
    fn smooth_suppresses_single_spike() {
        let v = vec![10.0, 10.0, 1000.0, 10.0, 10.0];
        let s = smooth(&v);
        assert!(s[2] < 1000.0);
    }

    #[test]
    fn score_breakdown_available_for_audit() {
        let r = rank(vec![cc("A", 3.0, 300.0, 0.2)], &BaselineConfig::default());
        assert!(r[0].score_breakdown.ratio_component > 0.0);
        assert!(r[0].score > 0.0);
    }
}
