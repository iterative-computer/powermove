# Design review: `feat/na-cloud-promedia` (render bar, Background Tasks)

Reviewer: design lead · 2026-11-01 · Commits reviewed: `81901c7d2` … `3ce82d1af`
Scope: `promedia/src/cache.rs` (`BarState`, `render_bar`), `promedia/src/tasks.rs`
(`BackgroundTasks`, window events) and the user-facing parts of
`docs/native/pro-media.md`. Read against [language.md](../language.md) §9
(progress), §11 (timeline), §16.

**Result: model accepted; data additions required before UI binds.**

## Render bar

The 3 pt strip directly under the timeline ruler, full timeline width,
segments from `render_bar` (adjacent equal states collapse — good).

| `BarState` | Appearance | Why |
|---|---|---|
| `Missing` | nothing drawn (ruler shows through) | Absence is quiet; unrendered is the normal state |
| `Rendering` | `muted` fill with a moving 45° stripe; static 50 % `muted` under Reduce Motion | Activity without colour |
| `Rendered` | `success` at 70 % | Rendered = good to play |
| `Kept` (Keep Rendered) | `success` solid **plus** a 1 pt `text` top keyline | Distinct from Rendered without relying on hue |

Must-fix:

1. **Range in display terms.** Segments are engine ticks; the UI needs them
   mapped through the composition's time base. Provide a helper
   `render_bar_frames(segments, rate) -> Vec<(first_frame, end_frame, state)>`
   or document that the timeline painter owns this mapping; do not let views
   convert ticks ad hoc.
2. **Invalidated is a state.** After an edit, cached frames become invalid
   before they are trimmed. Add `BarState::Stale` (drawn as `Missing` but
   reported to VoiceOver as "needs rendering"), or guarantee `state()` already
   reports `Missing` the moment the graph digest changes.

Should-fix:

3. Accessibility value for the bar: "Rendered 0:00 to 0:12, rendering 0:12 to
   0:14". Expose a summary string from the model so every host says the same.

## Background Tasks

Presentation (owner of the window/popover: shell lane):

- A **Background Tasks** toolbar item appears only while a task is queued or
  running: circular determinate progress (aggregate), `symbol_toolbar` size,
  tooltip `2 tasks: Rendering, Creating Proxy Media`. Click opens a popover;
  `Window ▸ Background Tasks` (⌘9, unbound in the W1 keymap)
  opens the same content as a utility window.
- Content: grouped by project (window), one row per task: kind symbol,
  title, detail line (`muted`: `128 of 300 frames · about 1 min`), progress
  bar, trailing Pause/Resume and Cancel borderless buttons (`pause.fill`,
  `xmark.circle.fill`). Failed tasks show `danger` text and Retry. Completed
  tasks fade to a `success` checkmark and leave after 5 s (unless the window
  is open). Solid `floating` content area inside the glass popover.

Must-fix (model data):

4. **Task needs a title and kind label.** `Task` has only `kind`, `project`
   and counters. Add `title: String` (the media or composition name) and use
   these kind labels (Title Case, used in menus and tooltips):

   | `TaskKind` | Label | Symbol |
   |---|---|---|
   | Proxy | Creating Proxy Media | `film.stack` |
   | Optimized | Creating Optimized Media | `film.stack.fill` |
   | Render | Rendering | `square.stack.3d.down.right` |
   | Import | Importing | `square.and.arrow.down` |
   | Export | Exporting | `square.and.arrow.up` |
   | Purge | Freeing Disk Space | `trash` |

5. **Progress units.** `completed`/`total` have no unit. Add
   `unit: Frames | Bytes | Items` so the detail line can say "128 of 300
   frames" or "1.2 of 4.1 GB", and treat `total == 0` as indeterminate
   (spinner, not a 0 % bar).
6. **Transitional states have words.** `PauseRequested` → "Pausing…",
   `CancelRequested` → "Cancelling…" with buttons disabled; `Paused` shows
   Resume. Never show the enum names.
7. **Errors are typed.** `error: Option<String>` must not reach the UI raw.
   Add an error code (`SourceOffline`, `DiskFull`, `CodecUnsupported`,
   `Cancelled`, `Unknown`) and keep the string for the log. UI copy per code,
   e.g. DiskFull → "Not enough disk space. Free some space and try again."

Should-fix:

8. **Time remaining** needs a rate: record `started_at` and progress samples
   (or a host-provided `eta_seconds: Option<u32>`), shown only after 5 s of
   samples and rounded ("about 1 min").
9. **Retired tasks** (`retire`) should keep the last terminal state for the
   window's session so a just-finished export can show "Exported · Show in
   Finder" for its completion row.

Nice-to-have:

10. Priority is invisible to users; do not expose it. Reordering in the list
    (drag) may set it later.
11. The old app's `Render queue` modal (v1.2.0 `render-queue.ts`) is replaced by
    this window; its `Retry` / `Remove` actions map to Retry / Cancel here.
