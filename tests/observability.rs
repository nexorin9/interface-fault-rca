//! task 19 验收：可观测性 —— 统计口径累加、日志逐行可解析、无凭据与日志正文、运行摘要齐全。
use interface_fault_rca::observability::{LogLine, RunLogger, RunSummary, SourceStats};

#[test]
fn log_line_is_single_json_object() {
    let l = LogLine::new("info", "esb_audit", "collect", "", "入库 100 行");
    let line = l.to_json_line();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["source"], "esb_audit");
    assert_eq!(v["event"], "collect");
    assert!(v.get("ts").is_some());
    assert!(!line.contains('\n'));
}

#[test]
fn stats_accumulate_correctly() {
    let mut total = SourceStats::new("all");
    let mut a = SourceStats::new("esb");
    a.attempted = 100; a.ingested = 95; a.dirty_lines = 3; a.too_long = 2; a.elapsed_ms = 50;
    let mut b = SourceStats::new("prom");
    b.attempted = 20; b.ingested = 20; b.dirty_lines = 0; b.too_long = 0; b.elapsed_ms = 10;
    total.add(&a);
    total.add(&b);
    assert_eq!(total.attempted, 120);
    assert_eq!(total.ingested, 115);
    assert_eq!(total.dirty_lines, 3);
    assert_eq!(total.elapsed_ms, 60);
}

#[test]
fn logger_exports_parseable_lines_without_credentials() {
    let logger = RunLogger::new();
    logger.info("esb", "collect", "入库 10 行");
    logger.warn("prom", "fetch", "SourceUnavailable", "连接失败");
    let text = logger.export();
    for line in text.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(v.get("ts").is_some());
    }
    assert_eq!(logger.len(), 2);
    assert!(!text.contains("password"));
}

#[test]
fn run_summary_is_complete_and_text_readable() {
    let mut s = RunSummary::new(1000, 2000);
    let mut st = SourceStats::new("esb_audit");
    st.attempted = 100; st.ingested = 95; st.fail_code = "".into();
    s.sources.push(st);
    s.candidate_count = 3;
    s.model_calls = 2;
    s.degraded = true;
    s.degrade_reason = "模型不可用".into();
    s.tickets_created = 1;
    let text = s.to_text();
    assert!(text.contains("候选 3 条"));
    assert!(text.contains("模型调用 2 次"));
    assert!(text.contains("建单 1 张"));
    assert!(text.contains("规则基线结论"));
    assert_eq!(s.total_ingested(), 95);
    let j: serde_json::Value = serde_json::from_str(&s.to_json()).unwrap();
    assert_eq!(j["candidate_count"], 3);
}

#[test]
fn summary_never_leaks_credentials() {
    let mut s = RunSummary::new(0, 100);
    let st = SourceStats::new("esb");
    s.sources.push(st);
    let text = s.to_text();
    assert!(!text.contains("SECRET"));
    assert!(!text.contains("Bearer"));
}
