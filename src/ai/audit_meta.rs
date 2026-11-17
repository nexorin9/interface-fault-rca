//! 可审计调用元数据：model / base_url host / request_id / 输入摘要哈希 / token 数 /
//! 耗时 / 响应哈希 / 是否降级 / 重试次数。不落 prompt 全文与响应全文。

use crate::ai::ports::{AuditMeta, AuditSink};
use crate::config::redact_url;
use crate::store::sqlite_index::IndexDb;
use sha2::{Digest, Sha256};

pub fn digest(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// 从请求/响应构造审计元数据（只留哈希与计数，不留正文）
pub fn build_meta(
    request_id: &str, model: &str, base_url: &str, input_body: &str,
    response_body: &str, token_count: u32, elapsed_ms: u64, degraded: bool, retries: u32,
) -> AuditMeta {
    AuditMeta {
        request_id: request_id.to_string(),
        model: model.to_string(),
        host: redact_url(base_url), // 只 host，不含路径与凭据
        input_hash: digest(input_body),
        token_count,
        elapsed_ms,
        response_hash: digest(response_body),
        degraded,
        retries,
    }
}

#[derive(Clone)]
pub struct DbAuditSink<'a> {
    pub db: &'a IndexDb,
}

impl<'a> AuditSink for DbAuditSink<'a> {
    fn write(&self, m: &AuditMeta) {
        let now = chrono::Utc::now().to_rfc3339();
        let _ = self.db.insert_model_call(&m.request_id, &m.model, &m.host, &m.input_hash,
            m.token_count as i64, m.elapsed_ms as i64, &m.response_hash, m.degraded,
            m.retries as i64, &now);
    }
}

/// 断言审计记录不含消息正文片段：只含哈希与结构化计数。
pub fn meta_is_clean(m: &AuditMeta, secret_fragment: &str) -> bool {
    let serialized = format!("{:?}", m);
    !serialized.contains(secret_fragment)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meta_fields_populated() {
        let m = build_meta("r1", "model-x", "http://host:8000/v1/path?key=secret",
            "prompt with secret text", "response body", 42, 100, false, 1);
        assert_eq!(m.request_id, "r1");
        assert_eq!(m.model, "model-x");
        assert_eq!(m.host, "host:8000");
        assert_eq!(m.token_count, 42);
        assert_eq!(m.retries, 1);
        assert_eq!(m.input_hash.len(), 64);
    }

    #[test]
    fn audit_never_contains_prompt_or_response_text() {
        let secret = "CONFIDENTIAL_PROMPT_BODY_12345";
        let m = build_meta("r", "m", "http://h/v1", secret, secret, 1, 1, false, 0);
        assert!(meta_is_clean(&m, secret));
        assert!(!m.input_hash.contains(secret));
        assert!(!m.response_hash.contains(secret));
    }

    #[test]
    fn candidate_can_query_call_by_request_id() {
        let db = IndexDb::open_in_memory().unwrap();
        let m = build_meta("req-42", "m", "http://h/v1", "i", "o", 10, 20, true, 1);
        let sink = DbAuditSink { db: &db };
        sink.write(&m);
        let got = db.model_call_by_request("req-42").unwrap();
        assert_eq!(got.0, "m");
        assert!(got.2); // degraded
        assert_eq!(db.model_call_count(), 1);
    }

    #[test]
    fn degrade_history_queryable() {
        let db = IndexDb::open_in_memory().unwrap();
        for (id, deg) in [("r1", true), ("r2", false)] {
            let m = build_meta(id, "m", "http://h", "i", "o", 1, 1, deg, 0);
            DbAuditSink { db: &db }.write(&m);
        }
        assert!(db.model_call_by_request("r1").unwrap().2);
        assert!(!db.model_call_by_request("r2").unwrap().2);
    }
}
