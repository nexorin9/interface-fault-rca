//! 主干竖切：统一运行事件表 → 证据窗口 → 确定性基线 → 证据守门 → 判定报告。
//!
//! 这一条是产品的**唯一判定主路径**：`main.rs` 的 `diagnose` 子命令与
//! `tests/gold_regression.rs` 的金标准回归跑的是同一个函数，不存在「测试里另写一套」。
//!
//! 落点顺序（每一步都能单独核对）：
//!   1. 多路来源按 `source` 路由到各自版本化摄取管线（esb_audit / app_log / prom）；
//!   2. 统一成一张运行事件表（tags 六键 + ts_ms + fields + __row_id + __tsid）；
//!   3. 切「当前窗口」与「前 N 分钟基线窗口」两段；
//!   4. 确定性基线：按 interface_code 聚合 P95/P99/错误率/调用量，算劣化倍数并排序；
//!   5. 证据守门：每条候选的 evidence_refs 逐条复核，不合规整条丢弃并留原因码；
//!   6. 出报告（可落 JSON 产物），候选停在 candidate_ranked，不建单、不改配置。

use crate::ai::attribution::{orchestrate, AttributionConfig};
use crate::ai::conclusion::Conclusion;
use crate::ai::evidence_gate::{ConcreteEvidenceGate, GateContext};
use crate::ai::ports::{ModelGateway, PromptBuilder};
use crate::ai::prompt::EvidencePromptBuilder;
use crate::baseline::aggregate::{conclude, median, BaselineConclusion};
use crate::baseline::changepoint::detect;
use crate::baseline::log_template::group_templates;
use crate::baseline::rank::rank;
use crate::binding::interface_dict::Dictionary;
use crate::config::BaselineConfig;
use crate::error::Result;
use crate::ingest::pipeline::{default_registry, Pipeline, RawRecord};
use crate::store::event_schema::EventRow;
use crate::store::tsid::RowIdAllocator;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

/// 判定入参
#[derive(Debug, Clone)]
pub struct DiagnoseOptions {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    /// 基线窗口长度（分钟）：取当前窗口之前 N 分钟作基线（spec.md §5.1）
    pub baseline_minutes: i64,
    pub baseline: BaselineConfig,
    pub dict: Dictionary,
    pub use_model: bool,
    pub request_id: String,
}

/// 单路来源的采集统计
#[derive(Debug, Clone, Serialize)]
pub struct SourceStat {
    pub source: String,
    pub event_kind: String,
    pub attempted: u64,
    pub ingested: u64,
    pub rejected: u64,
    pub dirty_lines: u64,
    /// 被拒绝的来源行（最多 5 条）：「接口码: 原因码」，可回查为什么没进事件表
    pub rejected_examples: Vec<String>,
}

/// 证据行（可回查的原文锚点）
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceView {
    pub row_id: String,
    pub ts_ms: i64,
    pub source: String,
    pub event_kind: String,
    pub summary: String,
}

/// 候选（判定 + 依据）
#[derive(Debug, Clone, Serialize)]
pub struct CandidateView {
    pub interface_code: String,
    pub service_name: String,
    pub verdict: String,
    pub confidence: f64,
    pub degrade_ratio: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub baseline_p95_ms: f64,
    pub error_rate: f64,
    pub call_count: u64,
    pub baseline_samples: usize,
    pub change_shape: String,
    /// 综合得分（延迟绝对值 + 劣化倍数 + 错误率，权重见 config）
    pub score: f64,
    pub score_breakdown: ScoreView,
    pub reasoning_summary: String,
    pub next_check: String,
    pub evidence_refs: Vec<String>,
    pub evidence: Vec<EvidenceView>,
    pub degraded: bool,
    pub request_id: String,
}

/// 排序分项（可核对「为什么它排第一」）
#[derive(Debug, Clone, Serialize)]
pub struct ScoreView {
    pub latency_component: f64,
    pub ratio_component: f64,
    pub error_component: f64,
    pub host_component: f64,
}

/// 被丢弃的候选与原因码（可回查「为什么这条没给我」）
#[derive(Debug, Clone, Serialize)]
pub struct DroppedView {
    pub interface_code: String,
    pub service_name: String,
    pub verdict: String,
    pub reason_code: String,
    pub reason: String,
    pub evidence_refs: Vec<String>,
}

/// 日志模板分组（跨源证据：同一类错误出现了几次、首次何时）
#[derive(Debug, Clone, Serialize)]
pub struct LogTemplateView {
    pub template: String,
    pub count: usize,
    pub first_ts_ms: i64,
    pub sample_row_ids: Vec<String>,
}

