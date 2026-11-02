# Documents and windows — save, close, quit, open, conflicts

Rows S002 S003 S004 S014 S016 S020 S055. Grammar: [README](README.md) §1–4.
Model contract: `pm-kernel::session` (`ShellCall::{Save, Close, DeferClose,
DiscardClose, PrepareQuit, DurabilityBarrier}`, `ShellReply`,
`RecoveryDecision`), `pm-kernel::locks::{OpenDecision, CopyReason}`,
`pm-shell::session`. Precedence: v1.2.0 behaviour, HIG placement, language
look.

## S002 Unsaved close / quit

Ports `R/legacy/app.ts` `confirmCloseProject`/`prepareToClose`, `M/save.ts`
L482. Behaviour kept: closing first flushes edits, waits for an active import,
**joins** an active save, then re-checks dirtiness; only a dirty project asks.

**Close one project (tab or window)** — alert, sheet on the window:

| Part | Copy |
|---|---|
| Message | `Save changes to “{name}” before closing?` |
| Informative | `Your project file hasn’t been updated. Local recovery is kept separately.` |
| Buttons | `Save` (default) · `Cancel` (Esc) · `Don’t Save` (⌘D, leftmost by NSAlert order) |

`Save` on a never-saved project runs the Save panel (S020) as a sheet on the
same window; cancelling that panel cancels the close.

**Close a window with several dirty tabs** — one alert per dirty tab, in tab
order, each tab selected before its alert so the user sees what they decide.
`Cancel` on any stops the whole close; earlier decisions stand.

**Quit with dirty projects** (`ShellCall::PrepareQuit`):

- One dirty project: the single-project alert above on its window.
- Two or more: app-modal alert (system multi-document convention):

| Part | Copy |
|---|---|
| Message | `You have {n} projects with unsaved changes. Do you want to review these changes before quitting?` |
| Informative | `If you discard them, their project files aren’t updated. Local recovery is kept separately.` |
| Buttons | `Review Changes…` (default) · `Cancel` · `Discard Changes` (destructive) |

`Review Changes…` walks the single-project alert per window, then quits.

**Waiting for work** (save, import or `DurabilityBarrier` still running at
close/quit): `applicationShouldTerminate` returns `.terminateLater`; the
toolbar subtitle shows `Saving…` (existing shell string). If still waiting
after 1 s, a sheet on that window: title `Finishing Up…`, `t.reading` line
`Powermove is saving “{name}”.` (or `…is importing media into “{name}”.`),
system indeterminate bar, `Cancel` (cancels the *quit/close*, never the
save). No progress percentages.

**Save failed during close** — the close is abandoned and the window stays:

| Part | Copy |
|---|---|
| Message | `“{name}” couldn’t be saved.` |
| Informative | from the `ServiceErrorCode` table (`DiskFull`, `PermissionDenied`…) |
| Buttons | `Save As…` (default) · `Cancel` |

| State | Visual |
|---|---|
| Clean | nothing asks; window closes. |
| Dirty | alerts above; title subtitle `Edited`. |
| Saving | subtitle `Saving…`; after 1 s, the Finishing Up sheet. |
| Error | Save failed alert; window and project stay open, still `Edited`. |

Don't: put `Don’t Save` on Return; offer "Save All" without review; show a
spinner over the editor; close the window before the save result arrives.

## S003 Changed on disk, uncertain save, recovery copies

Model: `CopyReason::ChangedOnDisk`, `ShellReply::{Saved, Closed,
Compacted}{recovery_copy}`, `ShellReply::Changed(bool)`.

**File changed by another app while open**:

- Project not edited since the last save/open → the kernel reloads it in
  place (NSDocument behaviour). Viewer banner (info, 4 s):
  `“{name}” was updated from another app.` No alert.
- Project edited → nothing happens until the next save. Then alert:

| Part | Copy |
|---|---|
| Message | `“{name}” was changed by another app since you opened or saved it.` |
| Informative | `If you save, the other version is kept as a recovery copy.` |
| Buttons | `Save Anyway` (default) · `Cancel` · `Save As…` |

**Recovery copy produced** (any `recovery_copy: Some`) — the user's work is
safe, so this is a viewer banner with an action, not an alert:
`A recovery copy of “{name}” was kept.` action `Show in Finder`. In the
Project Library (no viewer) it is an inline notice with the same copy.

**Uncertain save** (kernel cannot prove durability, e.g. a network volume
dropped mid-write): the document stays `Edited`; banner (warning)
`Couldn’t confirm that “{name}” was saved.` action `Save Again`.

Don't: auto-merge; show a three-way diff; delete or hide either version; use
the word "conflict" in UI copy.

## S004 Open errors, locked and copy-only projects

Model: `OpenDecision::{Writable, AsCopy(CopyReason)}`,
`pm-services::entrypoints::FileOpenFailure` (keeps the path).

**Can't open at all** — app-modal alert if no window yet, else a sheet on the
frontmost window; copy from the `ServiceErrorCode` table; buttons
`Show in Finder` (only when the path exists) · `OK` (default).

**Already open in this app** — no message: bring the owning window/tab to
front (system behaviour; replaces the shipped toast
`That project is already open in another window.`).

**Opens as a copy** (`AsCopy`) — the project opens immediately; saves never
touch the original. Two signals, no alert:

