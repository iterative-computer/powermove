#!/usr/bin/env python3
"""Interaction feel run: input-to-photon of the editor's everyday gestures.

Drives a development app through its own MCP server, the way an agent's
`computer_use_panel` does: pointer events dispatched into the window, each
tagged for the feel probe (PM_FEEL_TRACE, GPUI windows) and the viewer's
trace (PM_ENGINE_TRACE). Then prints feel-report.py's summary per gesture.
Measure release builds (`--profile soak` keeps the dev bundle's behaviour).

  feel.py --launch --binary <target>/soak/powermove-native --project <copy.pmv> \
          --root <scratch dir> --lane <name> [--only ...] [--hz 60]
      bundles the binary (scripts/bundle-dev.sh) under the bundle id
      com.powermove.native.dev.<lane> with a wrapper that sets the traces,
      copies the project into <root>, launches it in the background with
      cua-driver (no activation), measures, then quits it.
  feel.py --app <Powermove.app> --profile <dir> --run <trace dir>
      measures an app already launched with both traces into <trace dir>.

Gestures (timeline geometry is calibrated from two ruler clicks):
  seek      ruler clicks across the composition (playhead + viewer frame)
  select    clicking layer rows in turn (selection switch)
  viewer    dragging the selected layer in the viewer (transform box)
  move      dragging a clip along the timeline and back
  trim      dragging a clip's in point
  scroll    wheel over the timeline rows (zoom, ⌥/⌘-wheel, repaints the
            same way; MCP pointer input carries no modifiers)
  inspector scrubbing a number in Properties
  panels    opening the Performance and Effects panels
"""
import argparse, json, os, subprocess, sys, time
from pathlib import Path

HERE = Path(__file__).resolve().parent
CUA = Path.home() / ".local/bin/cua-driver"


def launch(o):
    """Bundle, wrap with the traces, launch in the background; returns (app, profile, run)."""
    root = Path(o.root).resolve()
    run, profile = root / "run", root / "profile"
    if run.exists():
        subprocess.run(["rm", "-rf", str(run)], check=True)
    run.mkdir(parents=True)
    profile.mkdir(parents=True, exist_ok=True)
    subprocess.run(["rm", "-rf", str(profile / "journals")], check=True)
    subprocess.run([str(HERE.parent.parent.parent / "scripts/bundle-dev.sh"), o.binary, str(root)],
                   check=True, capture_output=True)
    app = root / "Powermove.app"
    macos = app / "Contents/MacOS"
    subprocess.run(["/usr/libexec/PlistBuddy", "-c",
                    f"Set :CFBundleIdentifier com.powermove.native.dev.{o.lane}", str(app / "Contents/Info.plist")],
                   check=True)
    (macos / "powermove-native").rename(macos / "powermove-native-bin")
    wrapper = macos / "powermove-native"
    wrapper.write_text(f"""#!/bin/bash
export PM_NATIVE_MAIN_TRACE=1 PM_ENGINE_TRACE="{run}/engine.ndjson" PM_FEEL_TRACE="{run}/feel.ndjson"
exec "$(dirname "$0")/powermove-native-bin" "$@" 2>"{run}/stderr.log"
""")
    wrapper.chmod(0o755)
    copy = root / Path(o.project).name
    subprocess.run(["cp", "-c", o.project, str(copy)], check=True)
    args = {"name": str(app), "creates_new_application_instance": True,
            "additional_arguments": ["--profile", str(profile), "--draw-occluded", "1", str(copy)]}
    subprocess.run([str(CUA), "call", "launch_app", json.dumps(args)], check=True, capture_output=True)
    time.sleep(o.settle)
    return app, profile, run


def quit_app(lane, app):
    """A normal Quit; the run's edits leave the copy unsaved, so a save prompt
    may hold it: then the process is ended (its journal is cleared on launch)."""
    exe = f"{app}/Contents/MacOS/powermove-native-bin"
    subprocess.Popen(["osascript", "-e", f'tell application id "com.powermove.native.dev.{lane}" to quit'],
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(40):
        if subprocess.run(["pgrep", "-f", exe], capture_output=True).returncode:
            return
        time.sleep(.25)
    subprocess.run(["pkill", "-KILL", "-f", exe])


class Mcp:
    def __init__(self, exe, profile):
        self.p = subprocess.Popen([exe, "--powermove-mcp", "--profile", profile], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "feel", "version": "1"}})
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, m):
        self.p.stdin.write(json.dumps(m) + "\n")
        self.p.stdin.flush()

    def rpc(self, method, params):
        self.n += 1
        self.send({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params})
        while True:
            line = self.p.stdout.readline()
            if not line:
                raise SystemExit("the app's MCP server closed")
            m = json.loads(line)
            if m.get("id") == self.n:
                return m

    def call(self, tool, **args):
        r = self.rpc("tools/call", {"name": tool, "arguments": args})
        res = r.get("result") or {}
        texts = [c["text"] for c in res.get("content", []) if c.get("type") == "text"]
        if r.get("error") or res.get("isError"):
            raise RuntimeError(f"{tool}: {r.get('error') or texts}")
        try:
            return json.loads(texts[0]) if texts else None
        except json.JSONDecodeError:
            return texts[0]

    def pointer(self, panel, action, points, **extra):
        return self.call("computer_use_panel", panelId=panel, action=action,
                         points=[{"x": x, "y": y} for x, y in points], **extra)


