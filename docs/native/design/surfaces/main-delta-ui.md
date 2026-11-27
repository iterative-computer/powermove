# Main's UI changes in the native language

Rows U01–U16, plus the UI parts of the P, E and A rows of
[main-delta.md](../../main-delta.md). Grammar: [README](README.md) §1–5. Reference:
`origin/main` at `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`, the parity target since the
owner decision of 2026-11-04.

Main's **behaviour** is the requirement. Main's **look** is evidence, not instruction.
Each row below says how the change lands, and the panel specs carry it where a row changes
them. Where main's visual choice conflicts with an approved native call, the native call
stands: see §1.

## 1. Native calls that stand against main

| Topic | Main | Native, and it stays | Authority |
|---|---|---|---|
| Value alignment | Right-aligned in places | Left-aligned in the well | language §9 |
| Timecode | Orange | Neutral `c.text`; orange is the playhead's | language §11 |
| List selection | Accent-tinted | Neutral `c.selected_fill`; the accent is on the item's mark | language §4 |
| Workspace switcher | Titlebar menu, with Panel Library beside it | `Window ▸ Workspace` and `Window ▸ Panel Library…`, with optional toolbar items through Customize Toolbar | language §2, §15 |
| Header height | Variable, with a heading strip | `m.header` 32 for every panel, viewer bar and timeline header | language §6 |
| Layer list width | Wider, to fit its columns | `m.layer_column` 266 | panels/README owner decision 1 |
| Agent | Panel in the layout | Popover from the toolbar, with Pin to dock | panels/README owner decision 2 |
| Panel name | `Effects & Presets`, `Layer Effects` | `Effects`, `Effect Controls` | panels/README owner decision 3 |
| Viewer controls | Strip under the image | Viewer bar above it | viewer-chrome.md §2 |
| Menus | App-drawn | Real `NSMenu` | language §9 |
| Visibility switch | Leading edge of the layer row | Trailing switch group | timeline.md |
| Timeline transport | Ruler gutter | Timeline header | timeline.md |

## 2. Docking: bottom dock, tab stacks, drop zones, maximize (P01 U01)

**Bottom dock.** Main's `bottom` dock is a row under every column except the right one.
That's already the native anatomy (language §2), so nothing moves. The Timeline lives there
at its approved default height. Its floor is `m.header + 2 × m.ruler + 3 × m.track_row`.
Any panel can join it, as a tab or beside it. Saved workspaces, group membership and active
tabs migrate as data; no UI appears for that.

**Tab stacks.** A stack is the existing tab group: panels/README §2 segmented control, in
the panel's header. With a single panel, the header is its title.

| Case | Spec |
|---|---|
| Too many tabs for the header | Segments shrink equally to `m.hit × 3`, truncating with `…`. Beyond that, the trailing tabs fold into a borderless `chevron.down` pull-down at the end of the track, tooltip `More Panels`, listing them with a checkmark on the active one. The active tab never folds |
| Drag a tab | Moves that panel only |
| Drag the rest of the header | Moves the whole stack. Hover shows no grip: the header's empty area is the handle, as with a Mac title bar |
| Reorder | Drag within the track; the panels/README insertion line shows the slot |
| Panels that can't stack | The Viewer takes no tabs, so it offers only the above and below zones |

**Drop zones** (`layout/dock-zones.ts`), replacing the shell's half-body targets:

| Pointer over | Zone | Result |
|---|---|---|
| A panel's header, or the middle of its body | Group | Joins that stack at the tab slot under the pointer |
| Top or bottom 30 % of a panel's body | Above, below | Docks beside it in the column |
| Between panels, or a window edge | Edge | Docks there, as the shell does today |
| The panel's own place | Self | No highlight; drop does nothing |

There are no left and right zones on a panel: Powermove's columns stack vertically.

