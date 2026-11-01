# Settings

The Settings window (`Powermove ▸ Settings…`, ⌘,). Shared rules:
[README.md](README.md). Values: `m.settings_width` × `m.settings_height`,
sidebar `m.settings_sidebar`.

Behaviour source: `v1.2.0:apps/desktop/src/renderer/src/settings/SettingsScreen.svelte`
and `v1.2.0:apps/desktop/src/renderer/src/legacy/ui/` (`project-settings.ts`,
`chatgpt-settings.ts`, `compatible-settings.ts`, `extension-settings.ts`).
Workflow W012 (A257, A268–A276, A333–A336), W013, W024, W035, W041, W042.

## 1. Form (HIG placement, v1.2.0 behaviour)

v1.2.0 Settings is **one continuous page** with a section sidebar that
tracks scrolling, a search field that filters across sections (including
extension details), and closes with Done, Esc or ⌘, (restoring focus).
Native keeps the behaviour in a Mac Settings window:

- A separate, non-modal **`NSWindow`** titled after the active section
  (`General`, `Project`, …), standard close button, not resizable below
  `m.settings_width` × `m.settings_height` (resizable larger, height only).
  Opening it again brings the existing window forward. Position remembered.
- **Sidebar** (`NSSplitViewController` sidebar, source-list style, system
  sidebar material — this is the one place `Surface::Sidebar` glass is used),
  width `m.settings_sidebar`, search field at its top.
- **Detail**: one scroll view containing every section in order (continuous,
  as v1.2.0). Selecting a sidebar item scrolls to its section; scrolling
  updates the sidebar selection and window title.
- No **Done** button (Mac settings apply immediately). Esc and ⌘, close the
  window (v1.2.0 keys kept), as does ⌘W.

## 2. Model contract (proposed names)

| State | Values |
|---|---|
| `sections` | `Project` (only with a project open) · `General` · `Accounts` · `Transcription` · `Extensions` (+ later `Memory`, `Storage` when their lanes land) |
| `active_section` | id (scroll-tracked) |
| `query` | `String`; `matches: [(section, row)]` |
| row values | typed per row; writes persist immediately (per profile or per project as listed) |

## 3. Anatomy

```
┌ ● ● ●                     General ───────────────────────────────────────┐
│ ┌ sidebar ──────────┐ ┌ detail (c.panel) ──────────────────────────────┐ │
│ │ [⌕ Search       ] │ │ General                                         │ │ pane heading t.heading
│ │ ▣ Project         │ │ How Powermove looks and works on this Mac.     │ │ t.reading c.secondary
│ │ ▣ General   (sel) │ │ ┌ Appearance ─────────────────────────────────┐│ │ group box (card)
│ │ ▣ Accounts        │ │ │ Appearance          [Match System ⌄]        ││ │
│ │ ▣ Transcription   │ │ │ Follow your system setting, or pick a light…││ │
│ │ ▣ Extensions      │ │ │─────────────────────────────────────────────││ │
│ │                   │ │ │ Density             [Normal ⌄]              ││ │
│ └───────────────────┘ │ └─────────────────────────────────────────────┘│ │
│                       └─────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Sidebar rows | system source list: `NSTableView` source-list style, row height system (≥ `m.row`), SF Symbol per section at system sidebar size, label `t.body`. Selection: the system sidebar selection **tinted by AccentColor** is not used — set the source list to `.secondary`/neutral selection so it is `c.selected_fill`-like (language §17.3: list selection neutral). |
| Section symbols | Project `doc.richtext`, General `gearshape`, Accounts `person.crop.circle`, Transcription `waveform.and.mic`, Extensions `puzzlepiece.extension` |
| Detail background | `c.panel` (solid) |
| Detail inset | `m.sheet_inset` on all sides; max content width = window width − sidebar − 2 × `m.sheet_inset` |
| Section heading | `t.heading` `c.text`; `m.sheet_inset × 2` above each section except the first |
| Section description | `t.reading` `c.secondary`, `m.gap` below the heading, `m.inset` above the first group |
| Group (card) | macOS grouped form: fill `c.field` in light (`#F3F3F6`) / `c.floating` in dark, radius `m.control_radius + m.inner_radius` (10), no border (Increase Contrast: `m.border` `c.separator`). Group title (optional) `t.title` `c.text` `m.gap` above the card, outside it. Rows inside separated by hairlines `c.separator` inset `m.inset` from the leading edge. |
| Row | min height `m.row + m.inset` (40), padding `m.inset`; label `t.body` `c.text` leading; optional detail line below it `t.minimum` `c.muted` (sentence case, wraps); control **trailing**, vertically centred on the label line. |
| Controls | system: `NSPopUpButton` (regular), `NSSwitch` (regular) for booleans, push buttons (`m.control`), `NSTextField`, `NSSecureTextField`. Settings is the only surface that uses switches. |
| Search highlight | matching row gets `c.accent_wash` behind the label run (not the whole row) and the match text `t.semibold_weight`; sections without matches are hidden while searching; the sidebar dims non-matching sections (`c.muted`). |

