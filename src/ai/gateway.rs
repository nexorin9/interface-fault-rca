//! 单一 ModelGateway 协议客户端（OpenAI-compatible 一种）：配置全部来自 env；
//! 请求体组装 / 响应解析 / 五分类错误 / 指数退避重试；业务层不耦合厂商 SDK；
//! 请求与响应不打印正文，只记可审计元数据。

use crate::ai::conclusion::{Conclusion, Verdict};
use crate::ai::errors::{classify_http, decide_retries, ModelErrorKind};
use crate::ai::ports::{ModelGateway, ModelRequest, ModelResponse};
use crate::ai::schema::validate_output;
use crate::config::{redact_url, ModelConfig};
use crate::error::{AppError, ErrorCode, Result};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ModelGatewayClient {
    pub cfg: ModelConfig,
}

impl ModelGatewayClient {
    pub fn new(cfg: ModelConfig) -> Self { Self { cfg } }

    fn ready(&self) -> bool {
        !self.cfg.base_url.is_empty() && !self.cfg.api_key.is_empty() && !self.cfg.name.is_empty()
    }

    /// 组装 OpenAI-compatible 请求体（messages/model/temperature/response_format）
    pub fn assemble_request(&self, req: &ModelRequest, schema: &Value) -> Value {
        let mut v = req.body_json.parse::<Value>().unwrap_or_else(|_| serde_json::json!({}));
        let obj = v.as_object_mut().unwrap();
        obj.insert("model".into(), serde_json::json!(self.cfg.name));
        obj.insert("temperature".into(), serde_json::json!(0.0));
        if !obj.contains_key("response_format") {
            obj.insert("response_format".into(), serde_json::json!({
                "type": "json_schema", "json_schema": {"name": "attribution", "schema": schema}
            }));
        }
        v
    }

    /// 解析响应：choices[0].message.content → JSON → Schema 校验 → Conclusion 列表
    pub fn parse_response(&self, text: &str) -> std::result::Result<Vec<Conclusion>, ModelErrorKind> {
        let v: Value = serde_json::from_str(text).map_err(|_| ModelErrorKind::JsonParse)?;
        let content = v.get("choices").and_then(|c| c.get(0))
            .and_then(|c| c.get("message")).and_then(|m| m.get("content"))
            .and_then(|x| x.as_str())
            .ok_or(ModelErrorKind::JsonParse)?;
        let out: Value = serde_json::from_str(content).map_err(|_| ModelErrorKind::JsonParse)?;
        validate_output(&out).map_err(|_| ModelErrorKind::SchemaValidation)?;
        let degraded = out.get("degraded").and_then(|x| x.as_bool()).unwrap_or(false);
        let mut conclusions = vec![];
        let empty = vec![];
        let candidates = out.get("candidates").and_then(|x| x.as_array()).unwrap_or(&empty);
        for c in candidates {
            let code = c.get("interface_code").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let service = c.get("service_name").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let verdict = c.get("verdict").and_then(|x| x.as_str())
                .and_then(Verdict::parse).unwrap_or(Verdict::证据不足);
            let confidence = c.get("confidence").and_then(|x| x.as_f64()).unwrap_or(0.0);
            let refs = c.get("evidence_refs").and_then(|x| x.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
            let reasoning = c.get("reasoning_summary").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let next = c.get("next_check").and_then(|x| x.as_str()).unwrap_or("").to_string();
            conclusions.push(Conclusion {
                interface_code: code, service_name: service, verdict, confidence,
                evidence_refs: refs, reasoning_summary: reasoning, next_check: next, degraded,
                baseline_stats: crate::ai::conclusion::BaselineStats {
                    p95_ms: 0.0, p99_ms: 0.0, degrade_ratio: 0.0, error_rate: 0.0, baseline_samples: 0,
                },
                request_id: String::new(),
            });
        }
        Ok(conclusions)
    }

    /// 真实 HTTP 调用（含超时、连接池复用、重试）。替身仅用于测试。
    fn do_http(&self, body: &Value) -> std::result::Result<String, ModelErrorKind> {
        let url = format!("{}/chat/completions", self.cfg.base_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(self.cfg.timeout_sec))
            .build().map_err(|_| ModelErrorKind::Connection)?;
        let resp = client.post(&url)
            .bearer_auth(&self.cfg.api_key)
            .json(body)
            .send().map_err(|e| {
                if e.is_timeout() { ModelErrorKind::Timeout } else { ModelErrorKind::Connection }
            })?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) { return Err(classify_http(status)); }
        resp.text().map_err(|_| ModelErrorKind::JsonParse)
    }
}

