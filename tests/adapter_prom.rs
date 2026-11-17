//! task 5 验收：Prometheus query_range 适配器 —— 成功信封 / status=error / resultType 非 matrix / 超大体四种情形。
//! 本地替身仅作测试数据层。
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::prom_http::{build_query_url, parse_envelope, PromEnvelope};

const SUCCESS: &str = r#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"p95","instance":"host-1"},"values":[[1762636800,"123.5"],[1762636860,"200"]]}]}}"#;
const STATUS_ERR: &str = r#"{"status":"error","errorType":"bad_data","error":"invalid expression"}"#;
const NON_MATRIX: &str = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;

#[test]
fn success_envelope_parses_samples() {
    let samples = parse_envelope(SUCCESS, 1024 * 1024).unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0].ts_ms, 1762636800_000);
    assert_eq!(samples[0].value, "123.5");
    assert_eq!(samples[0].labels.get("instance").unwrap(), "host-1");
}

#[test]
fn status_error_keeps_original_message() {
    let env: PromEnvelope = serde_json::from_str(STATUS_ERR).unwrap();
    assert_eq!(env.status, "error");
    assert_eq!(env.error_type.as_deref(), Some("bad_data"));
    let err = parse_envelope(STATUS_ERR, 1024 * 1024).unwrap_err();
    assert_eq!(err.code, ErrorCode::SourceUnavailable);
}

#[test]
fn non_matrix_result_is_rejected() {
    let err = parse_envelope(NON_MATRIX, 1024 * 1024).unwrap_err();
    assert_eq!(err.code, ErrorCode::SourceUnavailable);
}

#[test]
fn oversized_body_is_rejected() {
    let err = parse_envelope(SUCCESS, 10).unwrap_err();
    assert_eq!(err.code, ErrorCode::SourceUnavailable);
}

#[test]
fn query_url_encodes_expression() {
    let url = build_query_url("http://prom:9090", "rate(http_requests[5m])", 100, 200, 60);
    assert!(url.contains("/api/v1/query_range"));
    assert!(url.contains("start="));
    assert!(url.contains("end="));
    assert!(url.contains("step="));
}
