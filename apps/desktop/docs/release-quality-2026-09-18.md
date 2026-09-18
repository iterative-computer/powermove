# Release performance and quality pass — 2026-09-18

## Changes

- Interactive video frames use a single source time across motion-blur samples; incomplete paused compositions are retried without presenting mixed frames.
- Repeated nested compositions own independent decoders. Playback prewarms loop entrances and releases inactive decoder instances.
- Completed seek pixels remain available while the next retimed frame decodes. Offline capture ignores stale seek-completion events.
- Motion-blur samples accumulate additively, preserving opacity and brightness.
- Graph move/scale planning indexes neighbours once instead of comparing every pair.
- Store reads serialize only the requested key; bootstrap avoids cloning all project histories.
- Growing animation tracks keep existing temporal accessors instead of redefining all of them.
- Export inherits Settings card/control geometry and spacing. Progress cannot be dismissed by clicking outside or pressing Escape; explicit cancellation remains visible until cleanup completes.

## Measurements

Local synthetic measurements; these are not end-to-end FPS promises.

| Operation | Before | After |
| --- | ---: | ---: |
| Move 10,000 graph keys | 361 ms | 1.02 ms |
| Scale 10,000 graph keys | 1,560 ms | 1.56 ms |
| Read one store key with 30 large histories | 211 ms | <0.01 ms |
| Serialize store bootstrap | 300 ms | 16 ms |

Graph timings are medians of seven runs. 4,000 randomized old/new move and scale results matched exactly. The 5,000-key edit/undo test reproduced a 22-second timeout before the accessor fix; the complete five-file focused suite then passed in 5.66 seconds.

## Validation

- Full desktop unit suite: 2,315 passed, one opt-in performance test skipped.
- Subsequent offline-seek and modal regression tests: 40 passed.
- Final focused Electron suite: 10 passed (video frame integrity, nested instances, scrubbing, decoded-frame readiness, export layout in both themes, progress dismissal/cancellation, export completion/restart, video export frames, agent frame capture).
- Agent-video capture passed three repeated runs after fixing the seek race. Standalone web export/editor frame parity also passed after that fix.
- H.264/AAC MP4 and VP8/VP9/Opus WebM delivery passed in the preceding focused run.
- Desktop typechecking and extension boundaries passed. Export light/dark screenshots visually reviewed.

## Remaining release coverage

The broad Electron run was stopped to honor the request to conserve usage and merge. It reached 78 passes, 19 failures, three skips and one interrupted test; 239 tests were not run. The agent-video and web-export frame failures from that run were fixed and reverified above. Other failures remain untriaged; some wait for obsolete HTML menus, settings tab roles, or removed UI elements. This is not a clean full-app release certification.

Website typechecking and production build stalled and were stopped; shared player bundles built successfully. Private 4K/production fixtures, signed/notarized distribution and live paid model integrations were not verified.

Broad-run failures (before the final fixes):
- e2e/ae-audit-implementation.spec.ts:9:5 › editable Pen, canvas text, graph and preview controls render coherently
- e2e/ae-audit-implementation.spec.ts:47:5 › multiple graph curves, velocity editing and a cached preview retain editable animation
- e2e/ae-audit-implementation.spec.ts:81:5 › a drawn mask, luma matte and live text selector change pixels and undo
- e2e/agent-canvas-capture.spec.ts:85:5 › autosave persists during panning and captures its thumbnail after navigation settles
- e2e/agent-computer-use.spec.ts:8:5 › agent bridge captures and draws on a real extension canvas with native input
- e2e/agent-connection-layout.spec.ts:4:5 › connection setup owns the empty panel and stays usable when resized
- e2e/agent-redesign.spec.ts:5:5 › studio agent keeps suggestions, steering, activity and narrow layouts usable
- e2e/agent-streaming.spec.ts:3:5 › streamed text stays inline and keeps existing nodes as chunks arrive
- e2e/agent-threads.spec.ts:116:5 › hidden renderer switches threads, keeps drafts, and restores history after relaunch
- e2e/agent-video-editing.spec.ts:4:5 › agent cuts imported footage, reviews decoded frames, and undoes the run
- e2e/chatgpt-connection.spec.ts:5:5 › Settings reads a ChatGPT subscription through the real main-process bridge
- e2e/chatgpt-connection.spec.ts:123:5 › Claude subscription status and structured runs cross the real hidden app boundary
- e2e/compatible-provider.spec.ts:4:5 › connects a local model, streams chat, recovers after a broken stream and remembers the connection
- e2e/design-language.spec.ts:3:5 › flat controls remain consistent across home, editor, settings, library, and export in both themes
- e2e/dock-layout.spec.ts:4:7 › @dock-layout Svelte DockLayout › keeps overfilled side panels usable between the titlebar and status bar
- e2e/editor-reliability.spec.ts:19:5 › inspector keyframes animate, linked scale edits undo together, and handles extend outside video
- e2e/export-code.spec.ts:9:5 › web export plays independently and matches editor frames
- e2e/extensions.spec.ts:165:7 › @extensions user extensions load through the kernel › settings explains an override and deletes it while restoring the built-in panel
- e2e/graph-editor-review.spec.ts:3:5 › graph editor exposes its state and keeps value and speed navigation usable
- e2e/graph-picking.spec.ts:109:5 › only selected keys and their curves appear, and clearing selection clears hit targets
