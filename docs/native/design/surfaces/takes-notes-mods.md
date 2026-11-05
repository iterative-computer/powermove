# Takes, Notes, Mods and extension hosting

Rows S103 S104 S105 S106. Grammar: [README](README.md) §1–5. Sources:
`R/panels/{TakesPanel,NotesPanel}.svelte`, `R/legacy/core/history.ts`,
`E/mods/{ModsPanel,ModRow}.svelte`, `R/kernel/{api,registries}.ts`. S013 owns panel
failure, S011 values, S012 removal, Store-host review permissions. No new tokens.

## S103 Takes

Dockable panel `takes`, title `Takes`, default extent 180 pt. Takes are deliberate
named versions of the project, separate from bounded Undo.

| Element | Spec |
|---|---|
| Row | `m.row`, click restores; name `t.body` (`Take {n}`), trailing time `t.minimum` tabular `c.muted` (system relative short time); tooltip `{label} · {full date and time}` |
| Hover | `trash` at the trailing edge, AX `Delete {label}` |
| Footer | push button `Save Take` (`plus`), full width |
| Status | line `t.minimum` `c.muted`, live region: `Saved {label}`, `Restored {label}`, `Deleted {label}`, `Take saved` |

| State | Spec |
|---|---|
| Empty | `EmptyState` `camera.aperture`, title `No Takes`, line `Save a take before exploring a new motion direction.`; footer stays |
| Restore | immediate, one undo step `Restore {label}`, so a stray click is ⌘Z away and no alert is needed |
| Restore failed | status `c.danger` `Couldn’t restore {label}.`; project unchanged |
| Save failed | status `c.danger` `Couldn’t save this take.` |
| Limit | v1.2.0 keeps 24 and drops the oldest silently. Native keeps 24 in the list and moves the oldest to the archive; status adds ` · Oldest take archived` |

**Delete**: immediate, as v1.2.0, with `Deleted {label}` and a borderless `Undo` in
the status line until the next status. The take goes to the archive either way (N4).
Restore from archive is README §6 question 2.

Keyboard: ↑/↓ move, Return restores, Delete deletes. VoiceOver: `{label}, saved
{relative time}`; action `Restore`, custom action `Delete`.

## S104 Notes

Dockable panel `notes`, title `Notes`, default extent 180 pt: the project's creative
brief.

| Element | Spec |
|---|---|
| Editor | `NSTextView` filling the content, `t.reading`, on `c.panel`, plain text, inset `m.inset`; spelling, dictation and Emoji & Symbols on. Placeholder `Direction notes — saved with this project as its creative brief.` |
| Status | line `t.minimum` `c.muted`: `Saving notes…`, `Notes saved`; fades after `mo.overlay_idle` |

Behaviour: writes are deferred while typing and flushed on blur, project switch, panel
close and project save (`project:flush-edits`), so a switch never loses text. Notes
belong to the project that was open when they were typed, even if the switch happens
mid-delay. Notes are not on the Undo stack; the text view has its own ⌘Z while focused.

| State | Spec |
|---|---|
| No project | `EmptyState` `note.text`, `No Project Open`, `Open a project to write its notes.` |
| Read-only project | editor read-only; status `This project is read-only.` |
| Save failed | status `c.danger` `Couldn’t save notes. They’ll stay here until the next save works.`; text retained |
| Changed elsewhere | agent or second window edits replace the text only while the editor isn't focused; while focused the local text wins and is flushed after |

Editor shortcuts don't fire while focused. VoiceOver: text area `Project notes`; save
status is not announced unless it fails.

## S105 Mods panel

**Decision: retain the panel.** It isn't a copy of Settings ▸ Extensions: it's the
in-editor list of what you and your agent changed, with quick actions, and `mods.open`
is a command. Settings stays the full manager.

Dockable panel `mods`, title `Mods`, default extent 260 pt.

| Element | Spec |
|---|---|
| Summary | `t.minimum` `c.muted`, live region: `{n} mods`, plus ` · {n} needs attention` |
| Groups | `Added by You`, `Built In`, `t.title` |
| Row | `m.row × 2`: name `t.body`; description `t.minimum` `c.muted`, truncating; lineage `Forked from {base} v{version}` when present |
| Needs setup | third line `c.warning` with symbol `Needs setup` + borderless `Set Up…` |
| Failed | third line Error, first line of the failure (full text in tooltip), + borderless `Turn Off` |
| On/off | `SegmentedControl` `Off` · `On`, panels/README tab size; AX group `{name} enabled` |
| More | `ellipsis.circle`, AX `More for {name}`; `NSMenu` |

