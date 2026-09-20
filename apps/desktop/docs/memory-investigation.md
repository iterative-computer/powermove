# Memory audit and integration — September 19, 2026

Validated reductions are integrated into application code. Persisted document undo, its cursor, and pending redo survive application restarts. Rendering precision, preview resolution, audio fidelity, and existing undo-retention limits were not lowered.

## Production measurements

| Measurement | Before | After | Reduction |
| --- | ---: | ---: | ---: |
| Three histories: retained V8 heap | 382.73 MiB | 12.84 MiB | 96.64% |
| Entire main store: retained V8 heap | 438.43 MiB | 68.70 MiB | 84.33% |
| Data serialized for each renderer's bootstrap | 256.33 MiB | 89.36 KiB | 99.97% |
| Three histories: dictionary on disk | 204.85 MiB | 6.24 MiB | 96.95% |

These are controlled measurements of real saved data using the production implementation, not a claim that the entire application uses 96.6% less memory. Savings overlap and must not be added together. All 355 entries, labels, forward/backward patches, and saved cursors produce exactly the same serialized data after sharing and dictionary round trips. History A has pending redo.

| History | Entries | Cursor | Parsed heap | Shared heap |
| --- | ---: | ---: | ---: | ---: |
| A | 160 | 158 | 228.35 MiB | 7.71 MiB |
| B | 146 | 145 | 80.54 MiB | 2.45 MiB |
| C | 49 | 48 | 73.85 MiB | 2.68 MiB |

Each history measurement uses a separate Node v26.7.0 process, explicit GC, and event-loop turns to release serialization temporaries. The store comparison bundles the actual FileStore and operates exclusively on a disposable copy. In the final store sample, raw loading took 1.78 seconds; initial conversion plus completed atomic writes took 4.96 seconds; reopening the compact store took 0.262 seconds. These are single observations, not latency guarantees. Migration has a one-time CPU/I/O cost; feature quality and undo retention are unchanged. Compact files avoid expanding duplicate records on subsequent launches.

The initial investigation's 791 MiB history baseline was invalid because temporary serialization strings remained live on the measurement stack. It is superseded by the corrected numbers above. Sanitized measurements and exact-round-trip results are in `memory-investigation-results.json`; no project content is included.

## Disposable Electron profile

A copied store restored the same 16-layer project, 49 history entries, 1,461 DOM nodes and 12.59 MiB of framebuffers in both builds. The build without these memory changes retained 324.27 MiB in the renderer heap and 291.75 MiB in the main heap. The integrated build retained 75.53 MiB and 67.85 MiB after reopening its compact store (about 77% lower combined live JS heap).

The integrated build's first legacy-store launch measured 610.1 MiB main, 229.6 MiB renderer and 397.8 MiB GPU physical footprints. Its next launch from compact files measured 192.0 MiB main, 168.7 MiB renderer and 322.4 MiB GPU. Ready time was 4.35 seconds then 1.29 seconds. These are single samples, and the store-only copy has no loaded media textures/audio. They establish the remaining allocator/startup cost and benefit of migration, not universal whole-app savings. The earlier baseline/production physical measurements briefly overlapped and are excluded from percentage claims.

A final sequential comparison used one frozen copy of the then-current store for both builds. This later source had zero active history entries; both builds restored the same 16-layer document. On the second launch, baseline renderer/main live heaps were 278.64/246.94 MiB versus 73.73/66.46 MiB integrated (73.3% less combined live JS heap). Separate `vmmap` readings were:

| Process | Baseline, second launch | Integrated, second launch |
| --- | ---: | ---: |
| Main | 1.4G | 171.9M |
| Renderer | 437.7M | 169.8M |
| GPU | 301.6M | 327.7M |

These are OS-reported physical footprints, including allocator behavior. They are workload observations, not guaranteed reductions on active media projects. The integrated first legacy-store launch used 474.4M main memory; the compact-store relaunch used 171.9M. The profiling processes exited and disposable profiles were removed successfully.

## Edit latency

The first 884-layer reorder benchmark exposed roughly 5–6 ms of additional synchronous work. Scoped commits now reuse one serialization for equality and byte accounting; a bounded weak lookup checks an unchanged record by identifier before hashing it. Identifiers never replace full content equality. Undo retention still counts exactly the same UTF-8 serialized bytes, including Unicode and commas.

In three final paired processes, baseline reorder medians were 19.76, 19.63 and 28.04 ms; integrated medians were 20.50, 20.52 and 20.48 ms. Ordinary scoped property edits stayed around 0.009–0.016 ms. Every benchmark traversed all undo/redo states afterward. These small samples, on a desktop with uncontrolled background load, do not establish literally zero CPU cost. The stable pairs show under 1 ms remaining overhead for snapshotting all 884 layers; the earlier 5–6 ms penalty is substantially reduced.

## Integrated changes

