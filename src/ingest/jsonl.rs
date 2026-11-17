//! 统一运行事件表的行读取：JSONL「文件投递」形态的适配层。
//!
//! 桌面程序（`main.rs`）、金标准回归测试（`tests/gold_regression.rs`）共用这一份解析，
//! 保证「测试跑的主路径」与「产品跑的主路径」是同一条，不会出现两套口径。
//!
//! 时间戳一律归一到毫秒（spec.md §3.1 `timestamp.ts_ms`）：来源侧三种写法都收——
//! unix 秒 / unix 毫秒 / RFC3339 / MySQL `DATETIME(3)`（本地墙钟，按 UTC+8 解释）。

use crate::error::{AppError, ErrorCode, Result};
use crate::ingest::pipeline::RawRecord;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 归一到毫秒。整数 < 1e11 视为 unix 秒，否则视为毫秒。
fn ts_from_int(v: i64) -> i64 {
    if v.abs() < 100_000_000_000 { v.saturating_mul(1000) } else { v }
}

/// 时间戳归一：接受 unix 秒 / unix 毫秒 / RFC3339 / MySQL DATETIME(3)。
/// 无法解释时返回 SchemaMismatch（不静默塞 0，避免脏行伪装成正常行）。
pub fn normalize_ts_ms(value: &Value) -> Result<i64> {
    match value {
        Value::Number(n) => n.as_i64().map(ts_from_int).ok_or_else(|| {
            AppError::new(ErrorCode::SchemaMismatch, format!("时间戳不是整数: {n}"))
        }),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Err(AppError::new(ErrorCode::SchemaMismatch, "时间戳为空".to_string()));
            }
            if let Ok(i) = t.parse::<i64>() {
                return Ok(ts_from_int(i));
            }
            if let Ok(d) = chrono::DateTime::parse_from_rfc3339(t) {
                return Ok(d.timestamp_millis());
            }
            // MySQL DATETIME(3)：无时区墙钟，按 UTC 解释
            // （与 esb_mysql::datetime3_to_ms、log_tail::normalize_ts 的口径保持一致）
            for fmt in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S%.f"] {
                if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(t, fmt) {
                    return Ok(naive.and_utc().timestamp_millis());
                }
            }
            Err(AppError::new(ErrorCode::SchemaMismatch, format!("时间戳无法解释: {t}")))
        }
        _ => Err(AppError::new(ErrorCode::SchemaMismatch, "时间戳类型不支持（需整数或字符串）".to_string())),
    }
}

/// 行内时间戳字段的候选名（按来源实际列名；见 config/pipelines.toml 的 type 提示）
const TS_KEYS: [&str; 6] = ["ts_ms", "request_time", "timestamp", "__ts", "time", "event_time"];

/// 一行 JSON → RawRecord。
/// `default_source` 是文件级来源（一个投递目录对应一路来源）；行内 `source` 字段优先。
pub fn record_from_json(v: &Value, default_source: &str) -> Result<RawRecord> {
    let obj = v.as_object().ok_or_else(|| {
        AppError::new(ErrorCode::SchemaMismatch, "行不是 JSON 对象".to_string())
    })?;
    let source = obj.get("source").and_then(|x| x.as_str()).unwrap_or(default_source).to_string();
    if source.trim().is_empty() {
        return Err(AppError::new(ErrorCode::SchemaMismatch, "行缺 source（文件级来源也为空）".to_string()));
    }
    let mut ts_value: Option<&Value> = None;
    let mut ts_key: Option<&str> = None;
    for k in TS_KEYS {
        if let Some(val) = obj.get(k) {
            if !val.is_null() {
                ts_value = Some(val);
                ts_key = Some(k);
                break;
            }
        }
    }
    let ts_value = ts_value.ok_or_else(|| {
        AppError::new(ErrorCode::SchemaMismatch, format!(
            "行缺时间戳（试过 {}）", TS_KEYS.join("/")))
    })?;
    let ts_ms = normalize_ts_ms(ts_value)?;

    let mut columns = BTreeMap::new();
    for (k, val) in obj {
        if k == "source" || Some(k.as_str()) == ts_key {
            continue;
        }
        let s = match val {
            Value::String(x) => x.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        columns.insert(k.clone(), s);
    }
    // Prometheus 的 __name / __value 归一到规范字段名（config/pipelines.toml 的 prom 映射口径）
    if let Some(v) = columns.remove("__name") { columns.insert("metric_name".into(), v); }
    if let Some(v) = columns.remove("__value") { columns.insert("value".into(), v); }
    Ok(RawRecord { source, columns, ts_ms })
}

/// 解析一行 JSONL 文本（坏行报 SchemaMismatch，不静默丢弃）。
pub fn parse_line(line: &str, default_source: &str) -> Result<RawRecord> {
    let v: Value = serde_json::from_str(line)
        .map_err(|e| AppError::new(ErrorCode::SchemaMismatch, format!("JSON 解析失败: {e}")))?;
    record_from_json(&v, default_source)
}

/// 读取一个 JSONL 文件。`default_source` 为该文件对应的来源。
pub fn load_file(path: &Path, default_source: &str) -> Result<Vec<RawRecord>> {
    let txt = std::fs::read_to_string(path).map_err(|e| {
        AppError::new(ErrorCode::SourceUnavailable, format!("读 {} 失败: {e}", path.display()))
    })?;
    let mut out = vec![];
    for (i, line) in txt.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match parse_line(line, default_source) {
            Ok(r) => out.push(r),
            Err(e) => {
                return Err(AppError::new(e.code, format!("{} 第 {} 行: {}", path.display(), i + 1, e.detail)));
            }
        }
    }
    Ok(out)
}

