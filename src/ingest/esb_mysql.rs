//! ESB 接口流水 MySQL wire 只读适配器：凭据全来自 env，缺失即 SourceUnavailable；
//! 列名走映射表（不硬编码）；DATETIME(3)→毫秒归一，原值留痕；参数化 SQL，按水位分页。

use crate::config::EsbConfig;
use crate::error::{AppError, ErrorCode, Result};
use sqlx::{Column, Row};
use std::collections::BTreeMap;
use std::str::FromStr;

/// 校验 ESB 凭据齐全（缺任一即 SourceUnavailable，不回退本地替身）
pub fn check_credentials(cfg: &EsbConfig) -> Result<()> {
    let mut missing = vec![];
    if cfg.host.is_empty() { missing.push("ESB_AUDIT_DB_HOST"); }
    if cfg.port == 0 { missing.push("ESB_AUDIT_DB_PORT"); }
    if cfg.user.is_empty() { missing.push("ESB_AUDIT_DB_USER"); }
    if cfg.password.is_empty() { missing.push("ESB_AUDIT_DB_PASSWORD"); }
    if cfg.db_name.is_empty() { missing.push("ESB_AUDIT_DB_NAME"); }
    if missing.is_empty() { Ok(()) } else {
        Err(AppError::new(ErrorCode::SourceUnavailable, format!("缺凭据: {}", missing.join(", "))))
    }
}

/// DATETIME(3) / ISO 时间戳 → ts_ms 毫秒；原值留痕。
/// 支持 "2026-11-08 20:00:00.123" 与 RFC3339。
pub fn datetime3_to_ms(raw: &str) -> Option<i64> {
    let s = raw.trim();
    for fmt in ["%Y-%m-%d %H:%M:%S%.3f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S%.3f%:z"] {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp_millis());
        }
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) { return Some(dt.timestamp_millis()); }
    // 纯毫秒
    if let Ok(n) = s.parse::<i64>() { return Some(n); }
    None
}

/// 把来源行（DB 列名→值）按映射表抽成规范列集合；未映射列落入 unmapped 计数。
pub fn map_db_row(row: &BTreeMap<String, String>, mapping: &[(&str, &str, bool)]) -> Result<MappedDbRow> {
    let mut canonical = BTreeMap::new();
    let mut unmapped = vec![];
    for (canon, col, required) in mapping {
        match row.get(*col) {
            Some(v) => { canonical.insert((*canon).to_string(), v.clone()); }
            None if *required => return Err(AppError::new(ErrorCode::SchemaMismatch, format!("缺必需列 {col}"))),
            None => {}
        }
    }
    for k in row.keys() {
        if !mapping.iter().any(|(_, col, _)| col == k) { unmapped.push(k.clone()); }
    }
    let ts_raw = canonical.get("ts_ms").cloned().unwrap_or_default();
    let ts_ms = datetime3_to_ms(&ts_raw).ok_or_else(|| {
        AppError::new(ErrorCode::SchemaMismatch, format!("时间归一失败: {}", crate::config::mask(&ts_raw)))
    })?;
    Ok(MappedDbRow { canonical, unmapped, ts_ms, ts_raw })
}

#[derive(Debug, Clone)]
pub struct MappedDbRow {
    pub canonical: BTreeMap<String, String>,
    pub unmapped: Vec<String>,
    pub ts_ms: i64,
    pub ts_raw: String,
}

/// 参数化分页 SQL（占位符 $1/$2，绝不拼字符串值）
pub fn page_query_sql(mapping_cols: &[&str]) -> String {
    let cols = mapping_cols.join(", ");
    format!("SELECT {cols} FROM esb_interface_audit WHERE request_time > $1 ORDER BY request_time, id LIMIT $2")
}

/// 真实连接串（不含口令明文进日志）；用 sqlx 运行时选项建立只读连接。
pub async fn connect(cfg: &EsbConfig) -> Result<sqlx::MySqlPool> {
    check_credentials(cfg)?;
    let url = format!("mysql://{}:{}@{}:{}/{}",
        cfg.user, cfg.password, cfg.host, cfg.port, cfg.db_name);
    let opts = sqlx::mysql::MySqlConnectOptions::from_str(&url)
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("连接串非法: {e}")))?;
    sqlx::MySqlPool::connect_with(opts).await
        .map_err(|_e| AppError::new(ErrorCode::SourceUnavailable, format!("ESB 库连接失败: {}", crate::config::redact_url(&cfg.host))))
}

/// 从连接池按水位拉一批（真实执行参数化查询）。测试环境无库时由调用方走 diagnose 的 fixture 路径。
pub async fn fetch_page(pool: &sqlx::MySqlPool, sql: &str, watermark: chrono::DateTime<chrono::Utc>, batch: i64) -> Result<Vec<BTreeMap<String, String>>> {
    let rows = sqlx::query(sql)
        .bind(watermark)
        .bind(batch)
        .fetch_all(pool).await
        .map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("查询失败: {e}")))?;
    let mut out = vec![];
    for r in rows {
        let mut m = BTreeMap::new();
        // 读取所有列名（运行时列集，配合可配置映射表）
        for (i, c) in r.columns().iter().enumerate() {
            let v: Option<String> = r.try_get::<Option<String>, _>(i).ok().flatten();
            m.insert(c.name().to_string(), v.unwrap_or_default());
        }
        out.push(m);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn esb() -> EsbConfig {
        EsbConfig { host: "db".into(), port: 3306, user: "ro".into(), password: "pw".into(), db_name: "audit".into() }
    }

    #[test]
    fn missing_credential_is_source_unavailable() {
        let mut c = esb();
        c.password.clear();
        assert_eq!(check_credentials(&c).unwrap_err().code, ErrorCode::SourceUnavailable);
        assert!(check_credentials(&esb()).is_ok());
    }

    #[test]
    fn datetime3_precision_normalized() {
        assert_eq!(datetime3_to_ms("2026-11-08 20:00:00.123"),
            Some(chrono::DateTime::parse_from_rfc3339("2026-11-08T20:00:00.123Z").unwrap().timestamp_millis()));
        assert_eq!(datetime3_to_ms("2026-11-08 20:00:00"),
            Some(chrono::DateTime::parse_from_rfc3339("2026-11-08T20:00:00Z").unwrap().timestamp_millis()));
    }

    #[test]
    fn column_mapping_and_unmapped_count() {
        let mut row = BTreeMap::new();
        row.insert("interface_code".into(), "A".into());
        row.insert("request_time".into(), "2026-11-08 20:00:00.123".into());
        row.insert("mystery".into(), "x".into());
        let mapping = [("interface_code", "interface_code", true), ("ts_ms", "request_time", true)];
        let mapped = map_db_row(&row, &mapping).unwrap();
        assert_eq!(mapped.canonical.get("interface_code").unwrap(), "A");
        assert!(mapped.unmapped.contains(&"mystery".to_string()));
        assert!(mapped.ts_ms > 0);
    }

    #[test]
    fn missing_required_column_rejected() {
        let row = BTreeMap::new();
        let mapping = [("interface_code", "interface_code", true)];
        assert_eq!(map_db_row(&row, &mapping).unwrap_err().code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn sql_is_parameterized() {
        let sql = page_query_sql(&["interface_code", "request_time"]);
        assert!(sql.contains("$1"));
        assert!(sql.contains("$2"));
        assert!(!sql.contains("WHERE request_time > '"));
    }
}
