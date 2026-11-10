#!/usr/bin/env python3
"""Input-to-photon summary of one interactive run (the feel lane's measure).

Reads the two opt-in traces a run writes:
  PM_FEEL_TRACE   GPUI windows: {"ui":kind,"input":t,"draw":t,"presented":t}
  PM_ENGINE_TRACE the viewer:   per frame "input" (event) and "display" (glass)
Times are host seconds (NSEvent.timestamp / presentedTime clock). Prints, per
input kind, p50/p95/max input-to-photon in ms and how many exceed one refresh,
plus each window's paint cost (input to draw start, draw to glass).
  feel-report.py <run-dir> [--hz 120] [--since T] [--until T] [--json]
"""
import argparse, json, math
from pathlib import Path


def rows(path):
    if not path.exists():
        return []
    out = []
    for line in path.read_text().splitlines():
        line = line.strip()
        if line:
            try:
                out.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    return out


def dist(values):
    v = sorted(values)
    if not v:
        return None
    q = lambda p: v[min(len(v) - 1, math.ceil(len(v) * p) - 1)]
    return {"n": len(v), "p50": round(q(.5), 2), "p95": round(q(.95), 2), "max": round(v[-1], 2)}


def main():
    a = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    a.add_argument("run")
    a.add_argument("--hz", type=float, default=120.)
    a.add_argument("--since", type=float, default=0.)
    a.add_argument("--until", type=float, default=math.inf)
    a.add_argument("--json", action="store_true")
    o = a.parse_args()
    run = Path(o.run)
    refresh = 1000. / o.hz
    keep = lambda t: o.since <= t < o.until
    report = {"refresh_ms": round(refresh, 2), "ui": {}, "viewer": {}}

    ui = {}
    for r in rows(run / "feel.ndjson"):
        if r.get("presented", 0) > 0 and keep(r["input"]):
            ui.setdefault(r["ui"], []).append(r)
    for kind, rs in sorted(ui.items()):
        lat = [(r["presented"] - r["input"]) * 1000 for r in rs]
        report["ui"][kind] = {
            "input_to_photon_ms": dist(lat),
            "over_2_refresh": sum(x > refresh * 2 for x in lat),
            "input_to_draw_ms": dist([(r["draw"] - r["input"]) * 1000 for r in rs]),
            "frame_cpu_ms": dist([(r["draw"] - r["frame"]) * 1000 for r in rs if r.get("frame", 0) > r["input"]]),
            "draw_to_glass_ms": dist([(r["presented"] - r["draw"]) * 1000 for r in rs]),
        }

    frames = [r for r in rows(run / "engine.ndjson") if r.get("display") and r["display"] > 0]
    tagged = [r for r in frames if r.get("input") and keep(r["input"]) and r["display"] >= r["input"]]
    by = {}
    for r in tagged:
        by.setdefault(r.get("moment", "?"), []).append(r)
    for moment, rs in sorted(by.items()):
        report["viewer"][moment] = {
            "input_to_photon_ms": dist([(r["display"] - r["input"]) * 1000 for r in rs]),
            "input_to_worker_ms": dist([(r["worker"] - r["input"]) * 1000 for r in rs if r.get("worker")]),
            "worker_to_glass_ms": dist([(r["display"] - r["worker"]) * 1000 for r in rs if r.get("worker")]),
            "render_cpu_ms": dist([r["render_cpu_ms"] for r in rs if r.get("render_cpu_ms")]),
            "scale": dist([r.get("scale", 1) for r in rs]),
        }
    # Visible hitches: gaps between viewer presents inside an interaction burst.
    shown = sorted(r["display"] for r in tagged)
    gaps = [(y - x) * 1000 for x, y in zip(shown, shown[1:]) if y - x < .25]
    report["viewer_gaps_ms"] = dist(gaps)
    report["viewer_gaps_over_2_refresh"] = sum(g > 2 * refresh for g in gaps)

    if o.json:
        print(json.dumps(report))
        return
    print(f"refresh {report['refresh_ms']} ms")
    for side in ("ui", "viewer"):
        for kind, v in report[side].items():
            parts = " ".join(f"{k}={v[k]}" for k in v if v[k] is not None)
            print(f"{side:6} {kind:10} {parts}")
    print(f"viewer gaps {report['viewer_gaps_ms']} over 2 refreshes: {report['viewer_gaps_over_2_refresh']}")


if __name__ == "__main__":
    main()