- `shared/history-memory.ts` shares equal immutable layer/edit records through bounded weak indexes, including a fast path for unchanged identifiers. Hashes select a bucket; full content equality prevents collision-based corruption. Abandoned redo branches and closed projects are not strongly retained by the index.
- A versioned dictionary stores each shared record once. Existing histories remain readable, and compact records travel over IPC and use the existing atomic temporary-file/fsync/rename save path. Invalid references are rejected. Legacy dictionaries migrate on load through the atomic writer; later saves keep the compact format. A failed migration preserves the original file and in-memory undo.
- Main keeps histories compact. Window restoration and extension startup read specific settings rather than cloning the entire store. Legacy snapshot/read APIs preserve their logical schema.
- Renderer bootstrap defers project bodies, histories, journals, agent conversations, and attachments until requested. Cross-window invalidation refreshes individual keys. Cached history stays compact; public reads remain isolated copies.
- Active history shares immutable records while undo/redo still clones values into the mutable document. Memory pressure drops the optional lookup index instead of deleting durable undo actions. Existing entry and serialized-byte limits remain unchanged.
- Audio decoding uses its exclusively owned Blob ArrayBuffer directly, eliminating the second complete encoded-input copy.
- Final video disposal cancels seek-frame caches and unloads both source and preview decoders. Media retained by undoable replacement stays usable until its final owner releases it.
- Project close releases obsolete image/video GPU textures and model buffers immediately.
- Native video export views the owned ImageData bytes instead of copying a second entire RGBA frame. It removes one 31.64 MiB allocation per 3840×2160 frame, or 126.56 MiB per 7680×4320 frame. Byte offsets, chunking, backpressure, and output bytes are preserved.
- Preview generation stops reading generated media when its project closes and releases its temporary native job.

## Every-layer audit

The coverage column identifies checks for each layer; it does not itself assert that all UI tests have passed.

| Layer | Finding and disposition | Regression coverage |
| --- | --- | --- |
| Main heap/persistence | Measured undo duplication removed; targeted reads avoid full-store clones. Other main store values remain eager. | Atomic writes, migration failure preserving the original, restarts, compatibility, IPC and production-store profile |
| Renderer heap/IPC | Inactive project data no longer expands in every window; opened values remain cached. | Lazy reads, clone isolation, local writes, sibling invalidation, multi-window persistence |
| Undo/transactions | Equal historical records share ownership; live documents remain mutable; pressure cannot truncate actions. | Frozen records, all replay states, branch edits, rollback, fresh process and Electron restart with pixels |
| GPU textures/meshes | Closed-project uploads are released. Active LRU budgets stay intact. | Exact-once disposal, compositor/media/model tests, rendering and playback |
| GPU framebuffers | RGBA16F/depth precision and protection of busy/presented targets remain unchanged. Lowering them would change quality or reuse latency. | Pool accounting, compositing, export/pixel tests |
| CPU raster/SVG/text | CPU canvases and uploaded GPU pixels serve different reuse paths. Existing byte/entry bounds and canvas-zeroing eviction remain. | Raster pressure/reuse, typography, zoom, geometry tests |
| Video decoders/seek frames | Unloaded sources previously retained decoder state. Final disposal now clears it; different clip times still get independent decoding. | Disposal reproduction, replacement/undo, copied footage, scrubbing and frame preparation |
| Audio/PCM/waveforms | Redundant encoded input copy removed; active/pinned voices, PCM budgets and waveform fidelity remain. | Ownership, decode safety, import/mixing/edge cases and exports |
| Cached preview frames | ImageBitmaps remain bounded at 256 MiB and close on invalidation. Stopped user-requested previews remain reusable. | Preview invalidation/audio ownership and cached playback |
| Media/import/project files | Content-addressed blobs avoid duplicate imports; bounded fingerprint sampling and import concurrency remain. File uploads use acknowledged 1 MiB disk chunks. The real Electron >1 GiB project round trip passed with 73.6 MiB peak main-process buffer growth in the final full run (71.6 MiB in the earlier run). | Aliases, relinking, cancellation, streaming project over 1 GiB and reload |
| Export/temporary buffers | Native frame duplicate removed; acknowledged writes remain at most 4 MiB. Complete WebM/web-bundle outputs still materialize in memory. | Exact bytes/offsets, encoder errors/cancellation, native delivery and formats |
| Agent conversations/attachments | Inactive payloads leave renderer bootstrap. Active thread payload maps clear on project load. | Thread/attachment units, project persistence and UI flows |
| Extensions/shaders/workers | Existing build/program caches and process cleanup remain. No idle shutdown that delays the next action. | Compilation, recovery, isolation, shaders and agent runners |
| UI/DOM/listeners | No blanket unmounting that would lose draft, focus, or layout state. Existing lifecycle behavior remains. | Component/panel units, workspace, keyboard, menu and layout UI tests |

## Regression status

