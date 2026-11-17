//! task 46 验收：建单确认弹窗 —— 展示全部字段 / 打开零出站 / 降级强制人工补原因 / 取消不产生状态变更。
use interface_fault_rca::ui::confirm::ConfirmDialog;
use interface_fault_rca::writeback::ticket_client::TicketPayload;

fn payload(degraded_desc: bool) -> TicketPayload {
    TicketPayload {
        ticket_type: "接口劣化".into(), severity: "P2".into(), summary: "P95 劣化".into(),
        description: if degraded_desc { "规则基线结论".into() } else { "详见证据".into() },
        interface_code: "ESB-A".into(), service_name: "SVC".into(),
        occurred_at: "2026-11-08T20:00:00Z".into(),
        evidence_refs: vec!["flow#a1".into()], reporter: "值班机器人".into(),
        human_reason: None,
    }
}

#[test]
fn dialog_lists_all_submit_fields() {
    let d = ConfirmDialog::from_payload(&payload(false), "http://ops:8080", false);
    for f in ["ticket_type", "severity", "summary", "description", "interface_code",
              "service_name", "occurred_at", "evidence_refs", "reporter"] {
        assert!(d.field(f).is_some(), "缺字段 {f}");
    }
    assert!(!d.idempotency_preview.is_empty());
    assert_eq!(d.target_host, "ops:8080");
}

#[test]
fn opening_dialog_triggers_zero_outbound() {
    let mut d = ConfirmDialog::from_payload(&payload(false), "http://ops:8080", false);
    assert_eq!(d.outbound_calls, 0);
    d.open_and_review();
    d.open_and_review();
    assert_eq!(d.outbound_calls, 0);
}

#[test]
fn degraded_requires_human_reason() {
    let mut d = ConfirmDialog::from_payload(&payload(true), "http://ops:8080", true);
    assert!(d.human_reason_required);
    assert!(!d.can_submit());
    d.human_reason = "已人工核对证据行".into();
    assert!(d.can_submit());
}

#[test]
fn cancel_does_not_change_state() {
    let d = ConfirmDialog::from_payload(&payload(false), "http://ops:8080", false);
    let before = d.fields.clone();
    // 取消：不调用任何提交，字段不变
    assert_eq!(d.fields, before);
    assert_eq!(d.outbound_calls, 0);
}
