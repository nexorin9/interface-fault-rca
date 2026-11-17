#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""金标准夹具生成器 —— 把手写用例表 + 真实语料落成 fixtures/gold/。

这个脚本**不生产期望结论**：期望结论全部写死在下面的 CASE_TABLE 与 PROFILE 表里，
由人工依据 domain/ 语料与 spec.md 判定纪律写出；脚本只做三件机械的事：

  1. 从 domain/cases/*.json **逐字拷贝**语料案例（payload 与 expect 原样搬运）；
  2. 按 traffic profile 生成该场景的接口流水行（时间戳按 t_min 相对锚点换算）；
  3. 把语料出处（来源文件 + 文号 + 条款）、域案例 id、期望结论拼成一行 JSON。

**没有**任何一步会运行被测代码再把输出抄回期望值，因此不存在自证循环。
金标准回归测试 tests/gold_regression.rs 会反向校验：每条用例的 payload 与
domain/cases/ 原文逐字一致，且期望结论与主路径实际判定一致（不一致即红）。

用法：
    python3 scripts/build_gold_cases.py                 # 用仓库内 ../../../domain/cases
    python3 scripts/build_gold_cases.py --corpus <dir>  # 指定语料目录
"""

from __future__ import annotations

import argparse
import json
import shutil
from pathlib import Path

# ── 锚点：t_min=0 对应的时刻（2026-11-09 02:00:00 +08:00）────────────────────
ANCHOR_MS = 1794160800000

# ── 文号（每个语料文件对应的官方依据）──────────────────────────────────────────
DOC_NO = {
    "prescription_review_cases": "《医疗机构处方审核规范》（国卫办医发〔2018〕14号）配套口径 /《处方管理办法》（卫生部令第53号）",
    "settlement_qc_cases": "《医疗保障基金结算清单填写规范》（国家医保局）",
    "fraud_review_cases": "《医疗保障基金使用监督管理条例》（国务院令第735号）第38条、第40条",
    "drg_mdc_cases": "《国家医疗保障DRG分组与付费技术规范》《CHS-DRG分组方案》（医保办发〔2019〕36号）",
}

# ── 承载「强制业务判定」的接口 → 语料族 ────────────────────────────────────────
FAMILY_INTERFACE = {
    "prescription_review_cases": "ESB-RX-REVIEW",
    "settlement_qc_cases": "ESB-SETTLE-UPLOAD",
    "fraud_review_cases": "ESB-FEE-AUDIT",
    "drg_mdc_cases": "ESB-CASE-GROUP",
}

FAMILY_CLAUSE = {
    "prescription_review_cases": "处方审核规则体系 §3 特殊药品处方量限 / §4 审核流程与信息系统要求",
    "settlement_qc_cases": "结算清单填报硬规则 §3 / 质控硬校验 §4",
    "fraud_review_cases": "医保违规负面清单 §3 违规行为判定细化 / §4 智能审核规则条目",
    "drg_mdc_cases": "CHS-DRG 分组逻辑 §1 分组三步法 / §2 先期分组（Pre-MDC）",
}

# ── 语料案例 → 人工判定的 traffic profile ─────────────────────────────────────
# profile 决定「这条语料案例对应的接口在观测段应该是什么样」，
# expectation 由 PROFILE 表给出（规则见 SOURCES.md「判定口径」一节）。
#
#   degraded          规则要求必须给出否定结论（拦截/质控不通过/违规/先期分组）→ 接口在观测段劣化
#   normal            规则要求正常放行（不拦截/质控通过/不违规/常规分组）→ 接口表现正常
CASE_TABLE = {
    # 处方审核（rx-*）：expect.拦截 = true → degraded；= false → normal
    "rx-001": "degraded", "rx-002": "normal", "rx-003": "degraded", "rx-004": "degraded",
    "rx-005": "degraded", "rx-006": "normal", "rx-007": "degraded", "rx-008": "degraded",
    "rx-009": "degraded", "rx-010": "normal",
    # 结算清单质控（qc-*）：expect.通过 = false → degraded；= true → normal
    "qc-001": "degraded", "qc-002": "degraded", "qc-003": "degraded", "qc-004": "degraded",
    "qc-005": "normal", "qc-006": "degraded", "qc-007": "degraded", "qc-008": "normal",
    # 费用违规核查（fraud-*）：expect.违规 非空 → degraded；= null → normal
    "fraud-001": "degraded", "fraud-002": "normal", "fraud-003": "degraded",
    "fraud-004": "degraded", "fraud-005": "degraded", "fraud-006": "degraded",
    "fraud-007": "degraded", "fraud-008": "degraded", "fraud-009": "degraded",
    "fraud-010": "degraded",
    # 病案分组（drg-*）：expect.类型 = 先期分组 → degraded；常规分组 → normal
    "drg-001": "normal", "drg-002": "normal", "drg-003": "degraded", "drg-004": "degraded",
    "drg-005": "degraded", "drg-006": "degraded", "drg-007": "degraded", "drg-008": "normal",
    "drg-009": "normal", "drg-010": "normal",
}

# ── 边界条件负样本：在同一语料场景上叠加一个边界条件 ─────────────────────────
# 这些用例的语料部分标注「这条边界条件对应的业务场景」，
# 期望结论来自判定纪律（spec.md §4 证据守门 / §5 基线退化处置），不是业务规则。
BOUNDARY_TABLE = [
    # (用例 id, 派生自语料案例, profile, 说明)
    ("gold-bound-001", "rx-002", "insufficient_baseline", "基线样本不足（基线窗口内只有 5 条流水，低于 min_baseline_samples=8）→ 证据不足以谈劣化，应判不出"),
    ("gold-bound-002", "qc-005", "insufficient_baseline", "基线样本不足（结算清单上传在基线段样本太少）→ 应判不出"),
    ("gold-bound-003", "fraud-002", "insufficient_baseline", "基线样本不足（费用审核回传在基线段样本太少）→ 应判不出"),
    ("gold-bound-004", "drg-009", "insufficient_baseline", "基线样本不足（分组回传在基线段样本太少）→ 应判不出"),
    ("gold-bound-005", "rx-006", "baseline_jitter", "基线自身抖动过大（基线段 CV>max_baseline_cv=1.2，中位数不可信）→ 应判不出"),
    ("gold-bound-006", "qc-008", "baseline_jitter", "基线自身抖动过大（结算上传基线段抖动）→ 应判不出"),
    ("gold-bound-007", "rx-010", "below_threshold", "劣化倍数未过阈值（劣化 1.2 倍 < 阈值 1.5）→ 应判不出"),
    ("gold-bound-008", "drg-002", "below_threshold", "劣化倍数未过阈值（分组回传只轻微变慢）→ 应判不出"),
    ("gold-bound-009", "fraud-003", "tamper_outside_window", "证据引用落在窗口外 → 证据守门整条丢弃（OUTSIDE_WINDOW）"),
    ("gold-bound-010", "qc-004", "tamper_nonexistent", "证据引用指向不存在的行 → 证据守门整条丢弃（NONEXISTENT_REF）"),
    ("gold-bound-011", "fraud-009", "unbound_entity", "接口码未在 ESB 接口登记表中映射 → 标 unbound 且不进候选（UNBOUND_REF）"),
    ("gold-bound-012", "drg-005", "ambiguous_code", "接口码一码多服务（字典歧义）→ 拒绝归因，整批不入事件表（AmbiguousInterface）"),
    ("gold-bound-013", "qc-006", "malformed_line", "来源行不是合法 JSON → 报 SchemaMismatch，不静默丢弃"),
    ("gold-bound-014", "rx-003", "missing_timestamp", "来源行缺时间戳 → 报 SchemaMismatch，不塞 0 伪装成正常行"),
]

# ── traffic profile → (期望判定, 是否进候选, 期望原因码/错误码, 流量构造) ───────
# 期望结论写在 PROFILE 里（人工判定），脚本不推导。
PROFILE = {
    "degraded": {
        "verdict": "接口自身劣化", "in_candidates": True, "reason_code": None,
        "negativity": None, "condition": "观测段该接口连续超时/5xx：承载强制业务判定的接口不可用",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500],
    },
    "normal": {
        "verdict": "证据不足", "in_candidates": False, "reason_code": "VERDICT_INSUFFICIENT",
        "negativity": "应判不出", "condition": "观测段该接口表现正常（业务规则本就不要求拦截/拒绝）",
        "base_dur": 100, "cur_dur": 104, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 200, 200, 200],
    },
    "insufficient_baseline": {
        "verdict": "证据不足", "in_candidates": False, "reason_code": "EVIDENCE_MISSING",
        "negativity": "应判不出", "condition": "基线窗口内样本不足（<8 条），不足以计算可信的劣化倍数",
        "base_dur": 100, "cur_dur": 360, "base_n": 5, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500],
    },
    "baseline_jitter": {
        "verdict": "证据不足", "in_candidates": False, "reason_code": "EVIDENCE_MISSING",
        "negativity": "应判不出", "condition": "基线段自身抖动过大（CV>1.2），基线中位数不可信",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "jitter": True,
    },
    "below_threshold": {
        "verdict": "证据不足", "in_candidates": False, "reason_code": "VERDICT_INSUFFICIENT",
        "negativity": "应判不出", "condition": "劣化倍数未超过阈值 1.5，不构成劣化",
        "base_dur": 100, "cur_dur": 120, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 200, 200, 200],
    },
    "unbound_entity": {
        "verdict": "接口自身劣化", "in_candidates": False, "reason_code": "UNBOUND_REF",
        "negativity": "应拒绝", "condition": "接口码未映射到 ESB 接口登记表：行保留但标 unbound，证据不可用，整条不进候选",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "unmapped_code": True,
    },
    "ambiguous_code": {
        "verdict": None, "in_candidates": False, "reason_code": None,
        "negativity": "应拒绝",
        "condition": "接口码在登记表中映射到多个服务：宁可少给结论，不可错连 → 整批拒绝入表",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "ambiguous_code": True,
    },
    "tamper_outside_window": {
        "verdict": None, "in_candidates": False, "reason_code": "OUTSIDE_WINDOW",
        "negativity": "应拒绝", "condition": "候选引用的证据行落在证据窗口之外 → 整条丢弃",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "tamper": "outside",
    },
    "tamper_nonexistent": {
        "verdict": None, "in_candidates": False, "reason_code": "NONEXISTENT_REF",
        "negativity": "应拒绝", "condition": "候选引用了不存在的证据行 ID → 整条丢弃",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "tamper": "nonexistent",
    },
    "malformed_line": {
        "verdict": None, "in_candidates": False, "reason_code": None,
        "negativity": "应报错", "condition": "来源行不是合法 JSON → 报 SchemaMismatch",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "malformed": "json",
    },
    "missing_timestamp": {
        "verdict": None, "in_candidates": False, "reason_code": None,
        "negativity": "应报错", "condition": "来源行缺时间戳 → 报 SchemaMismatch（不塞 0）",
        "base_dur": 100, "cur_dur": 360, "base_n": 10, "cur_n": 4,
        "cur_rc": [200, 500, 200, 500], "malformed": "missing_ts",
    },
}


def build_rows(profile: dict, interface_code: str) -> list[dict]:
    """按 profile 生成接口流水行（相对锚点的分钟偏移）。"""
    rows: list[dict] = []
    base_n, cur_n = profile["base_n"], profile["cur_n"]
    base_dur, cur_dur = profile["base_dur"], profile["cur_dur"]
    jitter = profile.get("jitter", False)
    # 基线段：t_min 从 -10 起，每分钟一条
    for i in range(base_n):
        dur = base_dur
        if jitter and i in (3, 8):
            dur = 1200          # 两次尖峰把基线自身方差拉大
        rows.append({"t_min": -10 + i, "d": dur, "r": 200})
    # 观测段：t_min 从 0 起，每分钟一条
    for i in range(cur_n):
        rc = profile["cur_rc"][i % len(profile["cur_rc"])]
        dur = cur_dur + (i % 3) * 8
        rows.append({"t_min": i, "d": dur, "r": rc})
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(description="生成金标准夹具 fixtures/gold/")
    ap.add_argument("--corpus", default="", help="语料目录（默认 ../../../domain/cases）")
    ap.add_argument("--out", default="", help="输出目录（默认 <项目>/fixtures/gold）")
    args = ap.parse_args()

    here = Path(__file__).resolve().parent
    project = here.parent
    corpus_dir = Path(args.corpus).resolve() if args.corpus else (project / ".." / ".." / ".." / "domain" / "cases").resolve()
    out_dir = Path(args.out).resolve() if args.out else (project / "fixtures" / "gold").resolve()

    if not corpus_dir.is_dir():
        print(f"[FATAL] 语料目录不存在: {corpus_dir}")
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)
    corpus_out = out_dir / "corpus"
    corpus_out.mkdir(parents=True, exist_ok=True)

    # 逐字拷贝语料文件（供测试离线核对）
    corpus_docs: dict[str, dict] = {}
    for family in sorted(FAMILY_INTERFACE):
        src = corpus_dir / f"{family}.json"
        if not src.exists():
            print(f"[FATAL] 缺语料文件: {src}")
            return 2
        shutil.copy2(src, corpus_out / f"{family}.json")
        corpus_docs[family] = json.loads(src.read_text(encoding="utf-8"))

    cases: list[dict] = []
    source_rows: list[list[str]] = []

    # ① 语料派生用例
    for family, doc in corpus_docs.items():
        for c in doc["cases"]:
            cid = c["case_id"]
            profile_name = CASE_TABLE[cid]
            p = PROFILE[profile_name]
            code = FAMILY_INTERFACE[family]
            if p.get("unmapped_code"):
                code = "ESB-NOT-IN-REGISTRY"
            if p.get("ambiguous_code"):
                code = "RX_REVIEW"       # 登记表中一码两服务（歧义别名）
            rows = build_rows(p, code)
            case = {
                "case_id": f"gold-{cid}",
                "assert": "main_path",
                "kind": "负样本" if p["negativity"] else "正样本",
                "negativity": p["negativity"],
                "anchor_ms": ANCHOR_MS,
                "interface_code": code,
                "window": {"start_min": 0, "end_min": 10, "baseline_minutes": 30},
                "expect": {
                    "verdict": p["verdict"],
                    "in_candidates": p["in_candidates"],
                    "reason_code": p["reason_code"],
                },
                "basis": {
                    "source_file": f"domain/cases/{family}.json",
                    "doc_no": DOC_NO[family],
                    "clause": FAMILY_CLAUSE[family],
                    "domain_case_id": cid,
                },
                "condition": p["condition"],
                "payload": c["input"],
                "domain_expect": c["expect"],
                "rows": rows,
            }
            if p.get("malformed"):
                case["assert"] = "loader_error"
                case["malformed_line"] = (
                    '{"source":"esb_audit","interface_code":"%s","duration_ms":100,' % code
                    + ('"response_code":200' if p["malformed"] == "missing_ts" else '"response_code":200')
                    + "}"
                )
            if p.get("tamper"):
                case["assert"] = "gate_drop"
                case["tamper"] = {
                    "mode": p["tamper"],
                    "refs": (["flow#a0002"] if p["tamper"] == "outside" else ["flow#a9999"]),
                    "gate_context": {
                        "window_ids": ["flow#a0001"],
                        "all_ids": ["flow#a0001", "flow#a0002"],
                        "unbound_ids": [],
                    },
                }
            if p.get("ambiguous_code"):
                case["dict_overlay"] = {
                    "ambiguous_alias": "RX_REVIEW",
                    "second_service_name": "处方审核引擎（另一套部署）",
                }
            cases.append(case)
            source_rows.append([
                case["case_id"], f"domain/cases/{family}.json", cid,
                DOC_NO[family], FAMILY_CLAUSE[family], p["negativity"] or "—",
            ])

    # ② 边界条件用例
    for case_id, domain_cid, profile_name, cond in BOUNDARY_TABLE:
        p = PROFILE[profile_name]
        # 找到父语料案例
        parent = None
        family_of = None
        for family, doc in corpus_docs.items():
            for c in doc["cases"]:
                if c["case_id"] == domain_cid:
                    parent, family_of = c, family
        if parent is None:
            print(f"[FATAL] 边界用例引用了不存在的语料案例: {domain_cid}")
            return 2
        code = FAMILY_INTERFACE[family_of]
        if p.get("unmapped_code"):
            code = "ESB-NOT-IN-REGISTRY"
        if p.get("ambiguous_code"):
            code = "RX_REVIEW"
        rows = build_rows(p, code)
        case = {
            "case_id": case_id,
            "assert": "main_path",
            "kind": "负样本",
            "negativity": p["negativity"],
            "anchor_ms": ANCHOR_MS,
            "interface_code": code,
            "window": {"start_min": 0, "end_min": 10, "baseline_minutes": 30},
            "expect": {
                "verdict": p["verdict"],
                "in_candidates": p["in_candidates"],
                "reason_code": p["reason_code"],
            },
            "basis": {
                "source_file": f"domain/cases/{family_of}.json",
                "doc_no": DOC_NO[family_of],
                "clause": FAMILY_CLAUSE[family_of],
                "domain_case_id": domain_cid,
            },
            "condition": cond,
            "payload": parent["input"],
            "domain_expect": parent["expect"],
            "rows": rows,
        }
        if p.get("malformed"):
            case["assert"] = "loader_error"
            case["malformed_line"] = (
                '{"source":"esb_audit","interface_code":"%s","duration_ms":100,"response_code":200}' % code
                if p["malformed"] == "missing_ts" else
                '{"source":"esb_audit","interface_code":"%s","duration_ms":100,"response_code":' % code
            )
        if p.get("tamper"):
            case["assert"] = "gate_drop"
            case["tamper"] = {
                "mode": p["tamper"],
                "refs": (["flow#a0002"] if p["tamper"] == "outside" else ["flow#a9999"]),
                "gate_context": {
                    "window_ids": ["flow#a0001"],
                    "all_ids": ["flow#a0001", "flow#a0002"],
                    "unbound_ids": [],
                },
            }
        if p.get("ambiguous_code"):
            case["dict_overlay"] = {
                "ambiguous_alias": "RX_REVIEW",
                "second_service_name": "处方审核引擎（另一套部署）",
            }
        cases.append(case)
        source_rows.append([
            case_id, f"domain/cases/{family_of}.json", domain_cid,
            DOC_NO[family_of], FAMILY_CLAUSE[family_of], p["negativity"] or "—",
        ])

    # 写 cases.jsonl
    lines = "\n".join(json.dumps(c, ensure_ascii=False, sort_keys=False) for c in cases)
    (out_dir / "cases.jsonl").write_text(lines + "\n", encoding="utf-8")

    # 写 SOURCES.md（每例来源文件 + 文号）
    md = [
        "# 金标准夹具来源表（每例的来源文件与文号）",
        "",
        "> 语料正文逐字拷贝自 `fixtures/gold/corpus/*.json`（与仓库 `domain/cases/*.json` 同源同内容）；",
        "> 期望结论由人工依据语料规则与 `spec.md` 判定纪律写出，**未**运行被测代码反推。",
        "> 金标准回归测试 `tests/gold_regression.rs` 会反向校验 payload 与语料逐字一致。",
        "",
        "## 判定口径（为什么有的案例是故障、有的不是）",
        "",
        "本产品监控的是院内 ESB 接口接缝，其中一部分接口承载**规范强制的业务判定**：",
        "",
        "- 《医疗机构处方审核规范》：所有处方经审核通过后方可进入划价收费和调配环节，未经审核通过不得收费调配；",
        "- 《医疗保障基金结算清单填写规范》：清单是医保结算唯一依据，DRG/DIP 分组全部基于清单而非首页；",
        "- 《医疗保障基金使用监督管理条例》第38条：违规费用须逐条判定并可追溯；",
        "- 《CHS-DRG分组方案》（医保办发〔2019〕36号）：分组结果须回传医保办。",
        "",
        "因此判定口径是：",
        "",
        "| 语料案例的期望 | 该接口在观测段的表现 | 期望结论 |",
        "|---|---|---|",
        "| 规则要求必须给出否定结论（拦截=true / 质控不通过 / 违规≠null / 先期分组） | 承载该判定的接口劣化（超时/5xx） | **接口自身劣化**，进候选，带证据行 |",
        "| 规则要求正常放行（拦截=false / 质控通过 / 违规=null / 常规分组） | 接口表现正常 | **不应判出**（业务正常量不是故障） |",
        "",
        "边界条件类负样本是在同一语料场景上叠加一个边界条件（基线样本不足、基线抖动、倍数未过阈值、",
        "证据引用越界/不存在、实体未映射、接口码歧义、来源行非法），其期望结论来自判定纪律"
        "（`spec.md` §4 证据守门 / §5 基线退化处置），语料部分用于标注这条边界条件对应的业务场景。",
        "",
        "丢弃原因码的读法：`VERDICT_INSUFFICIENT` = 判定本身是「证据不足」（劣化倍数没超阈值等）；",
        "`EVIDENCE_MISSING` = 该判定不携带证据引用（基线样本不足 / 基线抖动过大时判定不给引用），",
        "守门按「无证据引用」整条丢弃；`OUTSIDE_WINDOW` / `NONEXISTENT_REF` / `UNBOUND_REF` = 证据引用越界、",
        "引用不存在、实体未映射；歧义接口码与坏行不产生候选，分别以 `AmbiguousInterface` 拒收与 `SchemaMismatch` 报错体现。",
        "",
        "## 逐例来源",
        "",
        "| 用例 | 语料文件 | 域案例 | 文号 | 条款 | 类型 |",
        "|---|---|---|---|---|---|",
    ]
    for row in source_rows:
        md.append("| " + " | ".join(row) + " |")
    md += [
        "",
        "## 口径统计",
        "",
        f"- 总例数：{len(cases)}",
        f"- 正样本（应判出）：{sum(1 for c in cases if c['kind'] == '正样本')}",
        f"- 负样本（应判不出 / 应拒绝 / 应报错）：{sum(1 for c in cases if c['kind'] == '负样本')}",
        "",
        "锚点：`anchor_ms = %d`（2026-11-09 02:00:00 +08:00），行内 `t_min` 为相对锚点的分钟偏移；",
        "基线段取 t_min ∈ [-10, -1]，观测段取 t_min ∈ [0, 3]，基线窗口长度 30 分钟。",
        "",
    ]
    (out_dir / "SOURCES.md").write_text("\n".join(md) + "\n", encoding="utf-8")

    neg = sum(1 for c in cases if c["kind"] == "负样本")
    print(f"已生成 {len(cases)} 例（正样本 {len(cases) - neg} / 负样本 {neg}）→ {out_dir}")
    print(f"  cases.jsonl   {len(cases)} 行")
    print(f"  corpus/       {len(corpus_docs)} 个语料文件（逐字拷贝）")
    print(f"  SOURCES.md    {len(source_rows)} 行来源表")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
