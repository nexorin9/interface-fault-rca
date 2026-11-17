//! 统一运行事件表 schema：tags 六键白名单、ts_ms 毫秒、fields 按 event_kind 分支。
//! 越界 tag 键或 fields 键一律返回 SchemaMismatch。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::BTreeMap;

/// tags 白名单六键（spec.md §3.1）
pub const TAG_WHITELIST: [&str; 6] = [
    "source", "event_kind", "interface_code", "service_name", "vendor_name", "host",
];

/// 三种 event_kind 允许的 fields 键集合
pub const FLOW_FIELDS: [&str; 6] = [
    "response_code", "duration_ms", "caller_system", "raw_value", "pipeline_version", "unbound",
];
pub const LOG_FIELDS: [&str; 6] = [
    "level", "message", "stack_head", "raw_value", "pipeline_version", "unbound",
];
pub const METRIC_FIELDS: [&str; 8] = [
    "metric_name", "value", "labels", "raw_value", "pipeline_version", "host_deviation", "clock_shift_ms", "unbound",
];

pub fn allowed_fields(event_kind: &str) -> Option<&'static [&'static str]> {
    match event_kind {
        "flow" => Some(&FLOW_FIELDS),
        "log" => Some(&LOG_FIELDS),
        "metric" => Some(&METRIC_FIELDS),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub tags: BTreeMap<String, String>,
    pub ts_ms: i64,
    pub fields: BTreeMap<String, String>,
    pub row_id: String, // __row_id：证据行 ID
    pub tsid: String,   // __tsid：幂等键
}

impl EventRow {
    /// 构造并校验。tags 键必须在白名单；fields 键必须在对应 event_kind 允许集合。
    pub fn new(
        tags: BTreeMap<String, String>,
        ts_ms: i64,
        fields: BTreeMap<String, String>,
        row_id: String,
        tsid: String,
    ) -> Result<Self> {
        let row = Self { tags, ts_ms, fields, row_id, tsid };
        row.validate()?;
        Ok(row)
    }

    pub fn validate(&self) -> Result<()> {
        for k in self.tags.keys() {
            if !TAG_WHITELIST.contains(&k.as_str()) {
                return Err(AppError::new(ErrorCode::SchemaMismatch, format!("非法 tag 键: {k}")));
            }
        }
        let kind = self.tags.get("event_kind").map(|s| s.as_str()).unwrap_or("");
        let allowed = allowed_fields(kind).ok_or_else(|| {
            AppError::new(ErrorCode::SchemaMismatch, format!("未知 event_kind: {}", kind))
        })?;
        for k in self.fields.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(AppError::new(ErrorCode::SchemaMismatch, format!("越界 fields 键: {k} (kind={kind})")));
            }
        }
        Ok(())
    }

    pub fn source(&self) -> &str { self.tags.get("source").map(|s| s.as_str()).unwrap_or("") }
    pub fn kind(&self) -> &str { self.tags.get("event_kind").map(|s| s.as_str()).unwrap_or("") }
    pub fn interface_code(&self) -> &str { self.tags.get("interface_code").map(|s| s.as_str()).unwrap_or("") }
    pub fn service_name(&self) -> &str { self.tags.get("service_name").map(|s| s.as_str()).unwrap_or("") }
    pub fn is_unbound(&self) -> bool {
        self.fields.get("unbound").map(|v| v == "true").unwrap_or(false)
            || self.tags.get("interface_code").map(|v| v == "unbound").unwrap_or(false)
    }

    /// 证据原文摘要（用于终端输出与请求体，不含整段日志）
    pub fn evidence_summary(&self) -> String {
        let payload = match self.kind() {
            "flow" => format!("rc={} dur={}ms",
                self.fields.get("response_code").map(|s| s.as_str()).unwrap_or("-"),
                self.fields.get("duration_ms").map(|s| s.as_str()).unwrap_or("-")),
            "log" => {
                let msg = self.fields.get("message").map(|s| s.as_str()).unwrap_or("");
                let short: String = msg.chars().take(40).collect();
                format!("level={} msg={}", self.fields.get("level").map(|s| s.as_str()).unwrap_or("-"), short)
            }
            "metric" => format!("{}={}",
                self.fields.get("metric_name").map(|s| s.as_str()).unwrap_or("-"),
                self.fields.get("value").map(|s| s.as_str()).unwrap_or("-")),
            _ => String::new(),
        };
        format!("{} @{} {} [{}]", self.row_id, self.ts_ms, self.interface_code(), payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(kind: &str) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), "esb_audit".into());
        tags.insert("event_kind".into(), kind.into());
        tags.insert("interface_code".into(), "ESB-ORDER-CREATE".into());
        EventRow::new(tags, 1000, BTreeMap::new(), "flow#a0001".into(), "abc".into()).unwrap()
    }

    #[test]
    fn illegal_tag_key_is_schema_mismatch() {
        let mut r = sample("flow");
        r.tags.insert("bad_tag".into(), "x".into());
        assert_eq!(r.validate().unwrap_err().code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn out_of_range_field_key_is_schema_mismatch() {
        let mut r = sample("flow");
        r.fields.insert("secret_column".into(), "x".into());
        assert_eq!(r.validate().unwrap_err().code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn valid_rows_across_kinds_pass() {
        assert!(sample("flow").validate().is_ok());
        assert!(sample("log").validate().is_ok());
        let mut m = sample("metric");
        m.fields.insert("metric_name".into(), "p95".into());
        assert!(m.validate().is_ok());
    }
}
