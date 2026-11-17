//! 台账封装：候选落点、交班查询（按接口/窗口/状态/工单号）、状态机推进入口。
//! 状态推进边界：编排最多到 candidate_ranked；awaiting_confirmation 由人工确认触发。

use crate::error::{AppError, ErrorCode, Result};
use crate::store::sqlite_index::{IndexDb, LedgerEntry, LedgerStatus};

#[derive(Debug, Clone)]
pub enum LedgerQuery {
    ByInterface(String),
    ByWindow(i64, i64),
    ByStatus(LedgerStatus),
    ByTicketNo(String),
    All,
}

pub fn query(db: &IndexDb, q: &LedgerQuery) -> Vec<LedgerEntry> {
    let all = db.ledger_all();
    match q {
        LedgerQuery::ByInterface(c) => all.into_iter().filter(|e| &e.interface_code == c).collect(),
        LedgerQuery::ByWindow(ws, we) => all.into_iter().filter(|e| e.window_start_ms >= *ws && e.window_end_ms <= *we).collect(),
        LedgerQuery::ByStatus(s) => all.into_iter().filter(|e| e.status == *s).collect(),
        LedgerQuery::ByTicketNo(t) => all.into_iter().filter(|e| &e.ticket_no == t).collect(),
        LedgerQuery::All => all,
    }
}

/// 人工确认：candidate_ranked → awaiting_confirmation（唯一合法触发点）
pub fn confirm(db: &IndexDb, id: i64, actor: &str, at: &str) -> Result<()> {
    db.confirm(id, actor, at)
}

/// 作废：awaiting_confirmation → voided；已建工单不自动撤单，只提示（返回提示文本）
pub fn void_candidate(db: &IndexDb, id: i64) -> Result<String> {
    let e = db.ledger_by_id(id).ok_or_else(|| AppError::new(ErrorCode::EvidenceMissing, "台账行不存在"))?;
    if e.status == LedgerStatus::TicketCreated {
        return Ok(format!("候选 {} 已建工单 {}：本地作废，但工单不自动撤单，请人工到工单系统处理", id, e.ticket_no));
    }
    db.transition(id, LedgerStatus::Voided)?;
    Ok(format!("候选 {id} 已作废"))
}

/// 状态链推进（collected→...→candidate_ranked），编排只用到 candidate_ranked 为止。
pub fn advance_to_candidate_ranked(db: &IndexDb, id: i64) -> Result<()> {
    db.transition(id, LedgerStatus::EvidenceWindowOpen)?;
    db.transition(id, LedgerStatus::CandidateRanked)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(db: &IndexDb) -> i64 {
        db.insert_candidate("A", "S", 0, 100, "接口自身劣化", 0.9, &["flow#a1".into()],
            "req", "v1", false, "", "k", "now").unwrap()
    }

    #[test]
    fn query_by_four_dimensions() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = seed(&db);
        assert_eq!(query(&db, &LedgerQuery::ByInterface("A".into())).len(), 1);
        assert_eq!(query(&db, &LedgerQuery::ByWindow(0, 100)).len(), 1);
        assert_eq!(query(&db, &LedgerQuery::ByStatus(LedgerStatus::CandidateRanked)).len(), 1);
        assert_eq!(query(&db, &LedgerQuery::All).len(), 1);
        db.confirm(id, "op", "t").unwrap();
        assert_eq!(query(&db, &LedgerQuery::ByStatus(LedgerStatus::AwaitingConfirmation)).len(), 1);
    }

    #[test]
    fn model_path_cannot_advance_to_ticket() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = seed(&db);
        // 编排直接推到 ticket_created 必失败
        assert!(db.transition(id, LedgerStatus::TicketCreated).is_err());
        assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::CandidateRanked);
    }

    #[test]
    fn void_created_ticket_only_prompts() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = seed(&db);
        db.transition(id, LedgerStatus::AwaitingConfirmation).unwrap();
        db.writeback_ticket(id, "T-1").unwrap();
        let msg = void_candidate(&db, id).unwrap();
        assert!(msg.contains("不自动撤单"));
        assert_eq!(db.ledger_by_id(id).unwrap().status, LedgerStatus::TicketCreated);
    }

    #[test]
    fn confirm_moves_to_awaiting() {
        let db = IndexDb::open_in_memory().unwrap();
        let id = seed(&db);
        confirm(&db, id, "operator", "2026-11-09T08:00:00Z").unwrap();
        let e = db.ledger_by_id(id).unwrap();
        assert_eq!(e.status, LedgerStatus::AwaitingConfirmation);
        assert_eq!(e.confirmed_by, "operator");
    }
}
