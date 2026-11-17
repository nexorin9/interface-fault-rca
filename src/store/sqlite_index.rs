//! SQLite 本地索引：collection_cursor / materialized_metric / attribution_ledger / audit_chain
//! 四张表；提供幂等 upsert 与只增不改的追加接口。台账状态机在此落点。

use crate::error::{AppError, ErrorCode, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 状态机：collected → aggregated → evidence_window_open → candidate_ranked
///        → awaiting_confirmation → ticket_created；awaiting_confirmation → voided
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LedgerStatus {
    Collected,
    Aggregated,
    EvidenceWindowOpen,
    CandidateRanked,
    AwaitingConfirmation,
    TicketCreated,
    Voided,
}

impl LedgerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LedgerStatus::Collected => "collected",
            LedgerStatus::Aggregated => "aggregated",
            LedgerStatus::EvidenceWindowOpen => "evidence_window_open",
            LedgerStatus::CandidateRanked => "candidate_ranked",
            LedgerStatus::AwaitingConfirmation => "awaiting_confirmation",
            LedgerStatus::TicketCreated => "ticket_created",
            LedgerStatus::Voided => "voided",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "collected" => Some(Self::Collected),
            "aggregated" => Some(Self::Aggregated),
            "evidence_window_open" => Some(Self::EvidenceWindowOpen),
            "candidate_ranked" => Some(Self::CandidateRanked),
            "awaiting_confirmation" => Some(Self::AwaitingConfirmation),
            "ticket_created" => Some(Self::TicketCreated),
            "voided" => Some(Self::Voided),
            _ => None,
        }
    }

    /// 合法跃迁判定。模型 / 降级 / 异常路径最多停在 CandidateRanked。
    pub fn can_transition(self, next: Self) -> bool {
        use LedgerStatus::*;
        matches!((self, next),
            (Collected, Aggregated)
            | (Aggregated, EvidenceWindowOpen)
            | (EvidenceWindowOpen, CandidateRanked)
            | (CandidateRanked, AwaitingConfirmation)
            | (AwaitingConfirmation, TicketCreated)
            | (AwaitingConfirmation, Voided)
        ) || self == next
    }
}

#[derive(Debug, Clone)]
pub struct LedgerEntry {
    pub id: i64,
    pub interface_code: String,
    pub service_name: String,
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub verdict: String,
    pub confidence: f64,
    pub evidence_refs: Vec<String>,
    pub status: LedgerStatus,
    pub request_id: String,
    pub dict_version: String,
    pub ticket_no: String,
    pub confirmed_by: String,
    pub confirmed_at: String,
    pub reason: String,
    pub degraded: bool,
    pub created_at: String,
}

pub struct IndexDb {
    conn: Connection,
}

