#!/usr/bin/env python3
"""Markdown tables from `s6 --json` reports (used for docs/engine/spikes/S6.md).

usage: python3 -I scripts/summary.py REPORT.json [REPORT.json ...]
"""
import json
import os
import sys


def short_backend(b):
    name = b["backend"]
    if name == "FFmpeg":
        return "FFmpeg " + b["detail"].split(" (")[0]
    return name


def ref_text(rc, frames):
    if not rc:
        return "-"
    if rc.get("error") and rc["matched"] == 0:
        return "n/a"
    return f"{rc['matched']}/{frames}"


def files_table(files):
    print("| File | Container, codec, format | Timebase | Index (build) | Sync | Nominal rate | Rule 7 / naive |")
    print("|---|---|---|---|---|---|---|")
    for r in files:
        name = os.path.basename(r["file"])
        i, ix, nr, sel = r.get("info"), r.get("index"), r.get("nominal_rate"), r.get("selection")
        if not i or not ix:
            print(f"| {name} | error: {'; '.join(r['errors'])} | | | | | |")
            continue
        cont = i["container"].split(",")[0]
        tb = f"{ix['timebase']['num']}/{ix['timebase']['den']}"
        st = ix["stats"]
        src = {"Mp4Tables": "stbl", "MkvCuesAndBlockScan": "cues + headers", "MkvBlockScan": "headers", "DemuxScan": "demux scan"}[ix["source"]]
        rate = f"{nr['rate']['num']}/{nr['rate']['den']} ({nr['method']})" if nr else "-"
        r7 = f"{sel['rule7_mismatches']} / {sel['naive_mismatches']}" if sel else "-"
        print(
            f"| {name} | {cont} {i['codec']} {i['pix_fmt']} {i['width']}x{i['height']} | {tb} | {src} ({st['build_ms']:.2f} ms) "
            f"| {st['sync_samples']}/{st['samples']} | {rate} | {r7} |"
        )


def backends_table(files):
    print(
        "| File | Backend | Output | HW | Frames | Ref | AVF | Random, new session (mean / p95 ms) "
        "| Random, same session (mean / p95 ms) | Decode-only fps | Zero-copy (copies) | Import p50 / p95 µs |"
    )
    print("|---|---|---|---|---|---|---|---|---|---|---|---|")
    for r in files:
        name = os.path.basename(r["file"])
        for b in r["backends"]:
            if b["frames"] == 0:
                why = b.get("error") or b["detail"]
                print(f"| {name} | {b['backend']} | not used: {why} | | | | | | | | | |")
                continue
            g = b["gpu"]
            rn, rw = b["random"], b["random_warm"]
            out = g.get("plane_formats", "").split(":")[0] or "-"
            hw = {True: "yes", False: "no", None: "-"}[b.get("hardware")]
            ok = "" if b["ok"] else " **FAIL**"
            print(
                f"| {name} | {short_backend(b)}{ok} | {out} | {hw} | {b['frames']} "
                f"| {ref_text(b.get('reference'), b['frames'])} | {ref_text(b.get('avfoundation'), b['frames'])} "
                f"| {rn['matched']}/{rn['probes']} ({rn['mean_ms']:.2f} / {rn['p95_ms']:.2f}) "
                f"| {rw['matched']}/{rw['probes']} ({rw['mean_ms']:.2f} / {rw['p95_ms']:.2f}) "
                f"| {b['throughput_fps']:.0f} | {'yes' if b['zero_copy'] else 'NO'} ({g['cpu_copies']}) "
                f"| {g['import_us_p50']:.0f} / {g['import_us_p95']:.0f} |"
            )


def main():
    files = []
    for p in sys.argv[1:]:
        with open(p) as f:
            d = json.load(f)
        print(f"<!-- {os.path.basename(p)}: {d['adapter']} -->")
        files += d["files"]
    print()
    files_table(files)
    print()
    backends_table(files)


if __name__ == "__main__":
    main()
