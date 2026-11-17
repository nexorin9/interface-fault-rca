//! 幂等键 __tsid = sha256(source|ts_ms|interface_code|service_name|event_kind) 十六进制串；
//! 证据行 ID __row_id 来源内单调分配（flow#a1923 / log#a2011 / metric#a0744）。

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub fn compute_tsid(
    source: &str,
    ts_ms: i64,
    interface_code: &str,
    service_name: &str,
    event_kind: &str,
) -> String {
    let mut h = Sha256::new();
    // 用长度前缀避免分隔符碰撞；空值仍参与哈希（空 != 缺失）
    for part in [source, &ts_ms.to_string(), interface_code, service_name, event_kind] {
        h.update((part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    hex::encode(h.finalize())
}

/// TSID 构造字段（供区分性测试）
#[derive(Debug, Clone)]
pub struct TsidParts<'a> {
    pub source: &'a str,
    pub ts_ms: i64,
    pub interface_code: &'a str,
    pub service_name: &'a str,
    pub event_kind: &'a str,
}

impl<'a> TsidParts<'a> {
    pub fn compute(&self) -> String {
        compute_tsid(self.source, self.ts_ms, self.interface_code, self.service_name, self.event_kind)
    }
}

/// 来源内单调的 __row_id 分配器，可持久化恢复。
pub const PREFIXES: [(&str, &str); 3] = [
    ("esb_audit", "flow"),
    ("app_log", "log"),
    ("prom", "metric"),
];

pub fn row_prefix(source: &str) -> &'static str {
    PREFIXES.iter().find(|(s, _)| *s == source).map(|(_, p)| *p).unwrap_or("row")
}

#[derive(Debug, Clone)]
pub struct RowIdAllocator {
    counters: Arc<Mutex<HashMap<String, u64>>>,
}

impl Default for RowIdAllocator {
    fn default() -> Self { Self::new() }
}

impl RowIdAllocator {
    pub fn new() -> Self { Self { counters: Arc::new(Mutex::new(HashMap::new())) } }

    /// 从持久化快照恢复（prefix -> 已分配最大值）
    pub fn from_snapshot(snapshot: &HashMap<String, u64>) -> Self {
        Self { counters: Arc::new(Mutex::new(snapshot.clone())) }
    }

    pub fn next(&self, source: &str) -> String {
        let prefix = row_prefix(source);
        self.next_for_prefix(prefix)
    }

    pub fn next_for_prefix(&self, prefix: &str) -> String {
        let mut m = self.counters.lock().unwrap();
        let e = m.entry(prefix.to_string()).or_insert(0);
        *e += 1;
        format!("{}#a{:04}", prefix, *e)
    }

    /// 预置已分配值（用于避免与持久化 ID 冲突）
    pub fn set_counter(&self, prefix: &str, value: u64) {
        if let Ok(mut m) = self.counters.lock() { m.insert(prefix.to_string(), value); }
    }

    pub fn snapshot(&self) -> HashMap<String, u64> {
        self.counters.lock().map(|m| m.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_input_same_tsid() {
        let p = TsidParts { source: "esb_audit", ts_ms: 123, interface_code: "A", service_name: "S", event_kind: "flow" };
        assert_eq!(p.compute(), p.compute());
        assert_eq!(p.compute().len(), 64);
    }

    #[test]
    fn each_field_change_changes_tsid() {
        let base = TsidParts { source: "esb_audit", ts_ms: 123, interface_code: "A", service_name: "S", event_kind: "flow" };
        let b = base.compute();
        assert_ne!(b, TsidParts { source: "app_log", ..base.clone() }.compute());
        assert_ne!(b, TsidParts { ts_ms: 124, ..base.clone() }.compute());
        assert_ne!(b, TsidParts { interface_code: "B", ..base.clone() }.compute());
        assert_ne!(b, TsidParts { service_name: "T", ..base.clone() }.compute());
        assert_ne!(b, TsidParts { event_kind: "log", ..base }.compute());
    }

    #[test]
    fn empty_string_and_missing_differ() {
        // 空串参与哈希，与任一非空格子值不同
        let empty = compute_tsid("esb_audit", 1, "", "S", "flow");
        let filled = compute_tsid("esb_audit", 1, "unbound", "S", "flow");
        assert_ne!(empty, filled);
    }

    #[test]
    fn row_ids_monotonic_and_stable_after_restore() {
        let a = RowIdAllocator::new();
        assert_eq!(a.next("esb_audit"), "flow#a0001");
        assert_eq!(a.next("esb_audit"), "flow#a0002");
        assert_eq!(a.next("app_log"), "log#a0001");
        assert_eq!(a.next("prom"), "metric#a0001");
        let snap = a.snapshot();
        let b = RowIdAllocator::from_snapshot(&snap);
        assert_eq!(b.next("esb_audit"), "flow#a0003");
    }
}
