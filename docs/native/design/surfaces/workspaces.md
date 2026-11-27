# Workspaces, Panel Library and generated panels

Rows S099 S100 S101 S102. Grammar: [README](README.md) §1–5. Sources:
`R/panels/WorkspacesPanel.svelte`, `R/legacy/core/workspace.ts`,
`R/legacy/ui/library.ts`, `R/library/LibraryScreen.svelte`,
`R/panels/GeneratedPanel.svelte`. Workspace vocabulary is settled in
[reviews/na-shell2.md](../reviews/na-shell2.md) (*Workspace*, never *Layout*).
S012 owns the delete alert. No new tokens.

The word **Library** is taken: `File ▸ Project Library` is S017. v1.2.0's panel and
workspace Library is named **Panel Library** here.

## S099 Workspaces panel

**Decision: retain the panel.** The owner's call moves the switcher to `Window ▸
Workspace` and an optional toolbar pop-up, and that stands. The panel survives
because it is the only place with delete, JSON entry and the agent hint side by
side, and v1–3 projects may have it docked. It stays optional and is not in any
default workspace.

Dockable panel `workspaces`, title `Workspaces`, default extent 200 pt.

| Action | Panel | Menu equivalent |
|---|---|---|
| Activate | click row; current row has `checkmark` leading (AX `current`) | `Window ▸ Workspace ▸ {name}` |
| Save current | `Save Workspace…` | `Window ▸ Workspace ▸ Save Workspace…` |
| Edit JSON | `Edit Workspace JSON…`, tooltip `Edit workspace layout as JSON` | `Window ▸ Workspace ▸ Edit Workspace JSON…` |
| Delete | hover `trash`, AX `Delete {name}`, tooltip `Delete {name} Workspace`; not on built-ins | `Window ▸ Workspace ▸ Delete Workspace` |
| Reset | not in the panel | `Window ▸ Workspace ▸ Reset Workspace` |

Layout: `List` of workspaces (`m.row`), footer with the two push buttons, then hint
`t.minimum` `c.muted`: `Ask the Powermove agent to redesign or add an interface
section.`

Status (one line `t.minimum` `c.muted` above the hint, polite live region):
`Activated {name}`, `Deleted {name}`. v1.2.0's `Save workspace dialog opened` is
dropped: the sheet itself says so. `Built-in workspaces can’t be deleted` is a tooltip
on the disabled menu item.

Delete confirmation: S012 `Delete the workspace “{name}”?` stands.

Activating fires the viewer banner `Workspace · {name}` only when the switch came
from a shortcut; from this panel the row state is enough.

## S100 Save and JSON sheets

**Save Workspace** — sheet from the shell review, stands. Title `Save Workspace`, row
`Workspace Name`, buttons `Cancel`, `Save`.

**Save as New** — same sheet, title `Save Workspace as New`, used when editing a
built-in or when the name exists. A built-in can never be overwritten.

| Case | Spec |
|---|---|
| Empty name | `Save` disabled |
| Name exists, custom | Error under the field: `A workspace named “{name}” already exists.`; secondary leading button `Replace` |
| Saved | banner `Workspace saved` |

**Workspace Definition** — wide sheet (`m.sheet_width_wide`), title `Workspace
Definition`.

| Element | Spec |
|---|---|
| Editor | `NSTextView`, `MONO_FONT` `t.body`, `c.field`, `m.control_radius`, inset `m.text_padding`, height `m.palette_height`, substitutions off |
| Footer | `Cancel`, `Apply` (default) |
| Valid check | on Apply, not per keystroke, as v1.2.0 |
| Invalid JSON | sheet stays; Error above the footer `This isn’t valid JSON. {parser message}`; caret moves to the reported offset |
| Invalid definition | same line: `This workspace can’t be used. {validator message}`. Nothing is applied, so nothing half-changes |
| Changed elsewhere | if the workspace changed since the sheet opened, Error `This workspace changed while you were editing. Close and reopen to see the latest version.`; `Apply` disables |
| Applied | sheet closes, banner `Workspace updated` |

Tab inserts two spaces in the editor; Esc then Tab leaves it (shader-lab.md rule).
Plain Esc while the text is unchanged cancels the sheet; with changes, the first Esc
arms the exit and a second cancels.

## S101 Panel Library

v1.2.0 opens this as a full-screen overlay. Native: one window, `Window ▸ Panel
Library`, same frame recipe as S017 (`NSSplitViewController`, sidebar
`m.settings_sidebar`), title `Panel Library`.

| Sidebar group | Items |
|---|---|
| Top | search field `Search` (AX per view: `Search panels`, `Search workspaces`, `Search After Effects imports`) |
| `Panels` | `All Panels` `{n}`, `Built In`, `Made by You`, `From the Store` |
| `Workspaces` | `All Workspaces` `{n}`, `Built In`, `Saved by You` |
| `Imported` | `After Effects`, shown only when imports exist or a transfer is available |
| Bottom | `Browse the Store` (opens the Store) |

Content: page title `t.heading`; shelves of cards, heading `t.title` when grouped.

**Panel card**: `m.thumbnail_min` minimum width; live thumbnail or the panel's symbol
on `c.panel_secondary`, `m.control_radius`; title `t.body`; trailing round `m.hit`
button: `plus` (`Add {panel} to Workspace`) or `eye` (`Show {panel} in Workspace`).
Hover adds `trash` on user panels (`Delete {panel} Panel`). Click opens the editor.

**Store card**: icon, name, status line (`statusText`, else `Turned off` / `Not
loaded`). Click opens it in the Store's Library. A last card `Find Panels in the
Store`.

**Workspace card**: name, summary `{n} panels`, or `Active`. Click activates; hover
`trash` on saved ones.

| State | Spec |
|---|---|
| Empty search | `EmptyState` `magnifyingglass`, `No Results`, `There’s nothing here.` |
| Empty shelf | `EmptyState`, `Nothing on This Shelf Yet` |
| After Effects empty | `EmptyState`, `Your After Effects imports will appear here.` + `Bring My Workspace…` (S110) |
| Transfer failed | inline notice `Couldn’t start the workspace transfer. Try again.` |

**Panel editor** replaces the content pane, with `Back to Panels` in the toolbar and
`In Workspace` as a subtitle when it is.

| Region | Spec |
|---|---|
| Stage | leading, the real panel in its panel card at a fixed `m.agent_width`, on `c.viewer`, centred, labelled `Panel preview`. It is live: controls work |
| No preview | `EmptyState` `No Live Preview Yet`, `Add {panel} to your workspace to see and edit the real panel here.` + `Add to Workspace` |
| Conversation | trailing `m.agent_width`, agent conversation and composer recipes, placeholder `Describe a change to {panel}…`, labelled `Panel conversation` |
| Suggestions | three borderless buttons above the composer: `Make It More Compact`, `Clarify the Layout`, `Add a Control` |

**Delete a panel** — alert: `Delete the panel “{panel}”?` / `It leaves the Panel
Library and every workspace. Powermove keeps an archived copy on this Mac.` /
`Cancel` (default) · `Delete Panel` (destructive). This supersedes S012's sentence
on local panels for this one case, since v1.2.0's “cannot be undone” isn't true under
N4.

Keyboard: ⌘F search; arrows move across cards; Return opens; Delete deletes where
allowed. Esc leaves the editor (`Back to Panels`), then does nothing further; ⌘W
closes. VoiceOver: each shelf is a list; card actions are `AXCustomActions`.

## S102 Generated panels

A workspace definition or the agent can declare a panel made of controls. The host
renders them with kit components; nothing in here is authored HTML.

Control mapping (`GeneratedPanel.svelte`, `workspace.ts`):

| Schema | Kit component |
|---|---|
| `slider` | `NumberField` with `NSSlider` beneath in the value column at `m.field` height |
| `number` | `NumberField` |
| `color` | `ColourWell` |
| `select` | `PopUpButton` |
| `toggle` | `Checkbox` |
| `point` | paired `NumberField` |
| `gradient` ramp | inspector gradient recipe; see Gradient Editor below |
| `curve` | a small easing preview button; opens the easing popover |
| `readout` | value text `t.body` tabular `c.secondary`, not editable; `No keyframes selected` when empty |
| `button` | push button at `m.control`. A `primary` control takes the accent only when it is the one primary in the panel |
| `note` | `t.minimum` `c.muted` line |
| secondary group | `DisclosureGroup` `More`, remembered per panel; a changed value marks it with `m.status_dot` `c.accent` |

Panels open with a title from the definition, fallback `Controls`. Section titles
are `t.title`. Each control is one undo step named for its label.

**Built-in Gradient Editor**: `Type` (`Linear`, `Radial`), ramp with two stops,
`Start Color`, `End Color`, `Angle` −180–180°, `Midpoint` 1–100 %. It writes the
composition background.

**Tools with preview**:

| State | Spec |
|---|---|
| Idle | buttons as declared |
| Preview | block below: heading `Preview` `t.title`; list of changes `{name}`, `{before} ▸ {after}` tabular; at most 8, then `{n} more changes` |
| Applied | same block, heading `Applied to Source`; viewer banner `{label} applied` |
| No change | heading `Nothing Changed`; line = reason, else `The tool couldn’t produce a valid source change.` |

Preview never edits the project; apply is one undo step named for the control.

**Bindings**:

| Failure | Spec |
|---|---|
| Target layer gone | control disabled, tooltip `This control’s layer isn’t in the composition.`; the row stays so the panel keeps its shape |
| Property gone | same, `This control’s property isn’t available.` |
| Value mixed | inspector mixed-value state |
| Value out of range | shown as is; edits clamp to the declared range |
| Invalid schema | panel `EmptyState` `exclamationmark.triangle` `c.warning`, `{Panel} Can’t Be Shown`, `Its definition isn’t valid.`, details to the Error Log |
| Script action failed | viewer banner `{label} couldn’t finish.` |

Keyboard and VoiceOver come from the kit. A control's accessible name is its label
plus panel title on first focus.

## Don't

- Don't put the workspace switcher back in the toolbar by default.
- Don't apply a Workspace Definition partially.
- Don't let a preview write to the project.
- Don't overwrite a built-in workspace.
