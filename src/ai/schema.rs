//! 严格 JSON Schema 的结构化输出校验：类型 / 必填字段 / verdict 枚举 /
//! confidence 0..1 / evidence_refs 字符串数组，任一不符即校验失败（不"尽力修补"）。

use crate::ai::conclusion::Verdict;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub reason: String,
}

/// 校验模型结构化输出根对象
pub fn validate_output(v: &Value) -> std::result::Result<(), Vec<ValidationError>> {
    let mut errs = vec![];
    let obj = match v.as_object() {
        Some(o) => o,
        None => return Err(vec![ValidationError { field: "$".into(), reason: "非对象".into() }]),
    };
    let candidates = match obj.get("candidates").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return Err(vec![ValidationError { field: "candidates".into(), reason: "缺失或非数组".into() }]),
    };
    if !obj.contains_key("degraded") || !obj["degraded"].is_boolean() {
        errs.push(ValidationError { field: "degraded".into(), reason: "缺失或非布尔".into() });
    }
    for (i, c) in candidates.iter().enumerate() {
        validate_candidate(c, i, &mut errs);
    }
    if errs.is_empty() { Ok(()) } else { Err(errs) }
}

fn validate_candidate(c: &Value, i: usize, errs: &mut Vec<ValidationError>) {
    let prefix = format!("candidates[{i}]");
    let required = ["interface_code", "service_name", "verdict", "confidence",
                    "evidence_refs", "reasoning_summary", "next_check"];
    for k in required {
        if !c.get(k).is_some() {
            errs.push(ValidationError { field: format!("{prefix}.{k}"), reason: "缺失".into() });
        }
    }
    if let Some(ver) = c.get("verdict").and_then(|x| x.as_str()) {
        if Verdict::parse(ver).is_none() {
            errs.push(ValidationError { field: format!("{prefix}.verdict"), reason: format!("非法枚举 {ver}") });
        }
    }
    if let Some(conf) = c.get("confidence").and_then(|x| x.as_f64()) {
        if !(0.0..=1.0).contains(&conf) {
            errs.push(ValidationError { field: format!("{prefix}.confidence"), reason: "越界 0..1".into() });
        }
    }
    if let Some(refs) = c.get("evidence_refs") {
        match refs.as_array() {
            Some(a) if a.iter().all(|x| x.is_string()) => {}
            _ => errs.push(ValidationError { field: format!("{prefix}.evidence_refs"), reason: "非字符串数组".into() }),
        }
    }
    for k in ["interface_code", "service_name", "reasoning_summary", "next_check"] {
        if let Some(v) = c.get(k) {
            if !v.is_string() {
                errs.push(ValidationError { field: format!("{prefix}.{k}"), reason: "非字符串".into() });
            }
        }
    }
}

/// OpenAI-compatible 响应体 Schema 描述（供协议层 response_format 使用）
pub fn output_schema_json() -> String {
    let verdicts: Vec<&str> = Verdict::all().iter().map(|v| v.as_str()).collect();
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "candidates": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "interface_code": {"type": "string"},
                        "service_name": {"type": "string"},
                        "verdict": {"type": "string", "enum": verdicts},
                        "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0},
                        "evidence_refs": {"type": "array", "items": {"type": "string"}},
                        "reasoning_summary": {"type": "string"},
                        "next_check": {"type": "string"}
                    },
                    "required": ["interface_code", "service_name", "verdict", "confidence",
                                 "evidence_refs", "reasoning_summary", "next_check"]
                }
            },
            "degraded": {"type": "boolean"}
        },
        "required": ["candidates", "degraded"]
    });
    schema.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> Value {
        serde_json::from_str(r#"{"candidates":[{"interface_code":"A","service_name":"S","verdict":"接口自身劣化","confidence":0.9,"evidence_refs":["flow#a1"],"reasoning_summary":"r","next_check":"n"}],"degraded":false}"#).unwrap()
    }

    #[test]
    fn valid_output_passes() {
        assert!(validate_output(&valid()).is_ok());
    }

    #[test]
    fn illegal_verdict_fails() {
        let mut v = valid();
        v["candidates"][0]["verdict"] = Value::String("随便".into());
        assert!(validate_output(&v).is_err());
    }

    #[test]
    fn confidence_out_of_range_fails() {
        let mut v = valid();
        v["candidates"][0]["confidence"] = serde_json::json!(1.5);
        assert!(validate_output(&v).is_err());
    }

    #[test]
    fn evidence_refs_non_string_fails() {
        let mut v = valid();
        v["candidates"][0]["evidence_refs"] = serde_json::json!([123]);
        assert!(validate_output(&v).is_err());
    }

    #[test]
    fn missing_required_field_fails() {
        let mut v = valid();
        v["candidates"][0].as_object_mut().unwrap().remove("next_check");
        assert!(validate_output(&v).is_err());
    }

    #[test]
    fn schema_has_five_verdicts() {
        let s = output_schema_json();
        for v in Verdict::all() { assert!(s.contains(v.as_str())); }
    }
}
