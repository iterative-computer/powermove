#!/usr/bin/env python3
"""aero_bench regression and budget check (harness DESIGN.md 9.3).

aero-bench-check.py <run.json> [<rerun.json>]             compare with the baseline
aero-bench-check.py --idle <run.json> [<rerun.json>]      also gate the budgets
aero-bench-check.py --record <run.json> [<run.json> ...]  record the baseline
aero-bench-check.py --self-test                           the checker's own cases

The gated metric is instructions retired per operation (`minstr`, millions,
the whole process): wall time on this shared Mac swings 2-3x with the other
lanes' load; instructions move a few percent. A row regresses when its p99
grows more than 10 % over the baseline (plus 0.05 M, so a tiny row cannot
fail on jitter). The baseline is an envelope: each row's highest p99 over
the recorded runs. Record it on a quiet machine only: the close row's count
includes the core's shutdown spin, which swings with scheduling (Sol's
loaded reruns measured 0.65-0.74 M against a quiet 0.40 M).

Budgets (each row's budget_ms, the copy counts, memory per session, threads
after close, and resume against fx on the same journal) are always
reported, per run; they fail the check only with --idle, on a quiet machine.

The two-run rule holds for everything that decides the verdict: with a
rerun, a row's instructions or a budget fail only if they fail in both
runs, so the order of the runs never changes the outcome.

Nothing passes without data (Sol's m2 item 8): the check is an error unless
the baseline exists, every expected row is in every run and the baseline,
every reading is a finite number, and every run measured resume on the same
journal (its sha256) as the baseline. With --idle, resume needs fx's restore
measured on that journal too ($AERO_FX_RESUME, from tools/fx-resume-bench.mjs).

Baseline: $AERO_BENCH_BASELINE or ~/Developer/pm-durable/orch/aero-baseline.json.
"""
import json, math, os, sys, tempfile

SCHEMA = 1
SLACK = 1.10
FLOOR = 0.05
BASELINE = os.environ.get("AERO_BENCH_BASELINE", os.path.expanduser("~/Developer/pm-durable/orch/aero-baseline.json"))
FX_RESUME = os.environ.get("AERO_FX_RESUME", os.path.expanduser("~/Developer/pm-durable/orch/aero-fx-resume.json"))
ROWS = (
    "harness_start",
    "session_open",
    "resume_200",
    "close",
    "prompt_to_request",
    "body_to_first_text",
    "tool_call_to_host",
    "result_to_request",
)
QUANTILES = ("p50", "p90", "p99")
COPIES = ("rust_allocations_per_text_delta", "rust_allocations_per_tool_answer")
MEMORY = (
    ("resident_per_session_idle_bytes", "budget_idle_bytes"),
    ("resident_per_session_after_prompt_bytes", "budget_after_prompt_bytes"),
)


def number(value, what):
    """A finite, non-negative reading, or an error naming it."""
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError(f"{what} is {value!r}, not a finite reading")
    return value


def sha(value, what):
    if not isinstance(value, str) or len(value) != 64 or any(c not in "0123456789abcdef" for c in value):
        raise ValueError(f"{what} is {value!r}, not a journal sha256")
    return value


def load(path):
    if not os.path.exists(path):
        raise ValueError(f"no report at {path} (aero_bench did not run, or crashed)")
    with open(path) as f:
        report = json.load(f)
    if report.get("schema") != SCHEMA:
        raise ValueError(f"{path}: schema {report.get('schema')}, expected {SCHEMA}")
    rows = report.get("rows")
    if not isinstance(rows, dict):
        raise ValueError(f"{path}: no rows")
    for row in ROWS:
        value = rows.get(row)
        if not isinstance(value, dict):
            raise ValueError(f"{path}: row {row} is missing")
        if number(value.get("samples"), f"{path}: {row} samples") <= 0:
            raise ValueError(f"{path}: row {row} has no samples")
        for q in QUANTILES:
            if number((value.get("minstr") or {}).get(q), f"{path}: {row} minstr {q}") <= 0:
                raise ValueError(f"{path}: row {row} has no instruction reading")
            number((value.get("wall_ms") or {}).get(q), f"{path}: {row} wall_ms {q}")
        for q, limit in (value.get("budget_ms") or {}).items():
            if limit is not None:
                number(limit, f"{path}: {row} budget {q}")
    copies = report.get("copies") or {}
    for key in COPIES:
        number(copies.get(key), f"{path}: copies {key}")
    memory = report.get("memory") or {}
    for got, limit in MEMORY:
        number(memory.get(got), f"{path}: memory {got}")
        number(memory.get(limit), f"{path}: memory {limit}")
    threads = report.get("threads") or {}
    for key in ("before", "after_close"):
        number(threads.get(key), f"{path}: threads {key}")
    sha(report.get("resume_journal_sha256"), f"{path}: resume_journal_sha256")
    return report