Feedback uses panels/README *Drop target*: the target panel gets an `m.drop_stroke` `c.accent`
inner stroke at `m.panel_radius`, and the zone it would occupy fills `c.accent_wash`. For
Group that's the whole body, plus a ghost tab at the slot in the header (`t.body`, `c.accent_text`).
The dragged panel is the panels/README *Drag source*. Esc restores the layout from before the
drag. On drop, panels move with the `mo.enter` spring; under Reduce Motion they cross-fade.

**Keyboard and VoiceOver.** The pointer isn't the only way. The dock menu (panels/README §2)
gains `Group With ▸`, `Dock Above ▸` and `Dock Below ▸`, each listing the shown panels, and
`Maximize Panel`. Stacks read as `AXTabGroup`.

**Maximize.** `` ` `` (bare backtick) maximizes the panel under the pointer, or the focused one
from the keyboard; pressing it again restores. Not while a text field has focus.

| Part | Spec |
|---|---|
| Result | The stack fills the whole area between toolbar and status bar, with `m.gap` kept to the window edge. Everything else hides. Toolbar and status bar stay |
| Tabs | Switching tabs stays maximized |
| Restore | Trailing header button `arrow.down.right.and.arrow.up.left`, tooltip `Restore Panel`, shown only while maximized. Also `` ` ``, and `Window ▸ Restore Panel` |
| Menu | `Window ▸ Maximize Panel` with the shortcut |
| Motion | `mo.enter` spring from the panel's frame; Reduce Motion cross-fades |
| Persistence | Not saved. A layout change, or the panel closing, ends it |
| VoiceOver | `.medium` announcement `{panel} maximized` / `Restored` |

**Column scrolling.** As main's `layout/model.ts`: in a side column holding only Properties
or Effect Controls, the column scrolls as one page. Over a panel with a set height (Effects, by
default), each panel keeps its own scroller. Full-screen tools stay in the `NSToolbar`, which
the system keeps available in full screen.

## 3. Timeline Layers column (U04 U05 U06 E12)

The full spec is now in [timeline.md](../panels/timeline.md) §2. The decisions:

| Main | Native |
|---|---|
| A heading strip under the ruler | Headings go in the empty **ruler gutter** of the layer column, so the timeline gains no height (32 pt headers stand, and the gutter was unused) |
| Columns `eye`, `Layer`, `Link Layers` | `Layer` at the name's x, `eye` over the Visibility slot; `Parent` when that column is showing. Right-click for the Columns menu |
| `Link Layers` | **`Parent`**. It's the word Properties, the pick whip and undo already use, and in Tracks mode "link" means linked audio and video (pro-edit.md). Same behaviour |
| `Show All After Effects Columns` | `Show All Columns` |
| Switches always visible in fixed slots | Existing native rule: fixed slots, shown when set or on hover. New slots `Audio`, `3D` and `Raster`. Their default-state glyphs need a wider column, so 266 pt still shows Position and Scale values |
| Parent name column | Appears from `m.layer_column + m.value_column`. Below that, the `link` glyph and its tooltip carry it |
| Work area bar in the strip | Stays in the ruler: `c.accent_wash` bed and handles |
| Section rows (`Transform`, each effect, `Layer`) | `m.property_row` heading rows, `t.body` `t.medium_weight` `c.secondary`, at the property label's x. Not interactive, skipped by arrow keys |
| Keyframe navigator on hover | Adopted: the chevrons appear on row hover or focus, and the row's rest state is the diamond alone. Quieter, and it's main's behaviour |
| Group disclosure reveals descendants' keys | Adopted, as the collapsed key summary on the group row |
| Numeric key time past the strip, negative time | Adopted: `Edit Value…` accepts any time. A taken time is the sheet's Error `Keyframe time is taken by another keyframe.` The ruler draws negative time as `−0:02`, under the out-of-work-area veil |
| Selected keyframes distinguishable (U06) | Already native: `c.accent` plus `c.panel` keyline, so shape as well as colour. Graph points match |
| `Release Pre-comp`, layer switches (E12) | `Layer ▸ Release Pre-comp`, in the shared layer menu. `Continuously Rasterize` is the Raster switch and the `Layer` section row |

