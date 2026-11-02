#!/usr/bin/env python3
"""Private, disposable P2 capture orchestration. Only aggregate data leaves the run."""
import argparse
from collections import Counter
import hashlib
import io
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tarfile
import tempfile
import time

REFERENCE = "07ed5fa17ee640659cb075f9cad1af2095011d60"
FAMILIES = ("PM facade", "sync services", "raster override", "recorder canvas", "DOM panel", "lifecycle")
SOURCES = ("owner", "local-corpus", "store")
ROOT = Path(__file__).resolve().parents[2]
WORK = Path("/tmp/pm-extcheck")
PAUSE_LOCAL = Path("/tmp/pm-shots/PAUSE_LOCAL")
USER_ACTIVE = Path("/tmp/pm-shots/USER_ACTIVE")
DEFAULT_SCRIPT = {"version": 1, "complete": False, "steps": [
    {"op": "reload"}, {"op": "seek", "time": 0},
    {"op": "screenshot", "label": "active", "family": "DOM panel"},
    {"op": "reload"}, {"op": "dispose"}]}


class Blocked(Exception):
    """A public fixed reason code, never an extension error message."""


class LocalPause(Exception):
    """Retry owned capture after the owner's local-execution pause ends."""


def wait_for_local_execution():
    while PAUSE_LOCAL.exists():
        print("Local execution paused by owner; checking again in five minutes", flush=True)
        time.sleep(300)


def stop_capture(child):
    from gui_capture import stop_owned_process
    stop_owned_process(child)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def disk_check():
    while True:
        wait_for_local_execution()
        if shutil.disk_usage(WORK).free >= 6.5 * 1024**3:return
        print("Waiting for disk space at or above 6.5 GiB", flush=True)
        time.sleep(120)


def safe_file(root, value):
    p = PurePosixPath(value)
    if p.is_absolute() or not p.parts or any(s in ("..", ".") for s in p.parts):
        raise Blocked("unsafe-artifact-path")
    path = root.joinpath(*p.parts)
    if path.is_symlink() or not path.resolve().is_relative_to(root.resolve()):
        raise Blocked("unsafe-artifact-path")
    return path


def copy_extension(source, target):
    # Never follow source symlinks, including ones hidden inside a fork snapshot.
    if source.is_symlink():
        raise Blocked("input-symlink")
    for directory, dirs, files in os.walk(source, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in (".git", "node_modules"))
        for name in dirs + files:
            if (Path(directory) / name).is_symlink():
                raise Blocked("input-symlink")
    shutil.copytree(source, target, ignore=shutil.ignore_patterns(".git", "node_modules"))
    return tree_digest(target)


def tree_digest(root):
    h = hashlib.sha256()
    for p in sorted(root.rglob("*")):
        if p.is_file():
            name = p.relative_to(root).as_posix().encode()
            data = p.read_bytes()
            h.update(len(name).to_bytes(8, "big") + name)
            h.update(len(data).to_bytes(8, "big") + data)
    return h.hexdigest()


def discover(root):
    if not root.is_dir():
        raise Blocked("input-root-missing")
    return [p for p in sorted(root.iterdir()) if not p.is_symlink() and p.is_dir()
            and (p / "manifest.json").is_file()]


