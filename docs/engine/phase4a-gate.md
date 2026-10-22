# Phase 4a implementation and gate status

Round 2 (2026-10-22) on this Mac (Apple M4 Max). The other two reference
machines remain pending until beta. Decision PROXY-1 is recorded in
[proxy.md](proxy.md). The branch also merges the consumers lane, whose engine
consumer surfaces the C18 specs run on.

## Plan gates (section "Phase 4a: Proxies for cutover")

| Gate | Result | Evidence |
|---|---|---|
| Export safety byte identity | Pass | `engine-proxies.spec.ts` (engine on and off): exports with a Ready proxy equal exports without; the 36-cell / 144-proxy-frame contamination matrix (`pm-node/tests/phase4a_cache_matrix.rs`) |
| No proxy pixels in exports without a draft choice | Pass | Same; `source_for` and the export resolver never read proxies or reduced decode |
| Proxies survive moves, relinks and relaunch | Pass | `engine-proxies.spec.ts` "a linked original can go offline, relink, and relaunch without losing its proxy" |
| Disk guard enforced | Pass | Guard unchanged (max(20 GiB, 10% of the volume)); `pm-proxy/tests/disk_pause.rs` pause/resume |
| Real-app proxy Ready | Pass | `engine-proxies.spec.ts` engine on: real generation to Ready, deletion |
| Section 2 Proxies bar: chart ΔE2000 | Pass | 16-bit chart → ProRes 4444 half proxy: patch mean 0.0146, max 0.0563; full-frame p99 0.2149 (`pm-proxy/tests/chart.rs`) |
| Section 2 Proxies bar: paused original ≤150 ms (4K ProRes) / ≤300 ms (4K long-GOP HEVC) | Pass | 3.6 ms and 3.3 ms to the first paused original frame |
| PROXY-1 transition | Pass | 4K H.264 4:2:2 10-bit (assessed too slow here) plays its proxy; first paused frame is the original after 276.6 ms, one non-stand-in frame |
| Phase 4a performance rows on this Mac | See below | `engine-proxy-performance.spec.ts` |
| Other two machines | Pending beta | |

## Viewer parity (paired hidden apps)

| View | Max CV | Pixels > 1 CV |
|---|---|---|
| VP9 original, Auto | 1 | 0 / 230,400 |
| VP9 original, Half | 1 | 0 / 230,400 |
| VP9 original, Quarter | 1 | 0 / 230,400 |
| PROXY-1 Half: engine vs today's original path | 1 | 0 / 230,400 |
| Today's editing copy vs today's original (evidence only) | 34 | 230,359 / 230,400 |

Under PROXY-1 a paused Half frame is the original through today's scaling
path; today's viewer substitutes its 1280-pixel editing copy there. That
difference is recorded, not gated.

## C18: cached preview through the engine memory cache

Engine on, all pass: `ae-audit-implementation.spec.ts` (cached preview with
graph edits, and the preview controls), `automatic-preview-memory.spec.ts`
(remembered frames during playback and backward scrub),
`playback-audio.spec.ts` (adaptive quality keeps the soundtrack, cached
playback hands audio back, restored project audio). The specs that drove
today's compositor gained engine branches with the same assertions on engine
surfaces; specs that used the old resolution `<select>` drive the Preview
quality popover the plan specifies.

## Performance rows (this Mac)

Paired 60-second Automatic 4K (3840×2160, 24 fps) playback, current app vs
native, presented frames per second, measured with load averages of 20 to
40 from other lanes' builds:

| Source | Current | Native | Native representation | Native late skips |
|---|---|---|---|---|
| HEVC 10-bit long-GOP | 24.01 | 23.99 | original (1441 frames) | 0 |
| H.264 4:2:2 10-bit long-GOP | 24.01 (today's playback conversion) | 23.99 | automatic proxy (1442 frames) | 0 |

Paused upgrade to the original: 4K ProRes 17.1 ms (bar 150 ms), 4K
long-GOP HEVC 3.0 ms (bar 300 ms). PROXY-1 transition on the 4:2:2 source:
the playing proxy gives way to an original stand-in after 24 ms and the exact
original after 410 ms (software decode, no plan bar); no paused proxy frame.

The runs found and fixed four engine problems: the viewer did not use the
ready proxy of an original this Mac's cold assessment found too slow; the
render-ahead schedule starved after one slow frame (1440 late skips); the
process's first ProRes session (356 ms) landed on the first played frame
(the idle viewer now validates and decodes a new proxy once while paused);
and a playback start during the proxy's integrity lookup froze the original
for the clip. The paired rows settle 2 s while paused after a proxy completes.

One hour of native 4K HEVC Automatic playback (60 one-minute samples; the addon
included the scheduler fix but not the later idle-proxy preparation and
pending-lookup fixes, which HEVC playback without a proxy does not reach): total
process working set 1367 MiB at the start, warmup plateau 1395 MiB, final
1395 MiB, post-warmup slope 40.9 KiB/min (upper 121.5, bar 512), 83,870 timed
frames; native usage 595 MiB within its lease; media cache 175 of 256 MiB,
preview cache 166 of 256 MiB. Same engine processes throughout. 1097 late
skips (0.3%) occurred during the hour while other lanes held load at 20 to 40;
the memory row does not gate on cadence.

## Known residuals outside this lane

- `pm-render/tests/corpus.rs`: five video frames (motion/blur-video,
  retime/speed t=2.9, space25d/precomps t=1.5/2.6) fail identically on the
  integration base (issue V2); unchanged by this lane.
- `engine-viewer-canvas.spec.ts`: today's run adapted to half quality at
  237% Auto while the engine stayed at Full (crop 1052×756 vs 2056×1464);
  the ledger assigns this spec to bridge/mediaclose.
- Unit suites failing on the merged base, not in this lane's files:
  `src/main/codex/instructions.test.ts` (golden and 10 KB size),
  `src/engine/document-host/contract.test.ts` (an undocumented tool),
  `legacy/core/project-file.test.ts` (range Blobs), and the Playwright files
  under `scripts/gate-3b` that vitest collects.
