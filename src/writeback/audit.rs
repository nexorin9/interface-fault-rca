//! 审计链只增不改：记录状态跃迁、操作人、时间、证据行 ID 摘要与工单号；
//! 提供按 interface_code 与窗口回查。

use crate::store::sqlite_index::IndexDb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub ts: String,
    pub actor: String,
    pub action: String,
    pub interface_code: String,
    pub evidence_digest: String,
    pub ticket_no: String,
    pub detail: String,
}

pub fn record(db: &IndexDb, ev: &AuditEvent) {
    let _ = db.append_audit(&ev.ts, &ev.actor, &ev.action, &ev.interface_code,
        &ev.evidence_digest, &ev.ticket_no, &ev.detail);
}

/// 证据行 ID 摘要（不落全文，只落排序后的哈希前缀，防篡改可回查数量）
pub fn evidence_digest(refs: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut sorted = refs.to_vec();
    sorted.sort();
    let raw = sorted.join("|");
    let h = hex::encode(Sha256::digest(raw.as_bytes()));
    format!("{}:{}", refs.len(), &h[..12])
}

/// 回查：审计链中与某接口相关的时间点序列
pub fn lookup_interface(db: &IndexDb, code: &str) -> Vec<(String, String)> {
    db.audit_rows().into_iter().filter(|(_, _, c)| c == code)
        .map(|(ts, action, _)| (ts, action)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_is_append_only() {
        let db = IndexDb::open_in_memory().unwrap();
        record(&db, &AuditEvent { ts: "t1".into(), actor: "op".into(), action: "confirm".into(),
            interface_code: "A".into(), evidence_digest: "d".into(), ticket_no: "".into(), detail: "".into() });
        record(&db, &AuditEvent { ts: "t2".into(), actor: "op".into(), action: "ticket".into(),
            interface_code: "A".into(), evidence_digest: "d".into(), ticket_no: "T-1".into(), detail: "".into() });
        assert_eq!(db.audit_count(), 2);
        let rows = db.audit_rows();
        assert_eq!(rows[0].1, "confirm");
        assert_eq!(rows[1].1, "ticket");
    }

    #[test]
    fn digest_is_order_independent_and_counted() {
        let a = evidence_digest(&["flow#a1".into(), "log#a2".into()]);
        let b = evidence_digest(&["log#a2".into(), "flow#a1".into()]);
        assert_eq!(a, b);
        assert!(a.starts_with("2:"));
    }

    #[test]
    fn lookup_by_interface() {
        let db = IndexDb::open_in_memory().unwrap();
        record(&db, &AuditEvent { ts: "t1".into(), actor: "op".into(), action: "rank".into(),
            interface_code: "A".into(), evidence_digest: "".into(), ticket_no: "".into(), detail: "".into() });
        record(&db, &AuditEvent { ts: "t2".into(), actor: "op".into(), action: "rank".into(),
            interface_code: "B".into(), evidence_digest: "".into(), ticket_no: "".into(), detail: "".into() });
        assert_eq!(lookup_interface(&db, "A").len(), 1);
    }
}