class PublicStore:
    """Same anonymous GET endpoints as the desktop client; curl's normal TLS trust."""
    origin = "https://cloud.trypowermove.com"

    def __init__(self):
        self.last = 0

    def get(self, route, binary=False):
        time.sleep(max(0, self.last + 1 - time.monotonic()))
        self.last = time.monotonic()
        # No cookie jar, auth header, account state or redirects to other origins.
        result = subprocess.run(["curl", "--fail", "--silent", "--show-error",
                                 "--max-time", "30", "--max-filesize", str(32*1024**2),
                                 self.origin + route], capture_output=True)
        if result.returncode:
            raise Blocked("store-public-read-failed")
        try:
            return result.stdout if binary else json.loads(result.stdout)
        except ValueError:
            raise Blocked("store-invalid-json") from None

    def catalog(self):
        from urllib.parse import quote
        items, seen, cursor = [], set(), None
        while True:
            suffix = "&cursor=" + quote(cursor, safe="") if cursor else ""
            page = self.get("/v1/store/extensions?sort=name&limit=50" + suffix)
            for item in page["items"]:
                key = item["repoId"]
                if key in seen:
                    raise Blocked("store-duplicate-listing")
                seen.add(key)
                items.append(item)
            next_cursor = page.get("nextCursor")
            if not next_cursor:
                return items
            if next_cursor == cursor:
                raise Blocked("store-repeated-cursor")
            cursor = next_cursor

    def source(self, item, target):
        release_id = item.get("latest", {}).get("id")
        if not release_id or not re.fullmatch(r"[a-zA-Z0-9_-]+", release_id):
            raise Blocked("store-no-latest-release")
        prefix = "/v1/store/releases/" + release_id
        release, tree = self.get(prefix), self.get(prefix + "/tree")
        archive = self.get(prefix + "/tar", binary=True)
        if digest(archive) != release["tarSha256"] or tree["treeSha"] != release["treeSha"]:
            raise Blocked("store-integrity")
        expected = {row["path"]: row for row in tree["files"]}
        target.mkdir()
        actual, total = set(), 0
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            for member in tar:
                if member.isdir():
                    safe_file(target, member.name)
                    continue
                if not member.isfile() or member.name not in expected or member.name in actual:
                    raise Blocked("store-unsafe-archive")
                total += member.size
                if member.size > 2*1024**2 or total > 32*1024**2:
                    raise Blocked("store-size-limit")
                data = tar.extractfile(member).read()
                row = expected[member.name]
                blob = hashlib.sha1(b"blob " + str(len(data)).encode() + b"\0" + data).hexdigest()
                if len(data) != row["size"] or blob != row["sha"]:
                    raise Blocked("store-integrity")
                path = safe_file(target, member.name)
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(data)
                actual.add(member.name)
        if actual != set(expected) or len(actual) != release["fileCount"]:
            raise Blocked("store-incomplete-archive")
        return tree_digest(target)


def family(name):
    name = name.lower()
    if "raster" in name and "canvas" not in name:
        return "raster override"
    if any(s in name for s in ("canvas", "imagedata", "drawimage", "fillrect")):
        return "recorder canvas"
    if "service" in name:
        return "sync services"
    if any(s in name for s in ("dom.", "panel", "key.", "pointer", "input", "selectionchange")):
        return "DOM panel"
    if any(s in name for s in ("lifecycle", "reload", "dispose", "activate", "storage")):
        return "lifecycle"
    return "PM facade"


def exact(value):
    # Python considers True == 1 and ignores dict ordering; neither is a JS trace equivalence.
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def validate_trace(text):
    try:
        rows = [json.loads(line) for line in text.splitlines()]
        header, end = rows[0], rows[-1]
        if (header.get("type"), header.get("format"), header.get("version")) != ("header", "powermove.behavior", 1):
            raise ValueError()
        if end.get("type") != "end" or end.get("valid") is not True:
            raise ValueError()
        stack, calls = [], 0
        for index, row in enumerate(rows[1:], 1):
            if type(row.get("seq")) is not int or row.get("seq") != index or row.get("type") in ("header", "end") and index != len(rows)-1:
                raise ValueError()
            if row["type"] == "call":
                calls += 1
                if type(row["call"]) is not int or row["call"] != calls or row["parent"] != (stack[-1] if stack else None):
                    raise ValueError()
                stack.append(calls)
            elif row["type"] in ("return", "throw"):
                if not stack or stack.pop() != row["call"]:
                    raise ValueError()
            elif "parent" in row and row["parent"] != (stack[-1] if stack else None):
                raise ValueError()
        if stack or type(end["calls"]) is not int or end["calls"] != calls:
            raise ValueError()
        return rows
    except (ValueError, KeyError, IndexError, TypeError, AttributeError):
        raise Blocked("invalid-trace") from None


