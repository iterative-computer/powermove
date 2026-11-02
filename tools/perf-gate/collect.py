#!/usr/bin/env python3
"""Bundled macOS row adapter; optional counters stay missing, never inferred.

The gate accepts a replacement adapter using this same JSON protocol when the
Mac instrumentation lane supplies complete physical/energy/main-work streams.
"""

import json
from pathlib import Path
import random
import signal
import sys
import time
from types import SimpleNamespace

from driver import App, Processes, observe, position
from trace import load as load_trace


def latency(value):
    metrics = {
        key: value[key]
        for key in ("own_share_ms", "on_screen_ms")
        if value.get(key) is not None
    }
    evidence = {
        "input_origin": "command-accepted",
        "own_share_endpoint": "input-to-gpu-completion"
        if "own_share_ms" in metrics
        else None,
        "screen_endpoint": "input-to-presentedTime"
        if value.get("physical_present") is True
        else None,
        "matching_frame_identity": value.get("physical_present") is True,
    }
    return metrics, evidence


def collect(request, app_factory=App, proc_factory=Processes, observe_fn=observe):
    row, side = request["row"], request["side"]
    scratch = Path(request["scratch"])
    scenario = request["scenario"]
    args = SimpleNamespace(
        output=scratch / "raw.json",
        native=Path(request["apps"]["native"]),
        electron=Path(request["apps"]["electron"]),
        electron_args=request.get("electron_args", []),
        reference=Path(request["reference"]),
        inherit_process_group=True,
        allow_missing_display=True,
    )
    proc = proc_factory()
    result = {
        "metrics": {},
        "evidence": {"input_origin": "command-accepted"},
        "diagnostics": {},
    }
    app = app_factory(side, args, scratch)
    try:
        if row == "cold_launch":
            result["diagnostics"]["controller_ready_ms"] = app.ready_ms
            result["diagnostics"]["endpoint"] = app.ready["endpoint"]
            return result
        opened = app.action("open", path=request["project"])
        if row == "open":
            result["metrics"], result["evidence"] = latency(opened)
            if opened.get("physical_present"):
                result["metrics"]["latency_ms"] = opened["on_screen_ms"]
                result["evidence"]["latency_endpoint"] = (
                    "open-request-to-first-new-viewer-presentedTime"
                )
            result["diagnostics"]["open"] = opened
            return result
        if row in ("scrub_cached", "edit_repaint", "scrub_random_seek"):
            fps = scenario["fps"]
            if row == "scrub_cached":
                for t in (0, 1 / fps, 0):
                    position(app, t)
                value = app.action("seek", seconds=1 / fps)
            elif row == "edit_repaint":
                position(app, 0)
                value = app.action(
                    "edit", value=37 if request["index"] % 2 == 0 else 73
                )
            else:
                # Same deterministic frame on each side; no app-specific timing
                # or random generator state can change the paired workload.
                frame = random.Random(
                    scenario.get("seed", 1200) + request["index"]
                ).randrange(1, scenario["frames"])
                position(app, 0)
                value = app.action("seek", seconds=frame / fps)
                result["diagnostics"]["target_frame"] = frame
            metrics, evidence = latency(value)
            result["metrics"].update(metrics)
            result["evidence"].update(evidence)
            result["diagnostics"]["action"] = value
            if row == "scrub_cached":
                result["evidence"]["exact_cached_frame"] = (
                    value.get("cache_hit") is True
                )
            elif row == "scrub_random_seek":
                # The existing command seam only exposes the full frame. It has
                # no low-resolution-preview event; that clause remains missing.
                if value.get("physical_present"):
                    result["metrics"]["full_ms"] = value["on_screen_ms"]
                    result["evidence"]["full_endpoint"] = (
                        "input-to-full-resolution-presentedTime"
                    )
                result["evidence"]["long_gop_4k_verified"] = (
                    scenario.get("long_gop_4k_verified") is True
                )
            return result
        if row in ("idle", "memory_small"):
            app.request("pause")
            time.sleep(scenario.get("settle_s", 5))
            counters = observe_fn(app, proc, scenario.get("sample_s", 10))
            result["diagnostics"]["accounting"] = counters
            if row == "idle":
                result["metrics"]["cpu_percent"] = counters["cpu_percent"]
                # Even an extension-free document loads built-in extensions.
                # Family wakeups cannot substitute for per-extension attribution.
                # An operator must justify the stable window independently.
                result["evidence"]["stable_idle_window"] = (
                    scenario.get("stable_idle_window") is True
                )
            else:
                result["metrics"]["footprint_mb"] = counters["family_phys_footprint_mb"]
                result["evidence"]["whole_process_family"] = True
                result["evidence"]["deduplicated_cpu_gpu_allocations"] = False
            return result
        if row in ("playback", "playback_cost", "av"):
            position(app, 0)
            app.request("play")
            counters = observe_fn(app, proc, 60)
            playing = app.request("status").get("playing") is True
            app.request("pause")
            if not playing:
                raise RuntimeError(
                    "Playback stopped before full window; reject interval"
                )
            result["diagnostics"]["accounting"] = counters
            if row == "playback_cost":
                result["metrics"]["cpu_percent"] = counters["cpu_percent"]
                result["evidence"].update(
                    process_family_cpu=True,
                    full_playback_window=counters["duration_s"] >= 60,
                )
            elif row == "playback":
                result["metrics"].update(
                    duration_s=counters["duration_s"], fps=scenario["fps"]
                )
            return result
        raise ValueError("Unknown row: " + row)
    finally:
        app.close()
        if side == "native" and app.trace_path.exists():
            result["diagnostics"]["service_trace"] = load_trace(app.trace_path)


def main():
    request = json.load(sys.stdin)
    # App subprocesses inherit this adapter's group. The outer runner owns and
    # reaps that group on timeout/crash, including Electron helpers.
    result = collect(request)
    result.update(
        version=1,
        row=request["row"],
        side=request["side"],
        index=request["index"],
        scenario_sha256=request["scenario"]["sha256"],
    )
    print(json.dumps(result, allow_nan=False))


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(143))
    main()