def playhead(m):
    return m.call("get_timeline")["playhead"]


def line(a, b, n):
    return [(a[0] + (b[0] - a[0]) * i / n, a[1] + (b[1] - a[1]) * i / n) for i in range(n + 1)]


def main():
    a = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    a.add_argument("--launch", action="store_true")
    a.add_argument("--binary")
    a.add_argument("--project")
    a.add_argument("--root")
    a.add_argument("--lane", default="feel")
    a.add_argument("--settle", type=float, default=12., help="seconds for the project to open")
    a.add_argument("--app")
    a.add_argument("--profile")
    a.add_argument("--run")
    a.add_argument("--only", default="seek,select,viewer,move,trim,scroll,inspector,panels")
    a.add_argument("--hz", type=float, default=120.)
    o = a.parse_args()
    if o.launch:
        app, profile, run = launch(o)
        o.app, o.profile, o.run = str(app), str(profile), str(run)
    only = set(o.only.split(","))
    exe = str(Path(o.app) / "Contents/MacOS/powermove-native")
    if (Path(o.app) / "Contents/MacOS/powermove-native-bin").exists():
        exe += "-bin"
    m = Mcp(exe, o.profile)
    m.call("start_session", prompt="feel measurement", access="project")
    tl = m.call("capture_panel", panelId="timeline")["bounds"]
    vw = m.call("capture_panel", panelId="viewer")["bounds"]
    ruler = 47.
    strips = m.call("get_timeline")
    layers = strips["strips"]
    # Calibrate the time axis: two ruler clicks, read the playhead back.
    xa, xb = tl["width"] * .45, tl["width"] * .85
    m.pointer("timeline", "click", [(xa, ruler)])
    ta = playhead(m)
    m.pointer("timeline", "click", [(xb, ruler)])
    tb = playhead(m)
    pps = (xb - xa) / max(tb - ta, 1e-6)
    x_of = lambda t: xa + (t - ta) * pps
    row_y = lambda i: 81. + 30. * i
    started = time.monotonic()
    marks = {}

    def mark(name, f):
        if name not in only:
            return
        begin = time.clock_gettime(time.CLOCK_UPTIME_RAW)
        f()
        end = time.clock_gettime(time.CLOCK_UPTIME_RAW) + 1.
        marks[name] = (begin, end)

    def clip(i):
        s = layers[i]
        return s["from"], s["end"]

    def seek():
        for f in (.5, .55, .6, .3, .7, .4, .65, .52):
            m.pointer("timeline", "click", [(tl["width"] * f, ruler)])

    def select():
        for i in [0, 1, 2, 1, 0, 3, 1, 0]:
            if i < len(layers):
                m.pointer("timeline", "click", [(150., row_y(i))])

    def viewer():
        m.pointer("timeline", "click", [(150., row_y(0))])
        cx, cy = vw["width"] / 2, (36. + vw["height"]) / 2
        m.pointer("viewer", "drag", line((cx, cy), (cx + 80, cy + 40), 30))
        m.pointer("viewer", "drag", line((cx + 80, cy + 40), (cx, cy), 30))

    def move():
        i = 0
        s, e = clip(i)
        x = x_of(min(e, s + (e - s) / 2))
        m.pointer("timeline", "drag", line((x, row_y(i)), (x + 60, row_y(i)), 24))
        m.pointer("timeline", "drag", line((x + 60, row_y(i)), (x, row_y(i)), 24))

    def trim():
        i = 0
        s, _ = clip(i)
        x = x_of(s) + 2
        m.pointer("timeline", "drag", line((x, row_y(i)), (x + 40, row_y(i)), 20))
        m.pointer("timeline", "drag", line((x + 40, row_y(i)), (x, row_y(i)), 20))

    def scroll():
        for dy in (120, 120, -120, -120, 240, -240):
            m.pointer("timeline", "scroll", [(tl["width"] / 2, 200.)], deltaY=dy)

    def inspector():
        m.pointer("timeline", "click", [(150., row_y(0))])
        ins = m.call("capture_panel", panelId="inspector")["bounds"]
        # Scrub the first number field's label (Properties' Position x).
        y = 160.
        m.pointer("inspector", "drag", line((40., y), (120., y), 30))
        m.pointer("inspector", "drag", line((120., y), (40., y), 30))
        _ = ins

    def panels():
        for p in ("perf", "fxbrowser", "inspector"):
            m.call("open_panel", panelId=p)

    for name, f in (("seek", seek), ("select", select), ("viewer", viewer), ("move", move), ("trim", trim),
                    ("scroll", scroll), ("inspector", inspector), ("panels", panels)):
        try:
            mark(name, f)
        except Exception as e:  # one gesture's failure must not hide the others
            print(f"{name}: {e}", file=sys.stderr)
    try:
        m.call("finish_session", commit=False)
    except RuntimeError as e:
        print(e, file=sys.stderr)
    print(f"pps {pps:.2f}, {len(layers)} layers, {time.monotonic() - started:.1f} s")
    for name, (b, e) in marks.items():
        print(f"== {name}")
        sys.stdout.flush()
        subprocess.run([sys.executable, str(HERE / "feel-report.py"), o.run, "--hz", str(o.hz),
                        "--since", str(b), "--until", str(e)])
    if o.launch:
        quit_app(o.lane, o.app)


if __name__ == "__main__":
    main()
