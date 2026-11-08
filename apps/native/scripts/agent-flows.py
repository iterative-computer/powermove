#!/usr/bin/env python3
"""Agent flows through the running app's MCP connection: publish an extension,
see its effect, revert it, see it gone. Synthetic extensions only.

usage: agent-flows.py <powermove-native> <profile> [flow ...]
       (pm-gui launch <lane> <project> first; the profile is the lane's)
Flows: v4-theme, v4-overlay, v1-timeline, v4-timeline, v4-painter, v1-theme (default: all). Exit status 1 on any failure.
"""
import base64, json, os, shutil, subprocess, sys, tempfile, time, zlib

PURPLE = (0x5A, 0x1F, 0x8C)


class Mcp:
    def __init__(self, binary, profile):
        self.p = subprocess.Popen([binary, "--powermove-mcp", "--profile", profile],
                                  stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.n = 0
        self.rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                                "clientInfo": {"name": "agent-flows", "version": "1"}})

    def rpc(self, method, params):
        self.n += 1
        self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method,
                                       "params": params}) + "\n")
        self.p.stdin.flush()
        while True:
            message = json.loads(self.p.stdout.readline())
            if message.get("id") == self.n:
                return message

    def tool(self, name, args):
        result = self.rpc("tools/call", {"name": name, "arguments": args})["result"]
        if result.get("isError"):
            raise AssertionError(f"{name}: {result['content'][0]['text']}")
        return result["content"]

    def close(self):
        self.p.stdin.close()
        self.p.wait(timeout=10)


