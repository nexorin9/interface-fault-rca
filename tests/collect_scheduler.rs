//! task 22 验收：增量采集调度 —— 续跑不重复入库、单源失败其余照常、失败源退避且游标不动。
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::scheduler::{CursorState, IncrementalScheduler};

#[test]
fn resume_from_cursor_does_not_reingest() {
    let mut s = IncrementalScheduler::new();
    let sources = ["esb", "prom"];
    let r1 = s.run_round(&sources, |_src, cur| {
        let start = cur.last_offset;
        Ok((10, CursorState { last_offset: start + 10, ..Default::default() }))
    });
    assert!(r1.all_ok());
    assert_eq!(s.cursor("esb").last_offset, 10);
    // 第二轮从游标继续，模拟重复数据只有 0 条新增
    let r2 = s.run_round(&sources, |_src, cur| {
        assert!(cur.last_offset >= 10);
        Ok((0, cur.clone()))
    });
    assert_eq!(r2.sources[0].ingested, 0);
}

#[test]
fn single_source_failure_isolated() {
    let mut s = IncrementalScheduler::new();
    let r = s.run_round(&["esb", "prom", "app_log"], |src, cur| {
        if src == "esb" { Err(ErrorCode::SourceUnavailable) }
        else { Ok((5, CursorState { last_offset: cur.last_offset + 5, ..Default::default() })) }
    });
    assert!(!r.all_ok());
    assert_eq!(r.ok_sources(), vec!["prom", "app_log"]);
    let esb = r.sources.iter().find(|x| x.source == "esb").unwrap();
    assert!(!esb.ok);
    assert_eq!(esb.fail_code, "SourceUnavailable");
    // 失败源游标不动
    assert_eq!(s.cursor("esb").last_offset, 0);
    // 其他源推进
    assert_eq!(s.cursor("prom").last_offset, 5);
}

#[test]
fn failed_source_backs_off_exponentially() {
    let mut s = IncrementalScheduler::new();
    let mut backs = vec![];
    for _ in 0..3 {
        let r = s.run_round(&["esb"], |_, _| Err(ErrorCode::SourceUnavailable));
        backs.push(r.sources[0].backoff_ms);
    }
    assert_eq!(backs, vec![100, 200, 400]);
    assert_eq!(s.failure_count("esb"), 3);
}

#[test]
fn successful_round_clears_failure_count() {
    let mut s = IncrementalScheduler::new();
    s.run_round(&["esb"], |_, _| Err(ErrorCode::SourceUnavailable));
    assert_eq!(s.failure_count("esb"), 1);
    s.run_round(&["esb"], |_, cur| Ok((1, cur.clone())));
    assert_eq!(s.failure_count("esb"), 0);
}
