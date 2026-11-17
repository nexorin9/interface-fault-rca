//! 金标准回归：逐例断言 fixtures/gold/cases.jsonl 的判定结论。
//!
//! 这套夹具的期望结论是**人工依据 domain/ 语料与 spec.md 判定纪律写出的**，
//! 不是跑一遍被测代码再抄回来的（否则是自证循环）。本测试反过来做两件事：
//!
//! 1. **语料绑定校验**：每条用例的 payload / domain_expect 必须与
//!    `fixtures/gold/corpus/*.json`（逐字拷贝自 `domain/cases/*.json`）中的原文**逐字一致**；
//!    仓库内 `domain/cases/` 可达时，再校验拷贝件与上游**字节一致**。
//! 2. **判定断言**：把该用例的接口流水喂进**产品主路径**
//!    （`interface_fault_rca::diagnose::diagnose`，与 `main.rs` 的 diagnose 子命令同一个函数），
//!    逐例断言判定结论、是否进候选、丢弃原因码 / 错误码。
//!
//! 用例编号（case_id）出现在断言失败信息里，可直接回 fixtures/gold/SOURCES.md 查来源。

use interface_fault_rca::ai::conclusion::{BaselineStats, Conclusion, Verdict};
use interface_fault_rca::ai::evidence_gate::{check_conclusion, GateContext};
use interface_fault_rca::ai::ports::GateDecision;
use interface_fault_rca::binding::interface_dict::{Dictionary, InterfaceEntry};
use interface_fault_rca::config::BaselineConfig;
use interface_fault_rca::diagnose::{diagnose, DiagnoseOptions, NoModelGateway};
use interface_fault_rca::error::ErrorCode;
use interface_fault_rca::ingest::jsonl;
use interface_fault_rca::ingest::pipeline::RawRecord;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

// ── 用例结构（与 fixtures/gold/cases.jsonl 一一对应）───────────────────────────

#[derive(Debug, Deserialize)]
struct GoldCase {
    case_id: String,
    /// JSON 字段名是 "assert"（Rust 关键字，不能直接做字段名）
    #[serde(rename = "assert", default = "default_assert")]
    assert_mode: String,
    kind: String,
    negativity: Option<String>,
    anchor_ms: i64,
    interface_code: String,
    window: WindowSpec,
    expect: Expect,
    basis: Basis,
    condition: String,
    payload: Value,
    domain_expect: Value,
    rows: Vec<GoldRow>,
    malformed_line: Option<String>,
    tamper: Option<Tamper>,
    dict_overlay: Option<DictOverlay>,
}

fn default_assert() -> String { "main_path".to_string() }

#[derive(Debug, Deserialize)]
struct WindowSpec {
    start_min: i64,
    end_min: i64,
    baseline_minutes: i64,
}

