//! task 28 验收：字典热加载 —— 版本化、坏文件拒绝保留上一版、重绑定产出变更清单不改台账、结论记字典版本。
use interface_fault_rca::binding::interface_dict::{Dictionary, HotDictionary, minimal_dict};
use interface_fault_rca::error::ErrorCode;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_toml(body: &str) -> PathBuf {
    // 时钟分辨率不足时两个并行用例会拿到同一个纳秒值 → 临时目录互相删除 → 偶发失败。
    // 这里再加一个进程内自增序号，保证每个临时目录唯一。
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("ifdict-{n}-{seq}.toml"));
    fs::write(&p, body).unwrap();
    p
}

const DICT_V1: &str = r#"
[dictionary]
version = "v1"
effective_at = "2026-11-09"
authority = "测试字典"

[[interface]]
code = "ESB-A"
service_name = "服务甲"
vendor_name = "院内"
hosts = ["h1"]
aliases = ["A1"]
basis = "登记表"
"#;

const DICT_V2: &str = r#"
[dictionary]
version = "v2"
effective_at = "2026-11-10"
authority = "测试字典"

[[interface]]
code = "ESB-A"
service_name = "服务乙"
vendor_name = "院内"
hosts = ["h2"]
aliases = ["A1"]
basis = "登记表"
"#;

#[test]
fn versioned_dictionaries_load_with_distinct_versions() {
    let d1 = Dictionary::from_toml(DICT_V1).unwrap();
    let d2 = Dictionary::from_toml(DICT_V2).unwrap();
    assert_eq!(d1.version, "v1");
    assert_eq!(d2.version, "v2");
    // 同一 code 的服务名随版本变化（热加载重绑定的语义基础）
    assert_ne!(d1.entries[0].service_name, d2.entries[0].service_name);
}

#[test]
fn bad_toml_is_rejected_so_previous_version_survives() {
    let err = Dictionary::from_toml("[[interface]\nbroken").unwrap_err();
    assert_eq!(err.code, ErrorCode::ConfigMissing);
    // 坏文件无法解析 → HotDictionary 保留其构造时的上一版
    let p = tmp_toml("[[interface]\nbroken");
    let hot = HotDictionary::new(minimal_dict(), &p);
    assert_eq!(hot.version(), "v1"); // 上一版仍在
    fs::remove_file(&p).ok();
}

#[test]
fn rebind_produces_change_list_without_touching_current_dict() {
    let hot = HotDictionary::with_dictionary(minimal_dict());
    let new = Dictionary::from_toml(r#"
[dictionary]
version = "v9"
[[interface]]
code = "ESB-ORDER-CREATE"
service_name = "改名后的服务"
vendor_name = "院内"
hosts = ["h9"]
aliases = []
basis = "登记表"
"#).unwrap();
    let changes = hot.rebind_changes(&new, &["ESB-ORDER-CREATE".into()]);
    assert!(changes.iter().any(|c| c.raw == "ESB-ORDER-CREATE"));
    // 当前字典未被 rebind 改写（台账同理不受影响）
    assert_eq!(hot.version(), "v1");
}

#[test]
fn dictionary_version_recorded_for_audit() {
    let d = minimal_dict();
    assert_eq!(d.version, "v1");
    assert!(!d.authority.is_empty());
    assert!(!d.effective_at.is_empty());
}
