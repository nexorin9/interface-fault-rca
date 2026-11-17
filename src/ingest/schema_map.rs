//! 列名映射表：来源列 → 规范字段（不硬编码）；schema 漂移检测（缺必需列/多列/类型不符）
//! 显式报错并整批拒绝，不写半份数据。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug, Clone)]
pub struct ColumnRule {
    pub column: String,
    pub required: bool,
    pub type_hint: String,
}

#[derive(Debug, Clone)]
pub struct MappingTable {
    /// 规范字段 → 来源列规则
    pub rules: BTreeMap<String, ColumnRule>,
    /// unmapped 字段落点键
    pub unmapped_key: String,
}

impl MappingTable {
    pub fn new() -> Self {
        Self { rules: BTreeMap::new(), unmapped_key: "unmapped".into() }
    }

    pub fn add(&mut self, canonical: &str, column: &str, required: bool, type_hint: &str) {
        self.rules.insert(canonical.to_string(), ColumnRule {
            column: column.to_string(), required, type_hint: type_hint.to_string(),
        });
    }

    /// 检测 schema 漂移：返回 (缺失必需列, 新增列, 类型不符列)。
    pub fn detect_drift(&self, actual: &[String], actual_types: &HashMap<String, String>) -> DriftReport {
        let declared: Vec<&str> = self.rules.values().map(|r| r.column.as_str()).collect();
        let mut missing_required = vec![];
        for r in self.rules.values() {
            if r.required && !actual.iter().any(|c| c == &r.column) {
                missing_required.push(r.column.clone());
            }
        }
        let mut new_cols = vec![];
        for c in actual {
            if !declared.contains(&c.as_str()) { new_cols.push(c.clone()); }
        }
        let mut type_mismatch = vec![];
        for r in self.rules.values() {
            if !r.type_hint.is_empty() {
                if let Some(t) = actual_types.get(&r.column) {
                    if !type_compatible(&r.type_hint, t) {
                        type_mismatch.push(format!("{}:{}≠{}", r.column, t, r.type_hint));
                    }
                }
            }
        }
        DriftReport { missing_required, new_columns: new_cols, type_mismatch }
    }

    /// 把一行来源字段（来源列名 → 值）映射成规范字段集合；未映射列落入 unmapped 并计数。
    pub fn map_row(&self, row: &BTreeMap<String, String>) -> MappedRow {
        let mut fields = BTreeMap::new();
        let mut unmapped = BTreeMap::new();
        for (canonical, rule) in &self.rules {
            if let Some(v) = row.get(&rule.column) {
                // ts_ms 与 tags 由调用方取；这里先都放 fields/key
                if canonical == "ts_ms" || ["source", "event_kind", "interface_code", "service_name", "vendor_name", "host"].contains(&canonical.as_str()) {
                    continue;
                }
                fields.insert(canonical.clone(), v.clone());
            }
        }
        let declared: Vec<&str> = self.rules.values().map(|r| r.column.as_str()).collect();
        for (k, v) in row {
            if !declared.contains(&k.as_str()) { unmapped.insert(k.clone(), v.clone()); }
        }
        MappedRow { fields, unmapped }
    }
}

/// 类型兼容（宽松：来源类型包含期望关键字即视为兼容；int 接受 bigint 等）
fn type_compatible(hint: &str, actual: &str) -> bool {
    let h = hint.to_lowercase();
    let a = actual.to_lowercase();
    match h.as_str() {
        "int" => a.contains("int") || a == "number" || a == "numeric",
        "float" => a.contains("float") || a.contains("double") || a == "number" || a == "numeric",
        "datetime3" => a.contains("time") || a.contains("date") || a == "string",
        "unix_second" | "unix_millisecond" => a.contains("int") || a == "number" || a == "string",
        _ => true,
    }
}

#[derive(Debug, Clone, Default)]
pub struct DriftReport {
    pub missing_required: Vec<String>,
    pub new_columns: Vec<String>,
    pub type_mismatch: Vec<String>,
}

impl DriftReport {
    pub fn has_blocking(&self) -> bool {
        !self.missing_required.is_empty() || !self.type_mismatch.is_empty()
    }
    pub fn blocking_error(&self) -> Result<()> {
        if !self.missing_required.is_empty() {
            return Err(AppError::new(ErrorCode::SchemaMismatch,
                format!("缺必需列: {}", self.missing_required.join(", "))));
        }
        if !self.type_mismatch.is_empty() {
            return Err(AppError::new(ErrorCode::SchemaMismatch,
                format!("类型不符: {}", self.type_mismatch.join(", "))));
        }
        Ok(())
    }
    pub fn summary(&self) -> String {
        format!("missing_required={:?} new={:?} type_mismatch={:?}",
            self.missing_required, self.new_columns, self.type_mismatch)
    }
}

#[derive(Debug, Clone)]
pub struct MappedRow {
    pub fields: BTreeMap<String, String>,
    pub unmapped: BTreeMap<String, String>,
}

pub fn esb_mapping() -> MappingTable {
    let mut m = MappingTable::new();
    m.add("interface_code", "interface_code", true, "");
    m.add("service_name", "service_name", true, "");
    m.add("vendor_name", "vendor_name", false, "");
    m.add("host", "host", false, "");
    m.add("ts_ms", "request_time", true, "datetime3");
    m.add("response_code", "response_code", true, "int");
    m.add("duration_ms", "duration_ms", true, "int");
    m.add("caller_system", "caller_system", false, "");
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_required_column_is_blocking() {
        let m = esb_mapping();
        let actual = vec!["interface_code".into(), "service_name".into()];
        let drift = m.detect_drift(&actual, &HashMap::new());
        assert!(drift.has_blocking());
        assert_eq!(drift.blocking_error().unwrap_err().code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn new_column_is_recorded_not_blocking() {
        let m = esb_mapping();
        let actual = vec!["interface_code".into(), "service_name".into(), "request_time".into(),
            "response_code".into(), "duration_ms".into(), "future_column".into()];
        let drift = m.detect_drift(&actual, &HashMap::new());
        assert!(!drift.has_blocking());
        assert!(drift.new_columns.contains(&"future_column".to_string()));
    }

    #[test]
    fn type_mismatch_is_blocking() {
        let m = esb_mapping();
        let actual = vec!["interface_code".into(), "service_name".into(), "request_time".into(),
            "response_code".into(), "duration_ms".into()];
        let mut types = HashMap::new();
        types.insert("duration_ms".to_string(), "varchar".to_string());
        let drift = m.detect_drift(&actual, &types);
        assert!(drift.has_blocking());
        assert!(drift.type_mismatch.iter().any(|s| s.contains("duration_ms")));
    }

    #[test]
    fn unmapped_columns_counted() {
        let m = esb_mapping();
        let mut row = BTreeMap::new();
        row.insert("interface_code".into(), "A".into());
        row.insert("mystery_col".into(), "x".into());
        let mapped = m.map_row(&row);
        assert!(mapped.unmapped.contains_key("mystery_col"));
    }
}