/// 判定报告（可落成 JSON 产物）
#[derive(Debug, Clone, Serialize)]
pub struct DiagnoseReport {
    pub tool: String,
    pub report_version: String,
    pub generated_at: String,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub baseline_start_ms: i64,
    pub baseline_minutes: i64,
    pub dict_version: String,
    pub degraded: bool,
    pub degrade_reason: String,
    pub sources: Vec<SourceStat>,
    pub candidates: Vec<CandidateView>,
    pub dropped: Vec<DroppedView>,
    pub log_templates: Vec<LogTemplateView>,
    pub total_rows: usize,
    pub current_rows: usize,
    pub baseline_rows: usize,
}

/// 判定结果：报告（给人看 / 落产物）+ 归因编排结果（给台账写入用）
#[derive(Debug, Clone)]
pub struct DiagnoseOutcome {
    pub report: DiagnoseReport,
    pub attribution: crate::ai::attribution::AttributionResult,
}

/// 模型不可达时的占位网关：永远不可用，编排层据此走确定性基线路径并标注降级。
pub struct NoModelGateway;

impl ModelGateway for NoModelGateway {
    fn available(&self) -> bool { false }
    fn call(&self, _req: &crate::ai::ports::ModelRequest) -> Result<crate::ai::ports::ModelResponse> {
        Err(crate::error::AppError::code_only(crate::error::ErrorCode::ModelUnavailable))
    }
}

/// 按来源路由到各自管线（每路一条具名带版本的摄取管线，见 config/pipelines.toml）
fn route_and_ingest(records: &[RawRecord], dict: &Dictionary) -> (Vec<EventRow>, Vec<SourceStat>) {
    let registry = default_registry();
    let mut by_source: BTreeMap<String, Vec<RawRecord>> = BTreeMap::new();
    for r in records {
        by_source.entry(r.source.clone()).or_default().push(r.clone());
    }
    let mut rows = vec![];
    let mut stats = vec![];
    for (source, rs) in by_source {
        let pipeline_name = match source.as_str() {
            "prom" => "prom",
            "app_log" => "app_log",
            _ => "esb_audit",
        };
        let def = match registry.get(pipeline_name, "v1") {
            Ok(d) => d,
            Err(_) => {
                stats.push(SourceStat {
                    source, event_kind: "unknown".into(), attempted: rs.len() as u64,
                    ingested: 0, rejected: rs.len() as u64, dirty_lines: 0,
                    rejected_examples: vec![],
                });
                continue;
            }
        };
        let pipeline = Pipeline::new(def, Arc::new(dict.clone()), Arc::new(RowIdAllocator::new()));
        match pipeline.execute(&rs) {
            Ok(outcome) => {
                stats.push(SourceStat {
                    source: source.clone(),
                    event_kind: pipeline.def.event_kind.clone(),
                    attempted: rs.len() as u64,
                    ingested: outcome.rows.len() as u64,
                    rejected: outcome.rejected.len() as u64,
                    dirty_lines: outcome.dirty_lines as u64,
                    rejected_examples: outcome.rejected.iter().take(5).cloned().collect(),
                });
                rows.extend(outcome.rows);
            }
            Err(_) => {
                stats.push(SourceStat {
                    source, event_kind: pipeline.def.event_kind.clone(), attempted: rs.len() as u64,
                    ingested: 0, rejected: rs.len() as u64, dirty_lines: 0,
                    rejected_examples: vec![],
                });
            }
        }
    }
    (rows, stats)
}

