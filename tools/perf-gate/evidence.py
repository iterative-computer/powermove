"""Endpoint/workload contracts, paired reduction, and non-discarding outliers."""

from statistics import median

from budgets import ROWS, check, distribution
from quiet import blockers, finite

CONTRACTS = {
    "cold_launch": {
        "latency_endpoint": "process-start-to-first-interactive-presentedTime"
    },
    "open": {
        "latency_endpoint": "open-request-to-first-new-viewer-presentedTime",
        "main_thread_census": True,
    },
    "scrub_cached": {
        "own_share_endpoint": "input-to-gpu-completion",
        "screen_endpoint": "input-to-presentedTime",
        "exact_cached_frame": True,
    },
    "scrub_random_seek": {
        "preview_endpoint": "input-to-preview-presentedTime",
        "full_endpoint": "input-to-full-resolution-presentedTime",
        "long_gop_4k_verified": True,
    },
    "playback": {"complete_frame_census": True, "main_and_render_work_census": True},
    "playback_cost": {
        "process_family_cpu": True,
        "app_attributed_gpu": True,
        "app_attributed_energy": True,
        "full_playback_window": True,
    },
    "av": {"matched_device_timestamps": True, "content_onsets_only": True},
    "edit_repaint": {
        "own_share_endpoint": "input-to-gpu-completion",
        "screen_endpoint": "input-to-presentedTime",
        "shell_repaint_included": True,
        "main_thread_census": True,
    },
    "idle": {"per_extension_wakeups": True, "stable_idle_window": True},
    "memory_small": {
        "deduplicated_cpu_gpu_allocations": True,
        "whole_process_family": True,
    },
}

# Only these named counters can enter gate math. Diagnostic counters remain raw.
METRICS = {
    "cold_launch": ("latency_ms",),
    "open": ("latency_ms", "main_max_ms"),
    "scrub_cached": ("own_share_ms", "on_screen_ms"),
    "scrub_random_seek": ("preview_ms", "full_ms"),
    "playback": ("drops", "frame_work_p95_ms", "duration_s", "fps"),
    "playback_cost": ("cpu_percent", "gpu_ms", "energy_j"),
    "av": ("mean_abs_offset_ms", "jitter_p95_ms"),
    "edit_repaint": ("own_share_ms", "on_screen_ms", "main_max_ms"),
    "idle": ("cpu_percent", "extension_wakeups_s"),
    "memory_small": ("footprint_mb",),
}
EXTRA_METRICS = {
    row: ("own_share_ms", "on_screen_ms")
    for row in ("cold_launch", "open", "scrub_random_seek")
}


def endpoint_problems(row, native, electron):
    reasons = []
    for side, value in [("native", native), ("electron", electron)]:
        evidence = value.get("evidence", {})
        for key, expected in CONTRACTS[row].items():
            actual = evidence.get(key)
            matches = (
                actual is expected if isinstance(expected, bool) else actual == expected
            )
            if not matches:
                reasons.append(side + ": " + key + " unavailable/mismatched")
        if evidence.get("input_origin") != "command-accepted":
            reasons.append(side + ": matching command-accepted input origin required")
    # Physical callbacks must be tied to the driven frame, not the next arbitrary
    # window present. Adapters attest identity, not just a timestamp source name.
    if row in {
        "cold_launch",
        "open",
        "scrub_cached",
        "scrub_random_seek",
        "edit_repaint",
    }:
        for side, value in [("native", native), ("electron", electron)]:
            if value.get("evidence", {}).get("matching_frame_identity") is not True:
                reasons.append(side + ": matching frame identity unavailable")
    return reasons


def outliers(values):
    """Flag a far Tukey tail (3 IQR); retain every value in quantiles and verdict.

    A zero-IQR sample flags every deviation from the median. This is diagnostic,
    never permission to trim a slow measurement or repeat until it disappears.
    """
    if len(values) < 4:
        return []
    ordered = sorted(values)
    middle = len(ordered) // 2
    low = median(ordered[:middle])
    high = median(ordered[(len(ordered) + 1) // 2 :])
    spread = high - low
    return [
        i
        for i, value in enumerate(values)
        if value < low - 3 * spread or value > high + 3 * spread
    ]


def summarize(data):
    result = {}
    context = data["context"]
    for row in ROWS:
        metrics = tuple(dict.fromkeys(METRICS[row] + EXTRA_METRICS.get(row, ())))
        sides = {side: {m: [] for m in metrics} for side in ("native", "electron")}
        observed = {side: {m: [] for m in metrics} for side in ("native", "electron")}
        metric_pairs = {m: [] for m in metrics}
        accepted, rejected, missing, seen = [], [], [], set()
        for attempt in (a for a in data["attempts"] if a["row"] == row):
            reasons = list(attempt.get("errors", []))
            index = attempt["index"]
            if (
                not isinstance(index, int)
                or isinstance(index, bool)
                or index < 0
                or index in seen
            ):
                reasons.append("invalid or duplicate pair index")
            seen.add(index)
            states = attempt.get("conditions", [])
            # Before/after each app and at least one in-run sample per side.
            phases = {s.get("phase") for s in states}
            if (
                not {
                    s + ":" + p
                    for s in ("native", "electron")
                    for p in ("before", "during", "after")
                }
                <= phases
            ):
                reasons.append("missing before/during/after condition samples")
            baseline = data.get("baseline")
            for state in states:
                reasons.extend(
                    blockers(
                        state,
                        hz=context["hz"],
                        baseline=baseline,
                        allowed_apps=context.get("allowed_apps", []),
                        max_load_per_cpu=context.get("max_load_per_cpu", 0.5),
                    )
                )
            if not all(isinstance(attempt.get(s), dict) for s in sides):
                reasons.append("incomplete pair")
            if reasons:
                rejected.append(
                    {"index": attempt["index"], "reasons": sorted(set(reasons))}
                )
                continue
            native, electron = attempt["native"], attempt["electron"]
            accepted.append(attempt["index"])
            missing.extend(endpoint_problems(row, native, electron))
            for metric in metrics:
                a, b = (attempt[s].get("metrics", {}).get(metric) for s in sides)
                for side, value in [("native", a), ("electron", b)]:
                    if finite(value) and value >= 0:
                        observed[side][metric].append(value)
                # Intersection by pair ID and metric. Never compare differently
                # selected samples or use a missing side's implicit zero.
                if not all(finite(v) and v >= 0 for v in (a, b)):
                    if metric in METRICS[row]:
                        missing.append(metric + ": missing/invalid paired observation")
                    continue
                sides["native"][metric].append(a)
                sides["electron"][metric].append(b)
                metric_pairs[metric].append(attempt["index"])
        certified = (
            context.get("rig_certified") is True
            and context.get("quality_certified") is True
            and context.get("rig") != "stand-in"
        )
        outcome = check(
            row,
            sides["native"],
            sides["electron"],
            hz=context["hz"],
            runs=context.get("runs", 30),
            conditions=certified,
            endpoints=not missing,
        )
        outcome["missing"] = sorted(set(outcome["missing"] + missing))
        result[row] = dict(
            outcome,
            budget=ROWS[row],
            accepted_pairs=accepted,
            rejected_pairs=rejected,
            stats={
                s: {m: distribution(v) for m, v in metrics.items()}
                for s, metrics in observed.items()
            },
            paired_stats={
                s: {m: distribution(v) for m, v in metrics.items()}
                for s, metrics in sides.items()
            },
            paired_indices=metric_pairs,
            outliers={
                s: {m: outliers(v) for m, v in metrics.items()}
                for s, metrics in observed.items()
            },
        )
    return result
