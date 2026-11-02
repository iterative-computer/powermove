# Project (media browser)

Panel id `assets`, title **Project**, left dock, tabbed with Effect Controls.
Shared rules: [README.md](README.md). Look:
[language.md §9 Lists and tabs](../language.md#lists-and-tabs).

Behaviour source: `v1.2.0:apps/desktop/src/renderer/src/panels/AssetsPanel.svelte`,
`v1.2.0:apps/desktop/src/renderer/src/legacy/core/media.ts`,
`v1.2.0:apps/desktop/src/extensions/viewer/source-preview.ts`. Workflows
W018–W023 (A259, A412–A432, A454–A469, B019, B144).

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `folder` | `Root` · `Folder{id}`; `crumbs` |
| `items` | ordered: folders, then (root only) compositions, then assets |
| `AssetStatus` | `Ready` · `Loading` · `Offline{source}` · `Changed{source}` · `Unavailable{error}` · `Cloud{provider, state: Stored · Downloading · Error(msg)}` |
| `Composition` | `{id, name, size, duration, bg, open: bool, nestable: bool, thumb}` |
| `selection` | `assets: Set`, `folder: Option`, `comp: Option`, anchor; select-all is "all except" |
| `rename` | `None` · `Folder{id, draft}` |
| `drop` | `None` · `Panel` · `Folder{id}` · `Crumb{id}` |
| `marquee` | `Option<Rect>` |
| `import` | `None` · `Running{count, label}` (project-bound; late results discarded) |
| `search` | `Option<String>` |

## 2. Anatomy

```
┌ [Project | Effect Controls] ───────────── [⌕] [+] ┐  header
│ [⌕ Search media                             ]     │  search row (when open)
│ ‹ Media / Shots / Day 2                           │  path bar (inside a folder)
│ ┌────────┐ ┌────────┐ ┌────────┐                  │
│ │ ▣ 📁   │ │ ▶ comp │ │0:12 img│   grid           │
│ └────────┘ └────────┘ └────────┘                  │
│ Shots      Launch Film Atmosphere_loop.mov         │  name t.body
│ 4 items    1920×1080…  MOV · 1920×1080 · 0:12 · 48 MB  meta t.minimum c.muted
│                                                   │
│ ┌ 3 selected ─────────────────────────── [🗑] ┐   │  selection bar (≥ 2 selected)
└───────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Header | README §2, tab group. Trailing: **Search** (`magnifyingglass`, tooltip `Search Media`, toggles the search row; ⌘F while the panel is focused) and **Add** (`plus`, tooltip `Add`, opens menu: `New Composition…` ⌘N, `New Folder`, separator, `Import Media…` ⌘I, `Import Folder…`). |
| Search row | AppKit `NSSearchField`, system small control size (AppKit owns the height), row height `m.row`, inset `m.inset`, placeholder `Search media`. Filters names in the current folder and below; results show the folder path in the meta line. Esc clears; second Esc closes the row. |
| Path bar | height `m.row`, inset `m.inset`: `chevron.left` + `Media` (root crumb), then ` / ` separators (`c.muted`) and folder crumbs, `t.body`; crumbs `c.secondary`, current crumb `c.text`. Crumbs are buttons and drop targets. Shown only inside a folder. |
| Grid | columns = max(1, ⌊(W − 2·`m.inset` + `m.inset`) ÷ (`m.thumbnail_min` + `m.inset`)⌋); column gap `m.inset`, row gap `m.inset`; thumbnail width fills the column; thumbnail height = width × 9 ÷ 16. Content inset `m.inset`, top padding `m.gap`. |
| Thumbnail | `m.control_radius`; content: video poster frame / image / comp thumbnail letterboxed on the comp background colour / audio waveform (`c.text` 75 % bars on `c.labels.green`) / folder (`folder.fill` glyph at 2 × `m.symbol_panel` in `c.secondary` on `c.field`) / model (`cube` glyph). Missing poster: kind glyph `m.symbol_panel` `c.muted` on `c.field`. |
| Duration badge | top-left, `m.gap − 1` (5 pt) inset, capsule, fill `c.media_badge`, text `t.minimum` tabular `c.selected_text` (`0:12`). Comps show their duration; stills none. |
| Open-composition mark | compositions open in the timeline get a 6 pt `c.accent` dot, top-right at the badge inset, with a 1 pt `c.media_badge` keyline (AX `Open in the timeline`). |
| Hover actions | bottom-right of the thumbnail, a row of round buttons (`m.hit` diameter, `c.media_badge` fill, glyph `m.symbol_inline` `c.selected_text`): Ready → **Show in Finder** (`folder`), **Add to Timeline** (`plus`); Offline → **Locate…** (`link`); Cloud → **Download** (`icloud.and.arrow.down`, disabled while downloading); all → **Delete** (`trash`). Shown on hover, on keyboard focus of the card, and never during a drag. |
| Name | `t.body` `c.text`, one line, truncating middle (`Atmosphe…oop.mov` keeps the extension), tooltip full name, top margin `m.gap`. |
| Meta | `t.minimum` tabular `c.muted`, one line: assets `{FORMAT} · {W}×{H} · {duration} · {size}` (parts omitted when unknown; audio omits size `W×H`; SVG says `SVG`; models `{n} tris`), `Ready to use` if nothing is known; comps `{W}×{H} · {fps} fps · {duration}`; folders `{n} items` (`1 item`). |
| Status overlay on thumbnail | Loading: thumbnail dims to 50 % under a `t.minimum` `Loading…` capsule (`c.media_badge`); Offline/Unavailable/Cloud: `c.media_badge` capsule bottom-left with symbol + text (`Media offline` `exclamationmark.triangle.fill`, `Media unavailable`, `In cloud` `icloud`, `Downloading…` with a determinate ring if progress is known). The meta line then shows the detail in `c.danger` (offline/unavailable) or `c.warning` (changed) or `c.muted` (cloud). |
| Selected | 2 pt `c.accent` ring outside the thumbnail (`m.control_radius + 2`) and the name on a `c.selected_fill` capsule (padding `m.gap − 2` horizontal). Folder/comp selection identical. |
| Selection bar | appears at the panel bottom when ≥ 2 assets are selected: full width minus `m.inset` each side, `m.row` high, `c.floating` fill, `m.control_radius`, `m.inset` from the bottom; text `{n} selected` `t.body` `c.text`; trailing **Delete Selected Media…** (`trash`, `c.danger` glyph). Solid (not glass — it sits over dense content). |
| Marquee | 1 pt `c.secondary` stroke, `c.text` 6 % fill. |
| Drop overlay | while an external drag is over the panel: whole content area `c.accent_wash` with a centred card (`c.floating`, `m.panel_radius`, `m.inset` padding): `plus` glyph `m.symbol_panel` `c.accent`, title `Add to Project Media` `t.title`, detail `Drops here don’t touch the timeline.` `t.minimum` `c.muted`. |
| Folder drop target | README drop-target on the folder thumbnail/crumb. |

## 3. States

| State | Visual |
|---|---|
| **Empty (no project)** | README empty state: symbol `photo.on.rectangle`, `No Project`, `Open a project to see its media.` |
| **Empty project** | symbol `photo.on.rectangle.angled`, `No Media`, `Drag files here or import from your computer.`, button `Import…` (v1.2.0 copy; shell's `Import files or drag them here.` is replaced). |
| **Empty folder** | symbol `folder`, `Empty Folder`, `Drag media or files here.` |
| **No search results** | `t.reading` `c.secondary` centred: `No media matches “{query}”`. |
| **Loading (project restoring)** | Cards render immediately from the model with the Loading overlay; no panel spinner. A list-level `Loading media…` (`t.minimum` `c.muted`, centred) only when the model has no items yet after 0.5 s. |
| **Importing** | Background Tasks toolbar item shows the import (language §9); in-panel, new cards appear in Loading state as each file is admitted. No progress bar in the panel. |
| **Hover** | card: hover actions appear; thumbnail unchanged (no zoom). |
| **Focus** | the grid is one Tab stop; the focused card gets the focus ring around the thumbnail (outside the selection ring when both) and shows its hover actions. The hover-action buttons are not Tab stops; their commands are reachable from the keyboard (§5), the context menu and VoiceOver actions. |
| **Selected / multi-select** | §2; ⌘-click toggles, ⇧-click ranges in grid order, marquee from empty space with autoscroll (A415, A456). |
| **Disabled** | Add to Timeline disabled for a comp that can't nest (`nestable = false`); Delete Composition disabled when it is the last. Disabled hover buttons draw at `c.muted` glyph on `c.media_badge`. |
| **Unknown readiness** | (not yet checked) poster if any, else kind glyph on `c.field`; no status capsule; meta shows known parts only (never `Ready to use`); hover actions `Delete Media` only; not draggable; after 2 s still unknown → presented as Loading. |
| **Error** | panel transport lost: last view stays, card actions and Add disabled, one Error line at the top of the content (`c.warning` triangle, `t.body` `c.warning`) `Project media isn’t updating right now.`, announced once, cleared when publications resume. Offline/changed/unavailable per §2; import failures appear in Background Tasks (with `Retry`), and the failed file never becomes a card. A delete stopped by a project switch announces `Deletion stopped because you switched projects`. |
| **Drag source** | dragging cards to the timeline/viewer: system drag image = the thumbnail(s) stacked with a count badge (`c.accent` capsule, `t.minimum` `c.selected_text`); originals at 50 %. Offline/loading cards are not draggable. |
| **Drop** | external files → whole panel (overlay); onto a folder → into that folder; ⌥ while dropping a folder keeps individual files (not an image sequence, A418). Internal drag onto a folder/crumb moves the batch (one undo `Move to Folder`); a folder onto its descendant is rejected (no highlight, "not allowed" cursor). |
| **Rename** | folder name becomes an AppKit field in place, text selected; Return commits, Esc cancels, blank ignored. |
| **60 k items** | README §4: virtualise by grid row; posters decode only for visible + overscan rows, at thumbnail pixel size; scrolling fast shows the kind glyph until the poster arrives (no spinner). Select-all is O(1). |

## 4. Interactions

| Gesture | Result |
|---|---|
| Click card | select (announces `Selected {name}`) |
| Double-click asset | source preview in the viewer (W021); offline → `Locate…` picker; cloud → download |
| Double-click folder | open folder |
| Double-click composition | open it in the timeline (a tab) |
| Space (asset focused) | play/pause the source preview |
| Return | asset: add to timeline; folder: open; comp: open |
| Right-click asset | `Add to Timeline`, `New Comp from Selection` (not audio/model), separator, `Replace File…` (offline: `Locate File…`), `Download from Cloud` (cloud only), `Move Out of Folder` (inside a folder), `Show in Finder` (when a source path exists), separator, `Delete Media…` (several: `Delete Selected Media…`) |
| Right-click folder | `Open`, `Rename`, `New Folder Inside`, `Import Folder Into…`, `Move Out of Folder` (inside a folder), separator, `Delete Folder` |
| Right-click composition | `New Composition…`, `Open in Timeline` (disabled if open), `Add to Timeline` (disabled if not nestable), separator, `Composition Settings…`, `Duplicate`, separator, `Delete Composition…` (disabled if last) |
| Right-click empty space | `New Composition…` ⌘N, `New Folder`, separator, `Import Media…` ⌘I, `Import Folder…` |
| Delete referenced media | `NSAlert` (warning): title `Delete “{name}”?` (several: `Delete {n} media files?`), message `This also removes layers that use the selected media. You can undo this.`, buttons `Delete` (destructive, not default) and `Cancel` (default). Unreferenced media deletes without an alert, undoable. |

## 5. Keyboard and focus

Tab order: tab group → Search → Add → search field (if open) → path bar
(crumbs, one stop, ←/→) → grid (one stop) → selection bar's Delete (if
shown). In the grid (w1-keymap *Media selection*, A259/A413): arrows move
focus and selection in grid order **and wrap** (↑/↓ move by a row), ⇧ +
arrows extend, Home/End first/last, ⌘A select all media, Return/Space as §4,
⌫/⌦ delete (with alert rules), Esc clears selection, F2 renames a folder.
⌘F opens search.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Grid | list `Project media`, multiselectable, `AXRowCount` | |
| Asset card | cell | `Atmosphere_loop.mov, video, 1920 by 1080, 12 seconds, 48 megabytes` + status (`offline`, `loading`, `in cloud`) + `selected`; actions: `Add to Timeline`, `Preview`, `Show in Finder`, `Delete`, `Locate`, `Download` (as applicable) |
| Composition card | cell | `Launch Film, composition, 1920 by 1080, 20 seconds, open in the timeline` |
| Folder card | cell | `Shots, folder, 4 items`; actions `Open`, `Rename`, `Delete` |
| Path bar | toolbar `Folder path` with buttons | current crumb `AXValue` `current location` |
| Selection bar | group | `3 selected`, button `Delete selected media` |
| Search | search field | `Search media` |

Announcements (v1.2.0 live region): `Selected {name}`, `{n} media files
selected`, `Added {name} to the timeline`, `Showed {name} in Finder`,
`Couldn’t show {name} in Finder`, `Importing {name}` / `Importing {n} items`,
`Created a folder`, `Opened {folder}`, `Showing all media`, `Deleted {name}`
(+ ` and {n} layers`), `Deleted {folder}; its contents moved up a level`,
`Moved to {folder}`, `Deletion stopped because you switched projects`,
`Previewing {name}`, `Preview unavailable for {name}`.

## 7. Appearance

- Thumbnails are content: no tint, no material. Badges and hover buttons
  over thumbnails use `c.media_badge` + `c.selected_text` in **both**
  schemes (they sit on images).
- Light: card names `c.text` on `c.panel`; selected name capsule
  `c.selected_fill` (`#D5D5DA`).
- Increase Contrast: thumbnails get a 1 pt `c.separator` outline; selection
  ring 3 pt.
- Reduce Motion: none animated except hover-action fade, which becomes
  instant.

## 8. Copy

| Where | String |
|---|---|
| Header tooltips | `Search Media`, `Add` |
| Add menu | `New Composition…`, `New Folder`, `Import Media…`, `Import Folder…` |
| Placeholder | `Search media` |
| Path root | `Media` |
| Empty | `No Media` · `Drag files here or import from your computer.` · `Import…`; `Empty Folder` · `Drag media or files here.`; `No Project` · `Open a project to see its media.`; `No media matches “{query}”` |
| Status overlays | `Loading…`, `Media offline`, `Media unavailable`, `In cloud`, `Downloading…` |
| Meta details | `Ready to use`; `Missing · {path}`; `Missing · locate the file to relink it`; `Changed · {path}`; `Couldn’t load · {error}`; `Stored in {provider} · {path}`; `Downloading from {provider}…`; `Download failed · {error}` |
| Hover tooltips | `Show in Finder`, `Add to Timeline`, `Locate File…`, `Download from Cloud`, `Retry Download`, `Delete Media` |
| Drop overlay | `Add to Project Media` · `Drops here don’t touch the timeline.` |
| Selection bar | `{n} selected` · `Delete Selected Media…` |
| Folder default name | `Untitled Folder` |
| Errors | `Project media isn’t updating right now.` |
| Undo names | `Delete Media`, `New Folder`, `Rename Folder`, `Delete Folder`, `Move to Folder`, `Replace Media` |

v1.2.0 sentence-case menu items (`Add to timeline`, `Delete media…`,
`Reveal in Finder`) become Title Case and Mac vocabulary (`Show in Finder`).

## 9. Don't

- Don't fill selected cards with accent; ring + neutral name capsule only.
- Don't show a progress bar or spinner per card; don't show an import modal.
- Don't delete files on disk — copy never implies it (`Delete Media`, not
  `Move to Trash`, because the source file is untouched).
- Don't make the hover buttons glass (they sit on images; solid
  `c.media_badge`).
- Don't add a list view or sort controls (not in v1.2.0).