Pick whip (U04): no change; the drag starts from the Parent glyph or name.

**Composition flowchart** (`viewer-chrome.ts`). Shows which compositions use this one and which
it contains; click one to open it. An `NSPopover` from the viewer bar's leading edge, solid
`c.floating` content, width `m.popover_width`. Sections `Used In` and `Contains`, headings
`t.minimum` `c.muted`; rows `m.row`, symbol `rectangle.stack`, name `t.body`; the current
composition sits between them on `c.selected_fill`. Empty lines `Not nested anywhere` and `No
nested compositions`. Open it with `View ▸ Composition Flowchart`, or Tab on the focused canvas as
main, unless Full Keyboard Access is on, when Tab keeps moving focus.

## 4. Keyframe diamonds and per-axis keying (U07)

Stopwatches are gone from main, and native never had them. Native's diamond language stands
in Properties, Effect Controls and the timeline: outline `c.muted` not animated, half-filled
`c.accent` animated and off-key, filled `c.accent` a key at the playhead.

**Axis diamonds.** Grouped rows (`Anchor Point`, `Position`, `Scale`, `Orientation`, `Rotation`)
get one diamond per axis, full spec in [inspector.md](../panels/inspector.md) §2. The row's
diamond in the diamond column keys every axis together. The axis diamond keys that axis alone
and reveals its row in the timeline.

Main puts the diamond inside the well. A two-axis well is 62 pt, too small to hold a diamond
beside `1920 px` with text left-aligned. So the diamond shares the **unit slot**: the trailing
end of the well shows the unit at rest and the axis diamond on well hover, on keyboard focus
or whenever that axis is animated. The value never moves, and the row's other well still shows
the unit.

**Three axes.** Three wells in `m.value_column` would be 40 pt each. Rows with X, Y and Z stack
as three rows: label `{name}` then `Y`, `Z` in `c.muted`, each with a full-width well in the same
column. Main stacks too, when narrow.

**Separate Dimensions.** On by default, so existing projects keep one timeline row per axis.

| Part | Spec |
|---|---|
| Where | `Separate Dimensions` ✓ on the Position row's menu, in Properties and the timeline |
| Off | Position is one row, with keys grouped by time; its readout edits one axis per number, as the existing joined readout does |
| Undo | `Separate Dimensions` / `Join Dimensions` |
| Scale | Unchanged: never two readouts |

AX as main: `Add keyframe for Position X`, `Remove keyframe for Position X`; tooltip `Add Position
X Keyframe`.

The Panel Library button main restored to the titlebar stays in `Window ▸ Panel Library…` (§1).

## 5. Remaining U rows

| Row | Landing |
|---|---|
| U02 Point controls | No visual change. Point wells use the authored X and Y increments, and a missing parameter makes no row. inspector.md *Point pad* and S102 follow |
| U03 Paste media | `Edit ▸ Paste` unchanged. Pasted media imports as any import does; focus in a text field keeps text paste. No new UI |
| U08 Add Effect | Already in the native specs: `Add Effect` in the Effect Controls header and trailing `Add Effect…`, and in the Properties header, since effects don't live in Properties here. E04's disclosures are below |
| U09 Tooltips | §6 |
| U10 Menus | §7 |
| U11 Material, neutral dark, Title Case, aligned values, wells | Already the language. Title rename stays in the title menu (`Rename…`), which edits the title in place as document apps do. Axis wells take no `X`/`Y` prefix: position in the pair says it, and the AX label names the axis |
| U12 Easing | easing.md stands. `Simplify` runs only on an explicit command, with its own Undo |
| U13 Precision, audio, captions | Carried: colour.md, mixer-3d.md, captions.md |
| U14 Anchor Point | Pointer press selects or narrows to the hit layer, and the same drag moves its anchor; a plain click only selects. [viewer-chrome.md](../panels/viewer-chrome.md) *Interactions* |
| U15 Loading | panels/README Loading and viewer-chrome *Loading* stand. The last good frame stays; rows draw from the model as they arrive |
| U16 Identity | Native uses the current Powermove mark and app icon. No new assets; type stays SF Pro |

