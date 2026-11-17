//! 变更点检测四形态：阶梯劣化 / 尖刺 / 缓慢爬升 / 基线自身抖动。
//! 四类判定互不混淆，各自带置信度。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeShape { StepDegradation, Spike, SlowRamp, BaselineJitter, Stable }

impl ChangeShape {
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeShape::StepDegradation => "阶梯劣化",
            ChangeShape::Spike => "尖刺",
            ChangeShape::SlowRamp => "缓慢爬升",
            ChangeShape::BaselineJitter => "基线抖动",
            ChangeShape::Stable => "稳定",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChangePoint {
    pub shape: ChangeShape,
    pub confidence: f64,
    pub index: usize,
    pub detail: String,
}

/// 对时序序列做形态分类。输入为按时间排序的延迟值。
pub fn detect(vals: &[f64], baseline_median: f64) -> ChangePoint {
    if vals.len() < 4 {
        return ChangePoint { shape: ChangeShape::Stable, confidence: 0.2, index: 0, detail: "样本不足".into() };
    }
    let first_half = &vals[..vals.len() / 2];
    let second_half = &vals[vals.len() / 2..];
    let m1 = crate::baseline::aggregate::median(first_half);
    let m2 = crate::baseline::aggregate::median(second_half);
    let overall_mean = vals.iter().sum::<f64>() / vals.len() as f64;
    let max = vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min = vals.iter().cloned().fold(f64::INFINITY, f64::min);

    // 尖刺：单点极高，但中位数几乎不变
    if max > m1 * 2.5 && m2 / m1.max(1.0) < 1.4 {
        let idx = vals.iter().position(|&v| v == max).unwrap_or(0);
        return ChangePoint { shape: ChangeShape::Spike, confidence: 0.85, index: idx,
            detail: format!("单点 {max:.0} 远超中位数 {m1:.0}") };
    }

    // 阶梯劣化：后半段中位数相对前半段显著抬升，且抬升后保持（后半段自身平稳）
    if m2 > m1 * 1.5 {
        let second_cv = cv(second_half);
        if second_cv < 0.5 {
            let idx = find_step_index(vals, m1, m2);
            return ChangePoint { shape: ChangeShape::StepDegradation, confidence: 0.9, index: idx,
                detail: format!("中位数 {m1:.0}→{m2:.0} 并保持") };
        }
    }

    // 缓慢爬升：整体单调趋势明显（首尾比 > 1.5），无单点尖刺，非阶梯
    let first_q = slice_mean(vals, 0, vals.len() / 4);
    let last_q = slice_mean(vals, vals.len() * 3 / 4, vals.len());
    if last_q > first_q * 1.5 && max / overall_mean.max(1.0) < 2.5 {
        return ChangePoint { shape: ChangeShape::SlowRamp, confidence: 0.75, index: vals.len() / 2,
            detail: format!("首四分位 {first_q:.0}→末四分位 {last_q:.0}") };
    }

    // 基线抖动：围绕基线中位数反复波动，振幅大但无趋势
    let cv_all = cv(vals);
    if cv_all > 0.3 && (m2 - m1).abs() / m1.max(1.0) < 0.3
        && baseline_median > 0.0 && (min..=max).contains(&baseline_median) {
        return ChangePoint { shape: ChangeShape::BaselineJitter, confidence: 0.7, index: 0,
            detail: format!("围绕基线 {baseline_median:.0} 抖动 CV={cv_all:.2}") };
    }

    ChangePoint { shape: ChangeShape::Stable, confidence: 0.6, index: 0, detail: "无显著形态".into() }
}

fn cv(vals: &[f64]) -> f64 {
    if vals.is_empty() { return 0.0; }
    let m = vals.iter().sum::<f64>() / vals.len() as f64;
    if m == 0.0 { return 0.0; }
    let var = vals.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / vals.len() as f64;
    var.sqrt() / m
}

fn slice_mean(v: &[f64], a: usize, b: usize) -> f64 {
    if a >= b || b > v.len() { return 0.0; }
    v[a..b].iter().sum::<f64>() / (b - a) as f64
}

fn find_step_index(vals: &[f64], m1: f64, m2: f64) -> usize {
    let threshold = (m1 + m2) / 2.0;
    vals.iter().position(|&v| v > threshold).unwrap_or(vals.len() / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_degradation_detected() {
        let v = vec![100.0, 105.0, 98.0, 102.0, 300.0, 310.0, 295.0, 305.0];
        let cp = detect(&v, 100.0);
        assert_eq!(cp.shape, ChangeShape::StepDegradation);
    }

    #[test]
    fn spike_detected() {
        let v = vec![100.0, 102.0, 98.0, 101.0, 99.0, 800.0, 100.0, 97.0];
        let cp = detect(&v, 100.0);
        assert_eq!(cp.shape, ChangeShape::Spike);
    }

    #[test]
    fn slow_ramp_detected() {
        let v = vec![100.0, 115.0, 130.0, 145.0, 160.0, 175.0, 190.0, 205.0];
        let cp = detect(&v, 100.0);
        assert_eq!(cp.shape, ChangeShape::SlowRamp);
    }

    #[test]
    fn baseline_jitter_detected() {
        let v = vec![80.0, 130.0, 70.0, 140.0, 75.0, 135.0, 65.0, 145.0];
        let cp = detect(&v, 105.0);
        assert_eq!(cp.shape, ChangeShape::BaselineJitter);
    }

    #[test]
    fn shapes_are_distinguishable() {
        let step = detect(&[100.0, 102.0, 98.0, 101.0, 300.0, 305.0, 295.0, 302.0], 100.0);
        let spike = detect(&[100.0, 102.0, 98.0, 101.0, 99.0, 800.0, 100.0, 97.0], 100.0);
        assert_ne!(step.shape, spike.shape);
    }

    #[test]
    fn short_series_is_stable() {
        assert_eq!(detect(&[1.0, 2.0], 1.0).shape, ChangeShape::Stable);
    }
}
