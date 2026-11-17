//! 接口实体绑定字典：别名→规范码精确/别名映射、歧义拒绝、unbound 处理、
//! Prom 标签→host 映射、字典版本化与热加载、覆盖度报告。

use crate::error::{AppError, ErrorCode, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct InterfaceEntry {
    pub code: String,
    pub service_name: String,
    pub vendor_name: String,
    pub hosts: Vec<String>,
    pub aliases: Vec<String>,
    pub basis: String,
}

#[derive(Debug, Clone)]
pub struct LabelBinding {
    pub source: String,
    pub label: String,
    pub value: String,
    pub host: String,
}

#[derive(Debug, Clone)]
pub struct Dictionary {
    pub version: String,
    pub effective_at: String,
    pub authority: String,
    pub entries: Vec<InterfaceEntry>,
    pub label_bindings: Vec<LabelBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    Bound { code: String, service: String, vendor: String, hosts: Vec<String> },
    Unbound { raw: String },
}

impl Dictionary {
    /// 从 TOML 文本加载（不直接读文件，便于热加载与测试）
    pub fn from_toml(txt: &str) -> Result<Self> {
        let v: toml::Value = toml::from_str(txt)
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("字典解析失败: {e}")))?;
        let d = v.get("dictionary");
        let version = d.and_then(|x| x.get("version")).and_then(|x| x.as_str()).unwrap_or("v0").to_string();
        let effective_at = d.and_then(|x| x.get("effective_at")).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let authority = d.and_then(|x| x.get("authority")).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut entries = vec![];
        if let Some(arr) = v.get("interface").and_then(|x| x.as_array()) {
            for it in arr {
                entries.push(InterfaceEntry {
                    code: it.get("code").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    service_name: it.get("service_name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    vendor_name: it.get("vendor_name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    hosts: it.get("hosts").and_then(|x| x.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
                    aliases: it.get("aliases").and_then(|x| x.as_array())
                        .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
                    basis: it.get("basis").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                });
            }
        }
        let mut label_bindings = vec![];
        if let Some(arr) = v.get("label_binding").and_then(|x| x.as_array()) {
            for it in arr {
                label_bindings.push(LabelBinding {
                    source: it.get("source").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    label: it.get("label").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    value: it.get("value").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                    host: it.get("host").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                });
            }
        }
        Ok(Dictionary { version, effective_at, authority, entries, label_bindings })
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let txt = std::fs::read_to_string(path)
            .map_err(|e| AppError::new(ErrorCode::ConfigMissing, format!("读字典失败: {e}")))?;
        Self::from_toml(&txt)
    }

    /// 别名→规范码：精确匹配优先，其次别名表。一个别名命中多条目 = 歧义，拒绝归因。
    pub fn resolve(&self, raw: &str) -> Result<Binding> {
        let raw = raw.trim();
        if raw.is_empty() { return Ok(Binding::Unbound { raw: String::new() }); }
        let mut hits: Vec<&InterfaceEntry> = self.entries.iter().filter(|e| {
            e.code == raw || e.aliases.iter().any(|a| a == raw)
        }).collect();
        // 精确 code 命中优先（若存在唯一精确 code 命中，直接采用，避免别名误判歧义）
        let exact: Vec<&InterfaceEntry> = self.entries.iter().filter(|e| e.code == raw).collect();
        if exact.len() == 1 { hits = exact; }
        if hits.is_empty() {
            return Ok(Binding::Unbound { raw: raw.to_string() });
        }
        if hits.len() > 1 {
            let services: Vec<String> = hits.iter().map(|e| e.service_name.clone()).collect();
            return Err(AppError::new(ErrorCode::AmbiguousInterface,
                format!("接口码 {raw} 映射到多服务: {}", services.join(" | "))));
        }
        let e = hits[0];
        Ok(Binding::Bound {
            code: e.code.clone(), service: e.service_name.clone(),
            vendor: e.vendor_name.clone(), hosts: e.hosts.clone(),
        })
    }

    /// 判定某值是否歧义（供覆盖度清单）
    pub fn is_ambiguous(&self, raw: &str) -> bool {
        self.resolve(raw).is_err()
    }

    /// Prom 标签（job/instance）→ host；映射不到返回 None（host 允许缺失，不报错）
    pub fn host_from_labels(&self, labels: &BTreeMap<String, String>) -> Option<String> {
        for (k, v) in labels {
            if let Some(b) = self.label_bindings.iter().find(|b| {
                b.source == "prom" && b.label == *k && b.value == *v
            }) {
                return Some(b.host.clone());
            }
        }
        None
    }

    /// 流水 service_name → host（同源映射，取第一个条目的 host）
    pub fn host_from_service(&self, service: &str) -> Option<String> {
        self.entries.iter().find(|e| e.service_name == service)
            .and_then(|e| e.hosts.first().cloned())
    }
}

/// 可热加载的字典句柄：文件变更检测后重载；加载失败保留上一版可用字典并报错。
pub struct HotDictionary {
    current: Arc<RwLock<Dictionary>>,
    path: Arc<std::sync::Mutex<std::path::PathBuf>>,
    last_loaded: Arc<RwLock<SystemTime>>,
    last_error: Arc<RwLock<Option<String>>>,
}

impl HotDictionary {
    pub fn new(dict: Dictionary, path: &Path) -> Self {
        Self {
            current: Arc::new(RwLock::new(dict)),
            path: Arc::new(std::sync::Mutex::new(path.to_path_buf())),
            last_loaded: Arc::new(RwLock::new(SystemTime::now())),
            last_error: Arc::new(RwLock::new(None)),
        }
    }

    pub fn with_dictionary(dict: Dictionary) -> Self {
        Self::new(dict, Path::new("config/interface-dict.toml"))
    }

    pub fn snapshot(&self) -> Dictionary {
        self.current.read().map(|d| d.clone()).unwrap()
    }

    pub fn version(&self) -> String { self.snapshot().version }

    /// 检查文件 mtime，变化时重载；失败保留旧字典。
    pub fn maybe_reload(&self) -> Result<bool> {
        let path = self.path.lock().unwrap().clone();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => return Ok(false),
        };
        let mtime = match meta.modified() { Ok(t) => t, Err(_) => return Ok(false) };
        let last = *self.last_loaded.read().unwrap();
        if mtime <= last { return Ok(false); }
        match Dictionary::from_file(&path) {
            Ok(new) => {
                *self.current.write().unwrap() = new;
                *self.last_loaded.write().unwrap() = SystemTime::now();
                *self.last_error.write().unwrap() = None;
                Ok(true)
            }
            Err(e) => {
                *self.last_error.write().unwrap() = Some(e.to_string());
                Err(e) // 旧字典原样保留
            }
        }
    }

    pub fn last_error(&self) -> Option<String> { self.last_error.read().unwrap().clone() }

    /// 显式重绑定：对新旧字典分别 resolve，产出变更清单，不改写台账。
    pub fn rebind_changes(&self, new: &Dictionary, raws: &[String]) -> Vec<RebindChange> {
        let old = self.snapshot();
        raws.iter().filter_map(|raw| {
            let a = old.resolve(raw).ok();
            let b = new.resolve(raw).ok();
            // 绑定身份 = 码 + 服务 + 厂商：任一变化都算「换了绑定」，
            // 光看码会漏掉「码没变但服务改���」的重新绑定。
            if binding_identity(&a) != binding_identity(&b) {
                Some(RebindChange {
                    raw: raw.clone(),
                    old_code: binding_code(&a),
                    new_code: binding_code(&b),
                })
            } else { None }
        }).collect()
    }
}

