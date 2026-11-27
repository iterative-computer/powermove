# Captions and transcription

Rows S087 S088 S089 S090 S091 S092 S093. Grammar: [README](README.md) §1–5.
Sources: `R/captions/{CaptionsPanel.svelte,install.ts}`,
`E/inspector/CaptionsSection.svelte`, `E/timeline/caption-track.ts`,
`R/transcription/{ModelSheet,ModelMeters,TranscriptionSettings}.svelte`,
`R/transcription/format.ts`, `M/export-sidecar.ts`. Delivery sidecars stay in
[export-sheet.md](../panels/export-sheet.md); settings.md's Transcription rows
are replaced by S092.

No new tokens. One new recipe: `Tag`, a capsule `m.field − m.gap` tall,
`c.selected_fill`, `t.minimum` `c.text`, inset `m.gap`, used for `Recommended`
and `In Use`.

## S087 Captions panel

Dockable panel `captions`, title `Captions`, default extent as v1.2.0 (240 pt in
the left dock group). It follows the captions layer selected anywhere in the
editor.

```
┌ Captions ──────────────────────── [⋯] ┐  header
│ [Layer pop-up, only with 2+ layers]   │  m.row
│ [⌕ Search captions                  ] │  m.row
│ 00:00:01:12  Welcome back to the…     │  rows m.row
│▌00:00:04:03  This is the part…        │  ▌ = cue under the playhead
│ 00:00:07:20  Watch the keyframes…     │
└───────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Header button | `ellipsis.circle`, tooltip `Caption Actions`; `NSMenu`: `Add Caption at Playhead` · separator · `Import Captions…` · `Generate Captions` · `Export Captions…` |
| Layer pop-up | `Captions Layer`, full content width, `m.field` tall; only with two or more captions layers |
| Search | system search field, placeholder `Search captions`; filters the rows |
| Row | `m.row`; timecode `t.body` tabular `c.secondary` in a fixed column sized for the project's timecode; text `t.body` `c.text`, one line, line breaks shown as spaces, truncating with a tooltip |
| Under the playhead | 2 pt `c.accent` bar at the row's leading edge plus `t.medium_weight` text. The bar is a mark, not a fill, and the weight carries it without colour |
| Selected | panels/README Selected; several allowed |
| Editing | the row's text becomes an `NSTextView` (grows to 4 lines, row grows with it), system focus ring |

| State | Spec |
|---|---|
| No captions layer | `EmptyState` `captions.bubble`, title `No Captions`, line `Import an SRT or WebVTT file, or select a clip and generate captions from its speech.`, buttons `Import…` and `Generate`. Two actions on purpose: they are the two ways in |
| Empty layer | `This captions layer is empty.` centred, `t.reading` `c.secondary` |
| No search match | `No captions match “{query}”` |
| Add refused | viewer banner `A caption is already showing here`, or `There is no room for a caption here` |
| Large list | README §4; rows are fixed height, so an editing row is the only exception and is measured |

Gestures: click selects and moves the playhead to the cue; ⇧-click extends, ⌘-click
toggles; double-click edits. While playing, the list keeps the cue under the
playhead in view unless the list has keyboard focus, a row is editing, or search
has text.

Keyboard: search `↓` enters the list; `Esc` clears search. In the list `↑`/`↓`
move, `Return` edits, `Delete` or `⌫` deletes the selection. In the editor `Return`
commits, `⇧Return` adds a line, `Esc` cancels, clicking away commits. Tab order:
layer pop-up, search, list, header button.

Undo names: `Edit Caption`, `Delete Caption`, `Delete Captions`, `Split Caption`,
`Add Caption`.

VoiceOver: list labelled `Captions`, multi-select; row reads `{time}, {text}`, and
`now playing` for the marked row. No announcement per cue during playback.

## S088 Captions in Properties

Inspector section (panels/inspector.md order), shown for a captions layer. Row
grammar and mixed values follow the inspector. Title Case as listed; rows marked
*if* appear only when that condition holds.

| Group | Row | Control | Range / options |
|---|---|---|---|
| `Captions` | style presets | radio group of tiles | see below |
| | `Font` | font field | system font families |
| | `Weight` | pop-up | `Light`, `Regular`, `Medium`, `Semibold`, `Bold`, `Heavy`, `Black`, limited to the font's faces |
| | `Size` | number, `px` | ≥ 4, step 1 |
| | `Color` | colour well | |
| | `Outline` | number, `px` | ≥ 0, step 0.5 |
| | `Outline Color` *if Outline > 0* | colour well | |
| | `Case` | pop-up | `As Typed`, `Uppercase`, `Lowercase` |
| `Background` | `Background` | checkbox | |
| | `Color`, `Opacity`, `Padding`, `Corner Radius` *if on* | well; number `%` 0–100; number `px` ≥ 0; number `px` ≥ 0 | |
| `Placement` | `Position` | pop-up | `Bottom`, `Middle`, `Top` |
| | `Align` | pop-up | `Left`, `Center`, `Right` |
| | `Offset` | pair, `px` | X then Y; AX `Horizontal offset`, `Vertical offset` |
| | `Safe Margin` | number `%` | 0–45, step 0.5 |
| | `Max Width` | number `%` | 10–100 |
| | `Max Lines` | number | 1–6 |
| `Words` | `Highlight` | checkbox | |
| | `Color`, `Size` *if on* | well; number `%` 50–300 | |
| | `Animation` | pop-up | `None`, `Pop In`, `Fade In` |

**Style tiles**: wrapping grid, each tile `m.row × 2` tall on `c.viewer` (they
preview text over footage, so dark-scheme treatment), `m.control_radius`, the
sample `Say it` set in the preset's look with `it` highlighted. Selected tile gets
a 2 pt `c.accent` ring (a mark) and AX `selected`. Tooltip = the preset's
description. Arrow keys move in the radio group; choosing applies at once as one
undo step `Caption Style`. A layer whose values no longer match any preset selects
no tile.

v1.2.0 has no language row or per-cue text controls here: the transcript language
is S092 and cue text is S087, so neither is added.

## S089 Caption clips on the timeline

Completes timeline.md's captions rows; drawing stays there.

| Gesture | Result | Undo name |
|---|---|---|
| Click | selects the cue and syncs S087 | none |
| Drag body | moves within the limits of neighbours | `Move Caption` / `Move Captions` |
| Drag edge | trims, clamped between neighbours | `Trim Caption` |
| Split at playhead | S087 action or Split command | `Split Caption` |
| Double-click a cue | inline editor | `Edit Caption` |
| Double-click a gap | adds a cue, then inline editor | `Add Caption` |

**Inline editor**: an `NSTextField` over the cue rect, height `m.field`, width
max(cue width, `m.popover_width`), `c.field`, system ring, clipped to the timeline.
`Return` commits, `⇧Return` adds a line, `Esc` cancels, clicking away commits. If
S087 is open its matching row scrolls into view but does not enter edit; only one
editor exists at a time.

Cue selection and layer selection stay distinct, as v1.2.0. VoiceOver reads a
cue `{text}, caption, {start} to {end}`.

## S090 Generate Captions

`Generate Captions` runs on the selection; there is no form in v1.2.0 and none is
added. Entry: S087 menu, empty state, palette, `Layer` menu, agent tool.

| Step | Surface | Copy |
|---|---|---|
| Nothing suitable selected | viewer banner | `Select an audio or video clip to caption.` |
| Already running | viewer banner | `Captions are already being generated.` |
| No model | S091 sheet | reason `Captions need a speech model.` |
| Model can't time words | S091 sheet | reason `Captions need a speech model that times each word.` |
| Running | Background Tasks row (promedia review): `Generating Captions`, determinate, `Cancel`. Toolbar item appears | |
| Done | viewer banner | `Added {n} captions` |
| No speech | viewer banner | `No speech was found in the selected clips.` |
| Cancelled | viewer banner | `Caption generation was cancelled.` |
| Project changed | viewer banner | `The project changed before captions were ready.` |
| Failed | viewer banner, error style; Error Log details | `Couldn’t generate captions.` |

The result is one undo step `Generate Captions`. Language comes from S092; the
progress row is the only moving element and no modal blocks editing.

## S091 Speech model sheet

Sheet at `m.sheet_width` from S090, S109 and S092. Title `Download a Speech
Model`; line = caller's reason; optional second line `c.warning` with symbol, the
gap sentence from `captionGapLine` (explains why installed models can't serve this
language; port verbatim with curly quotes).

| Element | Spec |
|---|---|
| Model list | radio group labelled `Speech model`; rows `m.row × 2`, `NSButton` radio leading; name `t.body` `t.semibold_weight` + `Tag` `Recommended`; meta `t.minimum` `c.muted`: `{size} · {reach}` with reach `English only`, `{n} languages` or `Many languages`, and ` · Not for captions` for models without word timing |
| Meters | two trailing `NSLevelIndicator`s in continuous style tinted `c.secondary`, labels `Speed`, `Accuracy` `t.minimum` `c.muted`; AX `Speed, {n} percent`. Neutral on purpose: these compare models, they aren't warnings |
| Note | `Runs on this Mac. Your audio is never uploaded.` `t.minimum` `c.muted` |
| Footer idle | leading `Show All Models`; trailing `Not Now`, then default `Download` / `Resume Download` / `Try Again` |
| Footer downloading | progress block above the footer: `Downloading {name}`, `ProgressRow` determinate, `{done} of {total}` tabular. Buttons `Cancel Download`, `Hide` (default). `Hide` leaves the download running in Background Tasks |

| State | Copy |
|---|---|
| Start failed | Error `Couldn’t start the download. Try again.` |
| Cancel failed | Error `Couldn’t cancel the download.` |
| Model errored | Error, model's message, else `The download failed. Try again.` |
| Disk full | S064 `Your Disk Is Full` copy |
| Finished | sheet closes; the waiting action continues |

Focus: the selected radio. ↑/↓ change model; Return downloads.

## S092 Transcription Settings

Replaces settings.md *Transcription*. Description stays. Note under it: v1.2.0's
`Transcription runs on this Mac and your audio is never uploaded.` and licence
sentence, verbatim.

| Group | Row | Spec |
|---|---|---|
| `Downloaded` (omitted when none) | one per model | name + `Tag` `In Use`; meta line; trailing `Use` (not in use), `Delete…` |
| `Available to Download`, or `Models` when none downloaded | filter pop-up `Show Models For`: `All Languages` + languages | rows with name, `Tag` `Recommended`, meta; trailing `Download` / `Resume` / `Try Again` |
| | downloading row | `ProgressRow`, `Cancel` |
| | nothing left | `Every model for {language} is downloaded.` |
| `Language` | `Spoken Language` | pop-up `Detect Automatically` then languages; detail `Detected automatically. Choose one to label transcripts for captions and subtitles.`, or for a model without detection `{model} can’t detect the language, so it hears English until you choose another.` |
| `Storage` | `Downloaded Models` | `{size} in Powermove’s data folder.` / `No models on this Mac yet.`; `Show in Finder` |

Row error: Error state with the model's message or `The download failed.` Busy rows
disable their buttons. Failures use Error at the top of the section: `Couldn’t read
the transcription models.`, `Couldn’t change the language.`, `Couldn’t use {model}.`,
`Couldn’t delete {model}.`, `Couldn’t show the folder.`

**Delete** alert: `Delete {model}?` / `This frees {size}. You can download it again
at any time.` / `Cancel` (default) · `Delete` (destructive). Models are
re-downloadable files, not user work, so they are truly removed: this is the one
deliberate exception to the archive rule in this family. v1.2.0 says `Delete…`;
settings.md's `Remove…` is replaced.

v1.2.0's *Not available here* row is web-only and not ported.

## S093 Import and export captions

**Import**: `Import Captions…` opens `NSOpenPanel` for `.srt`, `.vtt`; a file may
also be dropped on the Captions panel (Drop target) or timeline. Result is one undo
step `Import Captions`.

| Result | Viewer banner |
|---|---|
| Success | `Imported {n} captions`, one: `Imported 1 caption`, plus ` · {m} unreadable skipped` |
| No cues | `No captions found in {file}` (error style) |
| Failed | `Couldn’t import captions.` (error style) |

**Export**: `Export Captions…` is a sheet at `m.sheet_width`, title `Export
Captions`.

| Row | Control | Detail |
|---|---|---|
| `Captions Layer` (only with 2+) | pop-up of layers | `Which captions to write.` |
| `Format` | pop-up `SubRip (.srt)`, `WebVTT (.vtt)` | |
| `Range` (only with a work area) | pop-up `Whole Composition`, `Work Area` | `Times in the file start from the beginning of the range.` |

Footer `Cancel`, `Export…` (default). Then `NSSavePanel` as a sheet on the sheet,
name `{project}.{ext}` or `{project} - {layer}.{ext}` with several layers.

| Result | Copy |
|---|---|
| No captions layer | viewer banner `There are no captions to export` |
| Saved | viewer banner `Exported SRT captions` / `Exported WebVTT captions` |
| Write failed | Error line in the sheet `Couldn’t write the caption file.`; sheet stays |
| Too large (sidecar) | `The caption file is too large.` |

## Don't

- Don't add a form in front of Generate Captions or per-cue styling.
- Don't let the playing-cue mark become a row fill.
- Don't announce cues during playback.
- Don't block editing behind a generation or download.
