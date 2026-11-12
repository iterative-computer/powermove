#!/usr/bin/env python3
"""Measure the app's real MCP -> Export panel worker -> encoder path.

Run under nice -n 10 flock /tmp/pm-build.lock. Uses a fresh, private profile
and a cloned input; edits only its in-memory work area through kernel commands.
No renderer sample/noise settings are changed. Results include startup/encode.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import select
import shutil
import signal
import subprocess
import sys
import time


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.sha256()
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
        return digest.hexdigest()


def free_bytes():
    text = subprocess.check_output(["diskutil", "info", "/"], text=True)
    match = re.search(r"Container Free Space:.*?\((\d+) Bytes\)", text)
    if not match:
        raise RuntimeError("Cannot determine container free space")
    return int(match[1])


class Client:
    def __init__(self, process):
        self.process = process
        self.buffer = b""
        self.serial = 0

    def rpc(self, method, params=None, notify=False):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        if not notify:
            self.serial += 1
            message["id"] = self.serial
        self.process.stdin.write(json.dumps(message).encode() + b"\n")
        self.process.stdin.flush()
        if notify:
            return None
        deadline = time.monotonic() + 3600
        last_disk = 0
        while time.monotonic() < deadline:
            if time.monotonic() - last_disk > 3:
                last_disk = time.monotonic()
                if free_bytes() < 4_000_000_000:
                    raise RuntimeError("Storage stop: below 4 GB")
            if b"\n" not in self.buffer:
                if not select.select([self.process.stdout], [], [], 1)[0]:
                    continue
                chunk = os.read(self.process.stdout.fileno(), 65536)
                if not chunk:
                    raise RuntimeError("Native MCP worker stopped; inspect native.log")
                self.buffer += chunk
                continue
            line, self.buffer = self.buffer.split(b"\n", 1)
            response = json.loads(line)
            if response.get("id") != self.serial:
                continue
            if "error" in response:
                raise RuntimeError(str(response["error"]))
            return response["result"]
        raise TimeoutError(method)

    def tool(self, name, args):
        result = self.rpc("tools/call", {"name": name, "arguments": args})
        if result.get("isError"):
            raise RuntimeError(str(result))
        text = "\n".join(c["text"] for c in result.get("content", []) if c.get("type") == "text")
        try:
            return json.loads(text) if text else result
        except json.JSONDecodeError:
            return {"text": text}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--exe", type=Path, required=True)
    parser.add_argument("--project", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True, help="new evidence directory outside Git")
    parser.add_argument("--frames", type=int, default=30)
    parser.add_argument("--library-dir", type=Path, help="FFmpeg libraries for an unbundled benchmark snapshot")
    parser.add_argument("--sample-counts", action="store_true", help="record measured per-pixel counts and readback overhead")
    parser.add_argument("--fixed-spp", type=int, help="diagnostic fixed samples with the chosen preset's other settings")
    parser.add_argument("--fps", type=float, default=30)
    parser.add_argument("--start", type=float, default=0)
    parser.add_argument("--bounces", type=int, help="diagnostic total bounce limit")
    parser.add_argument("--preset", choices=["draft", "preview", "production", "final"], default="production")
    parser.add_argument("--prepare-only", action="store_true", help="validate the isolated MCP setup without rendering")
    parser.add_argument("--secondary-tree", action="store_true", help="explicit experimental engine A/B")
    parser.add_argument("--single-tree", action="store_true", help="explicit experimental single-light A/B")
    parser.add_argument("--stratified-lights", action="store_true", help="explicit experimental replicated-light A/B")
    args = parser.parse_args()
    if args.frames < 1 or not math.isfinite(args.fps) or args.fps <= 0 or not math.isfinite(args.start) or args.start < 0:
        parser.error("frames/fps must be positive; start must be nonnegative")
    if args.fixed_spp is not None and not 1 <= args.fixed_spp <= 4096:
        parser.error("fixed spp must be 1..4096")
    if args.bounces is not None and not 1 <= args.bounces <= 32:
        parser.error("bounces must be 1..32")
    if not args.prepare_only and free_bytes() < 5_000_000_000:
        raise SystemExit("Below 5 GB: no export started")
    repo = Path(__file__).resolve().parents[3]
    if args.out.resolve().is_relative_to(repo):
        parser.error("Evidence must be outside the repository")
    executable = args.exe.resolve(strict=True)
    executable_hash = sha256(executable)
    args.out.mkdir(parents=True, exist_ok=False)
    out = args.out.resolve()
    project = args.project.resolve()
    cloned = out / "source.pmv"
    if subprocess.run(["cp", "-c", str(project), str(cloned)], capture_output=True).returncode:
        shutil.copyfile(project, cloned)
    source_hash = sha256(project)
    if sha256(cloned) != source_hash:
        raise RuntimeError("Input clone checksum mismatch")
    profile = out / "profile"
    profile.mkdir()
    env = dict(os.environ, STUDIO_TIMINGS="1", PM_EXPORT_PROFILE="1")
    if args.library_dir:
        env["DYLD_LIBRARY_PATH"] = str(args.library_dir.resolve(strict=True))
    if args.sample_counts:
        env["STUDIO_SAMPLE_COUNTS"] = "1"
    env["STUDIO_SECONDARY_LIGHT_TREE"] = "1" if args.secondary_tree else "0"
    env["STUDIO_SINGLE_LIGHT_TREE"] = "1" if args.single_tree else "0"
    env["STUDIO_STRATIFIED_LIGHTS"] = "1" if args.stratified_lights else "0"
    def interrupted(signum, frame):
        raise InterruptedError(f"Profiler interrupted by signal {signum}")
    signal.signal(signal.SIGTERM, interrupted)
    started = time.monotonic()
    with (out / "native.log").open("wb") as log:
        process = subprocess.Popen([str(executable), "--powermove-mcp", "--profile", str(profile), str(cloned)],
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log, env=env, start_new_session=True)
        client = Client(process)
        try:
            client.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                      "clientInfo": {"name": "native-export-perf", "version": "1"}})
            client.rpc("notifications/initialized", notify=True)
            client.tool("start_session", {"prompt": "Measure real export of this disposable project copy.", "access": "project"})
            client.tool("apply_commands", {"commands": [{"type": "set_composition", "patch": {
                "width": 1920, "height": 1080, "fps": args.fps,
                "workArea": [args.start, args.start + args.frames / args.fps]}}]})
            ready = time.monotonic()
            if args.prepare_only:
                report = {"prepared": True, "frames": args.frames, "input_sha256": source_hash,
                          "setup_seconds": ready - started}
                (out / "setup.json").write_text(json.dumps(report, indent=2) + "\n")
                print(json.dumps(report), flush=True)
                return
            if free_bytes() < 5_000_000_000:
                raise RuntimeError("Below 5 GB: no export started")
            export_quality=args.preset
            if args.fixed_spp is not None or args.bounces is not None:
                export_quality={"preset":args.preset}
                if args.fixed_spp is not None:
                    export_quality.update(minSamples=args.fixed_spp,maxSamples=args.fixed_spp,noiseThreshold=0)
                if args.bounces is not None:
                    export_quality["maxBounces"]=args.bounces
            result = client.tool("export_video", {"name": "Studio performance", "range": "work", "audio": True,
                                                   "motionBlur": True, "studioQuality": export_quality})
            while result.get("status") == "running":
                time.sleep(0.2)
                result = client.tool("export_video", {"jobId": result["jobId"]})
            done = time.monotonic()
            if result.get("status") not in (None, "completed", "complete", "success", "succeeded"):
                raise RuntimeError(f"Export did not complete: {result}")
            timing_log=(out / "native.log").read_text(errors="replace")
            frames=[]
            for line in timing_log.splitlines():
                if not line.startswith("STUDIO_FRAME_TIMING "):
                    continue
                frame={}
                for field in ["prepare_ms","total_ms","trace_ms","guides_ms","guide_readback_ms","denoise_ms","sample_statistics_ms"]:
                    match=re.search(r"\b"+field+r"[=:]\s*([0-9.eE+-]+)",line)
                    if match:frame[field]=float(match[1])
                for field in ["sample_mean","sample_p95"]:
                    match=re.search(r"\b"+field+r": Some\(([0-9.eE+-]+)\)",line)
                    if match:frame[field]=float(match[1])
                frames.append(frame)
            report = {"frame_timings":frames,"executable_sha256": executable_hash, "input_sha256": source_hash, "preset": args.preset, "frames": args.frames,
                      "fixed_spp":args.fixed_spp,"bounces_override":args.bounces,"sample_count_profiling":args.sample_counts,
                      "secondary_light_tree": args.secondary_tree,
                      "single_light_tree": args.single_tree,
                      "stratified_lights": args.stratified_lights,
                      "size": [1920, 1080], "motion_blur": True, "setup_seconds": ready - started,
                      "export_seconds": done - ready, "seconds_per_frame": (done - ready) / args.frames,
                      "end_to_end_seconds": done - started, "result": result}
            (out / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report), flush=True)
        finally:
            # A storage/error stop must stop an in-flight export immediately.
            # Closing MCP stdin alone waits for its synchronous render to finish.
            if sys.exc_info()[0] is not None and process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
            process.stdin.close()
            try:
                process.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
            if sha256(project) != source_hash:
                raise RuntimeError("Original input changed")


if __name__ == "__main__":
    main()
