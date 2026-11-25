#!/usr/bin/env python3
"""aero_bench regression and budget check (harness DESIGN.md 9.3).

aero-bench-check.py <run.json> [<rerun.json>]             compare with the baseline
aero-bench-check.py --idle <run.json> [<rerun.json>]      also gate the wall-clock budgets
aero-bench-check.py --record <run.json> [<run.json> ...]  record the baseline
aero-bench-check.py --self-test                           the checker's own cases

The gated metric is instructions retired per operation (`minstr`, millions,
the whole process): wall time on this shared Mac swings 2-3x with the other
lanes' load; instructions move a few percent. A row regresses when its p99
grows more than 10 % over the baseline (plus 0.05 M, so a tiny row cannot
fail on jitter). With a rerun, a row regresses only if it does in both runs
(the ui-latency two-run rule). The baseline is an envelope: each row's
highest p99 over the recorded runs.

Budgets (each row's budget_ms, the copy counts, memory per session) are
always reported; they fail the check only with --idle, on a quiet machine.
Baseline: $AERO_BENCH_BASELINE or ~/Developer/pm-durable/orch/aero-baseline.json.
"""
import json, os, sys, tempfile

SCHEMA = 1
SLACK = 1.10
FLOOR = 0.05
BASELINE = os.environ.get("AERO_BENCH_BASELINE", os.path.expanduser("~/Developer/pm-durable/orch/aero-baseline.json"))


def load(path):
    if not os.path.exists(path):
        raise ValueError(f"no report at {path} (aero_bench did not run, or crashed)")
    with open(path) as f:
        report = json.load(f)
    if report.get("schema") != SCHEMA:
        raise ValueError(f"{path}: schema {report.get('schema')}, expected {SCHEMA}")
    for row, value in report.get("rows", {}).items():
        if value.get("samples", 0) <= 0 or value["minstr"]["p99"] <= 0:
            raise ValueError(f"{path}: row {row} has no instruction reading")
    return report


def regressions(baseline, runs):
    """Rows whose minstr p99 regressed in every run given."""
    out = []
    for row, limit in sorted(baseline["rows"].items()):
        verdicts = []
        for run in runs:
            got = run["rows"].get(row)
            if got is None:
                raise ValueError(f"row {row} is missing from a run")
            verdicts.append(got["minstr"]["p99"] > limit * SLACK + FLOOR)
        if all(verdicts):
            p99s = ", ".join(f'{run["rows"][row]["minstr"]["p99"]:.3f}' for run in runs)
            out.append(f"{row}: minstr p99 {p99s} M over the baseline {limit:.3f} M (+10 %)")
    return out


def budget_failures(run):
    out = []
    for row, value in sorted(run["rows"].items()):
        for q in ("p50", "p90", "p99"):
            limit = value["budget_ms"].get(q)
            if limit and value["wall_ms"][q] > limit:
                out.append(f"{row}: wall {q} {value['wall_ms'][q]:.3f} ms over its {limit} ms budget")
    copies = run.get("copies", {})
    for key in ("rust_allocations_per_text_delta", "rust_allocations_per_tool_answer"):
        if copies.get(key, 0) != 0:
            out.append(f"{key}: {copies[key]} (budget 0)")
    memory = run.get("memory", {})
    for got, limit in (("resident_per_session_idle_bytes", "budget_idle_bytes"), ("resident_per_session_after_prompt_bytes", "budget_after_prompt_bytes")):
        if memory.get(got, 0) > memory.get(limit, float("inf")):
            out.append(f"{got}: {memory[got]} B over {memory[limit]} B")
    threads = run.get("threads", {})
    if threads.get("after_close", 0) > threads.get("before", 0):
        out.append(f"threads after close {threads['after_close']} > {threads['before']}")
    return out


def record(paths):
    runs = [load(p) for p in paths]
    rows = {}
    for run in runs:
        for row, value in run["rows"].items():
            rows[row] = max(rows.get(row, 0.0), value["minstr"]["p99"])
    return {"schema": SCHEMA, "runs": len(runs), "rows": rows}


def check(paths, idle, baseline_path=BASELINE):
    runs = [load(p) for p in paths]
    status = 0
    if os.path.exists(baseline_path):
        with open(baseline_path) as f:
            baseline = json.load(f)
        found = regressions(baseline, runs)
        for line in found:
            print(f"REGRESSION {line}")
        status |= 1 if found else 0
    else:
        print(f"NO BASELINE at {baseline_path}: record one with --record")
    for line in budget_failures(runs[-1]):
        print(f"{'BUDGET' if idle else 'budget (not gated: not an idle run)'} {line}")
        if idle:
            status |= 1
    if status == 0:
        print("aero_bench: ok")
    return status


def self_test():
    def report(minstr, wall=0.05):
        return {"schema": 1, "rows": {"r": {"samples": 10, "minstr": {"p50": minstr, "p90": minstr, "p99": minstr}, "wall_ms": {"p50": wall, "p90": wall, "p99": wall}, "budget_ms": {"p50": 0.1, "p90": None, "p99": None}}}, "copies": {}, "memory": {}, "threads": {}}

    with tempfile.TemporaryDirectory() as d:
        def write(name, value):
            path = os.path.join(d, name)
            with open(path, "w") as f:
                json.dump(value, f)
            return path

        base = write("base.json", record([write("a.json", report(1.0)), write("b.json", report(1.1))]))
        assert json.load(open(base))["rows"]["r"] == 1.1, "the envelope keeps the highest p99"
        ok, slow = write("ok.json", report(1.2)), write("slow.json", report(1.5))
        assert check([ok], False, base) == 0, "within 10 % passes"
        assert check([slow], False, base) == 1, "past 10 % fails"
        assert check([slow, ok], False, base) == 0, "a regression must reproduce in the rerun"
        assert check([slow, slow], False, base) == 1
        over = write("over.json", report(1.0, wall=0.5))
        assert check([over], False, base) == 0, "budgets are not gated on a busy run"
        assert check([over], True, base) == 1, "budgets gate an idle run"
        bad = write("bad.json", {"schema": 1, "rows": {"r": {"samples": 0, "minstr": {"p99": 0}}}})
        try:
            check([bad], False, base)
            raise AssertionError("a run with no reading must be an error")
        except ValueError:
            pass
    print("self-test: ok")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    if argv[:1] == ["--record"] and len(argv) > 1:
        with open(BASELINE, "w") as f:
            json.dump(record(argv[1:]), f, indent=1)
        print(f"recorded {BASELINE} from {len(argv) - 1} runs")
        return 0
    idle = argv[:1] == ["--idle"]
    paths = argv[1:] if idle else argv
    if not 1 <= len(paths) <= 2:
        print(__doc__)
        return 2
    try:
        return check(paths, idle)
    except ValueError as error:
        print(f"ERROR {error}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