## 4. Sections and rows (exact copy, v1.2.0 order)

Booleans are switches; choices are pop-ups. "Scope" says where it persists.

### Project (only with a project open; selected by default when open)

Description: `Settings for “{project name}”.`

| Group | Row | Control / options | Detail | Scope |
|---|---|---|---|---|
| Project | `Name` | text field | `Shown in the title bar and used for exported file names.` | project |
| Composition | `Size` | pop-up: `HD 1080p · 1920×1080`, `HD 720p · 1280×720`, `Square · 1080×1080`, `Portrait 4:5 · 1080×1350`, … `Custom…` (reveals Width/Height rows) | `Pick a common size, or set your own pixel dimensions.` | project |
| | `Frame Rate` | number field `fps` | `Frames rendered per second of the composition — any rate from 1 to 240.` | project |
| | `Duration` | number field `s` | — | project |
| | `Background` | colour well (system `NSColorWell`, minimal style) | `Colour shown behind every layer of the composition.` | project |
| | `Color Depth` | pop-up `8-bit`, `16-bit`, `32-bit float` | `Precision per color channel. Applies to every composition in this project.` | project |
| Export defaults | `Format`, `Resolution`, `Frame Rate`, `Range`, `Quality`, `Motion Blur`, `Include Audio`, `Transparent Background` | same controls, options and detail lines as the Export sheet ([export-sheet.md](export-sheet.md)) | — | project |

### General

Description: `How Powermove looks and works on this Mac.`

| Group | Row | Control / options | Detail | Scope |
|---|---|---|---|---|
| Appearance | `Appearance` | pop-up `Match System` (default), `Light`, `Dark` | `Follow your system setting, or pick a light or dark theme.` | profile |
| | `Density` | pop-up `Compact`, `Normal` (default), `Comfortable` | `Spacing of panels, rows and controls.` | profile |
| Workspace | `After Effects Workspace` | button `Bring My Workspace…` | `Bring your saved panels and tools into Powermove with your agent.` | — |
| Playback | `Preview Memory Limit` | pop-up `Off`, `128 MB`, `256 MB` (default), `512 MB`, `1 GB`, `2 GB` | `Frames kept in memory so replays are instant.` | profile |
| Media | `Automatically Download Cloud Media` | switch (off) | `Download offloaded files when opening a project.` | profile |
| Windows | `Reopen Last Project on Launch` | switch (on) | `Open only the project you were last working on.` | profile; row only with multi-window support (A276) |

### Accounts

Description: `Your Powermove account, and the models your agent works with.`

| Group | Row | Control | Detail |
|---|---|---|---|
| Powermove Account | signed out: `Powermove Account` | button `Sign In…` | `Remember installs and publish to the Store.` |
| | signed in: `{name}` + `@{handle}` (`t.minimum` `c.muted`) | buttons `Choose Handle…` (when none), `Sign Out` | — |
| | `Remember Installs on This Account` | switch | — |
| | `Delete Account` | button `Delete Account…` (`c.danger` text; NSAlert confirm, destructive default Cancel) | `Your published extensions stay on the Store under your handle.` |
| Subscriptions | `ChatGPT`, `Claude` (one row each) | status text (`Connected` `c.success` + `checkmark.circle.fill`, `Checking…`, `Waiting…`, `Restart needed` `c.warning`) + button `Connect` / `Disconnect` / `Retry` | `Finish signing in in your browser.` while waiting |
| API or Local Model | `Connection Type` | pop-up `OpenAI API`, `Ollama`, `LM Studio`, `Other compatible provider` | — |
| | `API Base URL` | text field | `Base URL of the API, ending in /v1.` |
| | `API Key` | secure field | `Key saved securely. Leave blank to keep it.` / `Optional for local models.` / `Required by this provider.` |
| | `Model Name` | text field | `The exact model name your provider expects.` |
| | `This Model Supports Images` | switch | `The model can read pictures you attach to a message.` |
| | `Status` | status text + button `Test and Connect` (`Testing…` while running) | errors: `Couldn’t connect. Check the address and model.` / `Not connected. Start your local server, or enter your provider’s details.` / `Restart Powermove to enable API and local connections.` |

