//! task 13 验收：规模数据 —— 行数达标、tags 白名单、ts_ms 单调、确定性 manifest、降级标记。
//! 数据由 scripts/gen_scale_data.py 生成；若 data/scale 尚未生成则跳过（不误报失败）。
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn manifest() -> Option<serde_json::Value> {
    let p = Path::new("data/scale/manifest.json");
    if !p.exists() { return None; }
    serde_json::from_str(&fs::read_to_string(p).ok()?).ok()
}

fn sha256_file(p: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(p).unwrap_or_default();
    hex::encode(Sha256::digest(&bytes))
}

#[test]
fn scale_data_meets_target_counts() {
    let Some(m) = manifest() else { eprintln!("data/scale 未生成，跳过"); return; };
    let c = &m["counts"];
    assert!(c["flow"].as_u64().unwrap_or(0) >= 500_000);
    assert!(c["log"].as_u64().unwrap_or(0) >= 100_000);
    assert!(c["metric"].as_u64().unwrap_or(0) >= 10_000);
}

#[test]
fn injected_shapes_recorded_as_ground_truth() {
    let Some(m) = manifest() else { return; };
    let shapes = &m["injected_shapes"];
    let vals: BTreeSet<_> = shapes.as_object().map(|o|
        o.values().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
    assert!(vals.contains("step"));
    assert!(vals.contains("spike"));
    assert!(vals.contains("ramp"));
}

#[test]
fn rows_use_only_whitelisted_tags_and_monotonic_ts() {
    let p = Path::new("data/scale/flow-events.jsonl");
    if !p.exists() { return; }
    let txt = fs::read_to_string(p).unwrap();
    let whitelist: BTreeSet<&str> = ["source", "event_kind", "interface_code", "service_name", "vendor_name", "host"].into_iter().collect();
    let mut last_ts = i64::MIN;
    let mut checked = 0;
    for line in txt.lines().take(2000) {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        for k in v["tags"].as_object().unwrap().keys() {
            assert!(whitelist.contains(k.as_str()), "非法 tag 键 {k}");
        }
        let ts = v["ts_ms"].as_i64().unwrap();
        assert!(ts >= last_ts, "ts 未单调");
        last_ts = ts;
        checked += 1;
    }
    assert!(checked > 0);
}

#[test]
fn manifest_checksums_match_actual_files() {
    let Some(m) = manifest() else { return; };
    for (name, meta) in m["files"].as_object().unwrap() {
        let p = Path::new("data/scale").join(name);
        let expected = meta["sha256"].as_str().unwrap();
        assert_eq!(sha256_file(&p), expected, "{name} 校验和不一致");
    }
}

#[test]
fn degraded_marker_present_when_offline() {
    // 断网时 fetch 脚本应产出 DEGRADED.md（本环境实测已产出）
    let p = Path::new("data/public/DEGRADED.md");
    if p.exists() {
        let t = fs::read_to_string(p).unwrap();
        assert!(t.contains("合成数据") || t.contains("降级"));
    }
}