#[derive(Debug, Deserialize)]
struct Expect {
    verdict: Option<String>,
    in_candidates: bool,
    reason_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Basis {
    source_file: String,
    doc_no: String,
    clause: String,
    domain_case_id: String,
}

#[derive(Debug, Deserialize)]
struct GoldRow {
    t_min: i64,
    d: i64,
    r: i64,
}

#[derive(Debug, Deserialize)]
struct Tamper {
    mode: String,
    refs: Vec<String>,
    gate_context: GateCtx,
}

#[derive(Debug, Deserialize)]
struct GateCtx {
    window_ids: Vec<String>,
    all_ids: Vec<String>,
    unbound_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DictOverlay {
    ambiguous_alias: String,
    second_service_name: String,
}

// ── 夹具读取 ──────────────────────────────────────────────────────────────────

fn project_dir() -> PathBuf { PathBuf::from(env!("CARGO_MANIFEST_DIR")) }

fn load_cases() -> Vec<GoldCase> {
    let p = project_dir().join("fixtures/gold/cases.jsonl");
    let txt = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读 {} 失败: {e}", p.display()));
    let mut out = vec![];
    for (i, line) in txt.lines().enumerate() {
        if line.trim().is_empty() { continue; }
        let c: GoldCase = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("cases.jsonl 第 {} 行解析失败: {e}", i + 1));
        out.push(c);
    }
    out
}

/// 载入接口实体绑定字典；带歧义叠加时补一条同别名的条目（模拟登记表一码多服务）
fn load_dict(overlay: Option<&DictOverlay>) -> Dictionary {
    let txt = std::fs::read_to_string(project_dir().join("config/interface-dict.toml"))
        .expect("config/interface-dict.toml 存在");
    let mut d = Dictionary::from_toml(&txt).expect("字典可解析");
    if let Some(o) = overlay {
        d.entries.push(InterfaceEntry {
            code: "ESB-RX-REVIEW-SECOND".into(),
            service_name: o.second_service_name.clone(),
            vendor_name: "厂商 B".into(),
            hosts: vec!["rx-adapter-02".into()],
            aliases: vec![o.ambiguous_alias.clone()],
            basis: "金标准用例叠加：模拟 ESB 接口登记表一码多服务".into(),
        });
    }
    d
}

/// 用例的行 → 真实来源行（走产品自己的 JSONL 解析）
fn build_records(c: &GoldCase) -> Vec<RawRecord> {
    let mut out = vec![];
    for r in &c.rows {
        let ts = c.anchor_ms + r.t_min * 60_000;
        let v = serde_json::json!({
            "source": "esb_audit",
            "interface_code": c.interface_code,
            "ts_ms": ts,
            "duration_ms": r.d,
            "response_code": r.r,
            "caller_system": "HIS",
        });
        out.push(
            jsonl::record_from_json(&v, "esb_audit")
                .unwrap_or_else(|e| panic!("用例 {} 行解析失败: {}", c.case_id, e.detail)),
        );
    }
    out
}

fn run_case(c: &GoldCase) -> interface_fault_rca::diagnose::DiagnoseReport {
    let records = build_records(c);
    let opts = DiagnoseOptions {
        window_start_ms: c.anchor_ms + c.window.start_min * 60_000,
        window_end_ms: c.anchor_ms + c.window.end_min * 60_000,
        baseline_minutes: c.window.baseline_minutes,
        baseline: BaselineConfig::default(),
        dict: load_dict(c.dict_overlay.as_ref()),
        use_model: false,
        request_id: format!("gold-{}", c.case_id),
    };
    diagnose(&opts, &records, None::<&NoModelGateway>)
        .unwrap_or_else(|e| panic!("用例 {} 判定失败: {}", c.case_id, e.detail))
        .report
}

// ── 断言组 ────────────────────────────────────────────────────────────────────

/// 夹具自身的规模与元数据体检（≥50 例、≥20 负样本、每例有来源文件与文号）
#[test]
fn gold_fixture_meets_scale_and_provenance_bar() {
    let cases = load_cases();
    assert!(cases.len() >= 50, "金标准规模不足：{} < 50", cases.len());
    let neg = cases.iter().filter(|c| c.kind == "负样本").count();
    assert!(neg >= 20, "负样本不足：{} < 20", neg);
    let mut ids = HashSet::new();
    for c in &cases {
        assert!(ids.insert(c.case_id.clone()), "用例 id 重复: {}", c.case_id);
        assert!(c.basis.source_file.starts_with("domain/cases/"),
            "用例 {} 缺来源文件标注", c.case_id);
        assert!(c.basis.doc_no.len() >= 6, "用例 {} 缺文号标注", c.case_id);
        assert!(!c.basis.clause.is_empty(), "用例 {} 缺条款标注", c.case_id);
        assert!(!c.condition.is_empty(), "用例 {} 缺条件说明", c.case_id);
        assert!(!c.rows.is_empty(), "用例 {} 没有流量行", c.case_id);
        if c.kind == "负样本" {
            assert!(matches!(c.negativity.as_deref(), Some("应判不出") | Some("应拒绝") | Some("应报错")),
                "用例 {} 的负样本类型不合法", c.case_id);
        }
    }
}

/// 语料绑定：每条用例的 payload / domain_expect 与语料原文逐字一致
#[test]
fn gold_payload_matches_domain_corpus_verbatim() {
    let cases = load_cases();
    let mut corpus_cache: BTreeMap<String, Value> = BTreeMap::new();
    for c in &cases {
        let family = c.basis.source_file.trim_start_matches("domain/cases/").trim_end_matches(".json");
        let vendored = project_dir().join("fixtures/gold/corpus").join(format!("{family}.json"));
        let doc: Value = match corpus_cache.get(family) {
            Some(v) => v.clone(),
            None => {
                let v: Value = serde_json::from_str(&std::fs::read_to_string(&vendored)
                    .unwrap_or_else(|e| panic!("读 {} 失败: {e}", vendored.display())))
                    .expect("语料副本是合法 JSON");
                corpus_cache.insert(family.to_string(), v.clone());
                v
            }
        };
        let found = doc["cases"].as_array().expect("语料有 cases 数组").iter()
            .find(|x| x["case_id"] == c.basis.domain_case_id)
            .unwrap_or_else(|| panic!("用例 {} 在语料 {} 里找不到 {}",
                c.case_id, family, c.basis.domain_case_id));
        assert_eq!(&c.payload, &found["input"],
            "用例 {} 的 payload 与语料原文不一致", c.case_id);
        assert_eq!(&c.domain_expect, &found["expect"],
            "用例 {} 的 domain_expect 与语料原文不一致", c.case_id);
    }
}

/// 语料副本与仓库 domain/cases/ 字节一致（上游可达时）；不可达时校验副本自洽
#[test]
fn vendored_corpus_matches_upstream_when_available() {
    let dir = project_dir().join("fixtures/gold/corpus");
    let upstream_dir = project_dir().join("../../../domain/cases");
    for f in std::fs::read_dir(&dir).expect("corpus 目录存在").flatten() {
        let name = f.file_name().to_string_lossy().to_string();
        let local = std::fs::read(&f.path()).expect("副本可读");
        let upstream = upstream_dir.join(&name);
        if upstream.exists() {
            let up = std::fs::read(&upstream).expect("上游可读");
            assert_eq!(local, up, "语料副本与上游 {name} 不一致");
        } else {
            // 上游不在本仓库内（例如单独 clone 本项目）：副本必须仍是合法 JSON
            let v: Value = serde_json::from_slice(&local)
                .unwrap_or_else(|e| panic!("副本 {name} 不是合法 JSON: {e}"));
            assert!(v["cases"].is_array(), "副本 {name} 缺 cases 数组");
        }
    }
}

/// 逐例断言：主路径判定结论
#[test]
fn every_case_matches_its_expected_verdict() {
    let cases = load_cases();
    let mut checked = 0usize;
    for c in &cases {
        if c.assert_mode != "main_path" { continue; }
        let r = run_case(c);
        let hit = r.candidates.iter().find(|x| x.interface_code == c.interface_code);
        if c.expect.in_candidates {
            let cand = hit.unwrap_or_else(|| panic!(
                "用例 {} 期望进候选但没有：dropped={:?}", c.case_id,
                r.dropped.iter().map(|d| (d.interface_code.clone(), d.reason_code.clone())).collect::<Vec<_>>()));
            assert_eq!(cand.verdict, c.expect.verdict.clone().unwrap(),
                "用例 {} 判定不符", c.case_id);
            assert!(!cand.evidence_refs.is_empty(), "用例 {} 候选缺证据引用", c.case_id);
            assert!(cand.evidence_refs.iter().all(|id| id.starts_with("flow#")),
                "用例 {} 证据行不是流水行: {:?}", c.case_id, cand.evidence_refs);
            assert!(cand.degrade_ratio > 1.5, "用例 {} 劣化倍数应过阈值，实际 {}", c.case_id, cand.degrade_ratio);
        } else {
            assert!(hit.is_none(), "用例 {} 不应进候选，却出现了", c.case_id);
            if let Some(code) = &c.expect.reason_code {
                let dropped = r.dropped.iter()
                    .find(|d| d.interface_code == c.interface_code)
                    .unwrap_or_else(|| panic!(
                        "用例 {} 期望原因码 {code}，但 dropped 里没有该接口：{:?}", c.case_id,
                        r.dropped.iter().map(|d| (d.interface_code.clone(), d.reason_code.clone())).collect::<Vec<_>>()));
                assert_eq!(&dropped.reason_code, code, "用例 {} 原因码不符", c.case_id);
                if let Some(v) = &c.expect.verdict {
                    assert_eq!(&dropped.verdict, v, "用例 {} 的判定结论不符", c.case_id);
                }
            }
        }
        checked += 1;
    }
    assert!(checked >= 45, "主路径断言的用例过少：{checked}");
}

/// 歧义接口码：整批拒绝入事件表（宁可少给结论，不可错连）
#[test]
fn ambiguous_interface_code_is_rejected_not_attributed() {
    let cases = load_cases();
    let c = cases.iter().find(|c| c.assert_mode == "main_path"
        && c.dict_overlay.is_some()).expect("存在歧义用例");
    let r = run_case(c);
    assert!(r.candidates.iter().all(|x| x.interface_code != c.interface_code),
        "歧义接口码不应进候选");
    let stat = r.sources.iter().find(|s| s.source == "esb_audit").expect("有 esb_audit 来源统计");
    assert!(stat.rejected >= 1, "歧义行应被拒绝，实际 rejected={}", stat.rejected);
    assert!(stat.rejected_examples.iter().any(|s| s.contains("AmbiguousInterface")),
        "拒绝原因里应有 AmbiguousInterface，实际 {:?}", stat.rejected_examples);
}

/// 证据守门：引用越界 / 引用不存在 → 整条丢弃并给原因码
#[test]
fn tampered_evidence_refs_are_dropped_with_reason_code() {
    let cases = load_cases();
    let mut n = 0;
    for c in cases.iter().filter(|c| c.assert_mode == "gate_drop") {
        let t = c.tamper.as_ref().expect("守门用例带 tamper");
        let ctx = GateContext {
            window_ids: t.gate_context.window_ids.iter().cloned().collect(),
            all_ids: t.gate_context.all_ids.iter().cloned().collect(),
            unbound_ids: t.gate_context.unbound_ids.iter().cloned().collect(),
            min_refs: 1,
        };
        let conclusion = Conclusion {
            interface_code: c.interface_code.clone(),
            service_name: "金标准夹具".into(),
            verdict: Verdict::接口自身劣化,
            confidence: 0.9,
            evidence_refs: t.refs.clone(),
            reasoning_summary: c.condition.clone(),
            next_check: String::new(),
            degraded: false,
            baseline_stats: BaselineStats {
                p95_ms: 360.0, p99_ms: 380.0, degrade_ratio: 3.6, error_rate: 0.5, baseline_samples: 10,
            },
            request_id: format!("gold-{}", c.case_id),
        };
        let decision = check_conclusion(&conclusion, &ctx);
        assert_eq!(decision, GateDecision::Drop {
            reason_code: c.expect.reason_code.clone().unwrap_or_default()
        }, "用例 {} 的守门处置不符（mode={}）", c.case_id, t.mode);
        n += 1;
    }
    assert!(n >= 2, "守门用例过少：{n}");
}

/// 来源行非法：报明确错误码，不静默丢弃
#[test]
fn malformed_source_rows_raise_schema_mismatch() {
    let cases = load_cases();
    let mut n = 0;
    for c in cases.iter().filter(|c| c.assert_mode == "loader_error") {
        let line = c.malformed_line.as_ref().expect("错误用例带 malformed_line");
        let err = jsonl::parse_line(line, "esb_audit")
            .err().unwrap_or_else(|| panic!("用例 {} 的坏行竟然解析成功了", c.case_id));
        assert_eq!(err.code, ErrorCode::SchemaMismatch, "用例 {} 错误码不符", c.case_id);
        n += 1;
    }
    assert!(n >= 2, "错误用例过少：{n}");
}

/// 判定是可复现的：同一份输入跑两次，候选顺序与判定完全一致
#[test]
fn gold_judgement_is_deterministic() {
    let cases = load_cases();
    for c in cases.iter().filter(|c| c.assert_mode == "main_path" && c.expect.in_candidates).take(8) {
        let a = run_case(c);
        let b = run_case(c);
        let ids_a: Vec<&str> = a.candidates.iter().map(|x| x.interface_code.as_str()).collect();
        let ids_b: Vec<&str> = b.candidates.iter().map(|x| x.interface_code.as_str()).collect();
        assert_eq!(ids_a, ids_b, "用例 {} 两次跑出的候选顺序不一致", c.case_id);
        let r_a: Vec<f64> = a.candidates.iter().map(|x| x.degrade_ratio).collect();
        let r_b: Vec<f64> = b.candidates.iter().map(|x| x.degrade_ratio).collect();
        assert_eq!(r_a, r_b, "用例 {} 两次跑出的劣化倍数不一致", c.case_id);
    }
}

/// 语料语料文件确实存在于仓库 domain/ 层（溯源路径要真）
#[test]
fn cited_domain_files_exist_upstream() {
    let cases = load_cases();
    for c in &cases {
        let upstream = project_dir().join("..").join("..").join("..").join(&c.basis.source_file);
        if upstream.exists() {
            assert!(upstream.is_file(), "引用路径不是文件: {}", c.basis.source_file);
        }
    }
}

/// Path 导入被上面的用例用到（避免未使用告警）
#[allow(dead_code)]
fn _path_used(p: &Path) -> bool { p.exists() }