## 6. Tooltips (U09)

One tooltip for the main window's GPUI and custom surfaces, as main. Sheets and Settings use
the system's `toolTip`.

| Part | Spec |
|---|---|
| Component | Kit `Tooltip`, a child panel, solid `c.floating`, system shadow, `m.control_radius` |
| Content | Line 1: name `t.minimum` `c.text`, shortcut trailing `c.muted`. Line 2, if any: `t.minimum` `c.secondary`, sentence case. Padding `m.gap`, max width `m.popover_width` |
| Timing | Shows after `mo.tooltip_delay`. Moving to a neighbouring control within `mo.tooltip_warm` swaps it at once, with no fade |
| Place | Below the target, `m.gap` away, flipping and shifting to stay `m.gap` inside the window |
| Hides | On press, typing, scroll, pointer leave, or focus leaving |
| Keyboard | Shows for keyboard focus with the same delay |
| VoiceOver | Text goes to `AXHelp`; the panel itself isn't an element |

## 7. Menus (U10)

Real `NSMenu` throughout, so main's app-drawn menus, type-select and press-drag-release come
from the system. What ports is the content:

| Change | Native |
|---|---|
| Separate context menus for graph, ruler, lanes, keyframes, layers, canvas, media, easing, tabs and text animators | Yes, with main's items, in Title Case. Timeline ones extend timeline.md §4 |
| Shared `New` order | One order in the `New` menus everywhere: main's `menu.ts` |
| Effect categories as submenus | Yes, as effect-controls.md §4 |
| Unavailable items | Dimmed, never hidden; shortcuts stay visible |
| Icons in menus | None, except what the system adds |
| Menu bar | `Powermove ▸ Show All`, `Edit ▸ Delete`, `Help`, as `main/menu.ts`. No duplicate `New Window` |
| Text fields | AppKit's own menu, with Writing Tools, Services, Look Up and spelling |

## 8. Project rows

| Row | Landing |
|---|---|
| P02 Memory | New Settings section, §9 |
| P03 Linked media | No new UI. Export sheet's `Project` kind keeps its copy. A missing link is S006's offline treatment and never blocks a save |
| P05 Storage upgrade | Not ported. N001–N003 replace it |
| P06 Storage Settings | §10 |
| P07 Version History | Sheet below |
| P11 Close waits | documents.md *Finishing Up…* covers it |
| P12 Library drops, folders | [library.md](library.md) takes file drops with the *Drop target* treatment on the whole grid: projects open, media starts a new project, neither touches the open document. Folder import and download are Background Tasks rows. The Figma alert is S040 |
| P13 Web export | Removed already |

**Version History.** Main opens it from the status bar's save text. The sheet is new.

| Part | Spec |
|---|---|
| Open | `File ▸ Version History…`, or click the status bar's save state, which stays quiet `c.muted` text with `c.hover` on hover. Tooltip `Version History` |
| Frame | Sheet, `m.sheet_width`, title `Version History` |
| Rows | `m.row`: date and time `t.body` tabular; source `t.minimum` `c.muted` (`Saved File`, `Local Recovery`, `macOS Snapshot`); trailing button `Open Copy` |
| Open Copy | Opens a new document, name `{project} (Recovered)`. Nothing replaces the current project |
| Empty | `EmptyState` `clock`, `No Earlier Versions`, `Earlier saved versions appear here.` |
| Loading | README Loading |
| Footer | `Done` (default) |
| Failed | Alert per README §4: `Couldn’t open this version.` / `Couldn’t load versions.` |

## 9. Settings ▸ Memory (P02 A07)

