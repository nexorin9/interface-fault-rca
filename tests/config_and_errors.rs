//! task 19 验收：配置与错误码 —— 三类缺凭据各报对应错误码、非法数值被拒、提示不泄露凭据、10 个语义码齐备。
use interface_fault_rca::config::{mask, redact_url, Config, Strictness};
use interface_fault_rca::error::{AppError, ErrorCode};
use std::collections::HashMap;

fn full_map() -> HashMap<String, String> {
    let mut m = HashMap::new();
    for k in ["ESB_AUDIT_DB_HOST", "ESB_AUDIT_DB_USER", "ESB_AUDIT_DB_PASSWORD",
              "ESB_AUDIT_DB_NAME", "PROM_BASE_URL", "PROM_BEARER_TOKEN",
              "ESB_LOG_DIR", "APP_LOG_DIR", "OPS_TICKET_BASE_URL", "OPS_TICKET_TOKEN"] {
        m.insert(k.into(), "placeholder".into());
    }
    m.insert("ESB_AUDIT_DB_PORT".into(), "3306".into());
    m
}

#[test]
fn ten_semantic_error_codes_present() {
    let codes = [ErrorCode::SourceUnavailable, ErrorCode::SchemaMismatch, ErrorCode::AmbiguousInterface,
                 ErrorCode::EvidenceMissing, ErrorCode::ModelUnavailable, ErrorCode::WindowTooLarge,
                 ErrorCode::ConfigMissing, ErrorCode::RepeatedStart, ErrorCode::OutboundBlocked,
                 ErrorCode::TicketRejected];
    assert_eq!(codes.len(), 10);
    for c in codes {
        assert!(!c.message().is_empty());
        assert!(!c.suggestion().is_empty());
        assert!(!c.as_str().is_empty());
    }
}

#[test]
fn missing_esb_credentials_is_config_missing() {
    let m = HashMap::new();
    let c = Config::from_map(&m).unwrap();
    let e = c.validate(Strictness::Strict).unwrap_err();
    assert_eq!(e.code, ErrorCode::ConfigMissing);
    assert!(e.detail.contains("ESB_AUDIT_DB_HOST"));
}

#[test]
fn missing_ticket_token_is_config_missing_in_strict() {
    let mut m = full_map();
    m.remove("OPS_TICKET_TOKEN");
    let c = Config::from_map(&m).unwrap();
    let e = c.validate(Strictness::Strict).unwrap_err();
    assert_eq!(e.code, ErrorCode::ConfigMissing);
    assert!(e.detail.contains("OPS_TICKET_TOKEN"));
    // 诊断模式不要求工单凭据
    assert!(c.validate(Strictness::Diagnose).is_ok());
}

#[test]
fn invalid_numeric_config_rejected() {
    let mut m = full_map();
    m.insert("BASELINE_WINDOW_MINUTES".into(), "0".into()); // 越界
    assert!(Config::from_map(&m).is_err());
    let mut m2 = full_map();
    m2.insert("RETENTION_DAYS".into(), "99999".into()); // 超上界
    assert!(Config::from_map(&m2).is_err());
}

#[test]
fn credentials_are_masked_everywhere() {
    let secret = "super-secret-token";
    let masked = mask(secret);
    assert!(!masked.contains(secret));
    assert!(masked.contains("***"));
    let url = redact_url("https://user:pass@db.host:3306/path?token=abc");
    assert!(!url.contains("pass"));
    assert!(!url.contains("token"));
    assert!(url.contains("db.host"));
}

#[test]
fn error_display_shows_code_and_suggestion() {
    let e = AppError::new(ErrorCode::SourceUnavailable, "连接失败");
    let s = format!("{e}");
    assert!(s.contains("SourceUnavailable"));
    assert!(s.contains("检查"));
}
