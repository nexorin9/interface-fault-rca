//! 证据行 ID 稳定分配与跨重启可回查：来源内单调、持久化计数器、ID→行反查、
//! 引用必须命中本窗口集合（硬约束）。

use crate::store::event_schema::EventRow;
use crate::store::sqlite_index::IndexDb;
use crate::store::tsid::{row_prefix, RowIdAllocator};
use std::collections::{BTreeMap, HashMap, HashSet};

/// 从 SQLite 恢复分配器 → 单调不重号。
pub fn restore_allocator(db: &IndexDb) -> RowIdAllocator {
    let mut snap: HashMap<String, u64> = HashMap::new();
    for p in ["flow", "log", "metric"] {
        let v = db.load_rowid_counter(p);
        if v > 0 { snap.insert(p.to_string(), v); }
    }
    RowIdAllocator::from_snapshot(&snap)
}

/// 分配后持久化计数器。
pub fn persist_counter(db: &IndexDb, allocator: &RowIdAllocator) {
    for (p, v) in allocator.snapshot() {
        let _ = db.save_rowid_counter(&p, v);
    }
}

/// ID → 行反查索引（含来源与字段一致性校验）
pub struct RowIdIndex {
    map: BTreeMap<String, EventRow>,
}

impl RowIdIndex {
    pub fn new(rows: &[EventRow]) -> Self {
        let mut map = BTreeMap::new();
        for r in rows { map.insert(r.row_id.clone(), r.clone()); }
        Self { map }
    }

    pub fn lookup(&self, id: &str) -> Option<&EventRow> { self.map.get(id) }

    #[allow(clippy::too_many_arguments)]
    pub fn lookup_consistent(&self, id: &str, ts_ms: i64, interface_code: &str) -> bool {
        self.map.get(id).map(|r| r.ts_ms == ts_ms && r.interface_code() == interface_code).unwrap_or(false)
    }

    /// 本窗口合法 ID 白名单
    pub fn allowed_ids(&self) -> HashSet<String> {
        self.map.keys().cloned().collect()
    }
}

/// 引用合法性判定：不存在 / 窗口外 / 未映射，三条路径区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefViolation { Nonexistent, OutsideWindow, Unbound }

pub fn check_ref(id: &str, window: &RowIdIndex, unbound_ids: &HashSet<String>, all_ids: &HashSet<String>) -> Option<RefViolation> {
    if !window.allowed_ids().contains(id) {
        if all_ids.contains(id) { Some(RefViolation::OutsideWindow) }
        else { Some(RefViolation::Nonexistent) }
    } else if unbound_ids.contains(id) {
        Some(RefViolation::Unbound)
    } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn row(source: &str, kind: &str, code: &str, ts: i64, id: &str, unbound: bool) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), source.into());
        tags.insert("event_kind".into(), kind.into());
        tags.insert("interface_code".into(), code.into());
        let mut fields = BTreeMap::new();
        fields.insert("unbound".into(), unbound.to_string());
        EventRow::new(tags, ts, fields, id.into(), compute_tsid(source, ts, code, "", kind)).unwrap()
    }

    #[test]
    fn allocator_restores_without_collision() {
        let db = IndexDb::open_in_memory().unwrap();
        let a = restore_allocator(&db);
        assert_eq!(a.next("esb_audit"), "flow#a0001");
        assert_eq!(row_prefix("prom"), "metric");
    }

    #[test]
    fn lookup_consistency() {
        let r = row("esb_audit", "flow", "A", 100, "flow#a1", false);
        let idx = RowIdIndex::new(&[r]);
        assert!(idx.lookup_consistent("flow#a1", 100, "A"));
        assert!(!idx.lookup_consistent("flow#a1", 999, "A"));
    }

    #[test]
    fn three_ref_violations_distinguished() {
        let in_window = row("esb_audit", "flow", "A", 100, "flow#a1", false);
        let unbound = row("esb_audit", "flow", "B", 100, "flow#a2", true);
        let outside = row("esb_audit", "flow", "A", 999, "flow#a3", false);
        let window = RowIdIndex::new(&[in_window, unbound]);
        let unbound_ids: HashSet<_> = ["flow#a2".to_string()].into_iter().collect();
        let all_ids: HashSet<_> = ["flow#a1".to_string(), "flow#a2".to_string(), "flow#a3".to_string()].into_iter().collect();
        assert_eq!(check_ref("flow#a1", &window, &unbound_ids, &all_ids), None);
        assert_eq!(check_ref("flow#a2", &window, &unbound_ids, &all_ids), Some(RefViolation::Unbound));
        assert_eq!(check_ref("flow#a3", &window, &unbound_ids, &all_ids), Some(RefViolation::OutsideWindow));
        assert_eq!(check_ref("flow#a999", &window, &unbound_ids, &all_ids), Some(RefViolation::Nonexistent));
    }
}
