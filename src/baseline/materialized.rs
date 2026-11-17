//! 物化指标表：每 interface_code 的 P95/P99/错误率/调用量，按刷新间隔周期刷新；
//! 幂等（同窗口重算同结果）；刷新失败保留上一版；抽样一致性校验发现过期。

use crate::store::sqlite_index::IndexDb;
use crate::baseline::aggregate::{aggregate_flows, Aggregate};
use crate::store::event_schema::EventRow;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct MetricSnapshot {
    pub p95: f64,
    pub p99: f64,
    pub error_rate: f64,
    pub call_count: i64,
}

pub fn compute_snapshot(rows: &[EventRow]) -> BTreeMap<String, MetricSnapshot> {
    aggregate_flows(rows).into_iter().map(|(k, a): (String, Aggregate)| (k, MetricSnapshot {
        p95: a.p95(), p99: a.p99(), error_rate: a.error_rate(), call_count: a.total as i64,
    })).collect()
}

/// 周期刷新（幂等）：同窗口重算得同结果；把结果写入物化表。
pub fn refresh(db: &IndexDb, window_start: i64, rows: &[EventRow], refreshed_at: &str) -> BTreeMap<String, MetricSnapshot> {
    let snap = compute_snapshot(rows);
    for (code, m) in &snap {
        let _ = db.upsert_materialized(code, window_start, m.p95, m.p99, m.error_rate, m.call_count, refreshed_at);
    }
    snap
}

/// 抽样一致性校验：物化值与事件表直算值不一致即返回差异列表，触发重算。
pub fn consistency_check(db: &IndexDb, window_start: i64, rows: &[EventRow], tolerance: f64) -> Vec<String> {
    let direct = compute_snapshot(rows);
    let mut diffs = vec![];
    for (code, m) in &direct {
        if let Some((p95, _, _, count, _)) = db.materialized_at(code, window_start) {
            if (p95 - m.p95).abs() > tolerance || count != m.call_count {
                diffs.push(format!("{code}: 物化 p95={p95:.1}/count={count} vs 直算 p95={:.1}/count={}", m.p95, m.call_count));
            }
        } else {
            diffs.push(format!("{code}: 物化表中缺失"));
        }
    }
    diffs
}

/// 新鲜度判定：refreshed_at 距 now 超过 interval 的倍数即视为滞后。
pub fn is_stale(refreshed_at_epoch: i64, now_epoch: i64, interval_sec: i64, factor: i64) -> bool {
    now_epoch - refreshed_at_epoch > interval_sec * factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn flow(code: &str, ts: i64, dur: i64) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), "esb_audit".into());
        tags.insert("event_kind".into(), "flow".into());
        tags.insert("interface_code".into(), code.into());
        let mut fields = BTreeMap::new();
        fields.insert("duration_ms".into(), dur.to_string());
        fields.insert("response_code".into(), "200".into());
        EventRow::new(tags, ts, fields, "flow#a1".into(), compute_tsid("esb_audit", ts, code, "", "flow")).unwrap()
    }

    #[test]
    fn refresh_then_consistency_ok() {
        let db = IndexDb::open_in_memory().unwrap();
        let rows = vec![flow("A", 1, 100), flow("A", 2, 200)];
        refresh(&db, 0, &rows, "t1");
        let diffs = consistency_check(&db, 0, &rows, 0.01);
        assert!(diffs.is_empty());
    }

    #[test]
    fn refresh_is_idempotent() {
        let db = IndexDb::open_in_memory().unwrap();
        let rows = vec![flow("A", 1, 100), flow("A", 2, 200)];
        let s1 = refresh(&db, 0, &rows, "t1");
        let s2 = refresh(&db, 0, &rows, "t2");
        assert_eq!(s1, s2);
    }

    #[test]
    fn consistency_detects_stale_value() {
        let db = IndexDb::open_in_memory().unwrap();
        let rows = vec![flow("A", 1, 100)];
        refresh(&db, 0, &rows, "t1");
        // 用不同数据做校验，应发现不一致
        let changed = vec![flow("A", 1, 900)];
        let diffs = consistency_check(&db, 0, &changed, 0.01);
        assert!(!diffs.is_empty());
    }

    #[test]
    fn staleness_threshold() {
        assert!(is_stale(100, 1000, 100, 2));   // 900 > 200
        assert!(!is_stale(900, 1000, 100, 2));  // 100 < 200
    }
}
