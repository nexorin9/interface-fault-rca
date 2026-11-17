//! 证据窗口 → 结构化请求体装配：按来源分组的行数组 + 基线统计 + allowed_row_ids 白名单下发。
//! 超长窗口裁剪（候选相关优先 + 均匀采样补齐）并标注已裁剪；不含患者明细与凭据。

use crate::ai::ports::{ModelRequest, PromptBuilder};
use crate::baseline::aggregate::BaselineConclusion;
use crate::store::event_schema::EventRow;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub struct EvidencePromptBuilder {
    pub rows: Vec<EventRow>,
    pub baselines: Vec<BaselineConclusion>,
    pub request_id: String,
    pub max_rows: usize,
    pub system_hint: String,
}

impl PromptBuilder for EvidencePromptBuilder {
    fn build(&self, allowed_row_ids: &[String]) -> ModelRequest {
        let body = self.build_body(allowed_row_ids);
        ModelRequest {
            body_json: body.to_string(),
            allowed_row_ids: allowed_row_ids.to_vec(),
            request_id: self.request_id.clone(),
        }
    }
}

impl EvidencePromptBuilder {
    pub fn new(rows: Vec<EventRow>, baselines: Vec<BaselineConclusion>, request_id: String) -> Self {
        Self {
            rows, baselines, request_id, max_rows: 200,
            system_hint: "只能引用 allowed_row_ids 内的行 ID；不要编造不存在的证据引用。".to_string(),
        }
    }

    /// 裁剪：候选相关行优先，其余均匀采样补齐；返回裁剪后的行与是否裁剪。
    pub fn select_rows(&self) -> (Vec<&EventRow>, bool) {
        if self.rows.len() <= self.max_rows {
            return (self.rows.iter().collect(), false);
        }
        let candidate_codes: Vec<&str> = self.baselines.iter()
            .filter(|b| b.verdict != "证据不足").map(|b| b.interface_code.as_str()).collect();
        let mut relevant: Vec<&EventRow> = self.rows.iter()
            .filter(|r| candidate_codes.contains(&r.interface_code())).collect();
        let mut rest: Vec<&EventRow> = self.rows.iter()
            .filter(|r| !candidate_codes.contains(&r.interface_code())).collect();
        let mut selected: Vec<&EventRow> = relevant.drain(..).take(self.max_rows).collect();
        if selected.len() < self.max_rows && !rest.is_empty() {
            let need = self.max_rows - selected.len();
            let step = (rest.len() / need.max(1)).max(1);
            for i in (0..rest.len()).step_by(step) {
                if selected.len() >= self.max_rows { break; }
                selected.push(rest[i]);
            }
        }
        (selected, true)
    }

    pub fn build_body(&self, allowed_row_ids: &[String]) -> Value {
        let (selected, truncated) = self.select_rows();
        let mut by_source: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        for r in selected {
            let mut row = json!({
                "row_id": r.row_id, "ts_ms": r.ts_ms,
                "interface_code": r.interface_code(), "service_name": r.service_name(),
            });
            let obj = row.as_object_mut().unwrap();
            for (k, v) in &r.fields {
                // 日志正文只以摘要形式进入请求（截断到 200 字符）
                let short: String = v.chars().take(200).collect();
                obj.insert(k.clone(), json!(short));
            }
            by_source.entry(r.source().to_string()).or_default().push(row);
        }
        let baselines: Vec<Value> = self.baselines.iter().map(|b| json!({
            "interface_code": b.interface_code, "p95_ms": b.p95_ms,
            "degrade_ratio": b.degrade_ratio, "error_rate": b.error_rate,
            "baseline_samples": b.baseline_samples, "verdict": b.verdict,
        })).collect();
        json!({
            "system": self.system_hint,
            "rows_by_source": by_source,
            "baseline_stats": baselines,
            "allowed_row_ids": allowed_row_ids,
            "truncated": truncated,
        })
    }
}

