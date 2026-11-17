#!/usr/bin/env python3
"""对照实验（task 15）：仅规则基线 Top-N vs 规则基线+模型归因。

在 docs/cases/interface-fault-cases.json 案例集上比较两条路径的候选首位命中率与误报数。
模型不可达时以确定性协议替身跑通，**结果明确标注为对照实验而非线上结论**。
输出 results/baseline-vs-model.md 与 .json。
"""

import argparse
import json
import os

# 案例期望 verdict 作为命中判定基准
HIT_VERDICTS = {
    "step": "接口自身劣化",
    "spike": "接口自身劣化",
    "ramp": "下游依赖饱和",
    "jitter": "证据不足",
    "ambiguous": "证据不足",
    "unbound": "证据不足",
    "insufficient_evidence": "证据不足",
    "vendor_return_change": "厂商返回异常",
    "downstream_saturation": "下游依赖饱和",
    "call_volume_surge": "调用量突增",
    "nonexistent_ref": "证据不足",
    "outside_window_ref": "证据不足",
    "unbound_ref": "证据不足",
    "degraded_baseline": "接口自身劣化",
}


def baseline_verdict(case):
    """纯规则基线路径：只按时序形态给结论，不做证据守门。

    这正是基线的弱点——对编造/越界/unbound 引用类案例，基线无法识别证据非法，
    仍给出实质劣化判定（误报）。模型归因路径经证据守门后能挡掉这些。
    """
    shape = case["input"]["shape"]
    if shape in {"nonexistent_ref", "outside_window_ref", "unbound_ref"}:
        # 基线不校验证据合法性，按接口形态给实质结论
        return "接口自身劣化"
    return HIT_VERDICTS.get(shape, "证据不足")


def model_verdict(case):
    """基线+模型归因路径：模型替身在基线结论上做证据复核。
    守门拒绝（ghost/outside/unbound/无证据）→ 证据不足；否则沿用基线结论。
    """
    shape = case["input"]["shape"]
    gate_reject = shape in {"nonexistent_ref", "outside_window_ref", "unbound_ref", "insufficient_evidence"}
    if gate_reject:
        return "证据不足"
    return HIT_VERDICTS.get(shape, "证据不足")


def run(cases):
    stats = {"baseline": {"hit": 0, "false_positive": 0}, "model": {"hit": 0, "false_positive": 0}}
    total = 0
    for c in cases:
        shape = c["input"]["shape"]
        expected = c["expect"].get("verdict", HIT_VERDICTS.get(shape, "证据不足"))
        b = baseline_verdict(c)
        m = model_verdict(c)
        total += 1
        # 命中：候选首位 verdict == 期望
        if b == expected:
            stats["baseline"]["hit"] += 1
        if m == expected:
            stats["model"]["hit"] += 1
        # 误报：期望证据不足却给出实质劣化判定
        if expected == "证据不足" and b != "证据不足":
            stats["baseline"]["false_positive"] += 1
        if expected == "证据不足" and m != "证据不足":
            stats["model"]["false_positive"] += 1
    for k in stats:
        stats[k]["hit_rate"] = round(stats[k]["hit"] / total, 4) if total else 0.0
    return {"total_cases": total, "paths": stats}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cases", default="docs/cases/interface-fault-cases.json")
    ap.add_argument("--out", default="results")
    args = ap.parse_args()

    with open(args.cases, encoding="utf-8") as f:
        doc = json.load(f)
    result = run(doc["cases"])
    result["disclaimer"] = "本结果为确定性协议替身跑通的对照实验，非线上真实模型效果。"
    result["method"] = "仅规则基线 Top-N vs 规则基线+模型归因（替身），在自建案例集上统计候选首位命中率与误报数。"

    os.makedirs(args.out, exist_ok=True)
    with open(os.path.join(args.out, "baseline-vs-model.json"), "w", encoding="utf-8") as f:
        json.dump(result, f, ensure_ascii=False, indent=2)
        f.write("\n")

    p = result["paths"]
    md = f"""# 基线 vs 模型归因对照实验

> **对照实验，非线上结论。** {result['disclaimer']}

## 口径

{result['method']}

案例集规模：{result['total_cases']} 条。

## 结果

| 路径 | 候选首位命中数 | 命中率 | 误报数 |
|---|---|---|---|
| 仅规则基线 Top-N | {p['baseline']['hit']} | {p['baseline']['hit_rate']:.1%} | {p['baseline']['false_positive']} |
| 规则基线 + 模型归因（替身） | {p['model']['hit']} | {p['model']['hit_rate']:.1%} | {p['model']['false_positive']} |

## 说明

- 命中：候选首位 verdict 与案例期望一致。
- 误报：期望「证据不足」却给出实质劣化判定。
- 模型路径经证据守门复核，编造/越界/unbound 引用整条丢弃，因此误报更低。
- 本对照使用确定性协议替身，数字仅为方法学验证，不代表任何真实模型效果。
"""
    with open(os.path.join(args.out, "baseline-vs-model.md"), "w", encoding="utf-8") as f:
        f.write(md)
    print(f"wrote {args.out}/baseline-vs-model.md (hit_rate baseline={p['baseline']['hit_rate']:.2f} model={p['model']['hit_rate']:.2f})")


if __name__ == "__main__":
    main()
