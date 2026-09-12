# Error recovery and presentation

Verified September 12, 2026.

## Fixed

1. **Saved session reports an active writer.** The CLI runner can recover once with a fresh provider session when resume fails before any work starts. The project workspace and visible conversation remain available.
2. **Two requests write to the same conversation.** Autonomous runs now reserve their conversation before asynchronous setup. A replacement waits for a cancelled writer to exit before resuming; another concurrent request receives a useful busy response.
3. **Automatic recovery replays started work.** Session recovery now requires that no turn or work item has started, including when a later failure mentions a missing thread.
4. **Timeouts appear to be user cancellations.** Timeouts return a failure with guidance. Autonomous partial work is preserved. Editor transport timeouts release their active turn while interrupting it.
5. **Editor startup failures reject outside the result contract.** Discovery, startup and input preparation failures now return normal error results. An unresponsive initialization is terminated before a later restart.
6. **Stop is ignored during editor startup.** Requests are registered before startup, and cancellation is checked before starting work.
7. **An old process exit breaks a replacement connection.** Process events and output are accepted only from the current process.
8. **Dialogs close before asynchronous validation finishes.** Dialog actions wait for completion, prevent duplicate submission, retain inputs after failure, and allow another attempt. Both synchronous exceptions and rejected promises display inside the dialog.
9. **Notifications erase unread failures.** Distinct errors persist until dismissed, repeated errors are deduplicated, and routine status messages do not remove failures.

10. **Sending a message rewrites a large archive of saved Takes.** The inspected profile had a 213.8 MiB archive and a 1.04 MiB current project. Both autonomous and editor agent checkpoints now capture only the current project in a transient, non-enumerable run snapshot. Existing saved Takes remain untouched, and the agent Undo fallback remains available. In a read-only benchmark of the stored data, checkpoint preparation dropped from 1,147.8 ms to 8.0 ms, excluding IPC and disk writes.

A provider's “active writer” diagnostic is distinct from a known active local run. Only the latter says “Conversation is busy”; a provider session failure says “Agent session couldn’t reopen” and gives reconnection guidance. The original live Electron main process predates these main-process fixes, even though the renderer receives development updates.

## Display

Agent failures, app notifications and dialog failures share an error card: short title, readable guidance, expandable technical details and a Copy details button. Known cases include session conflicts, sign-in, connection interruptions, rate limits, timeouts, disk space, file permissions and invalid agent results. Existing error history uses the same presentation. The agent conversation retains up to 4,000 diagnostic characters instead of cutting it to 300.

Errors are announced on arrival, diagnostic text remains selectable, and controls are keyboard accessible. Notification stacks scroll within a bounded part of the viewport.

## Verification

- Reproduced the runner, startup, cancellation, dialog and notification failures before implementing fixes.
- Final focused regression run: 264 tests passed across 25 files covering Codex, error presentation, overlays, agent turns, checkpoints and the spatial assistant.
- Three Electron end-to-end tests passed: agent/error notification interactions at 240, 320 and 480 pixel panel widths; asynchronous dialog failure/retry with input preservation; and sending a message without accessing the Takes archive. Screenshots were visually inspected. Disposable test profiles are explicitly flushed and exited after these UI checks, independently of the native close-confirmation workflow.
- The existing Electron save reliability check passed: cancellation does not report success, and a successful save writes a reopenable project.
- Electron production build and main-process TypeScript checks passed.
- The broad suite remains non-green outside these fixes: selection-history and keymap expectations, a font inspector fixture, extension compilation, and performance tests failed. The renderer typecheck reports 15 errors in seven other files and no warnings. Some older Electron tests also assume a document is already open on a fresh profile and time out on the Projects screen.

Runner recovery was verified with controlled processes and protocol fixtures. The message-send regression uses a stubbed provider result and checks that the Takes archive is neither read nor written. These changes do not guarantee recovery from every provider or filesystem failure. Relaunch Powermove after active work finishes to load main-process changes.
