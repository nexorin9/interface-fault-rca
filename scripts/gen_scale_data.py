#!/usr/bin/env python3
"""合成规模数据生成器（task 13）。

按 spec.md §3.1 统一事件表生成单夜事件数据：
  flow  >= 500000 行
  log   >= 100000 行
  metric 序列 >= 10000 条

确定性种子：同一 --seed 两次运行逐字节一致。注入四类劣化形态作为可控真值：
  step / spike / ramp / jitter。
输出 data/scale/ 下的 *-events.jsonl 与 manifest.json。
"""

import argparse
import hashlib
import json
import os
import random
from datetime import datetime, timezone

WHITELIST = ["source", "event_kind", "interface_code", "service_name", "vendor_name", "host"]

INTERFACES = [
    ("ESB-ORDER-CREATE", "住院医嘱开立服务", "院内自研", "esb-gw-01"),
    ("ESB-REPORT-QUERY", "检验报告查询服务", "院内自研", "esb-gw-02"),
    ("EXT-HIS-PATIENT-BASE", "院外 HIS 患者主索引服务", "外部厂商 A", "ext-his-adapter-01"),
]

# 窗口：2026-11-08 20:00:00Z ~ 2026-11-09 08:00:00Z
BASE_MS = int(datetime(2026, 11, 8, 20, 0, 0, tzinfo=timezone.utc).timestamp() * 1000)
SPAN_MS = 12 * 3600 * 1000


def tsid(source, ts_ms, code, service, kind):
    """与 Rust compute_tsid 一致：长度前缀 + sha256。"""
    h = hashlib.sha256()
    for part in [source, str(ts_ms), code, service, kind]:
        raw = part.encode("utf-8")
        h.update(len(raw).to_bytes(8, "little"))
        h.update(raw)
    return h.hexdigest()


def apply_shape(base_duration, shape, progress, rng):
    """按劣化形态与窗口进度(0..1)返回当前 duration_ms。"""
    noise = rng.gauss(0, base_duration * 0.05)
    if shape == "step":
        factor = 1.0 if progress < 0.5 else 3.5
    elif shape == "spike":
        factor = 50.0 if 0.62 <= progress < 0.625 else 1.0
    elif shape == "ramp":
        factor = 1.0 + 2.5 * progress
    elif shape == "jitter":
        factor = 1.0
        noise = rng.gauss(0, base_duration * 0.9)
    else:
        factor = 1.0
    return max(1.0, base_duration * factor + noise)


def gen_flow(rng, count, shape_by_code):
    rows = []
    for i in range(count):
        code, service, vendor, host = INTERFACES[i % len(INTERFACES)]
        ts = BASE_MS + int(SPAN_MS * (i / count))
        shape = shape_by_code.get(code, "stable")
        base = 100.0 if code != "EXT-HIS-PATIENT-BASE" else 140.0
        dur = apply_shape(base, shape, i / count, rng)
        rc = 500 if (shape == "spike" and rng.random() < 0.3) else 200
        row = {
            "tags": {
                "source": "esb_audit", "event_kind": "flow",
                "interface_code": code, "service_name": service,
                "vendor_name": vendor, "host": host,
            },
            "ts_ms": ts,
            "fields": {
                "response_code": str(rc), "duration_ms": f"{dur:.2f}",
                "caller_system": "HIS", "pipeline_version": "esb_audit@v1", "unbound": "false",
            },
            "__row_id": f"flow#a{i + 1:05d}",
            "__tsid": tsid("esb_audit", ts, code, service, "flow"),
        }
        rows.append(row)
    return rows


