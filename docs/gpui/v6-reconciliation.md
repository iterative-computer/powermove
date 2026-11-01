# Branch `feat/gpui-mac` against plan v6

Plan v6 ("Powermove Native: plan for the full Rust app", Jude, 2026-10-29) is
the execution baseline from 2026-11-01. This branch was started before it
arrived. This note maps what exists here onto v6's work packages, so nothing is
redone or lost. Where v6 and `architecture.md` disagree, **v6 wins**.

## What v6 changes from architecture.md

| Topic | architecture.md (superseded) | v6 |
|---|---|---|
| UI framework | `gpui-pre` 0.3.8 from crates.io | GPUI-CE vendored under `vendor/gpui-ce`, with patches P-DEVICE, P-HEADLESS, P-LAYER, P-HIT, P-TEXTURE, P-TEXT, P-A11Y, P-120 |
| App location | `apps/mac` | `apps/native` on `feat/native-engine`; lanes `feat/na-<lane>`, merged `--no-ff` |
| Built-in panels | Rust GPUI crates | v4 extensions (JSX → native ops) in V8 isolates, ported in P3 after the v4 API freeze |
| v1–3 extensions | may be dropped if costly | **all keep working** (N4) through the compat page (one WKWebView, one realm) |
| Document authority | `pm-mac-core` on the main thread | `pm-kernel` actor on its own thread: leases, folding, external history entries, journal, uiState, registries |
| Viewer | RGB IOSurface sampled in a GPUI element | below-slot `CAMetalLayer` (rgba16f, EDR); engine-drawn locked overlays (S3) |
| Data | `.pmv` files | library-first profile store plus IndexedDB media migration (D-STATE, TR) |
| Estimates | given | none (Jude's rule) |

## Existing work, mapped

| Work on this branch | v6 package | Disposition |
|---|---|---|
| `docs/gpui/parity-a-shell.md`, `parity-b-editor.md` (1,392 items) | P0 W1 inventory (workflows → acceptance tests) | Input to W1. Not W1 itself: W1 is per-workflow acceptance tests against v1.2.0. |
| `docs/gpui/engine-boundary.md` | P1 #6 `pm-engine-service`, #5 `pm-kernel` | Reference for engine calls. Its process model is superseded by §3.0. |
| `apps/mac/crates/glass` (NSToolbar, glass behind/above GPUI) | #3 composition contract, §3.10 design system, S2 glass | Spike evidence. Re-done on GPUI-CE as P-LAYER slots. |
| `apps/mac/crates/kit` (all `controls/` in GPUI, 31 tests) | #13 `powermove/ui` native backings | Logic modules (arith, scrub, linked scale, colour spaces) carry over. Widgets become the native backings of `powermove/ui` dense rows; AppKit owns segmented, colour wells, toggles and toolbar items. |
| `apps/mac/vendor` RGB surface patch + `engine-host` | #3 P-TEXTURE, #6 engine service | Closest existing work to P-TEXTURE (BGRA8/RGBA16F, consumer completion). Port to GPUI-CE. The viewer itself moves to a CAMetalLayer slot (S3). |
| `apps/native/crates/core` (`pm-mac-core`: byte-identical PMV4 writer, atomic transactions, inverse-op undo, gestures, selection, 42 tests) | #5 `pm-kernel` | Seed of the kernel. Needs the actor thread, leases/folding (§3.2.1), external entries, journal, library store, uiState. The writer and its golden tests carry over as is. |
| Core command catalog (lane in progress) | #5 kernel commands for MCP/agents/UI | Carries over: pure document operations with validation. |
| Timeline and inspector lanes | P3 #15 built-ins | Stopped 2026-11-01: out of order (v4 API not frozen) and on the wrong framework. |

## Not yet started from P0

S1 (V8 sandbox build), S2 (GPUI-CE patches + composition matrix), S3
(EDR viewer layer), S4 (GLSL routes), S5 (compat page channels), S6
(IndexedDB reader, Squirrel, Keychain ACL); inventories C-MATRIX, T1, D-STATE,
W1, OS base, trace recorder; housekeeping. E-track items are tracked on the
engine side.

## Constraints of this machine

v6 §9 assumes the M4 Max (128 GB RAM, more than 80 GB free disk, 6–8 lanes).
This Mac has about 10 GB free. A from-source V8 sandbox build (S1) cannot run
here; GPUI-CE builds and research lanes can.