def png_pixel(data, x, y):
    """RGBA of one pixel of an 8-bit RGBA, non-interlaced PNG."""
    at, chunks, width = 8, b"", 0
    while at < len(data):
        length = int.from_bytes(data[at:at + 4], "big")
        kind, body = data[at + 4:at + 8], data[at + 8:at + 8 + length]
        if kind == b"IHDR":
            width = int.from_bytes(body[:4], "big")
        if kind == b"IDAT":
            chunks += body
        at += 12 + length
    raw, stride = zlib.decompress(chunks), width * 4
    rows, prev = [], bytearray(stride)
    for r in range(y + 1):
        line = raw[r * (stride + 1):(r + 1) * (stride + 1)]
        kind, row = line[0], bytearray(line[1:])
        for i in range(stride):
            a = row[i - 4] if i >= 4 else 0
            b, c = prev[i], prev[i - 4] if i >= 4 else 0
            if kind == 1: row[i] = (row[i] + a) & 255
            elif kind == 2: row[i] = (row[i] + b) & 255
            elif kind == 3: row[i] = (row[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        prev = row
    return tuple(prev[x * 4:x * 4 + 4])


def panel_colour(mcp):
    """The Properties panel's material, well inside its empty lower body.
    A newly published panel may open in front of it, so it comes forward."""
    mcp.tool("open_panel", {"panelId": "inspector"})
    content = mcp.tool("capture_panel", {"panelId": "inspector"})
    facts = json.loads(content[0]["text"])
    image = base64.b64decode(content[1]["data"])
    w, h = facts["image"]["width"], facts["image"]["height"]
    return png_pixel(image, w // 2, h - 40)[:3]


def png_rows(data):
    """(width, rows of RGBA bytes) of an 8-bit RGBA, non-interlaced PNG."""
    at, chunks, width, height = 8, b"", 0, 0
    while at < len(data):
        length = int.from_bytes(data[at:at + 4], "big")
        kind, body = data[at + 4:at + 8], data[at + 8:at + 8 + length]
        if kind == b"IHDR":
            width, height = int.from_bytes(body[:4], "big"), int.from_bytes(body[4:8], "big")
        if kind == b"IDAT":
            chunks += body
        at += 12 + length
    raw, stride = zlib.decompress(chunks), width * 4
    rows, prev = [], bytearray(stride)
    for r in range(height):
        line = raw[r * (stride + 1):(r + 1) * (stride + 1)]
        kind, row = line[0], bytearray(line[1:])
        for i in range(stride):
            a = row[i - 4] if i >= 4 else 0
            b, c = prev[i], prev[i - 4] if i >= 4 else 0
            if kind == 1: row[i] = (row[i] + a) & 255
            elif kind == 2: row[i] = (row[i] + b) & 255
            elif kind == 3: row[i] = (row[i] + (a + b) // 2) & 255
            elif kind == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                row[i] = (row[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(row)
        prev = row
    return width, rows


CLIP = (31, 75, 82)


def clip_left(mcp):
    """Where the first clip starts in the timeline, in panel points: it sits at
    the layer column's right edge."""
    content = mcp.tool("capture_panel", {"panelId": "timeline"})
    facts = json.loads(content[0]["text"])
    width, rows = png_rows(base64.b64decode(content[1]["data"]))
    points = facts["bounds"]["width"] / width
    left = min((x for row in rows for x in range(width)
                if near(tuple(row[x * 4:x * 4 + 3]), CLIP, 6)), default=None)
    return None if left is None else round(left * points)


def timeline_purple(mcp):
    """Whether purple is painted anywhere on the timeline's clips."""
    content = mcp.tool("capture_panel", {"panelId": "timeline"})
    width, rows = png_rows(base64.b64decode(content[1]["data"]))
    return any(near(tuple(row[x * 4:x * 4 + 3]), PURPLE, 30)
               for row in rows for x in range(width))


def viewer_centre(mcp):
    """The viewer capture's centre: the composition's middle."""
    content = mcp.tool("capture_panel", {"panelId": "viewer"})
    facts = json.loads(content[0]["text"])
    image = base64.b64decode(content[1]["data"])
    return png_pixel(image, facts["image"]["width"] // 2, facts["image"]["height"] // 2)[:3]


def near(a, b, tolerance=24):
    return all(abs(x - y) <= tolerance for x, y in zip(a, b))


def until(what, check, seconds=20):
    deadline = time.time() + seconds
    while time.time() < deadline:
        value = check()
        if value:
            return value
        time.sleep(0.5)
    raise AssertionError(f"timed out waiting for {what}")


EXTENSIONS = {
    "v4-theme": ({"id": "agent-flow-v4-theme", "name": "Flow Theme", "version": "1.0.0",
                  "apiVersion": 4, "entry": "index.tsx", "author": "agent"},
                 "index.tsx",
                 "import { jsx } from 'powermove/jsx-runtime';\n"
                 "export function activate({ themes }) {\n"
                 "  themes.register({ id: 'flow', title: 'Flow', tokens: { 'colors.panel': '#5a1f8c' } });\n"
                 "  themes.activate('flow');\n"
                 "}\n"
                 "export function view() { return jsx('text', { children: 'Flow theme' }); }\n"),
    "v4-overlay": ({"id": "agent-flow-v4-overlay", "name": "Flow Overlay", "version": "1.0.0",
                    "apiVersion": 4, "entry": "index.tsx", "author": "agent"},
                   "index.tsx",
                   "import { jsx } from 'powermove/jsx-runtime';\n"
                   "export function activate({ overlays }) {\n"
                   "  overlays.viewer(({ frame }) => jsx('div', { style: { position: 'absolute',\n"
                   "    left: frame.x, top: frame.y, width: frame.width, height: frame.height,\n"
                   "    background: '#5a1f8c' } }));\n"
                   "}\n"
                   "export function view() { return jsx('text', { children: 'Flow overlay' }); }\n"),
    "v1-timeline": ({"id": "agent-flow-v1-timeline", "name": "Flow Timeline", "version": "1.0.0",
                     "apiVersion": 3, "entry": "index.ts", "author": "agent"},
                    "index.ts",
                    "export default function activate(api) {\n"
                    "  api.workspace.mutate((workspace) => {\n"
                    "    workspace.chrome ??= {};\n"
                    "    workspace.chrome.timeline = { ...(workspace.chrome.timeline ?? {}), gutterWidth: 420 };\n"
                    "  });\n"
                    "}\n"),
    "v4-timeline": ({"id": "agent-flow-v4-timeline", "name": "Flow Timeline v4", "version": "1.0.0",
                     "apiVersion": 4, "entry": "index.tsx", "author": "agent"},
                    "index.tsx",
                    "import { jsx } from 'powermove/jsx-runtime';\n"
                    "export function activate({ timeline }) { timeline.chrome({ gutterWidth: 340 }); }\n"
                    "export function view() { return jsx('text', { children: 'Flow timeline' }); }\n"),
    "v4-painter": ({"id": "agent-flow-v4-painter", "name": "Flow Painter", "version": "1.0.0",
                    "apiVersion": 4, "entry": "index.tsx", "author": "agent"},
                   "index.tsx",
                   "import { jsx } from 'powermove/jsx-runtime';\n"
                   "export function activate({ timeline }) {\n"
                   "  timeline.paint((clips) => Object.fromEntries(clips.map((c) =>\n"
                   "    [c.id, [{ rect: [0, 0, c.width, c.height], color: '#5a1f8c' }]])));\n"
                   "}\n"
                   "export function view() { return jsx('text', { children: 'Flow painter' }); }\n"),
    "v1-theme": ({"id": "agent-flow-v1-theme", "name": "Flow Theme v1", "version": "1.0.0",
                  "apiVersion": 3, "entry": "index.ts", "contributes": ["themes"], "author": "agent"},
                 "index.ts",
                 "export default function activate(api) {\n"
                 "  api.theme.register({ id: 'agent-flow-v1-theme.flow', name: 'Flow', scheme: 'auto',\n"
                 "    tokens: { '--bg-panel': '#5a1f8c' }, css: '' });\n"
                 "  api.theme.activate('agent-flow-v1-theme.flow');\n"
                 "}\n"),
}


PROBES = {"v4-overlay": viewer_centre, "v1-timeline": clip_left, "v4-timeline": clip_left,
          "v4-painter": timeline_purple}
# (visible?, reverted?) given the probe before and now; colours by default.
CHECKS = {
    "v1-timeline": (lambda before, now: now is not None and now >= 410,
                    lambda before, now: now is not None and abs(now - before) <= 3),
    "v4-painter": (lambda before, now: now is True, lambda before, now: now is False),
    "v4-timeline": (lambda before, now: now is not None and abs(now - 341) <= 3,
                    lambda before, now: now is not None and abs(now - before) <= 3),
}
COLOUR = (lambda before, now: near(now, PURPLE), lambda before, now: near(now, before, 6))


def flow(mcp, name):
    manifest, entry, source = EXTENSIONS[name]
    probe = PROBES.get(name, panel_colour)
    visible, reverted = CHECKS.get(name, COLOUR)
    eid = manifest["id"]
    session = json.loads(mcp.tool("start_session", {"prompt": f"Agent flow {name}"})[0]["text"])
    stage = session["stagingDirectory"]
    try:
        folder = os.path.join(stage, eid)
        if os.path.exists(folder):
            # A previous run left it installed: take it out first.
            shutil.rmtree(folder)
            mcp.tool("publish_extensions", {"extensions": [
                {"id": eid, "action": "removed", "summary": f"{name} leftover"}]})
            time.sleep(2)
        before = probe(mcp)
        # A baseline that already passes the check would prove nothing.
        assert before is not None and not visible(before, before), \
            f"{name}: the baseline already shows the effect ({before})"
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "manifest.json"), "w") as f:
            json.dump(manifest, f)
        with open(os.path.join(folder, entry), "w") as f:
            f.write(source)
        mcp.tool("publish_extensions", {"extensions": [
            {"id": eid, "action": "created", "summary": f"{name} flow"}]})
        shown = until(f"{name} visible",
                      lambda: (lambda now: visible(before, now) and [now])(probe(mcp)))[0]
        shutil.rmtree(folder)
        mcp.tool("publish_extensions", {"extensions": [
            {"id": eid, "action": "removed", "summary": f"{name} flow revert"}]})
        until(f"{name} reverted", lambda: reverted(before, probe(mcp)))
        print(f"ok {name}: {before} -> {shown} -> {probe(mcp)}")
    finally:
        mcp.tool("finish_session", {"commit": False})


def main():
    binary, profile, *flows = sys.argv[1:]
    mcp = Mcp(binary, profile)
    failed = 0
    for name in flows or list(EXTENSIONS):
        try:
            flow(mcp, name)
        except AssertionError as error:
            failed += 1
            print(f"FAIL {name}: {error}")
    mcp.close()
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