/// 读取一个投递目录下的全部 `*.jsonl`（按文件名排序，保证可复现）。
/// 文件名里的来源前缀（`<source>-<kind>.jsonl`）决定该文件的默认来源。
pub fn load_dir(dir: &Path) -> Result<Vec<RawRecord>> {
    if dir.is_file() {
        let stem = dir.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        return load_file(dir, source_from_stem(stem));
    }
    let rd = std::fs::read_dir(dir).map_err(|e| {
        AppError::new(ErrorCode::SourceUnavailable, format!("读目录 {} 失败: {e}", dir.display()))
    })?;
    let mut files: Vec<PathBuf> = rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(AppError::new(ErrorCode::SourceUnavailable, format!("目录 {} 下没有 .jsonl 文件", dir.display())));
    }
    let mut out = vec![];
    for f in files {
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        out.extend(load_file(&f, source_from_stem(stem))?);
    }
    Ok(out)
}

/// 文件名 → 默认来源：`esb_audit-flow.jsonl` → `esb_audit`；`app_log-*.jsonl` → `app_log`。
pub fn source_from_stem(stem: &str) -> &'static str {
    if stem.starts_with("esb_audit") || stem.starts_with("flow") || stem.starts_with("esb") {
        "esb_audit"
    } else if stem.starts_with("prom") || stem.starts_with("metric") {
        "prom"
    } else {
        "app_log"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_second_and_millisecond_both_normalize() {
        let s = serde_json::json!({"source":"esb_audit","ts_ms":1762588800,"interface_code":"A"});
        let r = record_from_json(&s, "esb_audit").unwrap();
        assert_eq!(r.ts_ms, 1762588800000);
        let m = serde_json::json!({"source":"esb_audit","ts_ms":1762588800000i64,"interface_code":"A"});
        assert_eq!(record_from_json(&m, "esb_audit").unwrap().ts_ms, 1762588800000);
    }

    #[test]
    fn mysql_datetime3_read_as_utc_consistent_with_esb_adapter() {
        let s = serde_json::json!({"request_time":"2026-11-09 02:15:00.123","interface_code":"A"});
        let r = record_from_json(&s, "esb_audit").unwrap();
        // 与 esb_mysql::datetime3_to_ms 同口径：无时区墙钟按 UTC 解释
        let expected = crate::ingest::esb_mysql::datetime3_to_ms("2026-11-09 02:15:00.123").unwrap();
        assert_eq!(r.ts_ms, expected);
        assert_eq!(r.ts_ms, chrono::DateTime::parse_from_rfc3339("2026-11-09T02:15:00.123Z").unwrap().timestamp_millis());
    }

    #[test]
    fn bad_json_is_schema_mismatch() {
        let e = parse_line("{not json", "esb_audit").unwrap_err();
        assert_eq!(e.code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn missing_timestamp_is_schema_mismatch_not_zero() {
        let e = parse_line("{\"interface_code\":\"A\"}", "esb_audit").unwrap_err();
        assert_eq!(e.code, ErrorCode::SchemaMismatch);
    }

    #[test]
    fn columns_keep_source_column_names() {
        let s = serde_json::json!({"request_time":"2026-11-09 02:15:00.123","interface_code":"ESB-X",
            "service_name":"某服务","response_code":200,"duration_ms":88,"caller_system":"HIS"});
        let r = record_from_json(&s, "esb_audit").unwrap();
        assert_eq!(r.columns.get("duration_ms").unwrap(), "88");
        assert_eq!(r.columns.get("response_code").unwrap(), "200");
        assert!(!r.columns.contains_key("request_time"));
        assert!(!r.columns.contains_key("source"));
    }

    #[test]
    fn stem_maps_to_default_source() {
        assert_eq!(source_from_stem("esb_audit-flow"), "esb_audit");
        assert_eq!(source_from_stem("prom-metric"), "prom");
        assert_eq!(source_from_stem("app_log-errors"), "app_log");
    }
}
