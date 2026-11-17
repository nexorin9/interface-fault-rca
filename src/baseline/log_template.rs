//! 日志模板归并：归一化数字/时间戳/UUID/十六进制/异常类名变体，提取栈首行；
//! 覆盖同义模板、堆栈行、时间戳别名、异常类名变体四类合并；统计频次与首现时间。

use crate::store::event_schema::EventRow;
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
pub struct LogTemplateGroup {
    pub template: String,
    pub count: usize,
    pub first_ts_ms: i64,
    pub sample_row_ids: Vec<String>,
}

/// 归一化消息串：把易变内容替换成占位符，保留稳定结构。
pub fn normalize_message(msg: &str) -> String {
    let mut s = msg.to_string();
    s = replace_uuid(&s);
    s = replace_hex(&s);
    s = replace_timestamp(&s);
    s = replace_numbers(&s);
    // 同义动词归并
    for (a, b) in [("failed to connect", "connection failed"),
                   ("connection failed to", "connection failed"),
                   ("connect to", "connect"), ("err=", "error=")] {
        if s.contains(a) { s = s.replace(a, b); }
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn replace_uuid(s: &str) -> String {
    // 8-4-4-4-12 形态。
    // 逐字节拷贝会把中文等多字节字符拆成乱码，因此非 UUID 处必须**整字符**拷贝。
    let bytes = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if is_uuid_at(bytes, i) && s.is_char_boundary(i) && s.is_char_boundary(i + 36) {
            out.push_str("<UUID>");
            i += 36;
        } else {
            let ch = s[i..].chars().next().unwrap_or('?');
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn is_uuid_at(b: &[u8], i: usize) -> bool {
    if i + 36 > b.len() { return false; }
    for j in [8usize, 13, 18, 23] {
        if b[i + j] != b'-' { return false; }
    }
    (0..36).filter(|&k| ![8, 13, 18, 23].contains(&k)).all(|k| b[i + k].is_ascii_hexdigit())
}

fn replace_hex(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 8 && run.bytes().all(|b| b.is_ascii_hexdigit() || b == b'x') {
            out.push_str("<HEX>");
        } else { out.push_str(run); }
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_hexdigit() { run.push(c); }
        else { flush(&mut run, &mut out); out.push(c); }
    }
    flush(&mut run, &mut out);
    out
}

fn replace_timestamp(s: &str) -> String {
    // ISO8601 与 10/13 位纯数字时间
    let mut out = String::new();
    let words: Vec<&str> = s.split_whitespace().collect();
    for (i, w) in words.iter().enumerate() {
        if i > 0 { out.push(' '); }
        if (w.len() == 10 || w.len() == 13) && w.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str("<TS>");
        } else if w.contains('T') && w.contains(':') && w.len() >= 19 {
            out.push_str("<TS>");
        } else { out.push_str(w); }
    }
    out
}

fn replace_numbers(s: &str) -> String {
    let mut out = String::new();
    let mut in_num = false;
    for c in s.chars() {
        if c.is_ascii_digit() {
            if !in_num { out.push_str("<N>"); in_num = true; }
        } else { in_num = false; out.push(c); }
    }
    out
}

/// 提取异常类名变体归一（java.lang.NullPointerException / NullPointerException → <EXC>）
pub fn normalize_exception(s: &str) -> String {
    for marker in ["Exception", "Error", "Timeout"] {
        if let Some(pos) = s.find(marker) {
            let start = s[..pos].rfind(|c: char| c.is_whitespace() || c == ':' || c == '(').map(|x| x + 1).unwrap_or(0);
            // 异常类全名 + 冒号后变体消息整体归一为 <EXC>（归一掉 ": x" / ": y" 差异）
            let rest = &s[pos + marker.len()..];
            let tail = if rest.trim_start().starts_with(':') { "" } else { rest };
            return format!("{}<EXC>{}", &s[..start], tail);
        }
    }
    s.to_string()
}

/// 栈首行：取以 "at " 开头的第一行，归一化行号
pub fn stack_head(message: &str) -> String {
    for line in message.lines() {
        let t = line.trim();
        if t.starts_with("at ") || t.contains("Exception:") || t.contains("Error:") {
            return replace_numbers(t);
        }
    }
    String::new()
}

/// 归并日志行为模板组，统计频次与首现时间。
pub fn group_templates(rows: &[EventRow]) -> Vec<LogTemplateGroup> {
    let mut groups: BTreeMap<String, LogTemplateGroup> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.kind() == "log") {
        let msg = r.fields.get("message").map(|s| s.as_str()).unwrap_or("");
        let mut norm = normalize_message(msg);
        if !norm.contains("<EXC>") { norm = normalize_exception(&norm); }
        let entry = groups.entry(norm.clone()).or_insert_with(|| LogTemplateGroup {
            template: norm, count: 0, first_ts_ms: r.ts_ms, sample_row_ids: vec![],
        });
        entry.count += 1;
        if r.ts_ms < entry.first_ts_ms { entry.first_ts_ms = r.ts_ms; }
        if entry.sample_row_ids.len() < 5 { entry.sample_row_ids.push(r.row_id.clone()); }
    }
    let mut out: Vec<_> = groups.into_values().collect();
    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.template.cmp(&b.template)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tsid::compute_tsid;
    use std::collections::BTreeMap;

    fn log(msg: &str, ts: i64, id: &str) -> EventRow {
        let mut tags = BTreeMap::new();
        tags.insert("source".into(), "app_log".into());
        tags.insert("event_kind".into(), "log".into());
        tags.insert("interface_code".into(), "A".into());
        let mut fields = BTreeMap::new();
        fields.insert("message".into(), msg.into());
        EventRow::new(tags, ts, fields, id.into(), compute_tsid("app_log", ts, "A", "", "log")).unwrap()
    }

    #[test]
    fn numbers_uuid_timestamp_normalized() {
        let a = normalize_message("connect to 10.0.0.1 failed after 1234ms");
        let b = normalize_message("connect to 10.0.0.2 failed after 5678ms");
        assert_eq!(a, b);
        let u1 = normalize_message("req 550e8400-e29b-41d4-a716-446655440000 done");
        let u2 = normalize_message("req 123e4567-e89b-12d3-a456-426614174000 done");
        assert_eq!(u1, u2);
    }

    #[test]
    fn synonym_templates_merge() {
        let a = normalize_message("failed to connect to host");
        let b = normalize_message("connection failed host");
        assert_eq!(a, b);
    }

    #[test]
    fn exception_variants_merge() {
        let a = normalize_exception("java.lang.NullPointerException: x");
        let b = normalize_exception("NullPointerException: y");
        assert_eq!(a, b);
    }

    #[test]
    fn stack_head_extracted() {
        let m = "boom\n  at com.foo.Bar.baz(Bar.java:123)";
        assert!(stack_head(m).contains("Bar.baz"));
    }

    #[test]
    fn templates_grouped_with_frequency_and_first_seen() {
        let rows = vec![
            log("connect to 10.0.0.1 failed after 100ms", 50, "l1"),
            log("connect to 10.0.0.2 failed after 200ms", 30, "l2"),
            log("totally different message", 40, "l3"),
        ];
        let groups = group_templates(&rows);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].first_ts_ms, 30);
    }
}
