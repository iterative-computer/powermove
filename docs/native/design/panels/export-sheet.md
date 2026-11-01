# Export sheet

`File ▸ Export…` (⌘E, also ⌘⇧E — both v1.2.0 bindings) and the toolbar
**Export** button. A window-modal system sheet (language §9 *Sheets*; on
macOS 26+ the system centres it in the window — see language.md).
Shared rules: [README.md](README.md).

Behaviour source: `v1.2.0:apps/desktop/src/renderer/src/legacy/core/exporter.ts`
(dialog `L85–L278`, progress `L292–L330`),
`v1.2.0:apps/desktop/src/renderer/src/core/export-defaults.ts` (formats,
field support, labels). Workflows W069, W070 (B424–B445, B499–B504, A264).

**Scope change:** v1.2.0's fourth export type, **Code** (web animation ZIP),
is not offered — product decision 2026-10-29 (no web export). The three
types are Video, Images and Project.

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `ExportKind` | `Video` · `Images` · `Project` (remembered format per kind, v1.2.0 `remembered`) |
| `ExportFormat` | Video: `ProRes` (`QuickTime · ProRes 4444`), `Mp4` (`MP4 · H.264`), `WebM` (`WebM · VP9`); Images: `PngSequence` (`PNG sequence`), `Still` (`Still frame (PNG)`); Project: `Pmv` (`Project file (.pmv)`) |
| `FieldSupport` | per format, exactly `exportFieldSupport` (v1.2.0): ProRes {size, fps, range, mblur, audio, alpha}; MP4/WebM {size, fps, range, quality, mblur, audio}; PNG sequence {size, fps, range, mblur, alpha}; Still {size, mblur, alpha}; Project {} |
| `scale` | `0.5` · `1` · `2` · `Custom{w, h}` (aspect kept; 16…8192 px) |
| `fps` | 23.976, 24, 25, 29.97, 30, 50, 59.94, 60 |
| `range` | `WorkArea` · `Full` |
| `quality` | `Draft` · `High` · `Max` |
| `captions` | `burn: bool`, `sidecar: None · Srt · Vtt` (only when the project has captions layers) |
| `presets` | `[{id, name, options}]` (app-wide) |
| `ExportRun` | `Idle` · `ChoosingDestination` · `Preparing` · `Rendering{frame, total, eta}` · `Finishing` · `Cancelling` · `Done{path}` · `Failed{message}` |

## 2. Anatomy — settings sheet

Width `m.sheet_width_wide`, inset `m.sheet_inset`, content on `c.panel`
(language §3: the system owns the glass frame, content is panel material).

