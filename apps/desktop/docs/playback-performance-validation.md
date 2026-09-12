# Playback performance validation — 2026-09-12

The input was a recovery copy of the 87.4-second, 717-layer composition. The source project and its asset contents are not included in this repository. Profiling used an isolated hidden Electron session, a 1440×1000 window, preview quality 1, and automatic quality disabled.

The simultaneous window entrance at 4 seconds introduced about 622 text and shape sources. Preparing small sources in idle slices reduced entrance-frame texture uploads from 271 to zero. In the instrumented runs taken under substantial background machine load, that frame fell from about 105 ms to 22.3 ms. These timings are observations, not a stable hardware benchmark. Source preparation retains the existing 128 MiB texture budget and protects textures used by the last visible frame.

Fitted previews now crop oversized shape/text sources to the visible composition, with exclusions for effects, masks, transitions and 3D content. Timeline redraws avoid alternating gutter layout writes and repeated text measurement. Scrubbing only enumerates snap targets when Shift is used. Animation evaluation reuses same-frame local transforms and prepared temporal results, invalidating on edits and time changes.

The final profile in the temporary checkout measured the 4-second entrance at 3.1 ms, playback p95 at 4.0 ms, and sampled scrub p95 at 3.2 ms. Machine load differed from earlier runs, so texture-upload counts are the strongest direct comparison.

The graph editor now synchronizes keyboard and toolbar state, exposes Value/Speed graph modes, and fits selected curves after panning. Curve sample caching reduced repeated property evaluations from 804 to 2 in a two-axis scene. Hidden Electron checks cover speed edits without changing key values/times, Undo, fitting, and keyboard toggling. Screenshots were inspected in value and speed modes.

The deterministic hidden Electron tests require zero entrance-frame uploads for 200 unique sources and compare optimized pixels with source clipping disabled (maximum channel difference 1). A second test requires fewer draws and less than one quarter of the source texture storage for oversized offscreen artwork. Unit coverage checks idle deadlines, cancellation, cache ownership, backward seeks and same-frame edits.

The scale test's previous saved baseline (2026-08-27) measured a workload that reused first-frame hierarchy results. The test was subsequently corrected to call `beginEval` for every frame, but that baseline remained unchanged and failed before this work. This PR records the actual 1,000-layer × 60-key × 30-distinct-frame workload and labels it explicitly, retaining the 1.5× calibrated regression threshold. Its 30-second test timeout allows the 20 measured/warmup batches to finish on a busy machine; it does not change the CPU-time performance assertion.

Reproduce the project profile with a private copy:

```sh
cd apps/desktop
PM_PERF_PROJECT=/absolute/path/to/copied-project.json bun run test:e2e -- composition-playback-profile.spec.ts
```

The profile reports all frame timings, texture uploads, cache bytes and sampled scrubs across the full composition. It does not establish audio playback performance when referenced media files are absent from the copy.

Final validation: 1,870 unit tests passed (one intentionally skipped); 29 hidden Electron checks passed; typechecking and all 10 extension boundaries passed. After the final import-only boundary corrections, the affected keymap and property-reveal tests passed again.