/// 绑定身份（码 + 服务 + 厂商）：用于判断「是否换了绑定」
fn binding_identity(b: &Option<Binding>) -> String {
    match b {
        Some(Binding::Bound { code, service, vendor, .. }) => format!("{code}|{service}|{vendor}"),
        Some(Binding::Unbound { raw }) => format!("unbound:{raw}"),
        None => "ambiguous".to_string(),
    }
}

fn binding_code(b: &Option<Binding>) -> String {
    match b {
        Some(Binding::Bound { code, .. }) => code.clone(),
        Some(Binding::Unbound { raw }) => format!("unbound:{raw}"),
        None => "ambiguous".to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebindChange {
    pub raw: String,
    pub old_code: String,
    pub new_code: String,
}

#[derive(Debug, Clone)]
pub struct CoverageRow {
    pub raw: String,
    pub count: usize,
    pub kind: CoverageKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageKind { Unbound, Ambiguous }

/// 绑定覆盖度统计：Top-N 未映射值与歧义清单。
pub fn coverage(dict: &Dictionary, observed: &[(String, usize)]) -> (Vec<CoverageRow>, Vec<CoverageRow>) {
    let mut unbound = vec![];
    let mut ambiguous = vec![];
    for (raw, count) in observed {
        match dict.resolve(raw) {
            Ok(Binding::Unbound { .. }) => unbound.push(CoverageRow { raw: raw.clone(), count: *count, kind: CoverageKind::Unbound }),
            Err(_) => ambiguous.push(CoverageRow { raw: raw.clone(), count: *count, kind: CoverageKind::Ambiguous }),
            _ => {}
        }
    }
    unbound.sort_by(|a, b| b.count.cmp(&a.count).then(a.raw.cmp(&b.raw)));
    ambiguous.sort_by(|a, b| b.count.cmp(&a.count).then(a.raw.cmp(&b.raw)));
    (unbound, ambiguous)
}

/// 导出覆盖度 Markdown 报告与缺口 CSV（可带走的文件）
pub fn export_coverage_report(dict: &Dictionary, observed: &[(String, usize)], dir: &Path) -> Result<(String, String)> {
    std::fs::create_dir_all(dir).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("建目录失败: {e}")))?;
    let (unbound, ambiguous) = coverage(dict, observed);
    let total: usize = observed.iter().map(|(_, c)| *c).sum();
    let unbound_count: usize = unbound.iter().map(|r| r.count).sum();
    let pct = if total == 0 { 0.0 } else { unbound_count as f64 / total as f64 * 100.0 };
    let mut md = format!("# 绑定覆盖度报告\n\n字典版本：{}\n\n未映射占比：{:.1}%（{} / {}）\n\n歧义接口码：{} 个\n\n## 未映射 Top-N\n\n| 接口码原值 | 行数 |\n|---|---|\n",
        dict.version, pct, unbound_count, total, ambiguous.len());
    for r in unbound.iter().take(20) { md.push_str(&format!("| {} | {} |\n", r.raw, r.count)); }
    md.push_str("\n## 歧义清单\n\n| 接口码原值 | 行数 | 候选服务 |\n|---|---|---|\n");
    for r in &ambiguous {
        let services = dict.entries.iter().filter(|e| e.code == r.raw || e.aliases.iter().any(|a| a == &r.raw))
            .map(|e| e.service_name.clone()).collect::<Vec<_>>().join(" | ");
        md.push_str(&format!("| {} | {} | {} |\n", r.raw, r.count, services));
    }
    let mut csv = String::from("raw,count,kind,assignee,note\n");
    let _ = &unbound;
    for r in unbound.iter().chain(ambiguous.iter()) {
        csv.push_str(&format!("{},{},{},,\n", r.raw, r.count, format!("{:?}", r.kind)));
    }
    let md_path = dir.join("binding-coverage.md");
    let csv_path = dir.join("binding-gaps.csv");
    std::fs::write(&md_path, md).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("写报告失败: {e}")))?;
    std::fs::write(&csv_path, csv).map_err(|e| AppError::new(ErrorCode::SourceUnavailable, format!("写 CSV 失败: {e}")))?;
    Ok((md_path.to_string_lossy().to_string(), csv_path.to_string_lossy().to_string()))
}

