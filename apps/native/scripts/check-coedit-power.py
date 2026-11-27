#!/usr/bin/env python3
"""Fail the co-edit merge gate unless a disposable PM_PERF trace meets the spec.

Input is a JSON object with samples: idle_start/end (10 s), begin, rest_start/end
(at least 15 s after begin, then 10 s), playback_base_start/end and
playback_agent_start/end. Use coedit_begin, coedit_metrics, coedit_end and
coedit_load on a temporary 1080p project. Playback samples must include measured
ui_frame_p95_ms from the native frame trace. coedit_load supplies 20 writes/s.
This validator never opens an app or sends input to a user's window.
"""
import json
import sys

def require(condition, message):
    if not condition:
        raise AssertionError(message)

def span(trace, name, seconds):
    a, b = trace[name + "_start"], trace[name + "_end"]
    require(b["accepted_host_s"] - a["accepted_host_s"] >= seconds, name + " sample is too short")
    return a, b

def cpu(a, b):
    return 100 * (b["coedit"]["process_cpu_s"] - a["coedit"]["process_cpu_s"]) / (b["accepted_host_s"] - a["accepted_host_s"])

def drops(a, b):
    return sum(b[k] - a[k] for k in ("skipped_targets", "held_frames"))

def check(trace):
    idle_a, idle_b = span(trace, "idle", 10)
    rest_a, rest_b = span(trace, "rest", 10)
    require(rest_a["accepted_host_s"] - trace["begin"]["accepted_host_s"] >= 15, "rest must start 15 s after the last event")
    for a, b, label in ((idle_a, idle_b, "no run"), (rest_a, rest_b, "rest")):
        require(a["coedit"]["shell_frames"] == b["coedit"]["shell_frames"], label + ": toolbar requested frames")
        require(a["coedit"]["timeline"]["frames"] == b["coedit"]["timeline"]["frames"], label + ": timeline requested frames")
    require(cpu(rest_a, rest_b) <= cpu(idle_a, idle_b) + .5, "rest adds more than 0.5% CPU")
    base_a, base_b = span(trace, "playback_base", 2)
    agent_a, agent_b = span(trace, "playback_agent", 2)
    require(agent_b["revision"] - agent_a["revision"] >= 40, "20 agent transactions/s were not exercised")
    require(drops(agent_a, agent_b) <= drops(base_a, base_b), "agent activity adds dropped/held frames")
    require(agent_b["coedit"]["timeline"]["paint_p95_ms"] <= .4, "effect paint exceeds 0.4 ms p95")
    require(agent_b["ui_frame_p95_ms"] <= base_b["ui_frame_p95_ms"] + 1., "agent activity adds more than 1 ms UI p95")
    return {"idle_frames": 0, "rest_cpu_delta_percent": cpu(rest_a, rest_b) - cpu(idle_a, idle_b),
            "effect_p95_ms": agent_b["coedit"]["timeline"]["paint_p95_ms"]}

if __name__ == "__main__":
    with open(sys.argv[1], encoding="utf-8") as source:
        print(json.dumps(check(json.load(source))))
