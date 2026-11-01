"""Fail-closed P1 ratchet. Missing evidence is never a passing zero."""
import math

ROWS = {
    "cold_launch": "p95 <400 ms AND native/Electron ≤0.50",
    "open": "native/Electron ≤0.50; main-thread stall ≤1 frame",
    "scrub_cached": "p95 ≤refresh interval+1 ms AND ratio ≤0.60",
    "scrub_random_seek": "preview ≤refresh interval+2 ms; full ≤150 ms; ratio ≤0.60 (4K long-GOP)",
    "playback": "60 s at 60 fps; zero drops; frame-work p95 ≤half frame interval",
    "playback_cost": "CPU ratio ≤0.60; GPU-time ratio ≤0.80; energy ratio ≤0.70",
    "av": "matched device timestamps: mean absolute offset ≤1 ms; p95 jitter ≤2 ms",
    "edit_repaint": "p95 ≤refresh interval+1 ms; ratio ≤0.60; main-thread work ≤4 ms/frame",
    "idle": "CPU <1% AND ratio ≤0.50; ≤2 wakeups/s per idle extension",
    "memory_small": "family footprint ≤Electron −150 MB; shared CPU/GPU allocations counted once",
}


def percentile(values, q):
    values = sorted(values)
    if not values or any(not math.isfinite(v) or v < 0 for v in values):
        raise ValueError("Expected nonempty finite nonnegative measurements")
    # Nearest rank: p95 of 30 observations is rank 29, never an interpolated tail.
    return values[max(0, math.ceil(q * len(values)) - 1)]


def distribution(values):
    return {"n": len(values), "p50": percentile(values, .5), "p95": percentile(values, .95)} if values else None


def ratio(native, electron):
    return native / electron if electron > 0 else None


def check(row, native, electron, *, hz=60, runs=30, conditions=True, endpoints=True):
    """Each side: metric -> observations. Enforce ALL clauses at both quantiles.

    Endpoint/condition failures block acceptance even if a measured diagnostic is fast.
    """
    missing, failures = [], []
    if not math.isfinite(hz) or hz <= 0 or runs < 30:
        raise ValueError('Gate acceptance requires a valid refresh rate and at least 30 runs')
    interval = 1000 / hz

    def values(metric, paired=False):
        a, b = native.get(metric, []), electron.get(metric, [])
        if len(a) < runs or (paired and len(b) < runs):
            missing.append(metric + ": fewer than required valid runs")
            return None
        if any(not math.isfinite(v) or v < 0 for v in a + b):
            missing.append(metric + ': invalid measurement')
            return None
        return a, b

    def absolute(metric, bound, *, strict=False, every=False):
        pair = values(metric, True)
        if pair:
            n = max(pair[0]) if every else percentile(pair[0], .95)
            if n >= bound if strict else n > bound:
                failures.append(metric)

    def relative(metric, bound):
        pair = values(metric, True)
        if pair:
            for q in (.5, .95):
                r = ratio(percentile(pair[0], q), percentile(pair[1], q))
                if r is None:
                    missing.append(metric + ": zero reference denominator")
                elif r > bound:
                    failures.append(metric + f" p{int(q*100)} ratio")

    if row == "cold_launch":
        absolute("latency_ms", 400, strict=True)
        relative("latency_ms", .5)
    elif row == "open":
        relative("latency_ms", .5)
        absolute("main_max_ms", interval, every=True)
    elif row in {"scrub_cached", "edit_repaint"}:
        absolute("latency_ms", interval + 1)
        relative("latency_ms", .6)
        if row == "edit_repaint":
            absolute("main_max_ms", 4, every=True)
    elif row == "scrub_random_seek":
        absolute("preview_ms", interval + 2)
        absolute("full_ms", 150)
        relative("full_ms", .6)
    elif row == "playback":
        absolute("drops", 0, every=True)
        # Playback is the 60 fps row even on the 120 Hz display rig. Each run's
        # frame-work p95 must fit half of the content's 16.67 ms frame interval.
        absolute("frame_work_p95_ms", 1000 / 60 / 2, every=True)
        samples = values("duration_s", True)
        if samples and min(samples[0]) < 60:
            failures.append("duration_s")
        samples = values("fps", True)
        if samples and min(samples[0]) < 60:
            failures.append("fps")
    elif row == "playback_cost":
        for metric, bound in [("cpu_percent", .6), ("gpu_ms", .8), ("energy_j", .7)]:
            relative(metric, bound)
    elif row == "av":
        absolute("mean_abs_offset_ms", 1, every=True)
        absolute("jitter_p95_ms", 2, every=True)
    elif row == "idle":
        absolute("cpu_percent", 1, strict=True)
        relative("cpu_percent", .5)
        absolute("extension_wakeups_s", 2, every=True)
    elif row == "memory_small":
        pair = values("footprint_mb", True)
        if pair:
            for q in (.5, .95):
                if percentile(pair[0], q) > percentile(pair[1], q) - 150:
                    failures.append(f"footprint p{int(q*100)}")
    else:
        raise ValueError("Unknown row")
    if not conditions:
        missing.append("required machines/conditions not certified")
    if not endpoints:
        missing.append("required endpoint or workload unavailable")
    return {"status": "FAIL" if failures else "INCOMPLETE" if missing else "PASS",
            "failures": sorted(set(failures)), "missing": sorted(set(missing))}
