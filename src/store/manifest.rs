//! manifest.json 自描述 schema（分区清单 / 列名类型 / tag 白名单 / 来源 / 写入游标 / 校验和）。
//! 一致性校验：manifest 声明与实际 schema 不符即报 SchemaMismatch，不自动改写掩盖。

use crate::error::{AppError, ErrorCode, Result};
use crate::store::event_schema::TAG_WHITELIST;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const SCHEMA_COLUMNS: [(&str, &str); 10] = [
    ("source", "Utf8"), ("event_kind", "Utf8"), ("interface_code", "Utf8"),
    ("service_name", "Utf8"), ("vendor_name", "Utf8"), ("host", "Utf8"),
    ("ts_ms", "Int64"), ("fields_json", "Utf8"), ("__row_id", "Utf8"), ("__tsid", "Utf8"),
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub version: String,
    pub columns: BTreeMap<String, String>,
    pub tag_whitelist: Vec<String>,
    pub sources: Vec<String>,
    pub partitions: Vec<String>,
    pub write_cursor: BTreeMap<String, i64>,
    pub checksum: String,
}

impl Manifest {
    pub fn new() -> Self {
        let mut columns = BTreeMap::new();
        for (n, t) in SCHEMA_COLUMNS { columns.insert(n.to_string(), t.to_string()); }
        Self {
            version: "1".to_string(),
            columns,
            tag_whitelist: TAG_WHITELIST.iter().map(|s| s.to_string()).collect(),
            sources: vec![], partitions: vec![], write_cursor: BTreeMap::new(),
            checksum: String::new(),
        }
    }

    pub fn path(dir: &Path) -> PathBuf { dir.join("manifest.json") }

    /// 计算校验和（对除 checksum 外的稳定序列化取 sha256）
    fn compute_checksum(&self) -> String {
        let mut clone = self.clone();
        clone.checksum.clear();
        let s = serde_json::to_string(&clone).unwrap_or_default();
        hex::encode(Sha256::digest(s.as_bytes()))
    }

    pub fn save(&mut self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("建目录失败: {e}")))?;
        self.checksum = self.compute_checksum();
        let txt = serde_json::to_string_pretty(self).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("序列化失败: {e}")))?;
        std::fs::write(Self::path(dir), txt).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("写 manifest 失败: {e}")))?;
        Ok(())
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let txt = std::fs::read_to_string(Self::path(dir))
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("读 manifest 失败: {e}")))?;
        let m: Manifest = serde_json::from_str(&txt)
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("manifest 解析失败，拒绝写入: {e}")))?;
        Ok(m)
    }

    /// 与实际 schema 一致性校验：列集 / 类型 / 白名单任一不符即报错并指出差异。
    pub fn validate_against(&self, actual_columns: &BTreeMap<String, String>) -> Result<()> {
        let mut diffs = Vec::new();
        for (name, ty) in &self.columns {
            match actual_columns.get(name) {
                None => diffs.push(format!("缺列 {name}")),
                Some(t) if t != ty => diffs.push(format!("列 {name} 类型 {}≠{}", t, ty)),
                _ => {}
            }
        }
        for name in actual_columns.keys() {
            if !self.columns.contains_key(name) { diffs.push(format!("多列 {name}")); }
        }
        if self.tag_whitelist != TAG_WHITELIST.iter().map(|s| s.to_string()).collect::<Vec<_>>() {
            diffs.push("tag 白名单不符".to_string());
        }
        if diffs.is_empty() { Ok(()) } else {
            Err(AppError::new(ErrorCode::SchemaMismatch, diffs.join("; ")))
        }
    }

    pub fn checksum_ok(&self) -> bool {
        self.checksum.is_empty() || self.checksum == self.compute_checksum()
    }

    pub fn add_partition(&mut self, p: &str) {
        if !self.partitions.iter().any(|x| x == p) { self.partitions.push(p.to_string()); }
    }
    pub fn add_source(&mut self, s: &str) {
        if !self.sources.iter().any(|x| x == s) { self.sources.push(s.to_string()); }
    }
}

/// 由 SCHEMA_COLUMNS 生成实际列描述（供校验对拍）
pub fn actual_columns() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for (n, t) in SCHEMA_COLUMNS { m.insert(n.to_string(), t.to_string()); }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_columns_match_actual_schema() {
        let m = Manifest::new();
        assert!(m.validate_against(&actual_columns()).is_ok());
    }

    #[test]
    fn column_mismatch_reports_diff() {
        let m = Manifest::new();
        let mut actual = actual_columns();
        actual.remove("ts_ms");
        let e = m.validate_against(&actual).unwrap_err();
        assert_eq!(e.code, ErrorCode::SchemaMismatch);
        assert!(e.detail.contains("ts_ms"));
    }

    #[test]
    fn save_load_roundtrip_and_cursor_resume() {
        let dir = tempfile::tempdir().unwrap();
        let mut m = Manifest::new();
        m.add_source("esb_audit");
        m.write_cursor.insert("esb_audit".into(), 12345);
        m.save(dir.path()).unwrap();
        let loaded = Manifest::load(dir.path()).unwrap();
        assert_eq!(loaded.write_cursor.get("esb_audit"), Some(&12345));
        assert!(loaded.checksum_ok());
        assert_eq!(loaded.sources, vec!["esb_audit"]);
    }

    #[test]
    fn corrupted_manifest_rejected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(Manifest::path(dir.path()), "{not json").unwrap();
        let e = Manifest::load(dir.path()).unwrap_err();
        assert_eq!(e.code, ErrorCode::ConfigMissing);
    }
}