Menu: `Reload` · `Set Up…` or `Variables…` · `Show in Finder` · separator · `Remove…`
(user mods only).

| State | Copy |
|---|---|
| None added | `Ask the agent to change anything about Powermove — it shows up here.` |
| Action failed | viewer banner: `Couldn’t turn on {name}.`, `Couldn’t turn off {name}.`, `Couldn’t reload {name}.`, `Couldn’t show {name}.`, `Couldn’t remove {name}.`, with the reason appended |

Remove alert: `Remove {name}?` / `It moves to the Trash. You can always ask for it
again.` / `Cancel` (default) · `Remove` (destructive). Then S012 for saved values.

Keyboard: list ↑/↓; Tab enters the row's controls in order. VoiceOver reads the name,
state and any third line.

## S106 Arbitrary extension UI

Authored content is the author's and isn't redesigned. This section owns only what
the host draws around it and what happens when it misbehaves. `pm-compat-host` and
`pm-remote-dom` render v1–3 panels; v4 surfaces use the kit.

| Contribution | Host surface | Rule |
|---|---|---|
| Panel | standard panel card, header, dock menu | Content area clips to the card; authored CSS can't reach host chrome |
| Inspector section | Properties section heading, disclosure | Title Case not enforced on authored titles; order by `after` as v1.2.0 |
| Custom widget in a row | value column | Clipped to `m.value_column`; taller content grows the row |
| Menu item | real `NSMenuItem` in the named menu, after a separator, grouped by extension | Authored icons dropped; label truncated at 40 characters |
| Palette command | palette row with `{extension}` in `c.muted` trailing | |
| Shortcut | resolver | A conflict keeps the built-in binding; Settings ▸ Extensions lists it |
| Status item | S054 | Text only, 24 characters |
| Toolbar item | `Customize Toolbar` palette only, never auto-added | Symbol or generic `puzzlepiece.extension`, label required |
| Settings row | Settings ▸ Extensions detail | |
| Toast | viewer banner with `{extension}` prefix, rate-limited to one per 2 s per extension | |
| `confirm` | system alert, message and body plain text, `Cancel` default. Message is prefixed `{extension}: ` so its origin can't be disguised | |
| `modal` | system sheet at `m.sheet_width` (requested width clamped to `m.sheet_width_wide`), heading = title or `{extension}`, footer from actions, at most one default | |
| Theme | S074 | Increase Contrast always beats a theme |

**Host states**

| State | Spec |
|---|---|
| Loading | S013 `Starting {extension}…` |
| Failed, crashed, stopped | S013 unchanged |
| Needs values | S011 `EmptyState` |
| Permission undecided | content area `EmptyState` `lock.shield`, `{Panel} Needs Permission`, `{extension} needs your OK before it can run.`, button `Review…` opening the permission sheet |
| Denied | `EmptyState`, `{Panel} Isn’t Allowed to Run`, `Change this in Settings ▸ Extensions.`, button `Open Settings` |
| Unsupported API | `EmptyState`, `{Panel} Needs an Update`, `This extension uses something this version of Powermove doesn’t support.` |
| Missing owner | a workspace names a panel whose extension is gone: same card, `EmptyState` `puzzlepiece.extension`, `{Panel} Isn’t Installed`, `Add it from the Store to use this panel.`, button `Open Store`. The dock slot stays, so the layout survives |

**Focus and keyboard.** Tab into a panel enters authored content at its first focusable
element; ⌃Tab always moves to the next panel, so authored content can't trap focus. Esc
is offered to the content first, then the host. Editor shortcuts are suspended while an
authored text input has focus.

**VoiceOver.** Content is an `AXGroup` labelled with the panel title; authored ARIA maps
through. If authored content exposes nothing, the group's label adds `{extension}
content`.

Increase Contrast, Reduce Motion and appearance are passed to content as environment
values; the host doesn't rewrite authored CSS. Compat panels don't get glass.

## Don't

- Don't restyle authored extension content.
- Don't let an extension open an alert without its name in it.
- Don't auto-add extension toolbar items.
- Don't drop the oldest take without archiving it.
