//! Prometheus query_range HTTP 只读适配器：出站拉 JSON（不复用入站 remote write protobuf）。
//! 响应信封严格校验 + 响应体体积上限 + unix 秒→毫秒。替身仅作测试数据层。

use crate::error::{AppError, ErrorCode, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Deserialize)]
pub struct PromEnvelope {
    pub status: String,
    #[serde(default)]
    pub data: Option<PromData>,
    #[serde(default, rename = "errorType")]
    pub error_type: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromData {
    #[serde(rename = "resultType")]
    pub result_type: String,
    #[serde(default)]
    pub result: Vec<PromSeries>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromSeries {
    pub metric: BTreeMap<String, String>,
    pub values: Vec<(f64, String)>,
}

#[derive(Debug, Clone)]
pub struct PromSample {
    pub ts_ms: i64,
    pub metric_name: String,
    pub value: String,
    pub labels: BTreeMap<String, String>,
}

/// 解析并校验响应信封。status!=success / resultType!=matrix / 体积超限都显式报错。
pub fn parse_envelope(txt: &str, max_bytes: usize) -> Result<Vec<PromSample>> {
    if txt.len() > max_bytes {
        return Err(AppError::new(ErrorCode::SourceUnavailable,
            format!("响应体 {} 字节超上限 {max_bytes}", txt.len())));
    }
    let env: PromEnvelope = serde_json::from_str(txt)
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("非 JSON 响应: {e}")))?;
    if env.status != "success" {
        return Err(AppError::new(ErrorCode::SourceUnavailable,
            format!("status={} errorType={} error={}",
                env.status, env.error_type.as_deref().unwrap_or("-"), env.error.as_deref().unwrap_or("-"))));
    }
    let data = env.data.ok_or_else(|| AppError::new(ErrorCode::SourceUnavailable, "缺 data 字段"))?;
    if data.result_type != "matrix" {
        return Err(AppError::new(ErrorCode::SourceUnavailable,
            format!("resultType={} 非 matrix", data.result_type)));
    }
    let mut out = vec![];
    for s in data.result {
        let metric_name = s.metric.get("__name__").cloned().unwrap_or_default();
        for (ts, val) in s.values {
            out.push(PromSample {
                ts_ms: (ts * 1000.0).round() as i64,
                metric_name: metric_name.clone(),
                value: val,
                labels: s.metric.clone(),
            });
        }
    }
    Ok(out)
}

/// 构造 query_range URL（base_url + 参数，参数化编码）
pub fn build_query_url(base_url: &str, query: &str, start: i64, end: i64, step: i64) -> String {
    let q = urlencode(query);
    format!("{}/api/v1/query_range?query={}&start={}&end={}&step={}",
        base_url.trim_end_matches('/'), q, start, end, step)
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 真实 HTTP 拉取（reqwest）；凭据来自参数（由 config 从 env 加载），缺失即报错。
pub fn fetch(base_url: &str, token: &str, query: &str, start: i64, end: i64, step: i64, max_bytes: usize) -> Result<Vec<PromSample>> {
    if base_url.is_empty() || token.is_empty() {
        return Err(AppError::new(ErrorCode::SourceUnavailable, "PROM_BASE_URL / PROM_BEARER_TOKEN 缺失"));
    }
    let url = build_query_url(base_url, query, start, end, step);
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("HTTP 客户端建立失败: {e}")))?;
    let resp = client.get(&url).bearer_auth(token).send()
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("请求失败: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::new(ErrorCode::SourceUnavailable, format!("HTTP {}", resp.status().as_u16())));
    }
    let txt = resp.text().map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("读响应失败: {e}")))?;
    parse_envelope(&txt, max_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = r#"{"status":"success","data":{"resultType":"matrix","result":[{"metric":{"__name__":"http_p95","job":"esb"},"values":[[1762000000,"123.4"],[1762000060,"130.0"]]}]}}"#;

    #[test]
    fn success_envelope_parsed_and_seconds_to_ms() {
        let s = parse_envelope(OK, 1_000_000).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].ts_ms, 1_762_000_000_000);
        assert_eq!(s[0].metric_name, "http_p95");
        assert_eq!(s[0].labels.get("job").unwrap(), "esb");
    }

    #[test]
    fn status_error_preserved() {
        let txt = r#"{"status":"error","errorType":"bad_data","error":"invalid query"}"#;
        let e = parse_envelope(txt, 1_000_000).unwrap_err();
        assert_eq!(e.code, ErrorCode::SourceUnavailable);
        assert!(e.detail.contains("bad_data"));
        assert!(e.detail.contains("invalid query"));
    }

    #[test]
    fn non_matrix_rejected() {
        let txt = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
        assert!(parse_envelope(txt, 1_000_000).is_err());
    }

    #[test]
    fn oversized_body_rejected() {
        assert_eq!(parse_envelope(OK, 10).unwrap_err().code, ErrorCode::SourceUnavailable);
    }

    #[test]
    fn url_is_encoded() {
        assert!(build_query_url("http://p:9090", "rate(x[5m])", 1, 2, 3).contains("rate(x%5B5m%5D)"));
    }
}
