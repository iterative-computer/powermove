# Recovery, extension failure, notices and status

Rows S001 S013 S053 S054. Grammar: [README](README.md) §1–4.

## S001 Crash and out-of-memory

Ports `M/renderer-recovery.ts` (shipped alert copy kept verbatim) and
`R/legacy/core/projects.ts` recovery on relaunch. Model:
`pm-kernel::session::{RecoveryResult, RecoveryDecision}`,
`pm-services::diagnostics::CrashRecord.recovered_document`.

**A window's engine/editor process fails while running** (the native analogue
of the renderer crash) — alert, sheet on that window:

| Part | Copy |
|---|---|
| Message | `Powermove needs to reload this window.` |
| Informative | `The editor ran out of memory.` *or* `The editor stopped unexpectedly.` + ` Reload restores the latest local recovery. The most recent edits may be missing.` |
| Buttons | `Reload` (default) · `Close Window`. No Esc action. |

**Next launch after an unexpected quit**, per affected project, in the order
the windows are restored:

| `RecoveryResult` | Surface | Copy |
|---|---|---|
| `Recovered { truncated: false }` | viewer banner (info) | `Powermove recovered “{name}” after an unexpected quit.` |
| `Recovered { truncated: true }` | viewer banner (warning, persists, close button) | `Powermove recovered “{name}” after an unexpected quit. The last few seconds of edits may be missing.` |
| `OfferCopy` / `RecoveryRequired` (mode Ask) | alert, sheet on the project's window | below |
| `Unrecoverable` | alert | below |

Ask alert:

| Part | Copy |
|---|---|
| Message | `Powermove quit unexpectedly while “{name}” had unsaved changes.` |
| Informative | `Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.` (+ ` The last few seconds of edits may be missing.` when `truncated`) |
| Buttons | `Recover` (default → `Recover`) · `Open as Copy` (Esc → `OpenAsCopy`, the choice that changes nothing) · `Discard Edits` (destructive → `Discard`) |

`Discard Edits` must move the recovered edits to **Recently Deleted** (N4:
nothing is destroyed). If the kernel cannot archive them, the button is
omitted, not disabled.

Unrecoverable alert: message `“{name}” couldn’t be recovered.`, informative
`The saved project is unchanged. Details are in Window ▸ Error Log.`,
buttons `Open Saved Version` (default, only if the file exists) · `OK`.

Don't: show crash reports or stack traces; ask "Send report?"; use the word
"crash"; auto-discard after a timeout.

## S013 Extension panel failure and recovery

Model: `pm-store-host::view::{ViewStatus, Failure, Action}`. The failure is
shown **inside the panel card**, which keeps its header and dock menu. The
content becomes the shared `EmptyState` (panels/README §3) with symbol
`exclamationmark.triangle` in `c.warning`:

| `ViewStatus` / `Failure` | Title | Line | Button |
|---|---|---|---|
| `Starting` > 0.5 s | — (Loading state) | `Starting {extension}…` | — |
| `Starting` + `ReadyTimeout` | `{Panel} Didn’t Start` | `{Extension} took too long to start.` | `Try Again` |
| `WaitingToRestart` after `Crash` | `{Panel} Stopped` | `It will restart in a moment.` | `Restart Now` |
| `WaitingToRestart` with no recorded `Failure` | as after `Crash` | | |
| `WaitingToRestart` after `Hang` | `{Panel} Stopped Responding` | `Powermove stopped it and will restart it in a moment.` | `Restart Now` |
| `Disabled` (restart policy exhausted) | `{Panel} Is Turned Off` | `{Extension} stopped working several times.` | `Turn On` |
| `Stopping`, `Sealing` | — (Loading state) | `Stopping {extension}…` | — |
| `StopFailed` | `{Panel} Couldn’t Be Stopped` | `Quit and reopen Powermove to stop it.` | — |
| `Failure::PlatformFailure` | `{Panel} Can’t Run` | `This Mac couldn’t start {extension}.` | `Try Again` |
| `Failure::UserStop` | `{Panel} Is Stopped` | — | `Restart` |