def visual_diff(a, b):
    try:
        from PIL import Image
        with Image.open(a) as first, Image.open(b) as second:
            if first.format != "PNG" or second.format != "PNG":
                raise Blocked("invalid-screenshot")
            if first.size != second.size:
                return {"equal": False, "size_mismatch": True}
            first, second = first.convert("RGBA"), second.convert("RGBA")
            maximum, total = 0, 0
            for p, q in zip(first.getdata(), second.getdata()):
                # Compare premultiplied colors: RGB under alpha zero has no visible meaning.
                pa = [v*p[3]/255 for v in p[:3]] + [p[3]]
                qa = [v*q[3]/255 for v in q[:3]] + [q[3]]
                deltas = [abs(x-y) for x, y in zip(pa, qa)]
                maximum, total = max(maximum, *deltas), total + sum(deltas)
            return {"equal": maximum == 0, "max": maximum,
                    "mean": total / max(1, first.width*first.height*4)}
    except ImportError:
        raise Blocked("pillow-missing") from None
    except (OSError, ValueError):
        raise Blocked("invalid-screenshot") from None


def compare(left, right, script):
    a = validate_trace((left / "trace.jsonl").read_text())
    b = validate_trace((right / "trace.jsonl").read_text())
    # Header provenance differs intentionally; all behavioral rows remain exact, ordered and typed.
    mismatches, context = [], "lifecycle"
    for index in range(1, max(len(a), len(b))):
        x, y = a[index] if index < len(a) else None, b[index] if index < len(b) else None
        for row in (x, y):
            if row and row.get("type") == "call":
                context = family(row.get("name", ""))
        if exact(x) != exact(y):
            mismatches.append({"kind": "trace", "row": index, "family": context})
    x = json.loads((left / "interactions.json").read_text())
    y = json.loads((right / "interactions.json").read_text())
    if not isinstance(x,list) or not isinstance(y,list) or any(not isinstance(row,dict) for row in x+y):
        raise Blocked("invalid-interactions")
    if len(x) != len(script["steps"]) or len(y) != len(script["steps"]):
        raise Blocked("incomplete-interactions")
    for index, (one, two) in enumerate(zip(x, y)):
        if script["steps"][index]["op"] == "assert" and one.get("result", {}).get("pass") is not True:
            raise Blocked("reference-assertion-failed")
        if exact(one) != exact(two):
            mismatches.append({"kind": "interaction", "step": index,
                               "family": family(script["steps"][index]["op"])})
    for step in script["steps"]:
        if step["op"] != "screenshot":
            continue
        name = step["label"] + ".png"
        delta = visual_diff(safe_file(left, name), safe_file(right, name))
        if not delta["equal"]:
            mismatches.append({"kind": "visual", "family": step.get("family", "DOM panel"), **delta})
    return mismatches


def capture(command, request, output, timeout):
    if not command:
        raise Blocked("native-integrated-runner-unavailable")
    while True:
        disk_check()
        try:
            # Waiting for another lane is outside the capture's execution deadline.
            # A full native adapter also owns a WebKit window. Apply the same
            # lease to either role, including externally supplied adapters.
            from gui_capture import gui_slot
            with gui_slot():
                return capture_unlocked(command, request, output, timeout)
        except LocalPause:
            # Only this attempt's owned disposable output is removed. Retry the
            # same input/script after releasing the GUI lease and owner's pause.
            if output.exists():shutil.rmtree(output)


