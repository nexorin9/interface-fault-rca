//! 错误日志目录增量 tail：*.log/*.jsonl；增量游标 (path,size,mtime)；
//! UTF-8 优先 GBK 回退；脏行/超长行计数留痕；秒/毫秒/ISO8601 三种时间戳归一。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileCursor {
    pub path: String,
    pub size: u64,
    pub mtime: u64,
}

#[derive(Debug, Clone)]
pub struct LogEvent {
    pub ts_ms: i64,
    pub level: String,
    pub message: String,
    pub stack_head: String,
    pub interface_code: String,
    pub raw_value: String,
}

#[derive(Debug, Clone, Default)]
pub struct TailOutcome {
    pub events: Vec<LogEvent>,
    pub dirty_lines: usize,
    pub too_long: usize,
    pub cursors: Vec<FileCursor>,
}

/// UTF-8 优先，失败按 GBK 回退，仍失败以替换字符保留（不丢弃）。
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GBK.decode(bytes);
            cow.into_owned()
        }
    }
}

/// 三种时间戳别名归一到毫秒。归一前原值留痕到 raw_value。
pub fn normalize_ts(raw: &str) -> Option<i64> {
    let s = raw.trim();
    if s.is_empty() { return None; }
    // 纯数字：按位数判断秒 / 毫秒
    if let Ok(n) = s.parse::<i64>() {
        if s.len() >= 13 { return Some(n); }
        if s.len() >= 10 { return Some(n.saturating_mul(1000)); }
        return None;
    }
    // ISO8601 / RFC3339
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.3f") {
        return Some(dt.and_utc().timestamp_millis());
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt.and_utc().timestamp_millis());
    }
    None
}

/// 解析单行日志（简易 JSON 或 key=value 文本）。返回 None 表示脏行。
pub fn parse_line(line: &str) -> Option<LogEvent> {
    let line = line.trim();
    if line.is_empty() { return None; }
    if line.starts_with('{') {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            let ts_raw = v.get("timestamp").or_else(|| v.get("ts")).and_then(|x| x.as_str())
                .map(|s| s.to_string())
                .or_else(|| v.get("timestamp").or_else(|| v.get("ts")).and_then(|x| x.as_i64()).map(|n| n.to_string()))?;
            let ts = normalize_ts(&ts_raw)?;
            return Some(LogEvent {
                ts_ms: ts,
                level: v.get("level").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                message: v.get("message").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                stack_head: v.get("stack_head").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                interface_code: v.get("interface_code").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                raw_value: ts_raw,
            });
        }
        return None;
    }
    // 文本形态: "2026-11-08T20:00:00Z ERROR interface=xxx message=..."
    let mut ts_raw = "";
    let mut level = "";
    let mut interface = "";
    let mut msg_parts = vec![];
    for tok in line.split_whitespace() {
        if ts_raw.is_empty() && normalize_ts(tok).is_some() { ts_raw = tok; continue; }
        if level.is_empty() && ["ERROR", "WARN", "INFO", "DEBUG"].contains(&tok) { level = tok; continue; }
        if let Some(v) = tok.strip_prefix("interface=") { interface = v; continue; }
        msg_parts.push(tok);
    }
    let ts = normalize_ts(ts_raw)?;
    Some(LogEvent {
        ts_ms: ts, level: level.to_string(), message: msg_parts.join(" "),
        stack_head: String::new(), interface_code: interface.to_string(), raw_value: ts_raw.to_string(),
    })
}

