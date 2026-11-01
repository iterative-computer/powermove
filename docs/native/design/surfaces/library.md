# Project Library, new project, rename/duplicate, Trash

Rows S005 S017 S018 S019. Grammar: [README](README.md) §1–4. Ports
`R/legacy/ui/projects.ts`, `R/legacy/core/projects.ts`,
`R/legacy/ui/project-settings.ts` `createNewProjectForm`. Model:
`pm-services::library::{ProjectEntry, ProjectSummary, LibraryAction,
LibraryService}`; vocabulary from
[reviews/na-cloud-services.md](../reviews/na-cloud-services.md) (Move to
Trash, Put Back, Recently Deleted).

## S017 Project Library window

Replaces the shipped Home screen (language §15). `File ▸ Project Library`
(no shortcut: ⇧⌘O is Open in w1-keymap K047) and the Welcome window's
`Show All Projects`.

```
┌ toolbar: [sidebar]  Project Library      [grid|list] [Sort ⌄] [⌕ Search]  [+] ┐
├ sidebar ────────┬────────────────────────────────────────────────────────────┤
│ Recents         │  ┌────┐ ┌────┐ ┌────┐                                       │
│ All Projects    │  │thumb│ │thumb│ │thumb│   grid, m.thumbnail_min columns    │
│ Recently Deleted│  └────┘ └────┘ └────┘                                       │
│                 │  Name                                                        │
│                 │  Today, 11:42 · 1920×1080                                    │
└─────────────────┴────────────────────────────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Window | Standard titled window, title `Project Library`, `NSToolbar` unified, min `m.min_width` × `m.min_height` ÷ 1.25, frame autosaved; one instance. |
| Sidebar | `NSSplitViewController` sidebar (system source list), width `m.settings_sidebar`; items `Recents` (`clock`), `All Projects` (`square.grid.2x2`), `Recently Deleted` (`trash`). |
| Toolbar | View segmented `NSSegmentedControl` (`square.grid.2x2` / `list.bullet`, tooltips `Grid View` / `List View`); `Sort` pop-up (`Date Modified` ✓ default, `Name`); `NSSearchToolbarItem` placeholder `Search Projects`; `New Project` (`plus`, ⌥⌘N per w1-keymap K057/M007). |
| Grid | language §9 project grid: thumbnail columns ≥ `m.thumbnail_min`, `m.control_radius`, duration badge `c.media_badge`; name `t.body`; facts `t.minimum` `c.muted`: `{relative date} · {w}×{h}`. Selected = 2 pt `c.accent` ring + name on `c.selected_fill`. No thumbnail → `c.field` well with `film` symbol `c.muted`. |
| List | `NSTableView`, columns `Name`, `Date Modified`, `Duration`, `Size`, `Frame Rate`; sortable headers mirror the Sort pop-up. |
| Dates | system relative formatter (`Today, 11:42`, `Yesterday`). |
| Open | double-click / Return opens; ⌥-double-click / `Open in New Window` opens in a new window. Opening a project already open focuses it. |
| Context menu | `Open` · `Open in New Window` · separator · `Rename…` · `Duplicate` · `Show in Finder` (file-backed only) · separator · `Move to Trash` (⌘⌫). In Recently Deleted: `Put Back` · separator · `Delete Immediately…` (see S005). |
| Unsaved/file facts | List `Name` column trailing `t.minimum` `c.muted` `Edited` when the project has unsaved changes; tooltip `Saved to {path}` or `Not saved to a file`. |

| State | Visual |
|---|---|
| Empty (no projects) | `EmptyState` `No Projects`, line `Projects you create appear here.`, button `New Project`. |
| Empty search | `No Results`, line `No projects match “{query}”.` |
| Recently Deleted empty | `No Deleted Projects`, line `Projects you move to the Trash appear here.` |
| Loading | panels/README Loading; placeholder wells at card geometry. |
| Storage unavailable | inline notice (warning) `Your projects can’t be shown right now.` action `Try Again`. |
| Large data | panels/README §4 (virtualised grid rows). |

Keyboard: arrows move selection in grid and list; type-to-select by name;
⌘F focuses search. No other new shortcuts.
VoiceOver: grid is an `AXList` of `AXGroup` cards labelled
`{name}, edited {relative date}, {w} by {h}`.

Don't: paginate (shipped `Next page` removed — virtualise); show an update
banner here (updates are their own surface); put Store or Settings in this
window.

## S018 New Project sheet

Sheet on the frontmost editor or Project Library window (README §2). Fields
start from the current project's format (shipped behaviour), otherwise the
app default.

| Row | Control | Validation / copy |
|---|---|---|
| `Name` | `NSTextField`, prefilled `Untitled` (uniqued `Untitled 2`…) | empty → Error `Enter a name.` |
| `Resolution` | `NSPopUpButton`: `4K UHD · 3840×2160`, `HD 1080p · 1920×1080`, `HD 720p · 1280×720`, `Vertical HD · 1080×1920`, `Portrait 4:5 · 1080×1350`, `Square · 1080×1080`, separator, `Custom` | editing Size switches it to `Custom` |
| `Size` | two number wells (W, H) with `px` units, link toggle trailing outside | `16`–`8192`; Error `Enter a size from 16 to 8192 pixels.` |
| `Frame Rate` | number well + `fps` | `1`–`240`; Error `Enter a frame rate from 1 to 240 fps.` |
| `Duration` | number well + `s`; accepts timecode | `0.1`–`3600` s; Error `Enter a duration up to 1 hour.` |
| `Background` | colour well (opens inspector §7 popover) | — |

Title `New Project`; buttons `Cancel` · `Create`. Creation failure → error
alert (README §4) on the same window, sheet stays open.

## S019 Rename and Duplicate

- **Rename…** — in-place: the name in the grid card / list cell becomes an
  editable `NSTextField` (Finder behaviour), Return commits, Esc reverts.
  Empty or unchanged → reverts silently. Failure → error alert. Also from the
  editor: clicking the toolbar document title opens the system title
  popover's Name field (`NSWindow` title menu: Rename/Move To/Duplicate —
  only the items the kernel supports).
- **Duplicate** — no dialog: creates `{name} copy` (`{name} copy 2`…) next to
  the original, selects it and starts in-place rename.
- Project context menu: S017 table (same items in grid, list and the editor's
  title menu).

## S005 Trash, Put Back, delete immediately

| Action | Surface | Copy |
|---|---|---|
| Move to Trash | **No confirmation** (recoverable, macOS convention); the item leaves the view; Edit ▸ Undo Move to Trash (⌘Z) in the Library window puts it back. Inline notice (info, 4 s): `“{name}” was moved to the Trash.` action `Undo`. |
| Put Back | No confirmation; returns to its previous place. |
| Delete Immediately… | Alert, **critical**: message `Delete “{name}” immediately?` informative `The project is archived in Powermove’s storage and can’t be opened from the Library again.` buttons `Delete` (destructive) · `Cancel` (default). |
| Open project moved to Trash elsewhere | its window stays open; subtitle `In Trash`; first save asks via alert `“{name}” is in the Trash.` buttons `Put Back and Save` (default) · `Save As…` · `Cancel`. |

N4: `Delete Immediately` archives (`ArchiveReason`), never removes bytes; the
copy says so. The shipped `Delete Forever` wording and "cannot be undone"
claim are not ported. There is **no** automatic emptying of Recently Deleted.