def capture_unlocked(command, request, output, timeout):
    if not command:
        raise Blocked("native-integrated-runner-unavailable")
    output.mkdir()
    if PAUSE_LOCAL.exists() or USER_ACTIVE.exists() or shutil.disk_usage(WORK).free < 6.5 * 1024**3:
        raise LocalPause()
    env = dict(os.environ)
    env.pop("ELECTRON_RUN_AS_NODE", None)
    env.pop("POWERMOVE_USER_DATA", None)
    # Private errors stay in the disposable directory and never reach stdout/report.
    with (output / "driver.log").open("wb") as log:
        child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=log, stderr=log,
                                 env=env, start_new_session=True, cwd=ROOT)
        try:
            deadline=time.monotonic()+timeout
            data=json.dumps({**request, "output": str(output)}).encode()
            while True:
                if PAUSE_LOCAL.exists() or USER_ACTIVE.exists() or shutil.disk_usage(WORK).free < 6.5 * 1024**3:
                    stop_capture(child)
                    raise LocalPause()
                remaining=deadline-time.monotonic()
                if remaining<=0:raise subprocess.TimeoutExpired(command,timeout)
                try:
                    child.communicate(data,timeout=min(.25,remaining))
                    break
                except subprocess.TimeoutExpired:
                    data=None
        except subprocess.TimeoutExpired:
            stop_capture(child)
            raise Blocked("capture-timeout") from None
        finally:
            if child.poll() is None:stop_capture(child)
    if child.returncode:
        if child.returncode in (75, 76):
            raise Blocked("gui-busy" if child.returncode == 75 else "screen-locked")
        raise Blocked("capture-driver-failed")
    status = json.loads((output / "capture.json").read_text())
    if not isinstance(status,dict):
        raise Blocked("invalid-capture-status")
    if status.get("status") != "captured":
        reason = status.get("reason")
        # Adapters may only expose these reviewed public codes.
        if reason not in ("reference-not-ready", "reference-extension-not-active", "unsupported-script-step",
                          "reference-store-profile-adapter-unavailable",
                          "reference-capture-failed", "native-integrated-runner-unavailable",
                          "native-extension-compile-failed", "native-store-facade-unavailable", "native-runner-failed",
                          "native-webkit-script-adapter-unavailable", "native-activation-needs-backend"):
            reason = "capture-driver-blocked"
        raise Blocked(reason)
    expected = {"case": request["case"], "input_sha256": request["input_sha256"],
                "script_sha256": request["script_sha256"]}
    if any(status.get(k) != v for k, v in expected.items()):
        raise Blocked("capture-identity-mismatch")
    if request["role"] == "reference":
        if status.get("revision") != REFERENCE:
            raise Blocked("reference-revision-mismatch")
    elif status.get("path") != {"logic": "runtime-v8", "panels": "webkit-remote-dom", "canvas": "native-recorder"}:
        raise Blocked("native-path-unverified")
    return status


