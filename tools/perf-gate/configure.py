#!/usr/bin/env python3
"""Prepare a P1 manifest from already-linked public JSON projects; no app runs."""

import argparse
import copy
import json
from pathlib import Path

from budgets import ROWS
from gate import digest
from quiet import finite


def document(path):
    path = path.expanduser().resolve()
    doc = json.loads(path.read_text())
    fps, duration = doc.get("fps"), doc.get("dur")
    if not finite(fps) or not finite(duration) or fps <= 0 or duration <= 0:
        raise ValueError("Prepared JSON project requires positive fps and dur")
    return dict(
        project=str(path),
        sha256=digest(path),
        public_fixture=True,
        fps=fps,
        frames=max(2, round(fps * duration)),
    )


def build(args):
    shape, video, audio = (
        document(args.shape),
        document(args.video),
        document(args.audio),
    )
    if shape["fps"] != 60:
        raise ValueError("P1 playback needs a prepared 60 fps public shape workload")
    video.update(media=str(args.video_media.expanduser().resolve()))
    video["media_sha256"] = digest(video["media"])
    scenarios = {
        row: copy.deepcopy(
            video if row == "scrub_random_seek" else audio if row == "av" else shape
        )
        for row in ROWS
    }
    scenarios["idle"].update(settle_s=5, sample_s=10, stable_idle_window=False)
    return dict(
        version=1,
        context=dict(
            hz=args.hz,
            runs=30,
            rig=args.rig,
            rig_certified=args.certify_rig,
            quality_certified=args.certify_quality,
            quality_note=args.quality_note,
            max_load_per_cpu=0.5,
            allowed_apps=args.allow_app,
        ),
        apps=dict(
            native=str(args.native.expanduser().resolve()),
            electron=str(args.electron.expanduser().resolve()),
        ),
        reference=str(args.reference.expanduser().resolve()),
        electron_args=args.electron_arg,
        scenarios=scenarios,
        max_attempts_per_row=45,
        side_timeout_s=180,
        retry_settle_s=2,
    )


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    for name in (
        "native",
        "electron",
        "reference",
        "shape",
        "video",
        "video-media",
        "audio",
        "output",
    ):
        p.add_argument("--" + name, type=Path, required=True)
    p.add_argument("--hz", type=int, choices=(60, 120), required=True)
    p.add_argument("--rig", choices=("xdr-120", "base-60", "stand-in"), required=True)
    p.add_argument(
        "--allow-app",
        action="append",
        default=[],
        help="Explicit system bundle ID allowed during timing",
    )
    p.add_argument(
        "--electron-arg",
        action="append",
        default=[],
        help="Packaged apps default to no args; Electron runtime needs the reference desktop path",
    )
    p.add_argument(
        "--quality-note",
        default="",
        help="Operator record of matched brightness/HDR/resolution/quality",
    )
    p.add_argument("--certify-rig", action="store_true")
    p.add_argument("--certify-quality", action="store_true")
    args = p.parse_args(argv)
    try:
        if not args.output.resolve().is_relative_to(Path("/tmp").resolve()):
            p.error("Manifest output must be under the canonical system temporary root")
        config = build(args)
        with args.output.open("x") as out:
            args.output.chmod(0o600)
            json.dump(config, out, indent=2, allow_nan=False)
            out.write("\n")
    except (OSError, ValueError) as error:
        p.error(str(error))
    print(str(args.output))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