impl IndexDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AppError::new(ErrorCode::SourceUnavailable, format!("建立数据目录失败: {e}"))
            })?;
        }
        let conn = Connection::open(path).map_err(|e| {
            AppError::new(ErrorCode::SourceUnavailable, format!("打开 SQLite 失败: {e}"))
        })?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|e| {
            AppError::new(ErrorCode::SourceUnavailable, format!("打开内存库失败: {e}"))
        })?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS collection_cursor (
                source TEXT PRIMARY KEY, last_offset INTEGER NOT NULL DEFAULT 0,
                last_size INTEGER NOT NULL DEFAULT 0, last_mtime INTEGER NOT NULL DEFAULT 0,
                updated_at TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS materialized_metric (
                interface_code TEXT NOT NULL, window_start INTEGER NOT NULL,
                p95_ms REAL NOT NULL DEFAULT 0, p99_ms REAL NOT NULL DEFAULT 0,
                error_rate REAL NOT NULL DEFAULT 0, call_count INTEGER NOT NULL DEFAULT 0,
                refreshed_at TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (interface_code, window_start)
             );
             CREATE TABLE IF NOT EXISTS attribution_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                interface_code TEXT NOT NULL, service_name TEXT NOT NULL,
                window_start INTEGER NOT NULL, window_end INTEGER NOT NULL,
                verdict TEXT NOT NULL, confidence REAL NOT NULL,
                evidence_refs TEXT NOT NULL, status TEXT NOT NULL,
                request_id TEXT NOT NULL DEFAULT '', dict_version TEXT NOT NULL DEFAULT '',
                ticket_no TEXT NOT NULL DEFAULT '', confirmed_by TEXT NOT NULL DEFAULT '',
                confirmed_at TEXT NOT NULL DEFAULT '', reason TEXT NOT NULL DEFAULT '',
                degraded INTEGER NOT NULL DEFAULT 0, created_at TEXT NOT NULL DEFAULT '',
                dedup_key TEXT NOT NULL DEFAULT ''
             );
             CREATE UNIQUE INDEX IF NOT EXISTS idx_ledger_dedup ON attribution_ledger(dedup_key)
                WHERE dedup_key <> '';
             CREATE TABLE IF NOT EXISTS audit_chain (
                seq INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL,
                interface_code TEXT NOT NULL, evidence_digest TEXT NOT NULL DEFAULT '',
                ticket_no TEXT NOT NULL DEFAULT '', detail TEXT NOT NULL DEFAULT ''
             );
             CREATE TABLE IF NOT EXISTS seen_tsid (
                tsid TEXT PRIMARY KEY, source TEXT NOT NULL, created_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS rowid_counter (
                prefix TEXT PRIMARY KEY, last_value INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS model_call_audit (
                request_id TEXT PRIMARY KEY, model TEXT NOT NULL, host TEXT NOT NULL,
                input_hash TEXT NOT NULL, token_count INTEGER NOT NULL, elapsed_ms INTEGER NOT NULL,
                response_hash TEXT NOT NULL, degraded INTEGER NOT NULL, retries INTEGER NOT NULL,
                created_at TEXT NOT NULL
             );",
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("建表失败: {e}")))?;
        Ok(())
    }

    // ---- 采集游标 ----
    pub fn upsert_cursor(&self, source: &str, offset: i64, size: i64, mtime: i64, now: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO collection_cursor(source,last_offset,last_size,last_mtime,updated_at)
             VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(source) DO UPDATE SET last_offset=excluded.last_offset,
               last_size=excluded.last_size, last_mtime=excluded.last_mtime, updated_at=excluded.updated_at",
            params![source, offset, size, mtime, now],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("游标写入失败: {e}")))?;
        Ok(())
    }

    pub fn cursor_of(&self, source: &str) -> Option<(i64, i64, i64)> {
        self.conn.query_row(
            "SELECT last_offset,last_size,last_mtime FROM collection_cursor WHERE source=?1",
            params![source], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).optional().ok().flatten()
    }

    // ---- 幂等键 ----
    /// 若 tsid 未见过则插入并返回 true；已存在返回 false（入库前置检查）。
    pub fn mark_tsid_seen(&self, tsid: &str, source: &str, now: &str) -> Result<bool> {
        let n = self.conn.execute(
            "INSERT OR IGNORE INTO seen_tsid(tsid,source,created_at) VALUES(?1,?2,?3)",
            params![tsid, source, now],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("幂等键写入失败: {e}")))?;
        Ok(n == 1)
    }

    pub fn tsid_seen(&self, tsid: &str) -> bool {
        self.conn.query_row("SELECT 1 FROM seen_tsid WHERE tsid=?1", params![tsid], |_| Ok(()))
            .optional().ok().flatten().is_some()
    }

    pub fn distinct_tsid_count(&self) -> i64 {
        self.conn.query_row("SELECT COUNT(*) FROM seen_tsid", [], |r| r.get(0)).unwrap_or(0)
    }

    // ---- rowid 计数器 ----
    pub fn load_rowid_counter(&self, prefix: &str) -> u64 {
        self.conn.query_row("SELECT last_value FROM rowid_counter WHERE prefix=?1", params![prefix], |r| r.get(0))
            .optional().ok().flatten().unwrap_or(0)
    }
    pub fn save_rowid_counter(&self, prefix: &str, value: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO rowid_counter(prefix,last_value) VALUES(?1,?2)
             ON CONFLICT(prefix) DO UPDATE SET last_value=excluded.last_value",
            params![prefix, value as i64],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("rowid 计数写入失败: {e}")))?;
        Ok(())
    }

    // ---- 物化指标 ----
    #[allow(clippy::too_many_arguments)]
    pub fn upsert_materialized(&self, code: &str, window_start: i64, p95: f64, p99: f64, err: f64, count: i64, refreshed: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO materialized_metric(interface_code,window_start,p95_ms,p99_ms,error_rate,call_count,refreshed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(interface_code,window_start) DO UPDATE SET p95_ms=excluded.p95_ms,
               p99_ms=excluded.p99_ms, error_rate=excluded.error_rate, call_count=excluded.call_count,
               refreshed_at=excluded.refreshed_at",
            params![code, window_start, p95, p99, err, count, refreshed],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("物化写入失败: {e}")))?;
        Ok(())
    }

    pub fn materialized_at(&self, code: &str, window_start: i64) -> Option<(f64, f64, f64, i64, String)> {
        self.conn.query_row(
            "SELECT p95_ms,p99_ms,error_rate,call_count,refreshed_at FROM materialized_metric
             WHERE interface_code=?1 AND window_start=?2",
            params![code, window_start],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        ).optional().ok().flatten()
    }

    // ---- 台账 ----
    #[allow(clippy::too_many_arguments)]
    pub fn insert_candidate(
        &self, code: &str, service: &str, ws: i64, we: i64, verdict: &str, confidence: f64,
        refs: &[String], request_id: &str, dict_version: &str, degraded: bool,
        reason: &str, dedup_key: &str, now: &str,
    ) -> Result<i64> {
        let refs_json = serde_json::to_string(refs).unwrap_or_else(|_| "[]".into());
        let affected = self.conn.execute(
            "INSERT OR IGNORE INTO attribution_ledger
             (interface_code,service_name,window_start,window_end,verdict,confidence,evidence_refs,
              status,request_id,dict_version,degraded,reason,dedup_key,created_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![code, service, ws, we, verdict, confidence, refs_json,
                LedgerStatus::CandidateRanked.as_str(), request_id, dict_version,
                if degraded {1} else {0}, reason, dedup_key, now],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("台账写入失败: {e}")))?;
        // 去重键命中时 affected=0：返回 0 表示「未新增」，不能拿上一次的 rowid 充数
        Ok(if affected > 0 { self.conn.last_insert_rowid() } else { 0 })
    }

    /// 推进状态；非法跃迁返回错误，绝不放行。
    pub fn transition(&self, id: i64, next: LedgerStatus) -> Result<()> {
        let cur: String = self.conn.query_row(
            "SELECT status FROM attribution_ledger WHERE id=?1", params![id], |r| r.get(0),
        ).map_err(|_| AppError::new(ErrorCode::EvidenceMissing, format!("台账行不存在: {id}")))?;
        let cur = LedgerStatus::parse(&cur).ok_or_else(|| AppError::new(ErrorCode::EvidenceMissing, "状态非法"))?;
        if !cur.can_transition(next) {
            return Err(AppError::new(ErrorCode::EvidenceMissing,
                format!("非法状态跃迁 {} → {}", cur.as_str(), next.as_str())));
        }
        self.conn.execute("UPDATE attribution_ledger SET status=?2 WHERE id=?1", params![id, next.as_str()])
            .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("状态更新失败: {e}")))?;
        Ok(())
    }

    pub fn confirm(&self, id: i64, actor: &str, at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE attribution_ledger SET status=?2, confirmed_by=?3, confirmed_at=?4
             WHERE id=?1 AND status=?5",
            params![id, LedgerStatus::AwaitingConfirmation.as_str(), actor, at, LedgerStatus::CandidateRanked.as_str()],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("确认失败: {e}")))?;
        Ok(())
    }

    pub fn writeback_ticket(&self, id: i64, ticket_no: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE attribution_ledger SET status=?2, ticket_no=?3
             WHERE id=?1 AND status=?4",
            params![id, LedgerStatus::TicketCreated.as_str(), ticket_no, LedgerStatus::AwaitingConfirmation.as_str()],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("工单回写失败: {e}")))?;
        Ok(())
    }

    pub fn ledger_by_id(&self, id: i64) -> Option<LedgerEntry> {
        self.conn.query_row(
            "SELECT id,interface_code,service_name,window_start,window_end,verdict,confidence,
                    evidence_refs,status,request_id,dict_version,ticket_no,confirmed_by,confirmed_at,reason,degraded,created_at
             FROM attribution_ledger WHERE id=?1", params![id], map_entry,
        ).optional().ok().flatten()
    }

    pub fn ledger_all(&self) -> Vec<LedgerEntry> {
        self.conn.prepare(
            "SELECT id,interface_code,service_name,window_start,window_end,verdict,confidence,
                    evidence_refs,status,request_id,dict_version,ticket_no,confirmed_by,confirmed_at,reason,degraded,created_at
             FROM attribution_ledger ORDER BY id",
        ).ok().and_then(|mut stmt| {
            stmt.query_map([], map_entry).ok().map(|rows| rows.filter_map(|r| r.ok()).collect())
        }).unwrap_or_default()
    }

    pub fn ledger_by_status(&self, status: LedgerStatus) -> Vec<LedgerEntry> {
        self.ledger_all().into_iter().filter(|e| e.status == status).collect()
    }

    // ---- 审计链（只增不改）----
    pub fn append_audit(&self, ts: &str, actor: &str, action: &str, code: &str, digest: &str, ticket: &str, detail: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_chain(ts,actor,action,interface_code,evidence_digest,ticket_no,detail)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![ts, actor, action, code, digest, ticket, detail],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("审计追加失败: {e}")))?;
        Ok(())
    }

    pub fn audit_count(&self) -> i64 {
        self.conn.query_row("SELECT COUNT(*) FROM audit_chain", [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn audit_rows(&self) -> Vec<(String, String, String)> {
        self.conn.prepare("SELECT ts,action,interface_code FROM audit_chain ORDER BY seq")
            .ok().and_then(|mut s| s.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect())).unwrap_or_default()
    }

    // ---- 调用审计 ----
    #[allow(clippy::too_many_arguments)]
    pub fn insert_model_call(&self, request_id: &str, model: &str, host: &str, input_hash: &str,
                             tokens: i64, elapsed: i64, response_hash: &str, degraded: bool,
                             retries: i64, created: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO model_call_audit(request_id,model,host,input_hash,token_count,
                elapsed_ms,response_hash,degraded,retries,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![request_id, model, host, input_hash, tokens, elapsed, response_hash,
                if degraded {1} else {0}, retries, created],
        ).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("调用审计失败: {e}")))?;
        Ok(())
    }

    pub fn model_call_by_request(&self, request_id: &str) -> Option<(String, String, bool)> {
        self.conn.query_row(
            "SELECT model,host,degraded FROM model_call_audit WHERE request_id=?1",
            params![request_id], |r| {
                let d: i64 = r.get(2)?;
                Ok((r.get(0)?, r.get(1)?, d != 0))
            },
        ).optional().ok().flatten()
    }

    pub fn model_call_count(&self) -> i64 {
        self.conn.query_row("SELECT COUNT(*) FROM model_call_audit", [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn conn(&self) -> &Connection { &self.conn }
}

fn map_entry(r: &rusqlite::Row) -> rusqlite::Result<LedgerEntry> {
    let refs_json: String = r.get(7)?;
    let refs: Vec<String> = serde_json::from_str(&refs_json).unwrap_or_default();
    let status: String = r.get(8)?;
    Ok(LedgerEntry {
        id: r.get(0)?, interface_code: r.get(1)?, service_name: r.get(2)?,
        window_start_ms: r.get(3)?, window_end_ms: r.get(4)?, verdict: r.get(5)?,
        confidence: r.get(6)?, evidence_refs: refs,
        status: LedgerStatus::parse(&status).unwrap_or(LedgerStatus::Collected),
        request_id: r.get(9)?, dict_version: r.get(10)?, ticket_no: r.get(11)?,
        confirmed_by: r.get(12)?, confirmed_at: r.get(13)?, reason: r.get(14)?,
        degraded: r.get::<_, i64>(15)? != 0, created_at: r.get(16)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(db: &IndexDb) -> i64 {
        db.insert_candidate("A", "S", 0, 100, "接口自身劣化", 0.9,
            &["flow#a0001".into()], "req-1", "v1", false, "", "", "now").unwrap()
    }

    #[test]
    fn cursor_upsert_is_idempotent() {
        let db = IndexDb::open_in_memory().unwrap();
        db.upsert_cursor("esb_audit", 10, 100, 999, "t1").unwrap();
        db.upsert_cursor("esb_audit", 20, 200, 888, "t2").unwrap();
        assert_eq!(db.cursor_of("esb_audit"), Some((20, 200, 888)));
    }

    #[test]
    fn tsid_dedup_works() {
        let db = IndexDb::open_in_memory().unwrap();
        assert!(db.mark_tsid_seen("abc", "esb_audit", "t").unwrap());
        assert!(!db.mark_tsid_seen("abc", "esb_audit", "t").unwrap());
        assert_eq!(db.distinct_tsid_count(), 1);
    }

    #[test]
    fn illegal_state_transition_rejected() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = entry(&db);
        let e = db.transition(id, LedgerStatus::TicketCreated).unwrap_err();
        assert_eq!(e.code, ErrorCode::EvidenceMissing);
    }

    #[test]
    fn full_legal_state_machine() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = entry(&db);
        assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::CandidateRanked);
        db.transition(id, LedgerStatus::AwaitingConfirmation).unwrap();
        db.writeback_ticket(id, "TICKET-001").unwrap();
        let e = db.ledger_by_id(id).unwrap();
        assert_eq!(e.status, LedgerStatus::TicketCreated);
        assert_eq!(e.ticket_no, "TICKET-001");
    }

    #[test]
    fn audit_chain_is_append_only() {
        let db = IndexDb::open_in_memory().unwrap();
        db.append_audit("t", "op", "rank", "A", "d", "", "").unwrap();
        db.append_audit("t", "op", "confirm", "A", "d", "", "").unwrap();
        assert_eq!(db.audit_count(), 2);
        let rows = db.audit_rows();
        assert_eq!(rows[0].1, "rank");
        assert_eq!(rows[1].1, "confirm");
    }
}