1. Toolbar subtitle `Copy` (`Copy · Edited` when dirty), tooltip = the reason
   line below.
2. Viewer banner (info, with action so it persists until dismissed):
   reason line + action `Save As…`.

| `CopyReason` | Reason line |
|---|---|
| `Locked` | `“{name}” is open in Powermove on another Mac, so this is a copy.` |
| `UncertainLock` | `“{name}” may be open elsewhere, so this is a copy.` |
| `ReadOnly` | `“{name}” is read-only, so this is a copy.` |
| `CloudFolder` | `“{name}” is in a cloud folder that’s still syncing, so this is a copy.` |
| `OfflineFile` | `“{name}” isn’t downloaded, so this is a copy of the last local version.` |
| `ChangedOnDisk` | `“{name}” changed while it was opening, so this is a copy.` |
| `OlderElectron` | `“{name}” is open in an older Powermove, so this is a copy.` |
| `ForeignLibrary` | `“{name}” belongs to another Powermove library, so this is a copy.` |

The first ⌘S on a copy runs Save As (S020) with name `{name} copy`.

Don't: open read-only without saying so; offer "Steal Lock"; show a lock
owner's user name or host (privacy).

## S014 Project tabs — transfer and restore errors

Native tabs are system `NSWindow` tabbing (Shell review). Only these states
need design:

| Case | Behaviour |
|---|---|
| Tab dragged to another window while that project is saving | system drag proceeds; the move completes when the save finishes (no message). |
| Kernel refuses the move | tab returns to its strip (system snap-back); no alert. |
| Restore at launch: a project can't be reopened | its window/tab is not created. After launch, one alert: one project → `ServiceErrorCode` copy; several → message `Some projects couldn’t be reopened.` informative `“{a}”, “{b}” and {n} more may have been moved or deleted.` buttons `Show Project Library` · `OK` (default). |

## S016 Popped-out panel window

Ports `R/layout/popout.ts`: title `{Panel} — Powermove`, `Return to layout`,
closing returns the panel.

| Element | Spec |
|---|---|
| Window | Standard titled `NSWindow` (closable, miniaturizable, resizable; not a panel, not floating), title `{Panel title} — {project name}`; not part of the editor's tab group (`tabbingMode = .disallowed`). Frame autosaved per panel id. |
| Content | The panel's content on `c.panel` edge to edge (the window is the card: no `m.gap`, no `m.panel_radius`). The panel header row stays (`m.header`) without its title; its trailing buttons gain a final borderless `rectangle.inset.filled.and.arrow.left` button, tooltip `Return to Layout`. |
| Return | `Return to Layout`, the close button and `Window ▸ Return {Panel} to Layout` all put the panel back at its previous dock position (or its default dock if that is gone). |
| Project close | Closing the project closes its popped-out windows **and records them as popped out** so reopening restores them. Unsaved-changes prompts belong to the project window, never the popout. |
| Not allowed | Dock-only panels (Viewer, Tools): `Pop Out` is absent from their dock menu (not disabled). |
| Failure | Window can't be created → panel stays docked; no message (shipped toast removed). |

VoiceOver: window title as above; `Return to Layout` button label
`Return {Panel} to Layout`.

## S020 Open / Save / Import pickers

System `NSOpenPanel`/`NSSavePanel`, always a sheet on the window that asked
(app-modal only with no window). Nothing custom inside them.

| Picker | Title / prompt | Filter / options |
|---|---|---|
| Open Project | `Open` · button `Open` | `.pmv` UTType only; multiple selection on; opening a folder is not allowed. |
| Save / Save As | `Save As` · button `Save` | name field prefilled `{project name}`; extension `.pmv` enforced and hidden per system setting; `canCreateDirectories`. |
| Import Media | `Import` · button `Import` | all supported media UTTypes; files **and** folders; multiple selection. |
| Choose Folder (export sequence, relink folder) | `Choose` | directories only; `canCreateDirectories`. |

Overwrite confirmation is the system's (`NSSavePanel`), never ours. Errors
after the picker closes use the error alert (README §4) with
`Show in Finder` when the path exists. Unsupported files chosen in Import go
to S038's summary, never a picker-time alert.

## S055 Project opening and loading

Ports `R/overlays/ProjectGhost.svelte` (ghost after a delay, tab intent kept).

| Phase | Visual |
|---|---|
| 0–0.5 s | Window/tab exists with its final title; panels show their empty chrome; nothing else. |
| > 0.5 s | Viewer frame area: system indeterminate spinner (regular) centred, `t.minimum` `c.muted` line below: `Opening “{name}”…`. Timeline/Properties show their Loading state (panels/README §3). Toolbar actions that need a project are disabled. |
| Cancel | ⌘W or the tab's close cancels the open (no confirmation; nothing was edited). |
| Recovery needed | `ShellReply::RecoveryRequired` → [recovery.md §S001](recovery.md#s001-crash-and-out-of-memory) recovery alert on this window. |
| Fails | window/tab closes; S004 error alert on the window that remains (or app-modal). |

VoiceOver: announce `Opening {name}` once at 0.5 s and `{name} opened` on
completion. Reduce Motion: no change (nothing animates).

Don't: draw a ghost/skeleton of the previous project; block other windows;
show a percentage.
