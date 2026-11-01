# Phase 1 performance gate

Status, 2026-11-01 unlocked session: **INCOMPLETE / NOT PASSED**. Five
release-native / frozen-Electron pairs completed before the bounded retry reached
its 900-second deadline (builds included). A sixth partial pair is excluded.
These are real exploratory observations, not the required ≥30 accepted runs.
No performance or release gate is certified.

## Conditions

Apple M5 (Mac17,2), 16 GiB, macOS 27.2 / 26B5086k, AC power, unlocked.
Rust 1.96.1; native release build passed twice (7m48s and 8m28s).
The reference is unmodified v1.2.0 / `07ed5fa17ee640659cb075f9cad1af2095011d60`.
Only an extension-free public shape composition was timed: 640×360, 30 fps,
3 seconds, looped for CPU playback. No personal project/profile was opened.

Every completed retry pair recorded app presence and 1/5/15-minute load before
and after each side. Retry one-minute load ranged **3.89–9.62**;
the first attempt ranged 8.14–15.98, and compilation briefly exceeded 30.
Present and untouched throughout: Finder, Aside, Grok Bot, Notion, Mail, Claude,
T3 Code (Nightly), installed Powermove and its helpers; RobloxPlayer,
RobloxMenuBar and figma_agent were also present. Seven existing Powermove main
processes were initially detected (five at the end), including helper-mode hosts. No claim that all were
idle is made. These retained apps prevent a settled-machine acceptance claim.

The initial display check found Q27G41ZE, 2560×1440 at 240 Hz, EDR current /
potential 1.0 / 1.0. It was switched to 120 Hz. A later check found only the
built-in 3024×1964 Retina display at 120 Hz, potential EDR 16. The transition time
was not recorded; a continuously pinned display configuration is not proven.
Neither required hardware rig is certified. Later native checks physically pin
120/60 Hz on the built-in panel. See the dated S3/service sections.

`powermetrics` without sudo returned “powermetrics must be invoked as the
superuser”; no elevated retry. Disk started at about 22 GiB free, stayed above
8 GiB, and reference worktrees/node_modules and release artifacts were removed
at the end of each attempt. The external display was later reconnected and
its exact original mode 150 / 240 Hz restored and verified. Raw results, traces, profiles and screenshots stay
under `/tmp`, outside Git. The caller holds one inode-checked GUI lock for the
whole session; harnesses borrow it and cannot release it.

## Results

Values are **p50 / p95**, nearest rank; n=5 per side except as stated. Ratios
compare matching quantiles. At n=5, p95 is the maximum observation, not a stable
tail estimate. `—` means unmeasured, never zero. Readiness, open and repaint
endpoints differ or are incomplete; their ratios are diagnostic only. All rows
remain INCOMPLETE even where an individual observed clause is below its bound.

