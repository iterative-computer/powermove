#!/usr/bin/env python3
"""Headless trace-only comparison. Never an acceptance: no pixels, no panels, gate stays open."""
import argparse
from collections import Counter
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import run

DEFAULT_SCRIPT = {"version": 1, "complete": False, "steps": [
    {"op": "reload"}, {"op": "seek", "time": 0.5}, {"op": "seek", "time": 0}, {"op": "dispose"}]}
TRACE_STEPS = ("reload", "dispose", "seek", "panel", "command", "advance")


def unordered(value, ids):
    """Key order and the object ids it numbers removed. The kernel keeps JSON objects
    sorted, so this tier separates that one known difference from real behavior."""
    if isinstance(value, list):
        return [unordered(item, ids) for item in value]
    if not isinstance(value, dict):
        return value
    out = {}
    for key in sorted(value):
        item = value[key]
        # The reference records the whole Electron kernel object here.
        if key == "args" and value.get("name") == "command.kernel":
            continue
        if key in ("$id", "$ref", "$token") and isinstance(item, str):
            out[key] = ids.setdefault(item, f"o{len(ids)+1}")
        else:
            out[key] = unordered(item, ids)
    return out


def behavior_diffs(left, right):
    a = run.validate_trace((left/"trace.jsonl").read_text())
    b = run.validate_trace((right/"trace.jsonl").read_text())
    ids_a, ids_b, context, out = {}, {}, "lifecycle", []
    for index in range(1, max(len(a), len(b))):
        x, y = (a[index] if index < len(a) else None), (b[index] if index < len(b) else None)
        for row in (x, y):
            if row and row.get("type") == "call":
                context = run.family(row.get("name", ""))
        if run.exact(unordered(x, ids_a)) != run.exact(unordered(y, ids_b)):
            out.append({"row": index, "family": context,
                        "type": (x or y).get("type"), "name": (x or y).get("name")})
    return out


def check_script(script):
    if script.get("version") != 1 or not script.get("steps") or any(
            step.get("op") not in TRACE_STEPS for step in script["steps"]):
        raise run.Blocked("invalid-script")


def record(command, request, role, root, repeat, timeout):
    """Three complete recordings per role; a differing second cannot hide the third."""
    first = root/role
    run.capture_trace(command, {**request, "role": role}, first, timeout)
    others = []
    for index in range(1, repeat):
        other = root/f"{role}-repeat-{index}"
        run.capture_trace(command, {**request, "role": role}, other, timeout)
        others.append(other)
    if any(run.compare(first, other, request["script"]) for other in others):
        raise run.Blocked(role+"-nondeterministic")
    return first


def compare_case(args, source, case, input_path, sha, native, reference, script, store=None):
    request = {"version": 1, "case": case, "extension": str(input_path), "input_sha256": sha,
               "clock_fixture": run.fixture_identity(), "reference_revision": args.reference_revision,
               "script_sha256": run.digest(json.dumps(script, sort_keys=True).encode()), "script": script,
               "trace_only": True}
    if store:
        request["store"] = store
    root = input_path.parent
    left = record(reference, request, "reference", root, args.repeat, args.timeout)
    right = record(native, request, "native", root, args.repeat, args.timeout)
    if run.tree_digest(input_path) != sha:
        raise run.Blocked("input-mutated")
    return run.compare(left, right, script), behavior_diffs(left, right)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference-root", type=Path, required=True)
    parser.add_argument("--reference-revision", choices=list(run.REFERENCES.values()), default=run.REFERENCE)
    parser.add_argument("--native-runner", type=Path, required=True)
    parser.add_argument("--corpus-root", type=Path, default=run.ROOT/"apps/desktop/e2e/fixtures/compat-corpus")
    parser.add_argument("--owner-root", type=Path)
    parser.add_argument("--store", action="store_true")
    parser.add_argument("--only", help="Public fixture directory name")
    parser.add_argument("--scripts", type=Path)
    parser.add_argument("--repeat", type=int, default=3)
    parser.add_argument("--timeout", type=float, default=90)
    parser.add_argument("--summary", type=Path, default=run.WORK/"trace-summary.json")
    parser.add_argument("--keep", action="store_true", help="Leave raw traces under the work directory")
    args = parser.parse_args()
    if args.repeat < 3:
        parser.error("at least three captures per role are required")
    if args.summary.resolve().is_relative_to(run.ROOT):
        parser.error("raw output belongs outside the public checkout")
    actual = subprocess.check_output(["git", "-C", str(args.reference_root), "rev-parse", "HEAD"], text=True).strip()
    if actual != args.reference_revision:
        parser.error("reference checkout does not match the requested revision")
    run.WORK.mkdir(mode=0o700, exist_ok=True)
    reference = ["node", str(Path(__file__).with_name("reference.mjs")), str(args.reference_root)]
    native = ["node", str(Path(__file__).with_name("native.mjs")), str(args.reference_root), str(args.native_runner)]
    populations = {"local-corpus": run.discover(args.corpus_root)}
    if args.owner_root:
        populations["owner"] = run.discover(args.owner_root)
    public = run.PublicStore() if args.store else None
    if public:
        populations["store"] = public.catalog()
    summary = {"evidence": "trace-only", "gate": "OPEN", "accepted_matches": 0,
               "reference": args.reference_revision, "sources": {}, "families": {}, "reasons": {}}
    families, reasons = Counter(), Counter()
    for source, paths in populations.items():
        missing = 0 if args.only else max(0, run.EXPECTED_POPULATION[source]-len(paths))
        counts = Counter(total=len(paths)+missing, cant_run=missing)
        reasons["population-input-unavailable"] += missing
        for index, path in enumerate(paths, 1):
            case = f"{source}-{index:04d}"
            if args.only and getattr(path, "name", None) != args.only:
                counts["total"] -= 1
                continue
            with tempfile.TemporaryDirectory(prefix="trace-", dir=run.WORK, delete=not args.keep) as temporary:
                root = Path(temporary)
                if args.keep:
                    print(f"{case}: {root}", flush=True)
                try:
                    sha = public.source(path, root/"input") if source == "store" else run.copy_extension(path, root/"input")
                    store = {"repoId": path["repoId"], "releaseId": path["latest"]["id"]} if source == "store" else None
                    script = run.load_trace_script(args.scripts, case, DEFAULT_SCRIPT)
                    check_script(script)
                    exact, behavior = compare_case(args, source, case, root/"input", sha, native, reference, script, store)
                    if behavior:
                        counts["trace_mismatch"] += 1
                        summary.setdefault("rows", {})[case] = {"exact": len(exact), "behavior": len(behavior)}
                        for name, count in Counter(row["family"] for row in behavior).items():
                            families[name] += count
                    elif exact:
                        counts["trace_match_unordered"] += 1
                    else:
                        counts["trace_match"] += 1
                except run.Blocked as error:
                    counts["cant_run"] += 1
                    reasons[str(error)] += 1
                except (OSError, ValueError, KeyError, TypeError):
                    counts["cant_run"] += 1
                    reasons["input-or-artifact-invalid"] += 1
            print(f"{case}: completed", flush=True)
        summary["sources"][source] = dict(counts)
    summary["families"], summary["reasons"] = dict(families), dict(reasons)
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(json.dumps(summary, indent=2)+"\n")
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