def load_baseline(path):
    if not os.path.exists(path):
        raise ValueError(f"no baseline at {path}: record one on a quiet machine with --record")
    with open(path) as f:
        baseline = json.load(f)
    if baseline.get("schema") != SCHEMA:
        raise ValueError(f"{path}: schema {baseline.get('schema')}, expected {SCHEMA}")
    rows = baseline.get("rows") or {}
    for row in ROWS:
        if number(rows.get(row), f"{path}: baseline row {row}") <= 0:
            raise ValueError(f"{path}: baseline row {row} has no reading")
    sha(baseline.get("resume_journal_sha256"), f"{path}: resume_journal_sha256")
    return baseline


def load_fx_resume(path):
    """fx's restore from the same journal, or None if there is none."""
    if not path or not os.path.exists(path):
        return None
    with open(path) as f:
        fx = json.load(f)
    sha(fx.get("journal_sha256"), f"{path}: journal_sha256")
    number((fx.get("journal_restore_create_ms") or {}).get("p50"), f"{path}: journal_restore_create_ms p50")
    return fx


def regressions(baseline, runs):
    """Rows whose minstr p99 regressed in every run given."""
    out = []
    for row in ROWS:
        limit = baseline["rows"][row]
        if all(run["rows"][row]["minstr"]["p99"] > limit * SLACK + FLOOR for run in runs):
            p99s = ", ".join(f'{run["rows"][row]["minstr"]["p99"]:.3f}' for run in runs)
            out.append(f"{row}: minstr p99 {p99s} M over the baseline {limit:.3f} M (+10 %)")
    return out


def budget_misses(run, fx):
    """One run's budget misses, by a key the two-run rule matches across runs."""
    out = {}
    for row in ROWS:
        value = run["rows"][row]
        for q in QUANTILES:
            limit = value["budget_ms"].get(q)
            if limit and value["wall_ms"][q] > limit:
                out[f"{row}.{q}"] = f"{row}: wall {q} {value['wall_ms'][q]:.3f} ms over its {limit} ms budget"
    # Resume: at or under fx's restore from the same journal.
    got = run["rows"]["resume_200"]["wall_ms"]["p50"]
    if fx is None:
        out["resume.fx"] = "resume_200: no fx restore measured on this journal (AERO_FX_RESUME)"
    elif fx["journal_sha256"] != run["resume_journal_sha256"]:
        out["resume.fx"] = f"resume_200: fx's restore was measured on journal {fx['journal_sha256'][:12]}, this run on {run['resume_journal_sha256'][:12]}"
    elif got > fx["journal_restore_create_ms"]["p50"]:
        out["resume.fx"] = f"resume_200: wall p50 {got:.3f} ms over fx's {fx['journal_restore_create_ms']['p50']:.3f} ms on the same journal"
    for key in COPIES:
        if run["copies"][key] != 0:
            out[key] = f"{key}: {run['copies'][key]} (budget 0)"
    for got_key, limit_key in MEMORY:
        if run["memory"][got_key] > run["memory"][limit_key]:
            out[got_key] = f"{got_key}: {run['memory'][got_key]} B over {run['memory'][limit_key]} B"
    threads = run["threads"]
    if threads["after_close"] > threads["before"]:
        out["threads"] = f"threads after close {threads['after_close']} > {threads['before']}"
    return out


def record(paths):
    runs = [load(p) for p in paths]
    journals = {run["resume_journal_sha256"] for run in runs}
    if len(journals) != 1:
        raise ValueError("the runs measured resume on different journals: pass the same --resume-journal")
    rows = {row: max(run["rows"][row]["minstr"]["p99"] for run in runs) for row in ROWS}
    return {"schema": SCHEMA, "runs": len(runs), "rows": rows, "resume_journal_sha256": journals.pop()}


def check(paths, idle, baseline_path=BASELINE, fx_path=FX_RESUME):
    runs = [load(p) for p in paths]
    baseline = load_baseline(baseline_path)
    for path, run in zip(paths, runs):
        if run["resume_journal_sha256"] != baseline["resume_journal_sha256"]:
            raise ValueError(f"{path}: resume was measured on another journal than the baseline's (pass the baseline's --resume-journal, or record again)")
    status = 0
    found = regressions(baseline, runs)
    for line in found:
        print(f"REGRESSION {line}")
    status |= 1 if found else 0
    fx = load_fx_resume(fx_path)
    misses = [budget_misses(run, fx) for run in runs]
    for index, run_misses in enumerate(misses, 1):
        for line in run_misses.values():
            print(f"run {index}: budget miss {line}")
    # The two-run rule, as for instructions: a budget fails only if every
    # run missed it.
    failed = set(misses[0]).intersection(*misses[1:])
    for key in sorted(failed):
        print(f"{'BUDGET' if idle else 'budget (not gated: not an idle run)'} {misses[-1][key]}")
    if idle and failed:
        status |= 1
    if status == 0:
        print("aero_bench: ok")
    return status


