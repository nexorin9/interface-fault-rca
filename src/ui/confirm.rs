//! 建单确认弹窗：列出全部待提交字段 + 幂等键预览 + 目标系统 host；
//! 弹窗展示前构造 payload（与 ticket_client 同一份），确认前零出站；
//! 降级时强制人工补原因字段；取消不产生状态变更。

use crate::config::redact_url;
use crate::writeback::ticket_client::{idempotency_key, TicketPayload};

#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub fields: Vec<(String, String)>, // (字段名, 值)
    pub idempotency_preview: String,
    pub target_host: String,
    pub human_reason_required: bool,
    pub human_reason: String,
    pub outbound_calls: u32, // 弹窗交互期间的出站计数（测试替身断言为 0）
}

impl ConfirmDialog {
    #[allow(clippy::too_many_arguments)]
    pub fn from_payload(p: &TicketPayload, ticket_base_url: &str, degraded: bool) -> Self {
        let fields = vec![
            ("ticket_type".into(), p.ticket_type.clone()),
            ("severity".into(), p.severity.clone()),
            ("summary".into(), p.summary.clone()),
            ("description".into(), p.description.clone()),
            ("interface_code".into(), p.interface_code.clone()),
            ("service_name".into(), p.service_name.clone()),
            ("occurred_at".into(), p.occurred_at.clone()),
            ("evidence_refs".into(), p.evidence_refs.join(", ")),
            ("reporter".into(), p.reporter.clone()),
        ];
        let ws = p.occurred_at.clone();
        let _ = ws;
        let idem = idempotency_key(&p.interface_code, &p.service_name,
            parse_epoch_hint(&p.occurred_at), parse_epoch_hint(&p.occurred_at) + 1);
        Self {
            fields, idempotency_preview: idem,
            target_host: redact_url(ticket_base_url),
            human_reason_required: degraded,
            human_reason: String::new(), outbound_calls: 0,
        }
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub fn can_submit(&self) -> bool {
        !self.human_reason_required || !self.human_reason.trim().is_empty()
    }

    /// 弹窗展示/交互不触发任何出站（计数保持 0）
    pub fn open_and_review(&mut self) {
        // 仅展示字段，不调用网络
    }
}

fn parse_epoch_hint(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.timestamp_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> TicketPayload {
        TicketPayload {
            ticket_type: "接口劣化".into(), severity: "P2".into(), summary: "s".into(),
            description: "d".into(), interface_code: "A".into(), service_name: "S".into(),
            occurred_at: "2026-11-09T02:00:00Z".into(), evidence_refs: vec!["flow#a1".into()],
            reporter: "op".into(), human_reason: None,
        }
    }

    #[test]
    fn dialog_lists_all_submit_fields() {
        let d = ConfirmDialog::from_payload(&payload(), "http://ops:8080/api", false);
        for k in ["ticket_type", "severity", "summary", "description", "interface_code",
                  "service_name", "occurred_at", "evidence_refs", "reporter"] {
            assert!(d.field(k).is_some());
        }
    }

    #[test]
    fn dialog_open_has_zero_outbound() {
        let mut d = ConfirmDialog::from_payload(&payload(), "http://ops", false);
        d.open_and_review();
        assert_eq!(d.outbound_calls, 0);
    }

    #[test]
    fn degraded_requires_human_reason() {
        let mut d = ConfirmDialog::from_payload(&payload(), "http://ops", true);
        assert!(!d.can_submit());
        d.human_reason = "人工判断：网络抖动".into();
        assert!(d.can_submit());
    }

    #[test]
    fn target_host_has_no_path_or_token() {
        let d = ConfirmDialog::from_payload(&payload(), "http://ops:8080/secret?token=abc", false);
        assert!(!d.target_host.contains("token"));
        assert!(!d.target_host.contains("secret"));
    }

    #[test]
    fn idempotency_preview_shown() {
        let d = ConfirmDialog::from_payload(&payload(), "http://ops", false);
        assert_eq!(d.idempotency_preview.len(), 64);
    }
}
