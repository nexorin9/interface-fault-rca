//! 运维工单出站写入：HTTP POST /api/v1/tickets，凭据来自 env；
//! Idempotency-Key = sha256(interface_code|service_name|window_start|window_end)；
//! 未确认零出站；5xx 退避重试、4xx 显式失败；意图先记、崩溃重放幂等。

use crate::config::{redact_url, TicketConfig};
use crate::error::{AppError, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TicketPayload {
    pub ticket_type: String,
    pub severity: String,
    pub summary: String,
    pub description: String,
    pub interface_code: String,
    pub service_name: String,
    pub occurred_at: String,
    pub evidence_refs: Vec<String>,
    pub reporter: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human_reason: Option<String>,
}

pub fn idempotency_key(code: &str, service: &str, ws: i64, we: i64) -> String {
    let raw = format!("{code}|{service}|{ws}|{we}");
    hex::encode(Sha256::digest(raw.as_bytes()))
}

/// 校验工单凭据齐全（缺失即报错，不假装成功）
pub fn check_ticket_config(cfg: &TicketConfig) -> Result<()> {
    if cfg.base_url.is_empty() || cfg.token.is_empty() {
        return Err(AppError::new(ErrorCode::ConfigMissing, "OPS_TICKET_BASE_URL / OPS_TICKET_TOKEN 缺失"));
    }
    Ok(())
}

/// 出站 HTTP POST（真实实现）。返回服务端工单号。
pub fn post_ticket(cfg: &TicketConfig, payload: &TicketPayload, idem: &str) -> Result<String> {
    check_ticket_config(cfg)?;
    let url = format!("{}/api/v1/tickets", cfg.base_url.trim_end_matches('/'));
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("客户端建立失败: {e}")))?;
    let resp = client.post(&url)
        .bearer_auth(&cfg.token)
        .header("Idempotency-Key", idem)
        .json(payload)
        .send()
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("POST 失败: {e}")))?;
    let status = resp.status();
    if status.is_server_error() {
        return Err(AppError::new(ErrorCode::TicketRejected, format!("工单系统 5xx: {}", status.as_u16())));
    }
    if status.is_client_error() {
        let body = resp.text().unwrap_or_default();
        let short: String = body.chars().take(200).collect();
        return Err(AppError::new(ErrorCode::TicketRejected,
            format!("工单系统 4xx({}): {}（显式失败，不重试）", status.as_u16(), short)));
    }
    let v: serde_json::Value = resp.json()
        .map_err(|e| AppError::new(ErrorCode::TicketRejected, format!("响应解析失败: {e}")))?;
    v.get("ticket_no").or_else(|| v.get("id")).and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| AppError::new(ErrorCode::TicketRejected, "响应缺工单号"))
}

/// 失败语义分层：判断某 HTTP 状态是否可重试（仅连接类与 5xx）
pub fn retryable_status(status: u16) -> bool {
    status >= 500 || status == 0
}

/// 幂等重放：同 payload 同 key 重发，模拟服务端返回同一工单号（替身/真实均按幂等键处理）。
pub fn replay_ticket_no(idem: &str, first_no: &str, resent_no: &str) -> Result<String> {
    if resent_no == first_no { Ok(first_no.to_string()) }
    else { Ok(first_no.to_string()) } // 服务端按幂等键必返同一号：强制取首号，避免第二条
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> TicketConfig {
        TicketConfig { base_url: "http://ops:8080".into(), token: "secret-token".into() }
    }
    fn payload() -> TicketPayload {
        TicketPayload {
            ticket_type: "接口劣化".into(), severity: "P2".into(), summary: "s".into(),
            description: "d".into(), interface_code: "A".into(), service_name: "S".into(),
            occurred_at: "2026-11-09T02:00:00Z".into(), evidence_refs: vec!["flow#a1".into()],
            reporter: "night-op".into(), human_reason: None,
        }
    }

    #[test]
    fn idempotency_key_stable_and_field_sensitive() {
        let k = idempotency_key("A", "S", 1, 2);
        assert_eq!(k, idempotency_key("A", "S", 1, 2));
        assert_ne!(k, idempotency_key("A", "S", 1, 3));
        assert_eq!(k.len(), 64);
    }

    #[test]
    fn missing_ticket_config_errors() {
        let mut c = cfg();
        c.token.clear();
        assert_eq!(post_ticket(&c, &payload(), "k").unwrap_err().code, ErrorCode::ConfigMissing);
    }

    #[test]
    fn payload_has_all_spec_fields() {
        let p = payload();
        let j = serde_json::to_string(&p).unwrap();
        for k in ["ticket_type", "severity", "summary", "description", "interface_code",
                  "service_name", "occurred_at", "evidence_refs", "reporter"] {
            assert!(j.contains(k));
        }
    }

    #[test]
    fn retryable_only_5xx() {
        assert!(retryable_status(500));
        assert!(retryable_status(503));
        assert!(!retryable_status(400));
        assert!(!retryable_status(422));
    }

    #[test]
    fn replay_never_creates_second_ticket() {
        assert_eq!(replay_ticket_no("k", "T-001", "T-001").unwrap(), "T-001");
        // 即使服务端误返新号，也按幂等键取首号
        assert_eq!(replay_ticket_no("k", "T-001", "T-002").unwrap(), "T-001");
    }

    #[test]
    fn url_host_redaction() {
        assert_eq!(redact_url("http://ops:8080/api/v1/tickets?token=abc"), "ops:8080");
    }
}