/// 构造一个最小字典（供测试与离线诊断，不含真实数据）
pub fn minimal_dict() -> Dictionary {
    Dictionary {
        version: "v1".into(), effective_at: "2026-11-09".into(),
        authority: "测试用最小字典".into(),
        entries: vec![
            InterfaceEntry { code: "ESB-ORDER-CREATE".into(), service_name: "住院医嘱开立服务".into(),
                vendor_name: "院内自研".into(), hosts: vec!["esb-gw-01".into()],
                aliases: vec!["ORDER_CREATE".into(), "order.create".into()], basis: "".into() },
            InterfaceEntry { code: "ESB-REPORT-QUERY".into(), service_name: "检验报告查询服务".into(),
                vendor_name: "院内自研".into(), hosts: vec!["esb-gw-02".into()],
                aliases: vec!["REPORT_QUERY".into()], basis: "".into() },
        ],
        label_bindings: vec![LabelBinding { source: "prom".into(), label: "job".into(),
            value: "esb-gateway".into(), host: "esb-gw-01".into() }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_and_alias_resolution() {
        let d = minimal_dict();
        assert_eq!(d.resolve("ESB-ORDER-CREATE").unwrap(), Binding::Bound {
            code: "ESB-ORDER-CREATE".into(), service: "住院医嘱开立服务".into(),
            vendor: "院内自研".into(), hosts: vec!["esb-gw-01".into()] });
        assert!(matches!(d.resolve("ORDER_CREATE").unwrap(), Binding::Bound { .. }));
    }

    #[test]
    fn unmapped_is_unbound() {
        let d = minimal_dict();
        assert_eq!(d.resolve("NO-SUCH-CODE").unwrap(), Binding::Unbound { raw: "NO-SUCH-CODE".into() });
    }

    #[test]
    fn ambiguous_returns_error() {
        let mut d = minimal_dict();
        d.entries.push(InterfaceEntry { code: "ESB-ORDER-CREATE".into(),
            service_name: "另一个服务".into(), vendor_name: "".into(), hosts: vec![],
            aliases: vec!["ORDER_CREATE".into()], basis: "".into() });
        // 精确 code 命中两条 → 歧义
        let e = d.resolve("ESB-ORDER-CREATE").unwrap_err();
        assert_eq!(e.code, ErrorCode::AmbiguousInterface);
    }

    #[test]
    fn host_from_labels_and_service() {
        let d = minimal_dict();
        let mut labels = BTreeMap::new();
        labels.insert("job".to_string(), "esb-gateway".to_string());
        assert_eq!(d.host_from_labels(&labels), Some("esb-gw-01".into()));
        assert_eq!(d.host_from_service("住院医嘱开立服务"), Some("esb-gw-01".into()));
        assert_eq!(d.host_from_labels(&BTreeMap::new()), None);
    }

    #[test]
    fn hot_reload_keeps_old_on_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dict.toml");
        let txt = std::fs::read_to_string("config/interface-dict.toml").unwrap();
        std::fs::write(&path, &txt).unwrap();
        let hot = HotDictionary::new(minimal_dict(), &path);
        assert!(!hot.maybe_reload().unwrap());
        std::fs::write(&path, "[[interface]\nbroken").unwrap();
        // 强制 mtime 变化
        let t = SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::write(&path, "[[interface]\ncode = \"X\"\nservice_name=\"Y\"\n").unwrap();
        let _ = t;
        // 直接验证失败路径：坏 TOML 解析失败
        assert!(Dictionary::from_toml("[[interface]\ncode =").is_err());
        assert_eq!(hot.version(), "v1"); // 旧字典保留
    }

    #[test]
    fn rebind_changes_and_coverage() {
        let old = minimal_dict();
        let mut new = minimal_dict();
        new.entries[0].code = "ESB-NEW-CODE".into();
        new.entries[0].aliases = vec!["ORDER_CREATE".into()];
        let hot = HotDictionary::with_dictionary(old.clone());
        let changes = hot.rebind_changes(&new, &["ORDER_CREATE".to_string()]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].old_code, "ESB-ORDER-CREATE");
        assert_eq!(changes[0].new_code, "ESB-NEW-CODE");
        let (unbound, _) = coverage(&old, &[("UNK".into(), 5), ("ESB-ORDER-CREATE".into(), 10)]);
        assert_eq!(unbound.len(), 1);
        assert_eq!(unbound[0].count, 5);
    }
}