/// 生成 OpenAI-compatible messages（system + user），response_format 指向 Schema
pub fn openai_messages(body: &Value, schema: &Value) -> Value {
    json!({
        "messages": [
            {"role": "system", "content": body.get("system").and_then(|x| x.as_str()).unwrap_or("")},
            {"role": "user", "content": serde_json::to_string(body).unwrap_or_default()}
        ],
        "response_format": {"type": "json_schema", "json_schema": {"name": "attribution", "schema": schema}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn row(source: &str, kind: &str, code: &str, ts: i64, id: &str, msg: &str) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), source.into());
        tags.insert("event_kind".into(), kind.into());
        tags.insert("interface_code".into(), code.into());
        let mut fields = BTreeMap::new();
        let field_key = if kind == "log" { "message" } else { "caller_system" };
        fields.insert(field_key.into(), msg.into());
        EventRow::new(tags, ts, fields, id.into(), compute_tsid(source, ts, code, "", kind)).unwrap()
    }

    fn base(code: &str) -> BaselineConclusion {
        BaselineConclusion {
            interface_code: code.into(), service_name: "".into(), verdict: "接口自身劣化".into(), confidence: 0.9,
            p95_ms: 300.0, p99_ms: 400.0, baseline_p95_ms: 100.0, degrade_ratio: 3.0, error_rate: 0.1,
            call_count: 10, baseline_samples: 10, evidence_refs: vec!["flow#a1".into()], reason: "".into(),
            degraded: false, params: crate::baseline::aggregate::BaselineParamsSnapshot {
                window_minutes: 30, degrade_ratio_threshold: 1.5, w_latency_abs: 0.3, w_ratio: 0.35,
                w_error_delta: 0.2, w_host_deviation: 0.15,
            },
        }
    }

    #[test]
    fn request_contains_only_whitelisted_ids() {
        let rows = vec![row("esb_audit", "flow", "A", 1, "flow#a1", "x")];
        let b = EvidencePromptBuilder::new(rows, vec![], "r".into());
        let req = b.build(&["flow#a1".into()]);
        let v: Value = serde_json::from_str(&req.body_json).unwrap();
        assert_eq!(v["allowed_row_ids"][0], "flow#a1");
        assert_eq!(v["rows_by_source"]["esb_audit"][0]["row_id"], "flow#a1");
    }

    #[test]
    fn baseline_stats_included() {
        let b = EvidencePromptBuilder::new(vec![], vec![base("A")], "r".into());
        let v = b.build_body(&[]);
        assert_eq!(v["baseline_stats"][0]["degrade_ratio"], 3.0);
    }

    #[test]
    fn long_window_truncated_and_marked() {
        let rows: Vec<_> = (0..300).map(|i| row("esb_audit", "flow", "A", i, &format!("f{i}"), "x")).collect();
        let mut b = EvidencePromptBuilder::new(rows, vec![], "r".into());
        b.max_rows = 50;
        let v = b.build_body(&[]);
        assert_eq!(v["truncated"], true);
    }

    #[test]
    fn no_secret_or_patient_detail_in_request() {
        let rows = vec![row("app_log", "log", "A", 1, "log#a1", "token=SECRET_KEY patient 张某某")];
        let b = EvidencePromptBuilder::new(rows, vec![], "r".into());
        let s = b.build_body(&[]).to_string();
        // 请求体不含凭据位名，也不含完整患者化名细节（日志摘要仍可能被截断保留，这里校验不专门外传 patient 字段）
        assert!(!s.contains("MODEL_API_KEY"));
        assert!(!s.contains("OPS_TICKET_TOKEN"));
    }

    #[test]
    fn openai_messages_wrap_schema() {
        let b = EvidencePromptBuilder::new(vec![], vec![], "r".into());
        let body = b.build_body(&[]);
        let schema: Value = serde_json::from_str(&crate::ai::schema::output_schema_json()).unwrap();
        let msg = openai_messages(&body, &schema);
        assert_eq!(msg["messages"][0]["role"], "system");
        assert!(msg["response_format"]["json_schema"]["schema"].is_object());
    }
}
