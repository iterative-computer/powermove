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


Follow-up for the opening seconds and frame 00:03:29: idle source preparation now also runs while paused near an entrance, and a cancelled queue reschedules itself after a font or preview-size change. The audio device is prepared while an audible project is idle; the transport clock starts after synchronous audio setup, preserving the opening frames even on a cold device.

Profiling also found a 53 ms synchronous thumbnail readback after playback had started. Thumbnail completion now rechecks playback/navigation/export ownership after both asynchronous stages. Its deferred retry captures only the thumbnail instead of serializing the entire project every 500 ms.

The final isolated profile included the composition's actual MP3. Playback from zero and from 00:03:29 reached the window entrance with zero texture uploads; CPU render time was 3.4 ms and 4.0 ms respectively. Entrance intervals stayed around 33 ms and neither run reported a main-thread long task. The earlier nearby-start run required 143 uploads, 15.4 ms of CPU rendering, and 32.6 ms of GPU work on the entrance frame. Timings remain dependent on background machine load. Deterministic regression tests cover the delayed thumbnail callbacks, repeated thumbnail retries, cancelled idle queues, cold audio clock initialization, and paused source preparation with pixel parity.

A further live-session check reproduced the reported FPS dip: about 22 FPS and a 149 ms frame interval at the four-second entrance, despite only 9.6 ms of synchronous rendering. The previous isolated checks did not include the user's active Progressive Blur extension or the live 1348×758 Retina drawing buffer. With both reproduced, the entrance again uploaded 271 textures and the following interval reached 189 ms. Checking main-thread long tasks alone missed this GPU/upload stall.

The blur on the opening timeline group prevented its children from using preview source clipping. Oversized offscreen child bitmaps crowded the 128 MiB texture cache and evicted already-prepared entrance sources. Group effects consume the composed, frame-sized group target, so clipping ordinary child sources to that same target preserves their input pixels. The compositor now permits this; restrictions on a child's own effects, masks, transitions, paths and 3D rendering remain intact.

With the extension and Retina buffer present, playback from zero reached the entrance with zero texture uploads, a 4.0 ms CPU render and adjacent intervals of about 33 ms. Direct pixel comparisons at 0, 0.5, 1, 2, 00:03:29 and 4 seconds were identical to unclipped sources, including the progressive blur. At frame zero, source textures fell from 126.5 MiB to 35.0 MiB; at 00:03:29 they fell from 46.2 MiB to 11.3 MiB. These isolated source measurements clear caches between comparisons; running playback also retains useful animation variants within the existing budget.

The committed group-blur regression compares preview pixels with unclipped sources and requires at least a fourfold texture-storage reduction. It failed before this correction (9,695,152 bytes on both paths) and passes after it. The private composition and user extension remain outside the repository.
