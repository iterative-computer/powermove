# Background Tasks, renditions and storage

Row N005. Grammar: [README](README.md) §1–5. Sources: `pm-promedia::{tasks,cache,media,
storage}`, [pro-media.md](../../pro-media.md),
[reviews/na-cloud-promedia.md](../reviews/na-cloud-promedia.md). That review's window,
popover, kind labels and render bar stand. This file adds what it deferred and all of
storage. No new tokens.

## Background Tasks: remaining states

| State | Spec |
|---|---|
| Time remaining | detail line appends ` · about {n} min` or ` · about {n} s`, only after 5 s of samples; rounded; omitted while paused |
| Indeterminate | `total == 0`: small spinner, no bar, detail `Working…` |
| Queued | detail `Waiting…`, no bar; Cancel only |
| Pausing, Cancelling | `Pausing…`, `Cancelling…`; both buttons disabled |
| Paused | detail `Paused`; `play.fill` Resume |
| Completed | `checkmark.circle.fill` `c.success`; detail by kind: `Exported · Show in Finder` (a link), `Created`, `Rendered`, `Imported`, `Freed {size}`. Row leaves after 5 s in the popover, and stays for the session in the window |
| Cancelled | `Cancelled` `c.muted`; leaves as completed |
| Failed | Error text per code; `Retry` |
| Window empty | `EmptyState` `checkmark.circle`, `No Background Tasks`, `Exports, renders and proxy media appear here.` |
| Project closed | its group stays while tasks finish, titled `{project}` in `c.muted` |

Error copy by code:

| Code | Copy |
|---|---|
| `SourceOffline` | `The source is offline. Relink it and try again.` |
| `DiskFull` | `Not enough disk space. Free some space and try again.` |
| `CodecUnsupported` | `This format isn’t supported.` |
| `Cancelled` | not an error: Cancelled row |
| `Unknown` | `Something went wrong. Details are in Window ▸ Error Log.` |

**VoiceOver summary.** Toolbar item reads `Background Tasks, {n} running, {percent}
percent`. Each row reads `{label} {title}, {detail}`. Completion and failure post
`.medium`; progress doesn't.

Keyboard: ⌘9 opens the window (review). In the list ↑/↓ move, Space pauses or resumes,
Delete cancels. Popover Esc closes.

## Renditions

**Create.** Project panel context menu on media: `Create Proxy Media`, `Create Optimized
Media`, each queuing a task from online originals only. Disabled with tooltip `The
original is offline.` Selection of several creates one task per item.

**Status.** Media rows and Properties source row show one state:

| State | Badge | Detail |
|---|---|---|
| Original only | none | |
| Has proxy | `Proxy` | |
| Has optimized | `Optimized` | |
| Proxy only | `Proxy Only` `c.warning` with symbol | `The original isn’t available.` |
| Offline | existing Offline treatment | |

Badges follow media-browser's badge recipe (over the image, dark-scheme).

**Playback choice.** `View ▸ Playback Media`, radio: `Automatic`, `Original`, `Optimized`,
`Proxy`. Status bar adds `Proxy` when not playing originals. A missing rendition falls back
to the next available; the viewer banner says `Using {rendition}. {choice} isn’t
available.` once per media.

**Export.** Export never uses a proxy unless nothing else exists; then Export shows
migration.md's proxy-only warning line.

**Attach and relink**: `Relink…` (S006) matches content hashes. Refusal copy: `This file
doesn’t match the original.` Attaching a rendition checks time base, frame rate, start,
duration, alpha, audio and colour: `This file doesn’t match {media}. Check its frame rate
and length.`

## Keep Rendered

Command `Keep Rendered`, on the timeline render bar's context menu and `Composition ▸
Keep Rendered`, checked when on. Pinned frames show the review's bar treatment. Editing the
graph shows `Missing` at once, per the model; pins remain stored until freed. No alert.

## Settings ▸ Storage

New Settings section `Storage` (symbol `internaldrive`), after Transcription. Description:
`Where Powermove keeps media and renders on this Mac.`

**Usage** group: one row per class, `Originals`, `Proxies`, `Optimized Media`, `Renders`.
Each: `{used} of {quota}` tabular, `ProgressRow` determinate; over quota shows
`c.warning` symbol + `{n} over the limit. Nothing was deleted.` Row `Archive`: `{used}`, no
quota, `Show in Finder`.

**Locations** group: one row per class plus `Archive`; path `t.minimum` `c.muted`, middle
truncation with tooltip; button `Change…` → `NSOpenPanel` directories.

| Refusal | Error under the row |
|---|---|
| Overlaps another location | `This folder overlaps {class}. Choose a separate folder.` |
| Not empty, not Powermove's | `This folder has other files in it. Choose an empty folder.` |
| Symlink or traversal | `Choose a regular folder.` |
| No access | N007 |

Changing a location never moves existing files: detail reads `Existing files stay where
they are.`

**Limits** group: one pop-up per class, `Proxies`, `Optimized Media`, `Renders`: `No Limit`,
`10 GB`, `50 GB`, `100 GB`, `500 GB`, `Custom…`. Lowering a limit deletes nothing. New
work over a limit fails with `{class} is full. Raise its limit or free up space.`

**Render Cache** group: `Free Render Cache` button; detail `Keeps frames you marked Keep
Rendered.` When kept frames alone exceed the target: Error `Kept frames need more than this
limit. Turn off Keep Rendered on some compositions.`

**Free Up Space** — button `Free Up Space…` at the end, opening a sheet.

| Element | Spec |
|---|---|
| Title | `Free Up Space` |
| Line | `Powermove moves media that nothing uses into your archive folder. It’s never deleted.` |
| Rows | checkbox per class with `{n} items · {size}`; `Originals` off by default |
| Scanning | `Checking what’s still in use…`, spinner; rows wait |
| Note | when the archive shares a volume: `The archive is on the same disk, so this won’t free disk space. Choose another location for the archive to free it.` (`c.warning`) |
| Footer | `Cancel`, `Move to Archive` (default); disabled with nothing to move |
| Running | closes; a `Freeing Disk Space` task runs |
| Result | task completed `Freed {size}`; if some stayed: `{n} items are still in use and were kept.` |

An item is in use if any project, Undo or Redo step, Trash item, take, agent attachment or
rollback copy references it. A sole playable proxy is in use.

There's no way to delete the archive in the app, on purpose. `Show in Finder` is the path
for people who want to.

Full disk: archiving can't help when the archive shares the disk, so the note says so, and
the task's `DiskFull` copy applies.

## Don't

- Don't delete anything from Settings ▸ Storage.
- Don't move existing files when a location changes.
- Don't expose priority or queue order.
