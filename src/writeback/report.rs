//! 交班摘要导出：按窗口生成一段能直接念、每句带证据引用的文字；
//! 输出 Markdown + 纯文本两份；显式列出待确认项；不含患者可识别明细。

use crate::store::sqlite_index::{IndexDb, LedgerEntry, LedgerStatus};
use std::path::Path;

/// 生成交班摘要内容（纯文本）
pub fn handover_text(ws: i64, we: i64, entries: &[LedgerEntry], degraded: bool) -> String {
    let mut out = format!("交班摘要（窗口 {} 至 {}）\n", ws, we);
    if degraded { out.push_str("模型不可达，以下为规则基线结论。\n"); }
    let mut ranked: Vec<_> = entries.iter()
        .filter(|e| e.status != LedgerStatus::Voided)
        .collect();
    ranked.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
    out.push_str(&format!("昨夜先变慢的接口 Top-{}：\n", ranked.len().min(20)));
    for (i, e) in ranked.iter().take(20).enumerate() {
        let refs = if e.evidence_refs.is_empty() { "无证据引用".to_string() } else { e.evidence_refs.join(", ") };
        out.push_str(&format!("  {}. {}（{}）：判定 {}，置信度 {:.2}，证据行 {}。\n",
            i + 1, e.interface_code, e.service_name, e.verdict, e.confidence, refs));
    }
    let pending: Vec<_> = entries.iter().filter(|e| e.status == LedgerStatus::CandidateRanked).collect();
    if !pending.is_empty() {
        out.push_str(&format!("待确认 {} 条（尚未建单）：{}\n", pending.len(),
            pending.iter().map(|e| e.interface_code.clone()).collect::<Vec<_>>().join(", ")));
    }
    let created: Vec<_> = entries.iter().filter(|e| e.status == LedgerStatus::TicketCreated).collect();
    if !created.is_empty() {
        out.push_str(&format!("已建工单 {} 张：{}\n", created.len(),
            created.iter().map(|e| format!("{}={}", e.interface_code, e.ticket_no)).collect::<Vec<_>>().join(", ")));
    }
    out
}

/// 导出两份文件到目录
pub fn export_handover(dir: &Path, ws: i64, we: i64, entries: &[LedgerEntry], degraded: bool) -> (String, String) {
    std::fs::create_dir_all(dir).ok();
    let text = handover_text(ws, we, entries, degraded);
    let md = format!("# 交班摘要\n\n```text\n{}```\n", text);
    let txt_path = dir.join("handover.txt").to_string_lossy().to_string();
    let md_path = dir.join("handover.md").to_string_lossy().to_string();
    let _ = std::fs::write(&txt_path, &text);
    let _ = std::fs::write(&md_path, &md);
    (txt_path, md_path)
}

/// 校验摘要中引用均在白名单内（不出现未在台账的引用）
pub fn refs_in_ledger(text: &str, entries: &[LedgerEntry]) -> bool {
    let allowed: Vec<&String> = entries.iter().flat_map(|e| e.evidence_refs.iter()).collect();
    // 从文本里提取 flow#/log#/metric# 形态的引用
    for token in text.split(|c: char| c.is_whitespace() || c == ',' || c == '。') {
        if token.starts_with("flow#") || token.starts_with("log#") || token.starts_with("metric#") {
            let clean = token.trim_end_matches(|c: char| c == '。' || c == '.');
            if !allowed.iter().any(|a| a.as_str() == clean) { return false; }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(code: &str, status: LedgerStatus, refs: Vec<String>, ticket: &str, conf: f64) -> LedgerEntry {
        LedgerEntry {
            id: 1, interface_code: code.into(), service_name: "服务".into(),
            window_start_ms: 0, window_end_ms: 100, verdict: "接口自身劣化".into(),
            confidence: conf, evidence_refs: refs, status, request_id: "".into(),
            dict_version: "v1".into(), ticket_no: ticket.into(), confirmed_by: "".into(),
            confirmed_at: "".into(), reason: "".into(), degraded: false, created_at: "".into(),
        }
    }

    #[test]
    fn summary_has_topsis_pending_and_ticket() {
        let entries = vec![
            entry("A", LedgerStatus::CandidateRanked, vec!["flow#a1".into()], "", 0.9),
            entry("B", LedgerStatus::TicketCreated, vec!["log#a2".into()], "T-1", 0.8),
        ];
        let t = handover_text(0, 100, &entries, false);
        assert!(t.contains("Top-2"));
        assert!(t.contains("待确认 1 条"));
        assert!(t.contains("已建工单 1 张"));
        assert!(t.contains("flow#a1"));
    }

    #[test]
    fn degrade_banner_in_summary() {
        let t = handover_text(0, 100, &[], true);
        assert!(t.contains("规则基线结论"));
    }

    #[test]
    fn all_refs_are_in_ledger() {
        let entries = vec![entry("A", LedgerStatus::CandidateRanked, vec!["flow#a1".into()], "", 0.9)];
        let t = handover_text(0, 100, &entries, false);
        assert!(refs_in_ledger(&t, &entries));
        let bad = format!("{t} 伪造引用 flow#fake999");
        assert!(!refs_in_ledger(&bad, &entries));
    }

    #[test]
    fn export_writes_two_files() {
        let dir = tempfile::tempdir().unwrap();
        let (txt, md) = export_handover(dir.path(), 0, 100, &[], false);
        assert!(std::path::Path::new(&txt).exists());
        assert!(std::path::Path::new(&md).exists());
    }
}