```
┌──────────────────────────────────────────────────────────────┐
│ Export                                                       │  t.heading
│ [   Video   |   Images   |   Project   ]                     │  NSSegmentedControl, full width
│ MP4, WebM or ProRes for delivery and editing.                │  t.reading c.secondary
│ ┌ Output ──────────────────────────────────────────────────┐ │
│ │ Format          Container and codec…   [MP4 · H.264    ⌄]│ │  sheet_value_column
│ │ Resolution      Rendered size…         [Full · 1920×1080⌄]│ │
│ │ Frame Rate      Frames written…        [30 fps         ⌄]│ │
│ │ Range           Export the work area…  [Work area      ⌄]│ │
│ │ Quality         Video bitrate…         [High · 16 Mbps ⌄]│ │
│ └──────────────────────────────────────────────────────────┘ │
│ ┌ Rendering ───────────────────────────────────────────────┐ │
│ │ Motion Blur     Render motion blur…              [  ◯ ] │ │  checkbox trailing
│ │ Include Audio   Mix composition audio…           [  ☑ ] │ │
│ └──────────────────────────────────────────────────────────┘ │
│ ┌ Presets ─────────────────────────────────────────────────┐ │
│ │ Saved Preset    Load a set…            [Choose Preset  ⌄]│ │
│ │ Save as Preset  Name these settings…   [name      ][Save]│ │
│ │ Color: sRGB. ProRes uses tagged BT.709 primaries…        │ │  t.minimum c.muted
│ └──────────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤
│ 300 frames · 10 s · 1920×1080          [Cancel] [Export Video]│  footer
└──────────────────────────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Title | `t.heading` `c.text`, `Export` |
| Kind control | `NSSegmentedControl`, regular, `selectOne`, fills the content width, segments `Video` · `Images` · `Project`; `m.inset` below the title |
| Kind description | `t.reading` `c.secondary`, `m.gap` below the control: Video `MP4, WebM or ProRes for delivery and editing.`; Images `A single frame, or every frame as a PNG sequence.`; Project `An editable .pmv with every layer and its media.` |
| Section card | settings group recipe (settings.md §3 *Group*): title `t.title` outside, card `c.field` (light) / `c.floating` (dark), radius `m.control_radius + m.inner_radius`, rows separated by inset hairlines. A section with no visible rows is hidden. |
| Row | min height `m.row + m.inset`; label `t.body` `c.text`; detail `t.minimum` `c.muted` under it; control trailing in a column `m.sheet_value_column` wide (pop-ups fill it; checkboxes sit at its **trailing** edge, matching Settings switch placement — the sheet is a form, not an inspector). |
| Custom size row | appears under Resolution when `Custom size…` is chosen: two number fields (`px`) with `×` between and a `link` glyph (aspect kept, not toggleable), 16…8192. |
| Project kind body | instead of sections: a card with title `Keep Every Layer Editable` `t.title`, text `Save a portable Powermove project with its media, effects, keyframes and composition settings.` `t.reading` `c.secondary`, and three capsule labels (`c.panel_secondary`, `t.minimum`): `Editable layers`, `Original media`, `Animation & effects`. |
| Colour note | `t.minimum` `c.muted` at the bottom of Presets: `Color: sRGB. ProRes uses tagged BT.709 primaries and sRGB transfer. Transparent output uses straight alpha.` |
| Footer | hairline `c.separator` above; height `m.control + 2 × m.inset`; leading summary `t.body` tabular `c.secondary`; trailing buttons `Cancel` then the default (accent) button. |
| Summary | Video/PNG sequence: `{frames} frames · {seconds} s · {W}×{H}`; Still: `{W}×{H} · frame at {timecode}`; Project: `{name}.pmv`. Second line (when the planner has a note) `t.minimum` `c.muted`. |
| Default button | label by format (v1.2.0 `exportActionLabel`, Title Case): `Export Video` (ProRes/MP4/WebM), `Export Frames` (PNG sequence), `Export Frame` (still), `Save Project File` (pmv). |

Option labels (exact): Resolution `Half · {w}×{h}`, `Full · {w}×{h}`,
`2× · {w}×{h}`, `Custom size…`; Frame Rate `23.976 fps`, `24 fps`, `25 fps`,
`29.97 fps`, `30 fps`, `50 fps`, `59.94 fps`, `60 fps`; Range `Work area`
(`Work area · not set` when unset, still selectable — exports the whole
composition), `Full composition`; Quality `Draft · 4 Mbps`, `High · 16 Mbps`,
`Max · 40 Mbps`; Caption File `None`, `SubRip (.srt)`, `WebVTT (.vtt)`;
Saved Preset `Choose Preset` + preset names.

Rows (label · detail), shown per `FieldSupport`:

| Section | Row | Detail |
|---|---|---|
| Output | `Format` | `Container and codec of the exported file.` |
| | `Resolution` | `Rendered size relative to the composition, or any pixel size.` |
| | `Size` (custom) | `Exact pixel size to render; keeps the composition aspect.` |
| | `Frame Rate` | `Frames written per second of exported video.` |
| | `Range` | `Export the work area or the whole composition.` |
| | `Quality` | `Video bitrate used for video exports.` |
| Rendering | `Motion Blur` | `Render motion blur for moving layers.` |
| | `Include Audio` | `Mix composition audio into the video.` / disabled with `No audio layers in this project.` |
| | `Transparent Background` | `Keep the background see-through, with straight alpha.` |
| Captions (only with captions layers) | `Burn In Captions` | `Draw captions into the picture.` |
| | `Caption File` (Video only) | `Also write a subtitle file beside the video.` / `Also write one subtitle file per captions layer beside the video.` |
| Presets (not for Project) | `Saved Preset` | `Load a set of export settings you saved earlier.` |
| | `Save as Preset` | `Name these settings to reuse them in any project.` — text field placeholder `Preset name` + push button `Save` (disabled while empty; Return in the field saves) |

## 3. Destination and progress

1. Default button → the settings are remembered as this project's export
   defaults (A264) and an **`NSSavePanel`** (still/video/pmv) or
   **`NSOpenPanel` choose-folder** (PNG sequence) opens **as a sheet on the
   Export sheet** (B440: destination before any work). Suggested name
   `{project name}.{ext}`. Cancel returns to the Export sheet with every
   setting intact.
2. Save → the Export sheet is replaced by the **progress sheet** (same width,
   not dismissible by Esc or outside click):

```
┌──────────────────────────────────────────────────────────────┐
│ Exporting “Launch Film”                                      │  t.heading
│ ┌──────────────── live frame preview ───────────────┐        │  16:9 (comp aspect), m.control_radius
│ └───────────────────────────────────────────────────┘        │  on c.viewer surround
│ ▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░                         │  NSProgressIndicator bar
│ Frame 120 of 300 · 40 %                     About 2 min left │  t.body tabular / t.body c.secondary
├──────────────────────────────────────────────────────────────┤
│                                                     [Cancel] │
└──────────────────────────────────────────────────────────────┘
```

| Phase | Label (leading) | Trailing | Button |
|---|---|---|---|
| Preparing | `Preparing…` + indeterminate bar | — | `Cancel` |
| Rendering | `Frame {i} of {total} · {pct} %` (+ ` · {extra}` from the encoder) | ETA after ≥ 8 frames and ≥ 1 s: `About {n} min left` (≥ 90 s) / `About {n} s left` | `Cancel` |
| Finishing | `Finishing…` + full bar | — | `Cancel` |
| Cancelling | `Cancelling…` | — | button disabled, title `Cancelling…` |

3. **Done** → the sheet closes; a viewer banner (viewer-chrome.md §2)
   `Exported “{file}”` with `Show in Finder` (4 s, stays while hovered).
   If the app is not active, a system notification with the same text (W044).
   Project export: `Saved “{file}”`. Still: `Exported frame “{file}”`.
4. **Failed** → the progress sheet stays, bar hidden, `exclamationmark.triangle.fill`
   `c.danger` + message `t.body` (`Export failed: {reason}` — reasons from the
   engine in sentence case with the fix), buttons `Close` and `Try Again`
   (default). **Cancelled** → sheet closes, no banner.

## 4. States

| State | Visual |
|---|---|
| **Empty** | N/A (Export is disabled in the menu/toolbar without a project). |
| **Loading** | N/A; capabilities are known before the sheet opens. A format the Mac can't encode is listed disabled with its reason as the menu item subtitle (`H.264 MP4 export is unavailable on this Mac`). |
| **Hover/Focus/Disabled** | system controls. |
| **Error** | inline under the offending row (`Enter a size from 16 to 8192 px.`); default button disabled while invalid. |
| **Already running** | the Export command while an export runs brings the progress sheet's window forward; the toolbar Export button is disabled with tooltip `Export already running`. |
| **Drag/drop, multi-select, 60 k** | N/A. |

## 5. Keyboard and focus

Initial focus: the kind control. Tab order: kind → each visible row control
top to bottom → preset name field → Save → Cancel → default button.
←/→ on the kind control switches kind. Return activates the default button
(except in the preset name field, where it saves the preset). Esc = Cancel
(settings sheet only). ⌘. = Cancel in the progress sheet.

## 6. VoiceOver

Sheet: `Export, dialog`. Kind: radio group `Export type`. Rows: control label =
row label, help = detail. Summary: static text, re-announced (polite) when it
changes. Progress: `Export progress` progress indicator with value
`40 percent, frame 120 of 300`; ETA as help. Done/failed announced.

## 7. Appearance

- Sheet frame: system (glass on 26+, solid under Reduce Transparency).
  Content `c.panel`; cards lifted (`c.field` light / `c.floating` dark).
- Preview frame sits on `c.viewer`; it is content (no tint).
- Increase Contrast: card borders, footer hairline 2 pt.
- Reduce Motion: no scanning animation over the preview (v1.2.0
  `export-scan` decoration is dropped in all modes).

## 8. Copy

All strings are in §2–§3. Additional: menu item `Export…`; toolbar tooltip
`Export (⌘E)`; disabled-format subtitles as supplied by the engine;
`Export already running`; `Export failed: {reason}`; `Close`; `Try Again`.

## 9. Don't

- Don't start rendering or show progress before a destination is chosen.
- Don't offer Code/web export.
- Don't use a custom-drawn segmented control or toggles; system controls.
- Don't put two accent buttons in the footer; don't label the default `OK`.
- Don't show a fake percentage during Preparing.
- Don't place the sheet yourself; the system positions sheets.
