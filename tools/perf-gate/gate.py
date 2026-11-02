#!/usr/bin/env python3
"""30 independent pairs per P1 row. This command never builds or deletes apps."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
from functools import partial
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import tempfile
import threading
import time

from budgets import ROWS, ratio
from driver import gui_lock
from evidence import METRICS, summarize
from quiet import MacProbe, blockers, finite

HERE = Path(__file__).resolve().parent
FROZEN_REFERENCE = "07ed5fa17ee640659cb075f9cad1af2095011d60"


class Unquiet(RuntimeError):
    """Environmental rejection eligible for a bounded replacement pair."""


def digest(path):
    hasher = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(block)
    return hasher.hexdigest()


def executable(path):
    path = Path(path).expanduser().resolve()
    if path.suffix == ".app":
        import plistlib

        info = plistlib.loads((path / "Contents/Info.plist").read_bytes())
        path = path / "Contents/MacOS" / info["CFBundleExecutable"]
    if not path.is_file() or not os.access(path, os.X_OK):
        raise ValueError("Missing executable: " + str(path))
    return str(path)


def validate(config):
    """Validate every workload before opening either app. No real profiles."""
    if config.get("version") != 1:
        raise ValueError("Expected configuration version 1")
    context = config["context"]
    runs = context.get("runs", 30)
    if (
        context["hz"] not in (60, 120)
        or not isinstance(runs, int)
        or isinstance(runs, bool)
        or runs < 30
    ):
        raise ValueError("P1 requires 60/120 Hz and at least 30 runs per row")
    if context.get("rig") not in ("xdr-120", "base-60", "stand-in"):
        raise ValueError("Rig must be xdr-120, base-60, or stand-in")
    if (
        context["rig"] == "xdr-120"
        and context["hz"] != 120
        or context["rig"] in ("base-60", "stand-in")
        and context["hz"] != 60
    ):
        raise ValueError("Rig and refresh disagree")
    if context.get("quality_certified") and not context.get("quality_note", "").strip():
        raise ValueError("Certified quality requires a record of matched settings")
    threshold = context.get("max_load_per_cpu", 0.5)
    if not isinstance(threshold, (int, float)) or not 0 < threshold <= 0.5:
        raise ValueError("Quiet load threshold must be >0 and ≤0.5 per logical CPU")
    for values in (context.get("allowed_apps", []), config.get("electron_args", [])):
        if not isinstance(values, list) or any(not isinstance(v, str) for v in values):
            raise ValueError(
                "Application exemptions and Electron arguments must be string arrays"
            )
    adapter = config.get("adapter", [sys.executable, str(HERE / "collect.py")])
    if (
        not isinstance(adapter, list)
        or not adapter
        or any(not isinstance(a, str) for a in adapter)
    ):
        raise ValueError("Adapter must be a nonempty argument array")
    apps = config["apps"]
    for side in ("native", "electron"):
        apps[side] = executable(apps[side])
    reference = Path(config["reference"]).expanduser().resolve()
    revision = subprocess.run(
        ["git", "-C", str(reference), "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
        timeout=20,
    ).stdout.strip()
    clean = subprocess.run(
        ["git", "-C", str(reference), "diff", "--quiet", "HEAD", "--"], timeout=20
    )
    if revision != FROZEN_REFERENCE or clean.returncode:
        raise ValueError("Electron reference must be clean frozen v1.2.0")
    config["reference"] = str(reference)
    for row in ROWS:
        scenario = config["scenarios"][row]
        path = Path(scenario["project"]).expanduser().resolve()
        if not path.is_file() or digest(path) != scenario["sha256"]:
            raise ValueError(row + ": prepared public project missing or hash mismatch")
        if scenario.get("public_fixture") is not True:
            raise ValueError(row + ": only prepared public/synthetic fixtures allowed")
        scenario["project"] = str(path)
        prepared = json.loads(path.read_text())
        if (
            not isinstance(prepared, dict)
            or not finite(scenario.get("fps"))
            or scenario["fps"] <= 0
            or scenario["fps"] != prepared.get("fps")
            or not isinstance(scenario.get("frames"), int)
            or scenario["frames"] < 2
        ):
            raise ValueError(row + ": prepared workload fps/frames mismatch")
        if row in ("playback", "playback_cost") and scenario["fps"] != 60:
            raise ValueError("Playback rows require actual 60 fps workload")
        if row == "scrub_random_seek":
            media = Path(scenario["media"]).expanduser().resolve()
            if not media.is_file() or digest(media) != scenario["media_sha256"]:
                raise ValueError("Random seek media missing or hash mismatch")
            # Probe actual codec, dimensions and keyframe intervals before any
            # timing. A declaration in a manifest is not proof of long GOP.
            probe = subprocess.run(
                [
                    "ffprobe",
                    "-v",
                    "error",
                    "-select_streams",
                    "v:0",
                    "-show_streams",
                    "-show_frames",
                    "-show_entries",
                    "stream=codec_name,width,height:frame=key_frame,best_effort_timestamp_time",
                    "-of",
                    "json",
                    str(media),
                ],
                check=True,
                capture_output=True,
                text=True,
                timeout=120,
            )
            info = json.loads(probe.stdout)
            stream = info["streams"][0]
            keys = [
                float(f["best_effort_timestamp_time"])
                for f in info["frames"]
                if f.get("key_frame") == 1
            ]
            gaps = [b - a for a, b in zip(keys, keys[1:])]
            verified = (
                stream["width"] >= 3840
                and stream["height"] >= 2160
                and stream["codec_name"] in ("h264", "hevc", "av1")
                and bool(gaps)
                and max(gaps) >= 1
            )
            if not verified:
                raise ValueError(
                    "Random seek requires actual 4K long-GOP video (≥1 s keyframe interval)"
                )
            scenario["long_gop_4k_verified"] = True
            scenario["media_probe"] = dict(stream, max_keyframe_interval_s=max(gaps))
    requested = context.get("runs", 30)
    attempts = config.get("max_attempts_per_row", requested + 15)
    if (
        not isinstance(attempts, int)
        or isinstance(attempts, bool)
        or not requested <= attempts <= requested + 100
    ):
        raise ValueError(
            "Attempt cap must cover runs and permit at most 100 condition replacements"
        )
    if not 0 < config.get("side_timeout_s", 180) <= 240:
        raise ValueError(
            "Each side timeout must be positive and ≤240 s (9 minute pair lock)"
        )
    return config


def invoke(
    argv, request, *, timeout, group_started=None, group_finished=None, cancel=None
):
    """A fresh bounded adapter process per side; reap its process group on error."""
    process = subprocess.Popen(
        argv,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        if group_started:
            group_started(process.pid)
        pending_input = json.dumps(request, allow_nan=False)
        deadline = time.monotonic() + timeout
        while True:
            if cancel is not None and cancel.is_set():
                raise RuntimeError("Adapter cancelled; retain partial pair")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise subprocess.TimeoutExpired(argv, timeout)
            try:
                stdout, stderr = process.communicate(
                    pending_input, timeout=min(0.25, remaining)
                )
                break
            except subprocess.TimeoutExpired as error:
                pending_input = None
                stderr = error.stderr or b""
                if isinstance(stderr, bytes):
                    stderr = stderr.decode(errors="replace")
                (Path(request["scratch"]) / "adapter.stderr.log").write_text(stderr)
        (Path(request["scratch"]) / "adapter.stderr.log").write_text(stderr)
        if process.returncode:
            raise RuntimeError("Adapter failed; inspect retained adapter.stderr.log")

        def invalid_number(value):
            raise ValueError("Nonfinite JSON number: " + value)

        value = json.loads(stdout, parse_constant=invalid_number)
        if (
            not isinstance(value, dict)
            or not isinstance(value.get("metrics"), dict)
            or not isinstance(value.get("evidence"), dict)
        ):
            raise ValueError("Adapter must return metrics and evidence objects")
        for field in ("version", "row", "side", "index"):
            if value.get(field) != request[field]:
                raise ValueError("Adapter response identity mismatch: " + field)
        if value.get("scenario_sha256") != request["scenario"]["sha256"]:
            raise ValueError("Adapter response scenario hash mismatch")
        return value
    finally:

        def signal_group(sig):
            try:
                os.killpg(process.pid, sig)
                return True
            except ProcessLookupError:
                return False

        try:
            # Descendants inherit this adapter's unique group, including a child
            # app left behind by a crashed adapter. Never select by app name.
            signal_group(signal.SIGTERM)
            try:
                process.communicate(timeout=3)
            except subprocess.TimeoutExpired:
                signal_group(signal.SIGKILL)
                process.communicate(timeout=3)
            # Helpers may have closed stdio but ignored the parent's teardown.
            if signal_group(0):
                signal_group(signal.SIGKILL)
        finally:
            if group_finished:
                group_finished(process.pid)


def measured_side(config, row, side, attempt, scratch, probe, call=invoke):
    def sample(phase):
        return dict(probe.sample(), phase=side + ":" + phase)

    attempt["conditions"].append(sample("before"))
    reasons = blockers(
        attempt["conditions"][-1],
        hz=config["context"]["hz"],
        allowed_apps=config["context"].get("allowed_apps", []),
        max_load_per_cpu=config["context"].get("max_load_per_cpu", 0.5),
    )
    if reasons:
        raise Unquiet("; ".join(reasons))
    scenario = config["scenarios"][row]
    # Keep original fixtures immutable, even when an editor saves to its source.
    source = Path(scenario["project"])
    project = scratch / ("document" + source.suffix)
    import shutil

    shutil.copyfile(source, project)
    request = dict(
        version=1,
        row=row,
        side=side,
        index=attempt["index"],
        project=str(project),
        scenario=scenario,
        scratch=str(scratch),
        apps=config["apps"],
        reference=config["reference"],
        electron_args=config.get("electron_args", []),
        hz=config["context"]["hz"],
    )
    argv = config.get("adapter", [sys.executable, str(HERE / "collect.py")])
    # Probe off the timing thread; the same low-rate probe runs on both sides.
    with ThreadPoolExecutor(max_workers=1) as pool:
        cancelled = threading.Event()
        adapter = (
            partial(
                invoke,
                group_started=probe.own_group,
                group_finished=probe.release_group,
                cancel=cancelled,
            )
            if call is invoke
            else call
        )
        future = pool.submit(
            adapter, argv, request, timeout=config.get("side_timeout_s", 180)
        )
        try:
            while True:
                attempt["conditions"].append(sample("during"))
                if future.done():
                    break
                threading.Event().wait(0.5)
            attempt[side] = future.result()
        finally:
            cancelled.set()
            attempt["conditions"].append(sample("after"))
            if digest(source) != scenario["sha256"]:
                raise RuntimeError(
                    "Prepared source changed; pair invalid, source retained"
                )


def save(data, output):
    data["results"] = summarize(data)
    # New output directories only. Preserve a raw checkpoint after every attempt.
    temporary = output / "report.next.json"
    temporary.write_text(json.dumps(data, indent=2, allow_nan=False) + "\n")
    temporary.replace(output / "report.json")
    report = table(data)
    (output / "report.md").write_text(report)
    return report


def table(data):
    def stat(result, side, metric):
        value = result["stats"][side].get(metric)
        return (
            f"{value['p50']:.3f} / {value['p95']:.3f} (n={value['n']})"
            if value
            else "—"
        )

    mode = data.get("mode", "live")
    lines = [
        f"P1 gate — {mode}; rig: {data['context'].get('rig', 'uncertified')}; "
        f"{data['context']['hz']} Hz",
        "",
        "Values are p50 / p95, nearest rank. Outliers are flagged and retained.",
        "",
        "| Row | Native (primary) | Electron (primary) | Native own share (ms) | Native on screen (ms) | Electron on screen (ms) | Paired ratio p50 / p95 | Verdict |",
        "|---|---:|---:|---:|---:|---:|---:|---|",
    ]
    for row, result in data["results"].items():
        primary = METRICS[row][0]
        relative_metric = (
            "on_screen_ms" if row in ("scrub_cached", "edit_repaint") else primary
        )
        pair = [
            result["paired_stats"][s].get(relative_metric)
            for s in ("native", "electron")
        ]
        comparisons = (
            [ratio(pair[0][q], pair[1][q]) for q in ("p50", "p95")] if all(pair) else []
        )
        relative = (
            " / ".join(f"{v:.3f}" if v is not None else "—" for v in comparisons) or "—"
        )
        lines.append(
            f"| {row} ({primary}) | {stat(result, 'native', primary)} | "
            f"{stat(result, 'electron', primary)} | {stat(result, 'native', 'own_share_ms')} | "
            f"{stat(result, 'native', 'on_screen_ms')} | {stat(result, 'electron', 'on_screen_ms')} | {relative} | {result['status']} |"
        )
    lines.extend(
        [
            "",
            "All clauses, paired metric distributions, rejected attempts and outlier indices are in report.json.",
        ]
    )
    for row, result in data["results"].items():
        lines.extend(["", f"{row}: {result['budget']}"])
        for reason in result["failures"] + result["missing"]:
            lines.append("- " + reason)
        if result["rejected_pairs"]:
            lines.append(
                f"- {len(result['rejected_pairs'])} rejected pairs retained with reasons"
            )
    for reason in data.get("blockers", []):
        lines.extend(["", "BLOCKED: " + reason])
    passed = not data.get("blockers") and all(
        r["status"] == "PASS" for r in data["results"].values()
    )
    lines.extend(
        [
            "",
            "REPLAY ONLY — no machine measurements or gate certification."
            if mode == "replay"
            else "This rig passes all rows; the other required rig remains a separate gate."
            if passed
            else "Gate P1 NOT PASSED.",
        ]
    )
    return "\n".join(lines) + "\n"


def run(config, output, probe, call=invoke):
    data = dict(
        schema="powermove.perf-gate.p1/v2",
        mode="live",
        context=config["context"],
        attempts=[],
        blockers=[],
        policy="30 pairs; no timing-based rejection",
        provenance={s: digest(p) for s, p in config["apps"].items()},
    )
    data["baseline"] = probe.sample()
    try:
        initial = blockers(
            data["baseline"],
            hz=config["context"]["hz"],
            allowed_apps=config["context"].get("allowed_apps", []),
            max_load_per_cpu=config["context"].get("max_load_per_cpu", 0.5),
        )
        if initial:
            data["blockers"].extend(initial)
            return data
        for row in ROWS:
            accepted = 0
            requested = config["context"].get("runs", 30)
            for index in range(config.get("max_attempts_per_row", requested + 15)):
                attempt = dict(
                    row=row,
                    index=index,
                    conditions=[],
                    errors=[],
                    order=["native", "electron"]
                    if index % 2 == 0
                    else ["electron", "native"],
                )
                data["attempts"].append(attempt)
                try:
                    with gui_lock(config.get("lock_wait_seconds", 0)):
                        for side in attempt["order"]:
                            scratch = output / row / f"{index:03d}" / side
                            scratch.mkdir(parents=True)
                            measured_side(
                                config, row, side, attempt, scratch, probe, call
                            )
                except Unquiet as error:
                    attempt["errors"].append(str(error))
                    attempt["retryable_conditions"] = True
                except Exception as error:
                    attempt["errors"].append(str(error))
                save(data, output)
                outcome = data["results"][row]
                accepted = len(outcome["accepted_pairs"])
                print(
                    f"{row}: {accepted}/{requested} quiet pairs, attempt {index + 1}",
                    flush=True,
                )
                if accepted >= requested:
                    break
                if attempt["errors"] and not attempt.get("retryable_conditions"):
                    # A broken adapter is not repaired by repeatedly opening it.
                    data["blockers"].append(
                        row + ": collection interrupted; inspect retained attempt"
                    )
                    break
                time.sleep(config.get("retry_settle_s", 2))
    except BaseException as error:
        data["blockers"].append(type(error).__name__ + ": " + str(error))
    finally:
        save(data, output)
    return data


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=["p1"])
    parser.add_argument(
        "--config", type=Path, default=os.environ.get("PM_PERF_GATE_CONFIG")
    )
    parser.add_argument(
        "--replay",
        type=Path,
        help="Analyze recorded protocol fixtures; never run an app",
    )
    parser.add_argument("--output", type=Path, help="New private directory under /tmp")
    args = parser.parse_args(argv)
    output = args.output or Path(tempfile.mkdtemp(prefix="pm-gate-", dir="/tmp"))
    if args.output:
        if output.exists() or not output.resolve().is_relative_to(
            Path("/tmp").resolve()
        ):
            parser.error(
                "Output must be a new directory under /tmp (never overwrite prior evidence)"
            )
        output.mkdir(mode=0o700, parents=True)
    data = dict(
        schema="powermove.perf-gate.p1/v2",
        mode="live",
        context={"hz": 120, "rig": "uncertified"},
        attempts=[],
        blockers=[],
    )
    try:
        if args.replay:
            data = json.loads(args.replay.read_text())
            if data.get("schema") == "powermove.perf-gate.fixture/v1":
                data = expand_fixture(data)
            data["mode"] = "replay"
        else:
            if platform.system() != "Darwin":
                raise RuntimeError("Live gates require macOS; no Mac run was attempted")
            if not args.config:
                raise ValueError(
                    "Set PM_PERF_GATE_CONFIG or --config to a prepared public workload configuration"
                )
            config = validate(json.loads(Path(args.config).read_text()))
            data["context"] = config["context"]
            (output / "config.json").write_text(json.dumps(config, indent=2) + "\n")
            probe = MacProbe(output, config["apps"].values())
            data = run(config, output, probe)
    except (Exception, KeyboardInterrupt) as error:
        data["blockers"].append(str(error))
    report = save(data, output)
    print(report)
    print("Retained report: " + str(output / "report.md"))
    return (
        0
        if data["mode"] == "live"
        and not data.get("blockers")
        and all(r["status"] == "PASS" for r in data["results"].values())
        else 2
    )


def expand_fixture(fixture):
    """Compact synthetic recorded protocol responses, never a live run format."""
    import copy

    data = dict(
        schema="powermove.perf-gate.p1/v2",
        mode="replay",
        context=fixture["context"],
        baseline=fixture["baseline"],
        attempts=[],
        blockers=[],
    )
    for row, pair in fixture["rows"].items():
        for index in range(fixture["context"].get("runs", 30)):
            data["attempts"].append(
                dict(
                    row=row,
                    index=index,
                    errors=[],
                    native=copy.deepcopy(pair["native"]),
                    electron=copy.deepcopy(pair["electron"]),
                    conditions=[
                        dict(fixture["baseline"], phase=s + ":" + p)
                        for s in ("native", "electron")
                        for p in ("before", "during", "after")
                    ],
                )
            )
    return data


if __name__ == "__main__":

    def interrupted(_signum, _frame):
        raise KeyboardInterrupt(
            "Signal received; stop owned processes and retain results"
        )

    signal.signal(signal.SIGTERM, interrupted)
    sys.exit(main())