**Trusted (V8) realm not responding** — affects every panel of one
extension, so it is an alert (app-modal only if no window), raised after the
host watchdog's threshold:

| Part | Copy |
|---|---|
| Message | `“{extension}” isn’t responding.` |
| Informative | `You can wait, or stop it. A stopped extension stays off until you turn it on in Settings ▸ Extensions.` |
| Buttons | `Wait` (default) · `Stop Extension` (destructive) |

While the alert is up the affected panels show the `Stopped Responding` row
without a button. Settings ▸ Extensions shows the same state on the
extension's row with its `Turn On` switch.

VoiceOver: the EmptyState is one `AXGroup` labelled `{Title}. {Line}`; the
change posts a `.high` announcement.

Don't: show the extension's own error text in the panel (it goes to the Error
Log); blank the panel; add a red fill; close the panel.

## S053 Notices, Error Log and fix actions

- **Notices** are the two surfaces in README §3 only. Every notice has a
  typed source (surface + code); the copy is the owning spec's or the
  `ServiceErrorCode` table. A notice may carry **one** action named for the
  fix (`Retry`, `Show in Finder`, `Relink…`, `Turn Off`).
- **Error Log** — `Window ▸ Error Log` opens one standard titled window
  (`Error Log`, min 560 × 360, frame autosaved):
  - `NSTableView`, columns `Time` (system relative short time), `Source`
    (surface or extension display name), `Message` (the user-facing copy),
    `t.body`, `m.row`, newest first, alternating rows off.
  - Split below (`NSSplitView`): `Details` — selectable `MONO_FONT`
    `t.body` text with the diagnostic (developer) text for the selected row.
  - Footer: `Copy` (copies the selected rows with details) · `Clear Log`
    (asks nothing; log is diagnostic, not user work).
  - Empty: `EmptyState` `No Errors`, line `Problems Powermove notices appear here.`
- **Message catalog** = the `ServiceErrorCode` table + every Copy table in
  these surface specs. A developer string never appears outside `Details`.
- **Repair with the agent** (`R/errors/ErrorFix.svelte`) is S064 (P2) and out
  of scope here; a notice must not add a second action for it.

## S054 Status bar and slowdown details

Language §2: `metrics.status`, window ground, `t.minimum` `c.muted`, left
document facts, right playback/format facts. Contributions:

| Side | Item (in order) | Copy |
|---|---|---|
| Left | layer count · keyframe count | `{n} layers` · `{n} keyframes` (singular forms) |
| Left | slowdown indicator, only while issues exist | `gauge.with.dots.needle.33percent` `m.symbol_inline` `c.warning` + `Editor slowed down` |
| Right | playback state · size · rate | `Paused`/`Playing` · `1920×1080` · `30 fps` |
| Right | workspace pop-up | per Shell review (`Default ⌄`) |

Save state is **not** in the status bar (the toolbar subtitle owns
`Edited`/`Saving…`). Extension status items append at the trailing end of the
left group, `t.minimum` text only, max 24 characters, truncated with tooltip.

**Slowdown details** — clicking the indicator opens an `NSPopover` (width
`m.popover_width × 1.5`, arrow to the indicator), replacing the shipped modal:

| Element | Spec |
|---|---|
| Title | `t.title` `What’s Slowing the Editor Down` |
| Rows | per issue, `m.row` min, two lines: name `t.body` `c.text`; fact `t.minimum` `c.secondary` `{ms} ms on the editor thread` / `…compiling` / `…on the GPU` + `. A 60 fps frame has 16.7 ms available.`; trailing small push button: `Select Layer` (layer issue) or `Turn Off` (extension issue) |
| Empty | popover closes itself when the last issue clears |

`Select Layer` selects it and the popover stays open (shipped toast removed).
`Turn Off` failure → inline Error state in that row:
`Couldn’t turn off {extension}. Try again in Settings ▸ Extensions.`

VoiceOver: status bar is an `AXGroup` `Status`; the indicator is a button
`Editor slowed down, {n} issues`.