def validate_script(script):
    if script.get("version") != 1 or not isinstance(script.get("complete"), bool) or not script.get("steps"):
        raise Blocked("invalid-script")
    labels = []
    for step in script["steps"]:
        if step.get("op") == "screenshot":
            label = step.get("label", "")
            if not re.fullmatch(r"[a-zA-Z0-9_-]+", label) or label in labels:
                raise Blocked("invalid-script")
            if step.get("family", "DOM panel") not in FAMILIES:
                raise Blocked("invalid-script")
            labels.append(label)
    if not labels:
        raise Blocked("script-needs-visual-oracle")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--owner-root", type=Path, default=Path.home()/"Library/Application Support/Powermove/extensions")
    parser.add_argument("--corpus-root", type=Path, default=ROOT/"apps/desktop/e2e/fixtures/compat-corpus")
    parser.add_argument("--store", action="store_true")
    parser.add_argument("--reference-root", type=Path, required=True)
    parser.add_argument("--native-command", help="JSON argv array; adapter reads one request on stdin")
    parser.add_argument("--native-runner", type=Path, help="Built pm-compat-runner executable (uses bundled native adapter)")
    parser.add_argument("--scripts", type=Path, help="Private per-case scripts, e.g. owner-0001.json")
    parser.add_argument("--summary", type=Path, default=WORK/"summary.json")
    parser.add_argument("--timeout", type=float, default=90)
    parser.add_argument("--repeat", type=int, default=2, help="Paired repeat captures required before accepting a match")
    args = parser.parse_args()
    WORK.mkdir(mode=0o700, exist_ok=True)
    if args.repeat < 2:
        parser.error("at least two captures are required for an oracle")
    disk_check()
    if args.summary.resolve().is_relative_to(ROOT):
        parser.error("raw output belongs outside the public checkout")
    if args.native_command and args.native_runner:
        parser.error("choose --native-runner or --native-command")
    native = json.loads(args.native_command) if args.native_command else (
        ["node",str(Path(__file__).with_name("native.mjs")),str(args.reference_root),str(args.native_runner)]
        if args.native_runner else None)
    if native is not None and (not isinstance(native, list) or not native or not all(isinstance(x, str) for x in native)):
        parser.error("native command must be a nonempty JSON string array")
    summary = {"gate": "OPEN", "reference": REFERENCE, "sources": {}, "families": {}, "reasons": {}}
    families, reasons, build_blockers = Counter(), Counter(), Counter()
    with tempfile.TemporaryDirectory(prefix="run-", dir=WORK) as run:
        root = Path(run)
        os.chmod(root, 0o700)
        populations = {}
        for source, path in (("owner", args.owner_root), ("local-corpus", args.corpus_root)):
            try:
                populations[source] = discover(path)
            except Blocked as error:
                reasons[str(error)] += 1
        store, initial = None, []
        if args.store:
            store = PublicStore()
            try:
                initial = store.catalog()
                populations["store"] = initial
            except Blocked as error:
                reasons[str(error)] += 1
        for source in SOURCES:
            rows = populations.get(source, [])
            counts = {"total": len(rows), "match": 0, "mismatch": 0, "cant_run": 0,
                      "reference_captured": 0, "native_captured": 0, "native_logic_executed": 0, "native_activated": 0,
                      "enumerated": source in populations}
            summary["sources"][source] = counts
            for index, item in enumerate(rows, 1):
                case = f"{source}-{index:04d}"
                case_root = root / case
                case_root.mkdir()
                try:
                    disk_check()
                    extension = case_root / "input"
                    sha = store.source(item, extension) if source == "store" else copy_extension(item, extension)
                    manifest = json.loads((extension/"manifest.json").read_text())
                    if manifest.get("apiVersion", 1) not in (1, 2, 3):
                        raise Blocked("unsupported-api-version")
                    script_path = args.scripts / (case + ".json") if args.scripts else None
                    script = json.loads(script_path.read_text()) if script_path and script_path.is_file() else DEFAULT_SCRIPT
                    validate_script(script)
                    request = {"version": 1, "case": case, "extension": str(extension), "input_sha256": sha,
                               "script_sha256": digest(json.dumps(script, sort_keys=True).encode()), "script": script}
                    ref = case_root / "reference"
                    failures=[]
                    for role, command in (("reference",["node", str(Path(__file__).with_name("reference.mjs")), str(args.reference_root)]),
                                          ("native",native)):
                        try:
                            capture(command,{**request,"role":role},case_root/role,args.timeout)
                            for repeat in range(1,args.repeat):
                                repeated=case_root/(role+"-repeat-"+str(repeat))
                                capture(command,{**request,"role":role},repeated,args.timeout)
                                if compare(case_root/role,repeated,script):
                                    raise Blocked(role+"-nondeterministic")
                            counts[role+"_captured"]+=1
                        except Blocked as error:
                            failures.append(str(error))
                            reasons[str(error)]+=1
                        if role=="native" and (case_root/role/"capture.json").is_file():
                            status=json.loads((case_root/role/"capture.json").read_text())
                            counts["native_logic_executed"]+=status.get("logic_executed") is True
                            counts["native_activated"]+=status.get("activated") is True
                            code=status.get("compile_reason")
                            if code in ("invalid-reference-manifest","import-outside-extension","external-runtime-import","unresolved-import",
                                        "browser-capability-at-load","compiler-or-loader-error"):
                                build_blockers[code]+=1
                    if failures:
                        counts["cant_run"]+=1
                        continue
                    if tree_digest(extension) != sha:
                        raise Blocked("input-mutated")
                    diffs = compare(ref, case_root/"native", script)
                    if diffs:
                        counts["mismatch"] += 1
                        for name in set(row["family"] for row in diffs):
                            families[name] += 1
                    elif not script["complete"]:
                        raise Blocked("extension-script-incomplete")
                    else:
                        counts["match"] += 1
                except Blocked as error:
                    counts["cant_run"] += 1
                    reasons[str(error)] += 1
                except (OSError, ValueError, KeyError, TypeError, tarfile.TarError):
                    counts["cant_run"] += 1
                    reasons["input-or-artifact-invalid"] += 1
                finally:
                    shutil.rmtree(case_root)
                    print(f"{case}: completed", flush=True)
        if store and "store" in populations:
            try:
                if store.catalog() != initial:
                    reasons["store-catalog-changed"] += 1
            except Blocked as error:
                reasons[str(error)] += 1
    summary["families"], summary["reasons"] = dict(families), dict(reasons)
    summary["native_build_blockers"]=dict(build_blockers)
    if (all(v["enumerated"] and v["total"] and v["match"] == v["total"] for v in summary["sources"].values())
            and not reasons):
        summary["gate"] = "PASS"
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))
    return 0 if summary["gate"] == "PASS" else 2


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Blocked as error:
        print(str(error))
        raise SystemExit(2)
