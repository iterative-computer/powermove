# Surface specs — dialogs, alerts, recovery and secondary windows

Status: **build specs** for the rows of [surfaces.md](../../surfaces.md) that
carried `DESIGN-TODO(opus)`: P0 and P1 (first table), P2, P3 and the native-only
rows (second table). Owner:
design lead, 2026-11-01. Same contract as [panels/README.md](../panels/README.md):
a lane builds from the spec **without making design decisions**; if the spec
is silent it writes `DESIGN-TODO(opus): <question>` and stops. Token short
forms (`c.`, `m.`, `t.`, `mo.`) and copy conventions (`…`, `“ ”`, `{name}`)
are those of panels/README §1.

| File | Family | Rows |
|---|---|---|
| [agent-auth.md](agent-auth.md) | Agent surfaces, providers, runtime, sign-in, account | S057 S059 S060 S062–S066 S070–S072 S083–S086 |
| [store.md](store.md) | Store window, Library, extension management, updates, publishing, sandbox check | S073 S075–S082 |
| [captions.md](captions.md) | Captions panel, inspector, timeline, generation, speech models, import/export | S087–S093 |

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
| Buttons | Added in the order listed in each spec: first = default (rightmost, accent, Return). Where `Cancel` is the default (destructive or replacing alerts) it is listed — and added — first; the action sits to its left. `Cancel` responds to Esc. `Don’t Save` gets ⌘D (system convention). A destructive button sets `hasDestructiveAction = true` (system red text) and is **never** the default. Labels are Title Case verbs; `OK` only as the single button of a pure acknowledgement. |
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
| Stale target | If the object the sheet edits disappears (project closed, composition deleted elsewhere), the sheet closes itself and the window shows a banner (§3) `{Object} was changed, so the dialog closed.` — never an apply that errors. `{Object}` is the owner whose change closed it, with the article: `The project`, `The composition`, `The layer`, `The media` (import/relink/sequence sheets), `The preset`. Keyframe dialogs use `The layer`. |

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

Symbol (both surfaces), `m.symbol_inline`, unless the owning spec names one:
info `info.circle` `c.secondary` · warning `exclamationmark.triangle.fill`
`c.warning` · error `xmark.octagon.fill` `c.danger` (over the image: the
dark-scheme values). Use the kit `Banner` (`placement: viewer | inline`).

Both post an `NSAccessibility` announcement (`.medium`; `.high` for errors).

## 4. Error copy

Error alerts take their message and informative text from the
`ServiceErrorCode` table in [reviews/na-cloud-services.md](../reviews/na-cloud-services.md#re-review--2026-11-01).
The last row's informative text is amended here to
`Try again. If it keeps happening, choose Window ▸ Error Log for details.`
(the shipped app has no Help menu entry for reporting).

## 5. Baseline for the P2, P3 and native-only families

The family files state only what differs from this baseline. A state a family
does not mention is the panels/README §3 default, not an open question.

| Topic | Rule |
|---|---|
| Light and dark | Tokens only, so both schemes come from the same spec. Anything drawn over footage or artwork uses dark-scheme tokens in light mode too (language §4). |
| Increase Contrast | `m.border` 2; panels and cards draw their edge; selected rows gain an `m.border` `c.separator` outline; data marks (meters, curves, scope graticules) gain a `c.separator` keyline. |
| Reduce Transparency | Glass frames turn solid through `Tokens::material`. No panel content changes. |
| Reduce Motion | Springs and scale become the `mo.reduced` cross-fade. Live data (meters, scopes, progress) keeps moving because it is content. |
| Focus | Full Keyboard Access reaches every control in reading order. AppKit controls use the system ring; GPUI controls draw `c.focus`. Esc cancels the innermost transient. |
| Loading | Nothing for 0.5 s, then the panels/README Loading state. |
| Disabled | panels/README Disabled state. A control unavailable for a reason shows that reason as its tooltip. |
| VoiceOver | panels/README §7: visible text is the label, values read with units, status lines are polite live regions, errors post `.high`. |
| Window-level panels | A dockable panel takes the panels/README §2 card, header and dock menu. Its id and default dock are the v1.2.0 ones. |
| Errors | README §4 error copy. Developer text goes to the Error Log (S053). |


## 6. Owner questions raised by these specs

The specs below hold a default so a lane is never blocked, except where a row
says *do not build*. Reverse a default here and in the owning family file.

| # | Question | Default in the spec | Owning row |
|---|---|---|---|
| 1 | The region-selection overlay exists in v1.2.0 code but nothing opens it. Ship it, or retire the row? | Specified, **do not build** until decided | S066 |
| 2 | Native archives what v1.2.0 deleted (threads, and later takes and presets). Should people be able to restore those archives from the UI? | Archive is kept, copy says so, no restore UI | S060, S095, S103 |
| 3 | v1.2.0 silently adds Powermove to the Claude desktop app's settings at launch. Keep that, or ask first? | Keep automatic registration, and make it visible and reversible in Settings | S070 |