- Production build passes. TypeScript/Svelte reports zero errors and warnings; the architectural boundary check passes for all 10 extensions.
- The final full unit suite with the optional 5,000-layer fixture passes: **287 files, 2,385 tests, zero skipped**.
- Six standalone production-history tests pass, including a fresh process reopening disk history and traversing every state.
- Real Electron close/relaunch passes: compact dictionary, cursor, pending redo, all document actions, pressure preservation and pixel hashes match.
- **All 337 standard functional UI cases have passed across the complete run and focused reruns.** The final full snapshot run contained 349 cases: 324 passed, 13 failed, 12 optional cases skipped. Twelve failures were in export/media tests after concurrent export-destination changes, and one was a stale native capture of the color-reference fixture. Updated fixtures use delivered PNGs and native save paths, retaining pixel, alpha, frame-count and lifecycle checks. All failed cases passed their focused reruns. Thread switching, decoded WebM frames and strict spatial colors also passed **five repetitions each (15/15)**.
- The WebM frame test produced an undecodable file once during the full run, then passed the media rerun and all five repetitions. Exported media is now attached to the test result for diagnosis if it recurs; its original isolated failure is not hidden by retries.
- The large-project performance suite was also exercised with a copied 884-layer document. **Five existing performance ceilings still fail in both baseline and integrated builds.** Random-scrub maximum render time was 360 ms baseline and 148 ms integrated (100 ms ceiling); p95 was 22 ms and 13.2 ms. High-zoom wheel rendering submitted 61 frames in both (ceiling 15). Three region-pan cases submitted 181 baseline versus 181–182 integrated (ceiling 25); strict pixel-equivalence checks passed. The project has effects that exclude the existing cropped-frame reuse path. Thresholds were not relaxed.
- Three optional workflows still require unavailable inputs: live Codex execution, the private 4K video source, and the saved project export fixture. Nine optional project performance cases were enabled separately as described above.

**Clearance is scoped to the validated memory changes and available functional workflows. This is not an unconditional all-green claim for the entire application:** the five baseline performance failures, the one non-reproduced WebM failure, and unavailable external fixtures remain explicit limitations. No feature, precision, preview-resolution or undo-retention reduction was used to make the memory figures or tests pass.

UI tests use disposable profiles and a deterministic account-status fixture unless explicitly launched in live mode. Connection tests override its status; agent execution tests retain their explicit mocks. Concurrent export work changed the working tree during validation, so the final checks use an isolated source snapshot. All 744 desktop source and test files match the validated snapshot at completion (SHA-256 manifest fingerprint `9f3467f8b431cc02190fced85e9a04a92e7a1af9e85b0765f8ec9f4fa1a68e07`). Fixture corrections use native file delivery rather than intercepting the obsolete download path.

The installed app and live projects have not been restarted or overwritten. Existing unrelated workspace changes are preserved.

## Additional defects found during clearance

- Compiled panel forks omitted the public `propertyShortcuts` export. The runtime export list now matches the built-in namespace, with a parity test and real panel reload test.
- Collapsed panels kept an expanded minimum height. Collapse now uses the header minimum and restores the normal minimum on expansion; the actual splitter/collapse/reopen workflow passes.
- Inline attachment caret stops consumed an extra Backspace after typing. Stops are removed once adjacent real text exists while live caret ranges remain valid.
- The inline composer owns undo history to retain attachment bytes. Native text menus now read its undo/redo availability and dispatch those actions to its originating frame; other fields retain Chromium editing behavior.
- Reopening the thread picker during its close animation left a listener that dismissed the new popup. Closing now removes that listener, and panel unmount releases global listeners and timers; deterministic reproduction and lifecycle tests cover both.
- Spatial activation ignored a caller-supplied capture and displayed the older idle snapshot. It now uses the supplied frame and releases the superseded cached bitmap; the existing strict color comparison passes.

## Reproduce

From `apps/desktop`:

```sh
node --expose-gc scripts/profile-history-memory.mjs /path/to/projectHistory.ID.json
node --expose-gc scripts/profile-store-memory.mjs /path/to/store
node --test scripts/history-memory-prototype.test.mjs
node scripts/profile-history-edit-latency.mjs /path/to/history.ts /path/to/copied-project.json
PM_PERF_PROGRAM=1 bun run test
bun run typecheck
bun run lint:boundaries
bun run test:e2e --global-timeout=3600000
```

The historical `history-memory-prototype.mjs` filename now holds a profiling adapter around production code, not a separate codec. Regression tests use the real undo engine without re-sharing its records after import.

## Limits

Persistence covers document patch history. Existing session-only callback entries (including media replacement and agent interface changes) and selection history are not serialized by the application; this pass preserves that behavior rather than claiming those actions newly survive relaunch. Persisting those callback operations would require a separate durable operation representation and media/extension restoration design.


Earlier installed-app `vmmap` physical footprints were 1.1G main, 1.4G renderer and 224.8M GPU. Historical peaks were 2.1G, 2.4G and 887.7M. Readings were separate, include allocator/OS effects, and cannot be compared directly with retained JS heap or summed as simultaneous peaks. PartitionAlloc inspection reported incomplete malloc-zone introspection.

No whole-app physical-footprint percentage is claimed from the isolated data measurements. Decoded media, live GPU targets, offline audio mixes, prepared video frames, complete WebM output and web bundles can still dominate large active projects. Further reductions need workload measurements and, in some cases, streaming changes; shrinking working caches alone could trade memory for latency.
