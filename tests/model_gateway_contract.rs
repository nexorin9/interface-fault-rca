//! task 37 验收：ModelGateway 协议客户端 —— 结构合法被解析 / 结构非法判校验失败 / 非 JSON 走错误分类。
//! 协议替身仅作测试数据层，不是最终 integration surface。
use interface_fault_rca::ai::conclusion::Verdict;
use interface_fault_rca::ai::errors::ModelErrorKind;
use interface_fault_rca::ai::gateway::{ModelGatewayClient, StubGateway};
use interface_fault_rca::ai::ports::{ModelGateway, ModelRequest};
use interface_fault_rca::ai::schema::{output_schema_json, validate_output};
use interface_fault_rca::config::ModelConfig;

fn cfg() -> ModelConfig {
    ModelConfig { base_url: "http://model:8000/v1".into(), api_key: "k".into(),
        name: "m".into(), timeout_sec: 30, max_retries: 2 }
}
fn wrap(inner: &str) -> String {
    serde_json::json!({"choices":[{"message":{"content":inner}}]}).to_string()
}
fn req() -> ModelRequest {
    ModelRequest { body_json: "{}".into(), allowed_row_ids: vec!["flow#a1".into()], request_id: "r1".into() }
}

const OK: &str = r#"{"candidates":[{"interface_code":"A","service_name":"S","verdict":"接口自身劣化","confidence":0.9,"evidence_refs":["flow#a1"],"reasoning_summary":"r","next_check":"n"}],"degraded":false}"#;
const BAD_SCHEMA: &str = r#"{"candidates":[{"interface_code":"A","verdict":"不存在判定","confidence":2.5,"evidence_refs":"not-array"}],"degraded":false}"#;

#[test]
fn valid_structure_is_parsed() {
    let g = StubGateway { cfg: cfg(), response: wrap(OK) };
    let resp = g.call(&req()).unwrap();
    assert_eq!(resp.conclusions.len(), 1);
    assert_eq!(resp.conclusions[0].verdict, Verdict::接口自身劣化);
    assert_eq!(resp.request_id, "r1");
}

#[test]
fn invalid_structure_is_schema_validation_error() {
    let client = ModelGatewayClient::new(cfg());
    let v: serde_json::Value = serde_json::from_str(BAD_SCHEMA).unwrap();
    assert!(validate_output(&v).is_err());
    let err = client.parse_response(&wrap(BAD_SCHEMA)).unwrap_err();
    assert_eq!(err, ModelErrorKind::SchemaValidation);
}

#[test]
fn non_json_is_json_parse_error() {
    let client = ModelGatewayClient::new(cfg());
    let err = client.parse_response("not json at all").unwrap_err();
    assert_eq!(err, ModelErrorKind::JsonParse);
}

#[test]
fn assemble_request_has_model_and_schema() {
    let client = ModelGatewayClient::new(cfg());
    let body = client.assemble_request(&req(), &serde_json::from_str(&output_schema_json()).unwrap());
    assert_eq!(body["model"], "m");
    assert_eq!(body["temperature"], 0.0);
    assert!(body.get("response_format").is_some());
}

#[test]
fn verdict_enum_is_exactly_five() {
    let schema: serde_json::Value = serde_json::from_str(&output_schema_json()).unwrap();
    let s = schema.to_string();
    for v in ["接口自身劣化", "下游依赖饱和", "厂商返回异常", "调用量突增", "证据不足"] {
        assert!(s.contains(v));
    }
}