The sidebar groups as main: `Project` (`Project`, `Memory`) and `App` (`General`, `Accounts`,
`Transcription`, `Extensions`, `Storage`), as source-list section headers. `Project` and `Memory`
need an open project. Symbol `list.bullet`. Spec in [settings.md](../panels/settings.md).

Description: `Notes agents keep about this project: what the video is for, and why it looks the
way it does.`

| Part | Spec |
|---|---|
| Group `Project Memory` | `Use Project Memory`, switch. Detail `Agents read these notes before they edit, and add to them as they work. Turning it off keeps the notes but hides them from agents.` |
| Sections, in order | `Goal`, `Rules`, `Open Items`, `Decisions`, `Rejected Ideas`, `Feedback`; empty ones are omitted |
| Entry | Settings row: text `t.body` `c.text`, reason `t.reading` `c.secondary`, meta `t.minimum` `c.muted` `You · {date}` or `Agent · {date}`, plus ` · Resolved` |
| Resolved | Text `c.muted`, with `checkmark.circle` before the meta |
| Off | Sections stay, in `c.muted`. No whole-region opacity (panels/README *Disabled*) |
| Empty | Row `No Notes Yet` / `When an agent records a goal, rule or decision for this project, it will show up here.` |

Read-only, as main. Changing a note doesn't touch Undo or mark the project edited.

## 10. Settings ▸ Storage (P06)

Main's page reports usage and deletes old backups. Native's archive rule stands (N4): nothing is
deleted from Settings. So main's reporting ports, and its cleanup becomes archiving. Full spec
is in [tasks-storage.md](tasks-storage.md). What lands:

| Main | Native |
|---|---|
| `{n} used`, reclaimable bytes | New first group `Space on This Mac`, with total and `{n} can be freed.` |
| Category rows | Under it, one row per category, size tabular |
| `Clean Up` and its confirmation | `Free Up Space…`, the existing sheet, gaining rows `Older Backups` and `Temporary Caches` |
| `Clean up automatically` | `Clean Up Automatically`, moving to the archive instead of deleting |
| `Older backup budget` | `Older Backup Budget`, 1, 5, 10, 20 GB: how much stays in place before moving to the archive |
| `Storage folder`, `Refresh` | `Show in Finder`, `Refresh` |
| Section order | After `Extensions`, as main |

## 11. Effects rows

| Row | Landing |
|---|---|
| E04 Parameter disclosures | Grouped parameters in Effect Controls use the inspector's section disclosure inside the effect, indented one `m.tree_indent`, state kept per effect |
| E04 Colour swap | Effect action rows such as `Swap Colors`: push button at the leading edge of the value column, one Undo |
| E05–E08, E10, E11 New and revised effects | Effects panel lists them under their categories; rows follow inspector grammar. No new row type |
| E09 Apply Color LUT | The LUT parameter is a file row, as colour.md's LUT picker. Stroke's brush fields are ordinary rows |
| E13 HDR interpretation | colour.md N004 stands |

## 12. Agent rows

| Row | Landing |
|---|---|
| A01 Providers | S071: grouped pop-up, discovered models, warning. [agent-auth.md](agent-auth.md) |
| A02 Agent apps | S070 *Other Agent Apps*: the client list. [agent-auth.md](agent-auth.md) |
| A03 Claude Project Auto | Copy only changes once the runtime supports it; until then `Auto` keeps asking, and S057's copy stands |
| A04 Richer replies | [agent-panel.md](../panels/agent-panel.md) §3: lists, quotes and links in the language's type scale; local media under the prose; remote images opt-in; folded work |
| A05 Visual replies | S062 stands. Popover with Pin stays (§1) |

## Don't

- Don't add a column-heading strip, or widen `m.layer_column` to fit one.
- Don't copy main's colours, dimensions or menu drawing.
- Don't port Storage's deletion.
- Don't call parenting “link” in the UI.
- Don't add a workspace or Panel Library control to the default toolbar.