def gen_log(rng, count, shape_by_code):
    rows = []
    msgs = ["connection failed after {n} ms", "timeout waiting for downstream {n}",
            "upstream returned 5xx at {n}", "retry {n} exhausted"]
    for i in range(count):
        code, service, _, host = INTERFACES[i % len(INTERFACES)]
        ts = BASE_MS + int(SPAN_MS * (i / count))
        shape = shape_by_code.get(code, "stable")
        level = "ERROR" if (shape != "stable" and rng.random() < 0.25) else "INFO"
        msg = rng.choice(msgs).format(n=rng.randint(1, 9999))
        row = {
            "tags": {
                "source": "app_log", "event_kind": "log",
                "interface_code": code, "service_name": service, "host": host,
            },
            "ts_ms": ts,
            "fields": {
                "level": level, "message": msg, "stack_head": "",
                "pipeline_version": "app_log@v1", "unbound": "false",
            },
            "__row_id": f"log#a{i + 1:05d}",
            "__tsid": tsid("app_log", ts, code, service, "log"),
        }
        rows.append(row)
    return rows


def gen_metric(rng, count):
    rows = []
    for i in range(count):
        code, service, _, host = INTERFACES[i % len(INTERFACES)]
        ts = BASE_MS + int(SPAN_MS * (i / count))
        name = rng.choice(["cpu_usage", "mem_usage", "thread_pool_queue", "gc_pause"])
        value = round(rng.uniform(0.1, 0.9), 4)
        row = {
            "tags": {
                "source": "prom", "event_kind": "metric",
                "interface_code": code, "host": host,
            },
            "ts_ms": ts,
            "fields": {"metric_name": name, "value": str(value), "pipeline_version": "prom@v1"},
            "__row_id": f"metric#a{i + 1:05d}",
            "__tsid": tsid("prom", ts, code, service, "metric"),
        }
        rows.append(row)
    return rows


def write_jsonl(path, rows):
    with open(path, "w", encoding="utf-8") as f:
        for r in rows:
            f.write(json.dumps(r, ensure_ascii=False, sort_keys=True, separators=(",", ":")) + "\n")


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default="data/scale")
    ap.add_argument("--seed", type=int, default=20261109)
    ap.add_argument("--flow", type=int, default=500000)
    ap.add_argument("--log", type=int, default=100000)
    ap.add_argument("--metric", type=int, default=10000)
    ap.add_argument("--shapes", default="ESB-ORDER-CREATE:step,ESB-REPORT-QUERY:spike,EXT-HIS-PATIENT-BASE:ramp")
    args = ap.parse_args()

    os.makedirs(args.out, exist_ok=True)
    shape_by_code = {}
    for item in args.shapes.split(","):
        if ":" in item:
            c, s = item.split(":", 1)
            shape_by_code[c] = s

    rng = random.Random(args.seed)
    flows = gen_flow(rng, args.flow, shape_by_code)
    logs = gen_log(rng, args.log, shape_by_code)
    metrics = gen_metric(rng, args.metric)

    write_jsonl(os.path.join(args.out, "flow-events.jsonl"), flows)
    write_jsonl(os.path.join(args.out, "log-events.jsonl"), logs)
    write_jsonl(os.path.join(args.out, "metric-events.jsonl"), metrics)

    manifest = {
        "seed": args.seed,
        "generated_at": "2026-11-09T00:00:00Z",  # 固定值，保证逐字节一致
        "time_range_ms": [BASE_MS, BASE_MS + SPAN_MS],
        "counts": {"flow": len(flows), "log": len(logs), "metric": len(metrics)},
        "injected_shapes": shape_by_code,
        "tag_whitelist": WHITELIST,
        "files": {
            "flow-events.jsonl": {"rows": len(flows), "sha256": sha256_file(os.path.join(args.out, "flow-events.jsonl"))},
            "log-events.jsonl": {"rows": len(logs), "sha256": sha256_file(os.path.join(args.out, "log-events.jsonl"))},
            "metric-events.jsonl": {"rows": len(metrics), "sha256": sha256_file(os.path.join(args.out, "metric-events.jsonl"))},
        },
    }
    with open(os.path.join(args.out, "manifest.json"), "w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, sort_keys=True, indent=2)
        f.write("\n")
    print(f"generated flow={len(flows)} log={len(logs)} metric={len(metrics)} -> {args.out}")


if __name__ == "__main__":
    main()
