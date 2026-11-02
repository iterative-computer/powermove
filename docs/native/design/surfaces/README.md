# Surface specs — dialogs, alerts, recovery and secondary windows

Status: **build specs** for the P0 and P1 rows of
[surfaces.md](../../surfaces.md) that carried `DESIGN-TODO(opus)`. Owner:
design lead, 2026-11-01. Same contract as [panels/README.md](../panels/README.md):
a lane builds from the spec **without making design decisions**; if the spec
is silent it writes `DESIGN-TODO(opus): <question>` and stops. Token short
forms (`c.`, `m.`, `t.`, `mo.`) and copy conventions (`…`, `“ ”`, `{name}`)
are those of panels/README §1.

The cheapest correct design wins: **a standard AppKit alert, sheet, panel or
menu wherever HIG has one.** Custom drawing is only allowed where a spec
below says so.

| File | Group | Rows |
|---|---|---|
| [documents.md](documents.md) | Save, close, quit, open, conflicts, pickers, windows | S002 S003 S004 S014 S016 S020 S055 |
| [recovery.md](recovery.md) | Crash recovery, extension failure, notices, status | S001 S013 S053 S054 |
| [library.md](library.md) | Project Library, new/rename/duplicate, Trash | S005 S017 S018 S019 |
| [media.md](media.md) | Relink, cloud download, import failures, sequences, Figma, source errors | S006 S007 S024 S038 S039 S040 |
| [authority.md](authority.md) | Agent access, extension values, removal | S010 S011 S012 |
| [dialogs.md](dialogs.md) | Composition, pre-compose, keyframe, expression, export-preset dialogs | S021 S022 S029 S036 S043 |
| [editing.md](editing.md) | Path/text tools, text animators, colour picker parity, text services | S025 S034 S035 S051 |

---

## 1. Alert grammar (`NSAlert`)

Used for confirmations and errors only (language §9).

| Rule | Spec |
|---|---|
| Presentation | `beginSheetModal(for:)` on the window that owns the document or action. App-modal `runModal()` only when no window exists (e.g. a file opened from Finder fails before a window is created). |
| Icon | The app icon (system default). `alertStyle = .critical` (caution badge) only where a spec says *critical*: irreversible deletion and standing authority. Never a custom image. |
| Message | `messageText`, sentence case. A question ends with `?`, a statement with `.`. Names of documents and items in curly quotes. |
| Informative text | `informativeText`, at most two sentences, says what happens next or how to fix it. Omit rather than restate the message. |
| Buttons | Added in the order listed in each spec: first = default (rightmost, accent, Return). `Cancel` responds to Esc. `Don’t Save` gets ⌘D (system convention). A destructive button sets `hasDestructiveAction = true` (system red text) and is **never** the default. Labels are Title Case verbs; `OK` only as the single button of a pure acknowledgement. |
| Extras | `showsSuppressionButton` or an accessory view only where a spec lists one. |
| Stacking | One alert per window at a time; further ones queue in arrival order. |
| VoiceOver | System alert semantics; nothing added. |

## 2. Sheet grammar

Language §9 *Sheets and alerts*, plus:

| Element | Spec |
|---|---|
| Frame | System sheet (`beginSheet`), width `m.sheet_width` unless stated; system placement. |
| Content | Inset `m.sheet_inset`; title `t.heading` Title Case; optional one `t.reading` `c.secondary` sentence; rows in inspector grammar: label `t.body` `c.secondary`, value column `m.sheet_value_column`, row `m.row`. |
| Controls | AppKit only: `NSTextField`, `NSPopUpButton`, `NSButton` checkbox/radio, `NSSegmentedControl`, `NSStepper`. |
| Footer | Trailing: `Cancel`, then the accent default named for the action. A tertiary action (e.g. `Individual Images`) sits at the **leading** edge. |
| Validation | Live. An invalid field shows the panels/README *Error* state on the line under the field and the default button is disabled; no alert on submit. |
| Keyboard | First field focused with contents selected; Return = default; Esc = Cancel; Tab order = reading order. |
| Stale target | If the object the sheet edits disappears (project closed, composition deleted elsewhere), the sheet closes itself and the window shows a banner (§3) `{Object} was changed, so the dialog closed.` — never an apply that errors. |

## 3. Banner and notice

Two non-modal surfaces, nothing else (no toasts):

1. **Viewer banner** — [viewer-chrome.md §Banners](../panels/viewer-chrome.md):
   glass capsule bottom-centre of the viewer, one at a time, auto-dismiss
   after 4 s unless it has an action. Used for facts about the current
   document. This spec adds an optional trailing close button
   (`xmark` borderless, `m.symbol_inline`, tooltip `Dismiss`) on banners with
   an action, which otherwise persist until acted on or the document closes.
2. **Inline notice** in windows without a viewer (Project Library, Settings):
   a row at the top of the content, `c.panel_secondary` fill,
   `m.control_radius`, symbol + `t.body` text + one borderless action.

Both post an `NSAccessibility` announcement (`.medium`; `.high` for errors).

## 4. Error copy

Error alerts take their message and informative text from the
`ServiceErrorCode` table in [reviews/na-cloud-services.md](../reviews/na-cloud-services.md#re-review--2026-11-01).
The last row's informative text is amended here to
`Try again. If it keeps happening, choose Window ▸ Error Log for details.`
(the shipped app has no Help menu entry for reporting).
