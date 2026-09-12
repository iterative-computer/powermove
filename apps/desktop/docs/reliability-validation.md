# Editor reliability validation — September 12, 2026

The changes address save stalls, random timeline jumps, shader compilation, failing extensions, agent connections, and workspace restoration. Measurements below were collected on the development Mac with Electron 44; they are observations, not guarantees for every GPU or project.

## Measured results

| Scenario | Result |
| --- | --- |
| Save the complete 635-layer project, 11 embedded media assets, 100,837,357 bytes | 2.02 s, previously 7.17 s before fixing whole-buffer IPC copies |
| Input timer during that complete save | 24.4 ms p95; 61 ms maximum gap |
| 80 random timeline jumps after opening the complete saved file with its custom Progressive Image Pixels extension | Render CPU time 3.2 ms p95; 25.3 ms maximum; no recorded main-thread tasks over 50 ms |
| Input-to-next-frame samples during those jumps | 13.5 ms p95; 81.8 ms maximum |
| Separate 36 MiB undo-history save | 188 ms; maximum input timer gap 31.3 ms; saved backward patch verified byte-for-byte in length |

An earlier custom-shader profile exposed an approximately 400 ms synchronous compilation frame. Preview compilation now uses the parallel shader compilation extension when available, polls completion without blocking, and limits submissions per frame. Exports wait for complete programs. Frame deduplication, hierarchy caches, and idle thumbnail generation remove additional work from ordinary interactions.

Saves serialize and encode in yielding slices. Renderer-to-native upload chunks own their buffers so Electron cannot clone the entire file for each chunk. Lightweight project and workspace metadata no longer copy or overwrite large undo envelopes. Pending history writes participate in the close/quit durability barrier.

## User-facing behavior

- The status bar identifies expensive shaders and extensions. Open the warning to inspect the contributor, select the affected layer, or turn off an extension.
- Failed extension activations no longer retry indefinitely on their own health notifications. Editing the extension or explicitly retrying permits another attempt. Built-in viewer and timeline forks use the public runtime helpers and compile through the extension pipeline.
- Lost GPU contexts restore the viewer without replacing the project.
- Each file keeps its panel placement, sizes, collapsed state, and closed panels across file switches, reopening, and application restart. Headless panels normalize inaccessible collapse states before persistence.
- Disconnected agent chats retain their transcript and composer. Provider changes ignore obsolete connection responses. Failed turns expose a retry tied to the original request; incomplete streams report failure instead of silently claiming completion.
- Settings → General → Accounts includes ChatGPT, Claude, and **API or local model**. The latter supports Ollama, LM Studio, OpenAI API, and other OpenAI-compatible chat-completion endpoints. Choose the connection type, enter the exact model name and any required API key, then select **Test and connect**. Enable image support only for a model that supports images. API billing is separate from subscription connections.

## Validation

- Full unit suite: 1,828 passed, one skipped; 237 test files passed. Older command tests now establish their undo baseline after fixture selection, and keymap/inspector fixtures match the current APIs.
- Focused suite: 297 passing unit and integration checks covering storage, native IPC, serialization, history, graphics, extension lifecycle, agent state, and workspace layout.
- Desktop suite: 12 passing scenarios covering streaming, ChatGPT/Claude bridge flows, local-provider configuration and recovery, complete-file scrubbing, large saves, GPU reset, performance warnings, and workspace persistence.
- Workspace quit/reopen scenario additionally passed three consecutive runs.
- Both group-transform desktop scenarios passed, including property animation, canvas drag, keyboard nudging, rendered output, and undo.
- Full saved-file performance was rerun separately from unit tests; the complete file was opened through native Open, including embedded media and history.
- TypeScript and Svelte checks: zero errors and zero warnings for desktop and website.
- Subscription tests use controlled CLI responses through the real Electron bridge; local-provider tests use a real local HTTP server. They do not exercise paid production model accounts.

## Recovery note

During development, a renderer/native version mismatch in the history migration overwrote the live session's older undo envelope with metadata. The current composition and saved chats remained intact. Recovery restored 34 older undo entries and one reversible entry covering subsequent edits, for 35 entries total. Individual later undo steps could not be recovered. The saved file and local store were both verified to contain all 635 layers at revision 237, those 35 undo entries, and all 11 media assets in the file.

The migration now requires a capability handshake from both native storage and preload. Both renderer and native storage preserve an existing legacy history when receiving a metadata-only update. Separate history writes retain the legacy envelope as a recovery fallback. Regression checks cover these protections.

## Limits

The maximum input gaps above are still measurable; this is not a zero-latency guarantee. GPU timing and parallel compilation depend on driver support. A synchronous extension callback can consume its first frame before it can be measured and warned about; arbitrary extension JavaScript is not preempted. Slow provider responses and outages surface as recoverable errors, but cannot be eliminated by the editor.

Reproduce performance against a local saved file without modifying it:

```sh
cd apps/desktop
PM_PERF_FILE='/path/to/project.pmv' \
PM_PERF_EXTENSION='/path/to/progressive-image-pixels' \
bun run test:e2e e2e/editor-responsiveness.spec.ts
```

The profiling fixture copies the extension into an isolated application profile. Its save scenario writes a separate test project. JSON measurements and CPU profiles are attached under the Playwright results directory.