/// 主判定：跑完整条主路径并出报告。
/// `gateway` 传 None 时走确定性基线路径（并在报告里显式标注 degraded）。
pub fn diagnose<G: ModelGateway + ?Sized>(
    opts: &DiagnoseOptions,
    records: &[RawRecord],
    gateway: Option<&G>,
) -> Result<DiagnoseOutcome> {
    let (all_rows, sources) = route_and_ingest(records, &opts.dict);

    let baseline_start = opts.window_start_ms - opts.baseline_minutes * 60_000;
    let baseline_rows: Vec<EventRow> = all_rows.iter()
        .filter(|r| r.ts_ms >= baseline_start && r.ts_ms < opts.window_start_ms)
        .cloned().collect();
    let current_rows: Vec<EventRow> = all_rows.iter()
        .filter(|r| r.ts_ms >= opts.window_start_ms && r.ts_ms <= opts.window_end_ms)
        .cloned().collect();

    // 确定性基线：当前窗口聚合 ÷ 前 N 分钟基线中位数 → 劣化倍数 → 排序
    let baselines = conclude(&current_rows, &baseline_rows, &opts.baseline);
    let by_code: BTreeMap<String, BaselineConclusion> = baselines.iter()
        .map(|b| (b.interface_code.clone(), b.clone())).collect();
    // 证据窗口 prompt 构造器（模型路径用；规则基线路径不用）
    let builder = EvidencePromptBuilder::new(current_rows.clone(), baselines.clone(), opts.request_id.clone());
    let ranked = rank(baselines, &opts.baseline);
    let scores: BTreeMap<String, (f64, ScoreView)> = ranked.iter().map(|rc| {
        (rc.conclusion.interface_code.clone(), (rc.score, ScoreView {
            latency_component: rc.score_breakdown.latency_component,
            ratio_component: rc.score_breakdown.ratio_component,
            error_component: rc.score_breakdown.error_component,
            host_component: rc.score_breakdown.host_component,
        }))
    }).collect();
    let conclusions: Vec<Conclusion> = ranked.into_iter()
        .map(|rc| Conclusion::from_baseline(&rc.conclusion, &opts.request_id))
        .collect();

    // 证据守门：白名单 = 本窗口行 ID；全局集合用于区分「窗口外」与「不存在」
    let window_ids: HashSet<String> = current_rows.iter().map(|r| r.row_id.clone()).collect();
    let all_ids: HashSet<String> = all_rows.iter().map(|r| r.row_id.clone()).collect();
    let unbound_ids: HashSet<String> = current_rows.iter()
        .filter(|r| r.is_unbound()).map(|r| r.row_id.clone()).collect();
    let gate = ConcreteEvidenceGate { ctx: GateContext {
        window_ids, all_ids, unbound_ids,
        min_refs: opts.baseline.min_evidence_refs,
    } };

    let no_model = NoModelGateway;
    let use_model = opts.use_model && gateway.is_some();
    let result = match gateway {
        Some(g) => orchestrate(
            &AttributionConfig { use_model, request_id: opts.request_id.clone() },
            &conclusions, g, &builder, &gate),
        None => orchestrate(
            &AttributionConfig { use_model, request_id: opts.request_id.clone() },
            &conclusions, &no_model, &builder, &gate),
    };

    // 证据原文锚点
    let row_by_id: BTreeMap<String, &EventRow> = current_rows.iter().map(|r| (r.row_id.clone(), r)).collect();
    // 变更形态：把「基线段 + 当前段」拼成一条序列送进检测器，
    // 否则只看当前段的话，阶梯式劣化会退化成「稳定」（窗口内本来就一直是劣化后的水位）。
    let shape_of = |code: &str| -> String {
        let mut series: Vec<(i64, f64)> = baseline_rows.iter()
            .chain(current_rows.iter())
            .filter(|r| r.interface_code() == code && r.kind() == "flow")
            .filter_map(|r| r.fields.get("duration_ms").and_then(|s| s.parse::<f64>().ok()).map(|d| (r.ts_ms, d)))
            .collect();
        series.sort_by_key(|(t, _)| *t);
        let vals: Vec<f64> = series.iter().map(|(_, d)| *d).collect();
        if vals.is_empty() { return String::new(); }
        let base_vals: Vec<f64> = baseline_rows.iter()
            .filter(|r| r.interface_code() == code && r.kind() == "flow")
            .filter_map(|r| r.fields.get("duration_ms").and_then(|s| s.parse::<f64>().ok()))
            .collect();
        detect(&vals, median(&base_vals)).shape.as_str().to_string()
    };

    let candidates: Vec<CandidateView> = result.candidates.iter().map(|c| {
        let b = by_code.get(&c.interface_code);
        CandidateView {
            interface_code: c.interface_code.clone(),
            service_name: c.service_name.clone(),
            verdict: c.verdict.as_str().to_string(),
            confidence: c.confidence,
            degrade_ratio: c.baseline_stats.degrade_ratio,
            p95_ms: c.baseline_stats.p95_ms,
            p99_ms: b.map(|x| x.p99_ms).unwrap_or(0.0),
            baseline_p95_ms: b.map(|x| x.baseline_p95_ms).unwrap_or(0.0),
            error_rate: c.baseline_stats.error_rate,
            call_count: b.map(|x| x.call_count).unwrap_or(0),
            baseline_samples: c.baseline_stats.baseline_samples,
            change_shape: shape_of(&c.interface_code),
            score: scores.get(&c.interface_code).map(|x| x.0).unwrap_or(0.0),
            score_breakdown: scores.get(&c.interface_code).map(|x| x.1.clone())
                .unwrap_or(ScoreView { latency_component: 0.0, ratio_component: 0.0, error_component: 0.0, host_component: 0.0 }),
            reasoning_summary: c.reasoning_summary.clone(),
            next_check: c.next_check.clone(),
            evidence_refs: c.evidence_refs.clone(),
            evidence: c.evidence_refs.iter().filter_map(|id| row_by_id.get(id).map(|r| EvidenceView {
                row_id: r.row_id.clone(),
                ts_ms: r.ts_ms,
                source: r.source().to_string(),
                event_kind: r.kind().to_string(),
                summary: r.evidence_summary(),
            })).collect(),
            degraded: c.degraded,
            request_id: c.request_id.clone(),
        }
    }).collect();

    let dropped: Vec<DroppedView> = result.dropped.iter().map(|d| DroppedView {
        interface_code: d.interface_code.clone(),
        service_name: d.service_name.clone(),
        verdict: d.verdict.clone(),
        reason_code: d.reason_code.clone(),
        reason: d.reason.clone(),
        evidence_refs: d.evidence_refs.clone(),
    }).collect();

    let log_templates: Vec<LogTemplateView> = group_templates(&current_rows).into_iter().map(|g| LogTemplateView {
        template: g.template,
        count: g.count,
        first_ts_ms: g.first_ts_ms,
        sample_row_ids: g.sample_row_ids,
    }).collect();

    let report = DiagnoseReport {
        tool: "interface-fault-rca".to_string(),
        report_version: "v1".to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        window_start_ms: opts.window_start_ms,
        window_end_ms: opts.window_end_ms,
        baseline_start_ms: baseline_start,
        baseline_minutes: opts.baseline_minutes,
        dict_version: opts.dict.version.clone(),
        degraded: result.degraded,
        degrade_reason: result.degrade_reason.clone(),
        sources,
        candidates,
        dropped,
        log_templates,
        total_rows: all_rows.len(),
        current_rows: current_rows.len(),
        baseline_rows: baseline_rows.len(),
    };
    Ok(DiagnoseOutcome { report, attribution: result })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::interface_dict::minimal_dict;

    fn flow(code: &str, ts: i64, dur: i64, rc: i64) -> RawRecord {
        let mut c = BTreeMap::new();
        c.insert("interface_code".into(), code.into());
        c.insert("response_code".into(), rc.to_string());
        c.insert("duration_ms".into(), dur.to_string());
        RawRecord { source: "esb_audit".into(), columns: c, ts_ms: ts }
    }

    fn opts(ws: i64, we: i64) -> DiagnoseOptions {
        DiagnoseOptions {
            window_start_ms: ws, window_end_ms: we,
            baseline_minutes: 30,
            baseline: BaselineConfig::default(),
            dict: minimal_dict(),
            use_model: false,
            request_id: "t".into(),
        }
    }

    #[test]
    fn degraded_interface_becomes_candidate_with_evidence() {
        let mut recs = vec![];
        for i in 0..10 { recs.push(flow("ESB-ORDER-CREATE", -600_000 + i * 60_000, 100, 200)); }
        for i in 0..10 { recs.push(flow("ESB-ORDER-CREATE", 0 + i * 60_000, 350, 200)); }
        let r = diagnose(&opts(0, 600_000), &recs, None::<&NoModelGateway>).unwrap().report;
        assert_eq!(r.candidates.len(), 1);
        assert_eq!(r.candidates[0].verdict, "接口自身劣化");
        assert!(!r.candidates[0].evidence_refs.is_empty());
        assert!(r.candidates[0].evidence_refs.iter().all(|id| id.starts_with("flow#")));
    }

    #[test]
    fn normal_interface_is_not_a_candidate() {
        let mut recs = vec![];
        for i in 0..10 { recs.push(flow("ESB-ORDER-CREATE", -600_000 + i * 60_000, 100, 200)); }
        for i in 0..10 { recs.push(flow("ESB-ORDER-CREATE", 0 + i * 60_000, 102, 200)); }
        let r = diagnose(&opts(0, 600_000), &recs, None::<&NoModelGateway>).unwrap().report;
        assert!(r.candidates.is_empty());
        assert!(r.dropped.iter().any(|d| d.reason_code == "VERDICT_INSUFFICIENT"));
    }

    #[test]
    fn unbound_interface_is_dropped_with_reason_code() {
        let mut recs = vec![];
        for i in 0..10 { recs.push(flow("NOT-IN-DICT", -600_000 + i * 60_000, 100, 200)); }
        for i in 0..10 { recs.push(flow("NOT-IN-DICT", 0 + i * 60_000, 350, 200)); }
        let r = diagnose(&opts(0, 600_000), &recs, None::<&NoModelGateway>).unwrap().report;
        assert!(r.candidates.is_empty());
        assert!(r.dropped.iter().any(|d| d.reason_code == "UNBOUND_REF"));
    }
}