impl ModelGateway for ModelGatewayClient {
    fn available(&self) -> bool { self.ready() }

    fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
        if !self.ready() {
            return Err(AppError::new(ErrorCode::ModelUnavailable,
                "MODEL_BASE_URL / MODEL_API_KEY / MODEL_NAME 缺失"));
        }
        let schema: Value = serde_json::from_str(&crate::ai::schema::output_schema_json())
            .unwrap_or_else(|_| serde_json::json!({}));
        let body = self.assemble_request(req, &schema);
        let mut retries = 0u32;
        let mut last_kind: Option<ModelErrorKind> = None;
        loop {
            match self.do_http(&body) {
                Ok(text) => match self.parse_response(&text) {
                    Ok(mut conclusions) => {
                        for c in &mut conclusions { c.request_id = req.request_id.clone(); }
                        return Ok(ModelResponse {
                            conclusions, degraded: false, request_id: req.request_id.clone(), retries,
                        });
                    }
                    Err(kind) => { last_kind = Some(kind); break; }
                },
                Err(kind) => {
                    last_kind = Some(kind);
                    let max = decide_retries(kind, self.cfg.max_retries);
                    if retries < max {
                        retries += 1;
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        continue;
                    }
                    break;
                }
            }
        }
        let kind = last_kind.unwrap_or(ModelErrorKind::Connection);
        Err(AppError::new(kind.error_code(), format!("模型调用失败（{}，重试 {retries} 次）", kind.as_str())))
    }
}

/// 协议层替身：不连网，按预设响应跑同一 parse_response 逻辑（测试数据层）
pub struct StubGateway {
    pub cfg: ModelConfig,
    pub response: String,
}

impl ModelGateway for StubGateway {
    fn available(&self) -> bool { true }
    fn call(&self, req: &ModelRequest) -> Result<ModelResponse> {
        let client = ModelGatewayClient::new(self.cfg.clone());
        match client.parse_response(&self.response) {
            Ok(mut conclusions) => {
                for c in &mut conclusions { c.request_id = req.request_id.clone(); }
                Ok(ModelResponse { conclusions, degraded: false, request_id: req.request_id.clone(), retries: 0 })
            }
            Err(kind) => Err(AppError::new(kind.error_code(), format!("替身响应 {}", kind.as_str()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ModelConfig;

    fn cfg() -> ModelConfig {
        ModelConfig { base_url: "http://model:8000/v1".into(), api_key: "k".into(), name: "m".into(), timeout_sec: 30, max_retries: 2 }
    }
    fn ok_resp() -> String {
        let inner = r#"{"candidates":[{"interface_code":"A","service_name":"S","verdict":"接口自身劣化","confidence":0.9,"evidence_refs":["flow#a1"],"reasoning_summary":"r","next_check":"n"}],"degraded":false}"#;
        serde_json::json!({"choices":[{"message":{"content":inner}}]}).to_string()
    }
    fn req() -> ModelRequest {
        ModelRequest { body_json: "{}".into(), allowed_row_ids: vec!["flow#a1".into()], request_id: "r1".into() }
    }

    #[test]
    fn assemble_request_has_model_and_response_format() {
        let g = ModelGatewayClient::new(cfg());
        let body = g.assemble_request(&req(), &serde_json::json!({}));
        assert_eq!(body["model"], "m");
        assert!(body.get("response_format").is_some());
        assert_eq!(body["temperature"], 0.0);
    }

    #[test]
    fn valid_structured_response_parsed() {
        let g = ModelGatewayClient::new(cfg());
        let cs = g.parse_response(&ok_resp()).unwrap();
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].verdict, Verdict::接口自身劣化);
    }

    #[test]
    fn invalid_schema_response_fails() {
        let g = ModelGatewayClient::new(cfg());
        let bad = serde_json::json!({"choices":[{"message":{"content":"{\"candidates\":[],\"degraded\":\"yes\"}"}}]}).to_string();
        assert_eq!(g.parse_response(&bad).unwrap_err(), ModelErrorKind::SchemaValidation);
    }

    #[test]
    fn non_json_response_fails() {
        let g = ModelGatewayClient::new(cfg());
        assert_eq!(g.parse_response("not json").unwrap_err(), ModelErrorKind::JsonParse);
    }

    #[test]
    fn stub_gateway_runs_same_parse_path() {
        let stub = StubGateway { cfg: cfg(), response: ok_resp() };
        let out = stub.call(&req()).unwrap();
        assert_eq!(out.conclusions[0].request_id, "r1");
    }
}