def self_test():
    journal = "a" * 64

    def report(minstr, wall=0.05, idle_bytes=1000, journal=journal, resume_ms=1.0):
        quantiles = lambda v: {"p50": v, "p90": v, "p99": v}
        rows = {row: {"samples": 10, "minstr": quantiles(minstr), "wall_ms": quantiles(wall), "budget_ms": {"p50": 0.1, "p90": None, "p99": None}} for row in ROWS}
        rows["resume_200"]["wall_ms"] = quantiles(resume_ms)
        rows["resume_200"]["budget_ms"] = {"p50": 0, "p90": None, "p99": None}
        return {
            "schema": 1,
            "rows": rows,
            "copies": {key: 0 for key in COPIES},
            "memory": {"resident_per_session_idle_bytes": idle_bytes, "resident_per_session_after_prompt_bytes": 1000, "budget_idle_bytes": 358400, "budget_after_prompt_bytes": 819200},
            "threads": {"before": 3, "after_close": 3},
            "resume_journal_sha256": journal,
        }

    with tempfile.TemporaryDirectory() as d:
        def write(name, value):
            path = os.path.join(d, name)
            with open(path, "w") as f:
                json.dump(value, f)
            return path

        def error(paths, idle, base, fx=None):
            try:
                check(paths, idle, base, fx)
            except ValueError:
                return True
            return False

        fx = write("fx.json", {"journal_sha256": journal, "journal_restore_create_ms": {"p50": 2.0}})
        base = write("base.json", record([write("a.json", report(1.0)), write("b.json", report(1.1))]))
        assert json.load(open(base))["rows"]["close"] == 1.1, "the envelope keeps the highest p99"
        ok, slow = write("ok.json", report(1.2)), write("slow.json", report(1.5))
        assert check([ok], False, base, fx) == 0, "within 10 % passes"
        assert check([slow], False, base, fx) == 1, "past 10 % fails"
        assert check([slow, ok], False, base, fx) == 0, "a regression must reproduce in the rerun"
        assert check([ok, slow], False, base, fx) == 0, "whatever the order"
        assert check([slow, slow], False, base, fx) == 1
        over = write("over.json", report(1.0, wall=0.5))
        assert check([over], False, base, fx) == 0, "budgets are not gated on a busy run"
        assert check([over], True, base, fx) == 1, "budgets gate an idle run"
        # The two-run rule for budgets, in either order (Sol's m2 item 8).
        fat = write("fat.json", report(1.0, idle_bytes=400000))
        thin = write("thin.json", report(1.0))
        assert check([thin, fat], True, base, fx) == 0, "one missed budget does not reproduce"
        assert check([fat, thin], True, base, fx) == 0, "and the order does not change the verdict"
        assert check([fat, fat], True, base, fx) == 1
        # No data never passes.
        assert error([ok], False, os.path.join(d, "absent.json"), fx), "an absent baseline is an error"
        empty = write("empty.json", {"schema": 1, "rows": {}})
        assert error([empty], False, base, fx), "a run with no rows is an error"
        bad = write("bad.json", {"schema": 1, "rows": {"close": {"samples": 0, "minstr": {"p99": 0}}}})
        assert error([bad], False, base, fx), "a run with no reading is an error"
        for broken in (float("nan"), float("inf")):
            value = report(1.0)
            value["rows"]["close"]["wall_ms"]["p50"] = broken
            assert error([write("nan.json", value)], True, base, fx), f"a {broken} reading is an error"
            value = report(1.0)
            value["memory"]["resident_per_session_idle_bytes"] = broken
            assert error([write("nan2.json", value)], True, base, fx), f"a {broken} memory reading is an error"
        missing = report(1.0)
        del missing["copies"]["rust_allocations_per_text_delta"]
        assert error([write("missing.json", missing)], True, base, fx), "a missing reading is an error, not a pass"
        # Resume: on the baseline's journal, and against fx on the same one.
        other = write("other.json", report(1.0, journal="b" * 64))
        assert error([other], False, base, fx), "resume on another journal than the baseline's"
        slow_resume = write("slow-resume.json", report(1.0, resume_ms=3.0))
        assert check([slow_resume], True, base, fx) == 1, "resume over fx on the same journal fails"
        assert check([slow_resume], True, base, write("fx-other.json", {"journal_sha256": "c" * 64, "journal_restore_create_ms": {"p50": 9.0}})) == 1, "fx on another journal is no comparison"
        assert check([ok], True, base, os.path.join(d, "no-fx.json")) == 1, "an idle check needs fx's restore"
    print("self-test: ok")
    return 0


def main(argv):
    if argv[:1] == ["--self-test"]:
        return self_test()
    try:
        if argv[:1] == ["--record"] and len(argv) > 1:
            baseline = record(argv[1:])
            with open(BASELINE, "w") as f:
                json.dump(baseline, f, indent=1)
            print(f"recorded {BASELINE} from {len(argv) - 1} runs")
            return 0
        idle = argv[:1] == ["--idle"]
        paths = argv[1:] if idle else argv
        if not 1 <= len(paths) <= 2:
            print(__doc__)
            return 2
        return check(paths, idle)
    except ValueError as error:
        print(f"ERROR {error}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
