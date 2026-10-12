"""Render S8 result JSON files as Markdown tables for docs/engine/spikes/S8.md.

Usage: python3 -I report.py results/<dir> [scenario ...]
       python3 -I report.py results run3/full run5/full ...
       python3 -I report.py --cb results run3/sliced-2ms-align run6/sliced-2ms-align ...

`--cb` reads the raw logs (`results/<run>/raw/<scenario>-{a,b}.jsonl`, not committed)
and prints the distribution of B's command-buffer GPU spans while A was playing (from
A's first tick to its last completion), against the scenario's slice length.

With scenario names, only those rows are printed, in that order. A name of the form
`<run>/<scenario>` reads `results/<run>/<scenario>.json` and labels the row with the
run.
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
    after_b = "-" if not at else str(at.get("late_backlog_after_b", 0))
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
        after_b,
        "-" if pres is None else f"{pres['irregular_intervals']}",
        f"{100 * v.get('external_gpu_before', 0):.0f}/{100 * v.get('external_gpu_after', 0):.0f}",
        display(v),
    ]


def display(v):
    """Display state during the scenario: on, locked (on, screen locked), or off."""
    if v.get("display_asleep"):
        return "off"
    return "locked" if v.get("screen_locked") else "on"


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
    "b after B",
    "irr",
    "ext %",
    "disp",
]


def quantile(xs, p):
    return xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))]


def cb_row(d, name):
    run, scen = name.split("/", 1)
    raw = os.path.join(d, run, "raw")
    a = [json.loads(l) for l in open(os.path.join(raw, scen + "-a.jsonl"))]
    t0 = min(v["tick_ns"] for v in a if "tick_ns" in v)
    t1 = max(v["done_ns"] for v in a if v.get("done_ns"))
    spans = []
    for line in open(os.path.join(raw, scen + "-b.jsonl")):
        v = json.loads(line)
        if v["kind"] == "cb" and v["gpu_begin_ns"] >= t0 and v["gpu_end_ns"] <= t1:
            spans.append((v["gpu_end_ns"] - v["gpu_begin_ns"]) / 1e6)
    spans.sort()
    meta = json.load(open(os.path.join(d, run, scen + ".json")))
    sl = (meta.get("b") or {}).get("slice_ms")
    n = len(spans)
    if n == 0:
        return [run, scen, f(sl), "0"] + ["-"] * 6
    over = lambda x: f"{100 * sum(s > x for s in spans) / n:.1f}"
    return [
        run,
        scen,
        f(sl),
        str(n),
        f(quantile(spans, 0.5), 2),
        f(quantile(spans, 0.9), 2),
        f(quantile(spans, 0.99), 2),
        f(spans[-1], 2),
        over(2.0),
        over(2.5),
    ]


def main():
    if sys.argv[1] == "--cb":
        d = sys.argv[2]
        head = ["run", "scenario", "slice ms", "cbs", "p50", "p90", "p99", "max", "% > 2 ms", "% > 2.5 ms"]
        print("| " + " | ".join(head) + " |")
        print("|" + "---|" * len(head))
        for n in sys.argv[3:]:
            print("| " + " | ".join(cb_row(d, n)) + " |")
        return
    d = sys.argv[1]
    names = sys.argv[2:]
    mixed = any("/" in n for n in names)
    data = {} if mixed else load(d)
    names = names or sorted(data)
    head = (["run"] if mixed else []) + HEAD
    print("| " + " | ".join(head) + " |")
    print("|" + "---|" * len(head))
    for n in names:
        if mixed:
            run, scen = n.split("/", 1)
            path = os.path.join(d, run, scen + ".json")
            if os.path.exists(path):
                print("| " + " | ".join([run] + row(json.load(open(path)))) + " |")
        elif n in data:
            print("| " + " | ".join(row(data[n])) + " |")


main()