### Transcription

Description: `Turn speech into text for captions and your agent, on this Mac.`
Rows: one per speech model (name, size `t.minimum` `c.muted`), trailing
state: `Download` button / determinate progress ring + `Cancel` / `Installed`
+ `Remove…` (W041). Errors inline (`c.danger` + triangle, `Retry`).

### Extensions

Description: `Manage extensions and turn them on or off.`
A filter row (`Filter extensions` search field + pop-up `All Types`, `Effects`,
`Layer Types`, `Editor Tools`, `Themes`, `Commands`), then the list: rows with
icon (`m.row` square, `m.control_radius`), name `t.body`, type + source
`t.minimum` `c.muted` (`Built in`, `Installed`, `AI-created`, `Custom`),
trailing switch (enabled). Click a row → detail pane **in place** (the list
slides out; back button `chevron.left` + `Extensions`): `About`, `Includes`,
`Keyboard Shortcuts`, `Files` (`Show in Finder`), extension variables, and
`Delete…` (`c.danger`, NSAlert). Esc in a detail returns to the list
(A257 extension settings route). Store publishing row: `Publish and manage
your extensions on the Store.` with `Open Store`.

## 5. States

| State | Visual |
|---|---|
| **Empty** | Extensions list empty: README empty state `No Extensions` / `Extensions you install appear here.`; failing to load: `Extensions couldn’t be loaded.` + `Try Again`; unavailable: `Extensions are unavailable in this session.` |
| **No search results** | detail shows centred `No settings found.` (v1.2.0 copy) `t.reading` `c.secondary`; sidebar items all dim. |
| **Loading** | rows that load status (subscriptions, extensions) show the row with a small spinner in place of the trailing control; `Checking…`. |
| **Hover** | system controls only; sidebar hover none (source list). |
| **Focus** | system rings. |
| **Disabled** | system disabled controls; a row disabled by another row (e.g. API Key for Ollama) keeps its detail line explaining why. |
| **Error** | inline per row (`c.danger` + `exclamationmark.triangle.fill`, sentence with the fix). |
| **Drag/drop** | N/A. |
| **Multi-select** | N/A. |
| **60 k rows** | Extensions list virtualised (README §4). |

## 6. Keyboard and focus

Tab order: sidebar search → sidebar list → detail controls in reading order.
Sidebar: ↑/↓/Home/End move between visible sections and scroll the detail
(A257). ⌘F focuses search. Esc: clears search if it has text → closes an
extension detail → closes the window and restores focus to the editor
(A271). ⌘, toggles the window. Editor shortcuts never reach the editor while
Settings is key.

## 7. VoiceOver

Window title = section. Sidebar: `Settings sections` list. Each group is an
`AXGroup` labelled with its group title; each control's label = row label,
help = detail line. Search announces `{n} settings found` / `No settings found`.

## 8. Appearance

Sidebar: system sidebar material (glass on macOS 26+, vibrant before;
solid under Reduce Transparency). Detail: solid `c.panel`; groups `c.field`
(light) / `c.floating` (dark) — lifted above the panel in both. Increase
Contrast: group borders. Reduce Motion: section scroll jumps instead of
animating (A334).

## 9. Copy

All row labels, details and options are in §4. Additional: search
placeholder `Search`, window title = section name, `No settings found.`

v1.2.0 sentence-case labels (`Preview memory limit`, `Reopen last project on
launch`) become Title Case row labels (HIG form labels), details stay
sentence case.

## 10. Don't

- Don't present Settings as a sheet or a panel inside the editor.
- Don't add a Done/OK/Apply button.
- Don't use checkboxes for settings booleans (switches here), or switches
  anywhere else.
- Don't put a Settings gear in the toolbar.
- Don't tint the sidebar selection orange.
