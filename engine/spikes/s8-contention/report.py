"""Render S8 result JSON files as Markdown tables for docs/engine/spikes/S8.md.

Usage: python3 -I report.py results/<dir> [scenario ...]

With scenario names, only those rows are printed, in that order.
"""

import json
import os
import sys


def load(d):
    out = {}
    for n in os.listdir(d):
        if n.endswith(".json") and n != "summary.json":
            v = json.load(open(os.path.join(d, n)))
            out[v["scenario"]] = v
    return out


def f(x, nd=1):
    return "-" if x is None else f"{x:.{nd}f}"


def row(v):
    a = v["a"] or {}
    b = v.get("b") or {}
    dr = a.get("drops", {})
    ctl = v.get("control")
    at = v.get("attribution") or {}
    vt = b.get("vt") or []
    bl = b.get("blender") or {}
    pres = a.get("presentation")
    attr = "-"
    if at:
        attr = "/".join(
            str(at.get(k, 0))
            for k in ("late_b_held_start", "late_b_concurrent", "late_behind_own_backlog", "late_other")
        )
    mode = b.get("mode")
    b_fps = f(b.get("export_fps")) if mode and mode != "none" else "-"
    b_cb = f(b.get("cb_gpu_ms", {}).get("p99")) if mode and mode != "none" else "-"
    return [
        v["scenario"],
        "-" if ctl is None else str(ctl["drops_1f"]),
        f"{dr.get('deadline_1_frame')}/{dr.get('deadline_2_frames')}/{dr.get('deadline_3_frames')}",
        str(dr.get("skipped_pacing", 0) + dr.get("skipped_ring_full", 0)),
        f"{f(a['latency_ms']['p50'])}/{f(a['latency_ms']['p99'])}/{f(a['latency_ms']['max'])}",
        f(a["completion_jitter_ms"]["p99"]),
        f"{f(a['gpu_exec_ms']['p50'])}/{f(a['gpu_exec_ms']['p99'])}",
        f(a["gpu_start_delay_ms"]["p99"]),
        b_fps,
        b_cb,
        "/".join(f"{s['fps']:.0f}" for s in vt) or "-",
        str(bl.get("frames", "-")) if bl else "-",
        attr,
        "-" if pres is None else f"{pres['irregular_intervals']}",
        f"{100 * v.get('external_gpu_before', 0):.0f}/{100 * v.get('external_gpu_after', 0):.0f}",
    ]


HEAD = [
    "scenario",
    "ctl",
    "drops 1f/2f/3f",
    "skip",
    "lat p50/p99/max",
    "jit p99",
    "A gpu p50/p99",
    "start p99",
    "B fps",
    "B cb p99",
    "VT fps",
    "Bl fr",
    "late s/c/b/o",
    "irr",
    "ext %",
]


def main():
    d = sys.argv[1]
    data = load(d)
    names = sys.argv[2:] or sorted(data)
    print("| " + " | ".join(HEAD) + " |")
    print("|" + "---|" * len(HEAD))
    for n in names:
        if n in data:
            print("| " + " | ".join(row(data[n])) + " |")


main()
