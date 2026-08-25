# Powermove Electron capability probe

This is a standalone, unbundled Electron application. It does not import Powermove or write outside this directory (apart from an interactive save target explicitly chosen by the person running it). Each run writes `results-<mode>.json` and normally prints the same JSON to stdout.

## Install and run

```sh
cd /Users/judekim/Developer/powermove/spikes/probe
npm install
./run.sh file
./run.sh app
./run.sh finder
./run.sh interactive
```

`file` loads `index.html` with `loadFile`. `app` registers the privileged, secure `app` scheme and serves the probe and fixtures with byte-range support. `finder` launches Electron through LaunchServices with `open -n`, waits for its result file, and captures the environment an app receives outside the terminal. `interactive` is the file-mode probe plus a native Save dialog and a 1 GiB streaming write. The interactive file is removed through its handle when the platform supports that operation; otherwise the report names the file to remove manually.

Automated runs have a 90-second hard timeout. Interactive runs have a 10-minute timeout. Probe progress is checkpointed in a hidden partial JSON file, then promoted to the normal result even if the renderer exits during a large IPC test.

## Probe meanings

- **a — codecs:** static MediaSource/WebCodecs decode support, then real metadata, seeking, canvas sampling, color tolerance, and alpha preservation for every entry in `../fixtures/fixtures.json`. The VP9/AV1 limited-green cases include the decoded `VideoColorSpace` and main-process ffprobe color fields. A missing manifest or media file is recorded as `skipped: "fixture missing"`.
- **b — WebGPU:** obtains a high-performance adapter/device and copies a canvas `ImageBitmap` into a 256×256 `rgba8unorm` texture.
- **c — WebCodecs encode:** checks VP9, VP8, H.264, and Opus configurations; actually encodes 30 1080p VP9 frames and one second of planar-f32 stereo Opus.
- **d — sandbox/CSP:** runs the Powermove-shaped opaque `srcdoc` iframe and blob Worker, verifies the result is 42 and outbound fetch is blocked, and compares parent `eval` under permissive and strict CSP pages.
- **d2 — sandbox/CSP follow-ups:** compares a served external-script sandbox (including app-scheme explicit-source and `'self'` policies), blob document, parent-hash-allowlisted `srcdoc`, and frame-meta-only `srcdoc`; V4 includes the main-process console CSP violation.
- **e — fonts:** compares `system_profiler` plus filesystem fallback enumeration with `queryLocalFonts()` before and after an Electron-generated click. Every Electron permission request/check is recorded.
- **f — capturePage:** takes five Chromium window captures and reports PNG dimensions, bytes, and timing distribution.
- **g — File System Access:** records picker and OPFS API presence in every mode. Interactive mode writes 1 GiB in 8 MiB chunks and reports throughput and memory.
- **h — Codex discovery:** records the inherited PATH and CODEX_BINARY, current-PATH resolution, the two Swift hardcoded candidates, and an interactive login-shell lookup. Compare file/app with finder.
- **i — panel re-parenting:** moves a live panel 50 times and checks input/focus/listeners, WebGL pixels and texture, observers, CSS transition presence, and continued video playback.
- **i2 — focus restoration:** reports the focus restoration, selection range, and per-move blur/focus counts collected during probe i.
- **j — app scheme and boot barrier:** app mode only; checks origin, secure context, Web Crypto, IndexedDB, and the time to receive a 200 KiB store snapshot before any other renderer work.
- **k — large IPC:** sends 64 MiB, 256 MiB, and 1 GiB renderer-to-main payloads sequentially, then 256 MiB main-to-renderer, recording timing and available process/renderer memory. A failure here is a capability result, not a harness failure.

Each top-level probe `a` through `k`, plus `d2` and `i2`, has `{ ok, ms, detail, error? }`. `ok` means the probe function completed and its detailed capability fields should still be inspected; unsupported codecs and expected denials are preserved inside `detail` rather than normalized away.