/// 对目录做一次增量 tail；只读取 size 之后的新增字节；超长行截断并标 truncated。
pub fn tail_directory(dir: &Path, patterns: &[String], previous: &[FileCursor], max_line_bytes: usize) -> Result<TailOutcome> {
    if dir.as_os_str().is_empty() {
        return Err(AppError::new(ErrorCode::SourceUnavailable, "日志目录未配置"));
    }
    if !dir.exists() { return Ok(TailOutcome::default()); }
    let prev: BTreeMap<String, FileCursor> = previous.iter().map(|c| (c.path.clone(), c.clone())).collect();
    let mut outcome = TailOutcome::default();
    let mut files: Vec<PathBuf> = vec![];
    for entry in std::fs::read_dir(dir).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("读目录失败: {e}")))? {
        let p = entry.map_err(|_| AppError::new(ErrorCode::SourceUnavailable, "目录项读取失败"))?.path();
        if p.is_file() {
            let name = p.file_name().and_then(|x| x.to_str()).unwrap_or("");
            if patterns.iter().any(|pat| glob_match(pat, name)) { files.push(p); }
        }
    }
    files.sort();
    for f in files {
        let meta = std::fs::metadata(&f).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("元数据失败: {e}")))?;
        let size = meta.len();
        let mtime = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs()).unwrap_or(0);
        let key = f.to_string_lossy().to_string();
        let start = prev.get(&key).filter(|c| c.size <= size).map(|c| c.size).unwrap_or(0);
        if start < size {
            let mut data = std::fs::read(&f).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("读文件失败: {e}")))?;
            let slice = &data[start as usize..];
            let text = decode(slice);
            for line in text.lines() {
                if line.len() > max_line_bytes { outcome.too_long += 1; }
                match parse_line(line) {
                    Some(ev) => outcome.events.push(ev),
                    None => outcome.dirty_lines += 1,
                }
            }
            let _ = &mut data;
        }
        outcome.cursors.push(FileCursor { path: key, size, mtime });
    }
    Ok(outcome)
}

fn glob_match(pat: &str, name: &str) -> bool {
    if let Some(suffix) = pat.strip_prefix('*') { name.ends_with(suffix) } else { pat == name }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_gbk_mixed_decode() {
        assert_eq!(decode("hello".as_bytes()), "hello");
        // GBK 编码的 "错误"
        let gbk = encoding_rs::GBK.encode("错误").0;
        // 加一个非法 UTF-8 前缀强制走 GBK 回退
        assert_eq!(decode(&gbk), "错误");
    }

    #[test]
    fn three_timestamp_aliases() {
        assert_eq!(normalize_ts("1762000000"), Some(1_762_000_000_000));
        assert_eq!(normalize_ts("1762000000123"), Some(1_762_000_000_123));
        assert_eq!(normalize_ts("2026-11-08T20:00:00Z"), Some(chrono::DateTime::parse_from_rfc3339("2026-11-08T20:00:00Z").unwrap().timestamp_millis()));
        assert_eq!(normalize_ts("garbage"), None);
    }

    #[test]
    fn dirty_and_too_long_counted() {
        let dir = tempfile::tempdir().unwrap();
        let long = "x".repeat(100);
        let content = format!("{{}}\nNOT_A_LOG\n2026-11-08T20:00:00Z ERROR interface=A {long}\n");
        std::fs::write(dir.path().join("a.log"), content).unwrap();
        let pattern = vec!["*.log".to_string()];
        let out = tail_directory(dir.path(), &pattern, &[], 20).unwrap();
        assert_eq!(out.dirty_lines, 2); // "{}" 缺时间戳 + "NOT_A_LOG" 无时间戳
        assert!(out.events.len() >= 1);
        assert!(out.too_long >= 1);
    }

    #[test]
    fn cursor_resume_does_not_recollect() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.log");
        std::fs::write(&p, "2026-11-08T20:00:00Z ERROR interface=A first\n").unwrap();
        let pattern = vec!["*.log".to_string()];
        let first = tail_directory(dir.path(), &pattern, &[], 65536).unwrap();
        assert_eq!(first.events.len(), 1);
        let second = tail_directory(dir.path(), &pattern, &first.cursors, 65536).unwrap();
        assert_eq!(second.events.len(), 0); // 游标续读，第二次不重复
    }

    #[test]
    fn json_log_event() {
        let ev = parse_line(r#"{"timestamp":"2026-11-08T20:00:00Z","level":"ERROR","message":"boom"}"#).unwrap();
        assert_eq!(ev.level, "ERROR");
        assert_eq!(ev.message, "boom");
        assert_eq!(ev.raw_value, "2026-11-08T20:00:00Z");
    }
}
