//! task 38 验收：证据窗口 → 结构化请求体 —— 仅含白名单行 ID / 基线统计带入 / 超长裁剪并标注 / 无凭据明细。
mod common;

use common::{baseline_conclusion, flow, log};
use interface_fault_rca::ai::prompt::{openai_messages, EvidencePromptBuilder};
use interface_fault_rca::baseline::aggregate::BaselineConclusion;

fn baselines() -> Vec<BaselineConclusion> {
    vec![baseline_conclusion("A", 3.0, 300.0, vec![])]
}

#[test]
fn body_carries_only_whitelisted_row_ids() {
    let rows = vec![flow("A", "SVC", 1000, 300.0, 200), log("A", "SVC", 1100, "ERROR", "boom")];
    let allowed: Vec<String> = rows.iter().map(|r| r.row_id.clone()).collect();
    let b = EvidencePromptBuilder::new(rows, baselines(), "r1".into());
    let body = b.build_body(&allowed);
    let ids = body["allowed_row_ids"].as_array().unwrap();
    assert_eq!(ids.len(), 2);
    for id in ids { assert!(allowed.contains(&id.as_str().unwrap().to_string())); }
    // 行按来源分组
    assert!(body["rows_by_source"].get("esb_audit").is_some());
    assert!(body["rows_by_source"].get("app_log").is_some());
}

#[test]
fn baseline_stats_are_included() {
    let b = EvidencePromptBuilder::new(vec![flow("A", "SVC", 1000, 300.0, 200)], baselines(), "r".into());
    let body = b.build_body(&["flow#A".into()]);
    let stats = body["baseline_stats"].as_array().unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0]["interface_code"], "A");
    assert_eq!(stats[0]["degrade_ratio"], 3.0);
}

#[test]
fn oversized_window_is_truncated_and_marked() {
    let rows: Vec<_> = (0..1000).map(|i| flow("A", "SVC", 1000 + i, 100.0, 200)).collect();
    let b = EvidencePromptBuilder::new(rows, vec![], "r".into());
    let (selected, truncated) = b.select_rows();
    assert!(truncated);
    assert!(selected.len() < 1000);
    let body = b.build_body(&[]);
    assert_eq!(body["truncated"], true);
}

#[test]
fn system_hint_forbids_out_of_whitelist_refs() {
    let b = EvidencePromptBuilder::new(vec![], vec![], "r".into());
    let body = b.build_body(&["flow#A".into()]);
    let sys = body["system"].as_str().unwrap();
    assert!(sys.contains("allowed_row_ids") || sys.contains("只能引用"));
}

#[test]
fn messages_wrap_system_and_user() {
    let b = EvidencePromptBuilder::new(vec![], vec![], "r".into());
    let body = b.build_body(&[]);
    let schema = serde_json::from_str(&interface_fault_rca::ai::schema::output_schema_json()).unwrap();
    let msg = openai_messages(&body, &schema);
    let msgs = msg["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
}
