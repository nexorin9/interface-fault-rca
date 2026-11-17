//! 错误五分类与指数退避重试：连接失败 / 超时 / 非 2xx / JSON 解析失败 / Schema 校验失败。
//! 仅连接类与 5xx 重试（≤上限）；其余降级不重试。

use crate::error::ErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelErrorKind {
    Connection,
    Timeout,
    Http5xx,
    Http4xx,
    JsonParse,
    SchemaValidation,
}

impl ModelErrorKind {
    pub fn error_code(self) -> ErrorCode {
        match self {
            ModelErrorKind::Connection | ModelErrorKind::Timeout | ModelErrorKind::Http5xx => ErrorCode::ModelUnavailable,
            ModelErrorKind::Http4xx => ErrorCode::ModelUnavailable,
            ModelErrorKind::JsonParse | ModelErrorKind::SchemaValidation => ErrorCode::EvidenceMissing,
        }
    }
    /// 是否可重试：仅连接失败与 5xx
    pub fn retryable(self) -> bool {
        matches!(self, ModelErrorKind::Connection | ModelErrorKind::Http5xx)
    }
    pub fn as_str(self) -> &'static str {
        match self {
            ModelErrorKind::Connection => "连接失败",
            ModelErrorKind::Timeout => "超时",
            ModelErrorKind::Http5xx => "非2xx-5xx",
            ModelErrorKind::Http4xx => "非2xx-4xx",
            ModelErrorKind::JsonParse => "JSON解析失败",
            ModelErrorKind::SchemaValidation => "Schema校验失败",
        }
    }
}

/// 分类 HTTP 状态码
pub fn classify_http(status: u16) -> ModelErrorKind {
    if status >= 500 { ModelErrorKind::Http5xx }
    else if status >= 400 { ModelErrorKind::Http4xx }
    else { ModelErrorKind::Http5xx } // 非 2xx 又非 4xx 归 5xx 类（保守）
}

/// 指数退避序列（毫秒）：base, 2base, 4base...
pub fn backoff_ms(base: u64, n: u32) -> Vec<u64> {
    (0..n).map(|i| base.saturating_mul(1u64 << i)).collect()
}

/// 重试决策：返回实际重试次数（不超过 max_retries），仅可重试错误计数。
pub fn decide_retries(kind: ModelErrorKind, max_retries: u32) -> u32 {
    if kind.retryable() { max_retries } else { 0 }
}

/// 降级语义：模型路径上任何错误最终不可用时都切基线并置 degraded=true。
/// 可重试类穷尽重试后才降级；不可重试类立即降级。
pub fn should_degrade(_kind: ModelErrorKind) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_kinds_map_to_codes() {
        for k in [ModelErrorKind::Connection, ModelErrorKind::Timeout, ModelErrorKind::Http5xx,
                  ModelErrorKind::Http4xx, ModelErrorKind::JsonParse, ModelErrorKind::SchemaValidation] {
            assert!(!k.as_str().is_empty());
            let _ = k.error_code();
        }
    }

    #[test]
    fn only_connection_and_5xx_retry() {
        assert!(ModelErrorKind::Connection.retryable());
        assert!(ModelErrorKind::Http5xx.retryable());
        assert!(!ModelErrorKind::Timeout.retryable());
        assert!(!ModelErrorKind::Http4xx.retryable());
        assert!(!ModelErrorKind::JsonParse.retryable());
        assert!(!ModelErrorKind::SchemaValidation.retryable());
    }

    #[test]
    fn retry_count_capped() {
        assert_eq!(decide_retries(ModelErrorKind::Connection, 2), 2);
        assert_eq!(decide_retries(ModelErrorKind::JsonParse, 2), 0);
    }

    #[test]
    fn backoff_doubles() {
        assert_eq!(backoff_ms(100, 3), vec![100, 200, 400]);
    }

    #[test]
    fn http_classification() {
        assert_eq!(classify_http(503), ModelErrorKind::Http5xx);
        assert_eq!(classify_http(422), ModelErrorKind::Http4xx);
    }

    #[test]
    fn all_errors_eventually_degrade() {
        for k in [ModelErrorKind::Connection, ModelErrorKind::Timeout, ModelErrorKind::Http5xx,
                  ModelErrorKind::Http4xx, ModelErrorKind::JsonParse, ModelErrorKind::SchemaValidation] {
            assert!(should_degrade(k));
        }
    }
}