| Row | Native p50 / p95 | Electron p50 / p95 | Ratio p50 / p95 | Budget | Gate verdict | Conditions / limits |
|---|---:|---:|---:|---|---|---|
| Cold launch (readiness diagnostic) (ms) | 967.402 / 1620.746 | 2010.985 / 4093.630 | 0.481 / 0.396 | p95 <400 ms; ratio ≤0.50 | INCOMPLETE | Controller/harness readiness, not first interactive frame |
| Open (ms) | 574.760 / 636.730 | 75.800 / 585.900 | 7.583 / 1.087 | ratio ≤0.50; main stall ≤1 frame | INCOMPLETE | Different native/Electron boundaries; main stalls absent |
| Cached scrub (ms) | — / — | 15.100 / 15.500 | — / — | p95 ≤9.333 ms; ratio ≤0.60 | INCOMPLETE | Native presentation unavailable in all five attempts; Electron GPU + rAF |
| 4K long-GOP random seek (ms) | — / — | — / — | — / — | preview ≤10.333 ms; full ≤150 ms; ratio ≤0.60 | INCOMPLETE | No public 4K long-GOP corpus fixture prepared |
| 60 s playback (physical drops/headroom) (ms) | — / — | — / — | — / — | zero drops at 60 fps; p95 work ≤8.333 ms | INCOMPLETE | Both CPU windows exceeded 60 s; 30 fps shape fixture and incomplete physical census |
| Playback CPU cost (%) | 14.719 / 14.719 | 54.088 / 54.088 | 0.272 / 0.272 | CPU ratio ≤0.60; GPU ≤0.80; energy ≤0.70 | INCOMPLETE | n=1 full 60 s interval/side; CPU clause only; no GPU/energy |
| Matched-device A/V (ms) | — / — | — / — | — / — | mean ≤1 ms; p95 jitter ≤2 ms | INCOMPLETE | No paired Electron device timestamps; separate native clip check below |
| Edit → viewer presentation (ms) | 47.231 / 47.231 | 16.400 / 18.800 | 2.880 / 2.512 | p95 ≤9.333 ms; ratio ≤0.60; main work ≤4 ms | INCOMPLETE | Native n=1, Electron n=5; four native timeouts; shell repaint/main work absent |
| Idle CPU (%) | 1.264 / 1.893 | 9.749 / 15.393 | 0.130 / 0.123 | CPU <1%; ratio ≤0.50; ≤2 wakes/s/extension | INCOMPLETE | Observed native absolute CPU target missed; 3 s settle + 2 s sample; no extension attribution |
| Small-project family footprint (MB) | 186.746 / 207.636 | 1063.132 / 1071.471 | 0.176 / 0.194 | native ≤Electron −150 MB; allocations counted once | INCOMPLETE | Summed phys_footprint; deduplication unverified; no extension load |

The single CPU playback intervals were **60.009 s native** and **60.009 s
Electron**; CPU uses one logical core as 100%. Native CPU ratio was **0.272**,
below the CPU-only 0.60 bound. Native idle p95 **1.893%** exceeds the absolute
1% target in this diagnostic. Family footprint p95 saved **863.835 MB**, but
shared CPU/GPU allocation identity is not established. Native base footprint
p50/p95 was **33.588 / 33.867 MB**, Electron **571.988 / 602.921 MB**;
document-delta checkpoints are retained privately. Metal allocation is separate
and is never added to family footprint. No extension split is claimed.

## Harness fixes and validation

The first pair reproduced cross-app fixture contamination: native edits saved
PMV data back to the shared temporary JSON, and Electron's JSON parser refused
it. Each app/pair now receives its own writable copy, preserving the prepared
fixture hash. Completed observations survive a later playback failure. Setup
seeks skip an already displayed native frame; the measured cached seek still
requires a changed frame and positive drawable presentation. NDJSON service
traces now parse without losing absent timestamps; malformed rows still fail.

Explicit diagnostic mode permits only retained-app/high-load collection while
preserving invalid conditions and separate acceptance/diagnostic summaries.
Unlocked-session, disk, power and active-build checks remain mandatory. Borrowed
whole-session locks require the expected owner and inode. **21/21 harness tests
pass**, including real Darwin CPU/family accounting, fixture mutation isolation,
partial-result preservation, NDJSON, lock ownership and fail-closed conditions.
No product code changed.

The first attempt stopped after launch/open measurement errors. The retry
retained five completed pairs, then timed out and reaped its owned applications;
no partial sixth pair is silently counted. Screenshots during timed intervals
were not used as performance evidence. No push, release, tag or workflow dispatch.

## Still needed

Repeat ≥30 per row on both required rigs with stable display/quality/thermal
conditions and the required corpora. Supply first-interactive-frame readiness,
matching file-open boundaries/main-thread stalls, physical cached/random seek,
4K long-GOP hydration, full physical frame census, matched Electron device A/V,
per-frame work, GPU/energy cost, shell repaint/main work and deduplicated memory.
The separate foreground S3/service observations do not fill these paired gaps.
