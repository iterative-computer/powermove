# Media — relink, cloud download, import, sequences, Figma, source errors

Rows S006 S007 S024 S038 S039 S040. Grammar: [README](README.md) §1–4.
Builds on [panels/media-browser.md](../panels/media-browser.md) (statuses,
overlays, menus) and [reviews/na-cloud-promedia.md](../reviews/na-cloud-promedia.md)
(Background Tasks). Nothing here changes those; it fills their gaps.

## S006 Relink mismatch and proxy-only media

Entry points stay as media-browser.md (`Locate File…`, `Replace File…`).

**Chosen file doesn't match** (any of duration, dimensions, frame rate,
has-audio differs from the recorded source) — alert, sheet on the window:

| Part | Copy |
|---|---|
| Message | `“{chosen}” doesn’t match the missing media.` |
| Informative | One sentence per difference, max three: `It’s 0:08 long; the original was 0:12.` · `It’s 1280×720; the original was 1920×1080.` · `It’s 25 fps; the original was 30 fps.` · `It has no audio; the original did.` |
| Buttons | `Use Anyway` (default) · `Cancel` · `Choose Another…` |

**Others found in the same folder** (after a successful relink, other offline
assets resolve by name in that folder) — alert:
message `Relink {n} other missing files in “{folder}”?`, informative
`They have the same names as missing media in this project.`, buttons
`Relink All` (default) · `Just This One`. Mismatching files among them are
skipped and listed in the Background Tasks row (S038 summary).

**Proxy-only** (original offline, proxy present — `Availability` proxy):
card status overlay `Proxy only` with `exclamationmark.triangle.fill`
`c.warning`; meta line `Original missing · using proxy`; viewer draws the
proxy without a banner. Export adds one validation row (export-sheet.md
validation list, warning style): `{n} clips use proxies because their
originals are missing.` with borderless `Relink…` (opens the first one's
`Locate File…`). Export is **not** blocked.

## S007 Cloud media download consent and progress

Ports `M/cloud-media.ts` L119–122. Asked when a project opens with cloud
placeholders (or an import adds them), unless the Settings preference
`Automatically download cloud media when opening projects` is on.

| Part | Copy |
|---|---|
| Message | `Download cloud media?` · plural `Download {n} cloud media files?` |
| Informative | `These files are stored in the cloud and need to be downloaded to use them in this project.` then a blank line and up to 8 file names, one per line, then `…and {n} more` |
| Suppression checkbox | `Automatically download cloud media when opening projects` (unchecked; checking it sets the Settings preference) |
| Buttons | `Download` (default) · `Not Now` |

After `Download`: each card shows media-browser.md `Downloading…` overlay
with a determinate circular indicator when the provider reports progress
(indeterminate otherwise); Background Tasks shows one row
`Downloading {n} files from {provider}` with per-file disclosure, `Cancel`
(stops remaining; finished files stay). Failure per file: card
`Download failed` + hover `Retry Download`; task row terminal
`{k} of {n} downloaded` with `Retry`. `Not Now`: cards stay `In cloud`; the
viewer banner `Some media is still in the cloud.` action `Download`
appears once per open.

Don't: download without consent when the preference is off; block opening
the project; show provider error text outside Error Log details.

## S024 Source preview — decode errors

Source mode per viewer-chrome.md. When the source can't be shown, the frame
area shows a centred group (dark-scheme tokens, over the viewer surround):
`exclamationmark.triangle` 2 × `m.symbol_panel` `c.muted`, title
`t.heading` `Can’t Preview “{file}”`, one line `t.reading` `c.secondary`,
optional push button. Source bar and `Back to Composition` stay.

| Cause | Line | Button |
|---|---|---|
| Unsupported codec | `Powermove can’t decode {codec} files.` (or `this format` when the codec is unknown) | — |
| Damaged/unreadable | `The file may be damaged.` | — |
| Offline | `The file is missing.` | `Locate File…` |
| Cloud | `The file is stored in {provider}.` | `Download` |
| Decoder failed mid-playback | frame keeps the last good image; banner (warning) `Couldn’t decode part of this clip.` | — |
| Audio stream unreadable, video fine | video previews; banner (info) `This clip’s audio can’t be played.` | — |

## S038 Import progress, failures and cancellation

All imports (drag, picker, folder, paste) are one Background Tasks row per
user action (promedia review). Titles: `Importing {n} files`,
`Importing “{folder}”`, `Pasting media`. Transitions:

| Event | Row / effect |
|---|---|
| Running | determinate by file count: `{i} of {n}`; conversion sub-step subtitle `Converting “{file}”…` / `Creating proxy for “{file}”…` |
| Cancel (row `Cancel`, or Esc while the row is focused) | stops remaining files; completed ones stay as one undo step `Import Media`; terminal row `Import stopped · {i} of {n} imported` |
| All ok | row moves to completed; no notice (the media is visible). |
| Some failed | terminal row (warning) `{i} of {n} files imported` with disclosure listing each failed file + reason; `Retry` retries only failed files. |
| Unsupported files | listed in the same disclosure with reason `Not a supported format`; never retried. |
| Conversion/proxy failed | asset still imports on its original; row line `Couldn’t create a proxy for “{file}”.` + `Retry`. |
| Nothing imported | terminal row (error) `Couldn’t import {n} files` + disclosure + `Retry`; viewer banner (warning) `No media was imported.` action `Show Details` (opens Background Tasks). |
| Disk full | error copy from `ServiceErrorCode::DiskFull`; remaining files stop. |

Reasons (sentence case): `Not a supported format` · `The file may be
damaged` · `The file is missing` · `Not enough disk space` · `You don’t have
permission to read it`.

## S039 Import Image Sequence sheet

Ports `R/legacy/core/image-sequence.ts`. Offered when the import contains a
numbered run of stills.

| Element | Spec |
|---|---|
| Title / line | `Import Image Sequence` · `Turn your numbered images into one clip.` |
| Source row | `film` symbol `m.symbol_inline`, `{n} images` `t.body`, below `{first} to {last}` `t.minimum` `c.muted`, middle-truncated with full tooltip |
| `Frame Rate` | number well + `fps`, default project fps; `1`–`240`; Error `Enter a frame rate from 1 to 240 fps.` |
| `Duration` | read-only value `0:04 · 120 frames` (updates live) |
| Warnings (warning style, under the rows) | gaps: `{k} numbers are missing from the sequence; those frames hold the previous image.` · mixed sizes: `The images aren’t all the same size; each is fitted to the first.` |
| Buttons | leading `Individual Images` (absent when the caller requires a sequence) · `Cancel` · `Import Sequence` |
| Replace mode | from `Replace File…` on a sequence: title `Replace Image Sequence`, default `Replace`, no `Individual Images` |
| Progress | Background Tasks row `Checking frames · {i} of {n}` then `Creating image sequence…` |

## S040 Paste Figma selection — fallback

Ports `R/legacy/core/figma/paste.ts` L45–50. `{reasons}` = one short sentence
per unsupported feature (`Masks with blur can’t stay editable.`), max four,
then `and {n} more`.

**No preview image in the clipboard** — acknowledgement alert:
message `These Figma layers need an image fallback.`, informative
`{reasons} Figma didn’t include a preview image. Use Copy as PNG in Figma, then paste again.`,
button `OK`.

**Fallback available** — alert:
message `Paste the Figma selection as an image?`, informative
`{reasons} The whole selection becomes one image you can animate; its parts won’t be editable.`,
buttons `Paste as Image` (default) · `Cancel`.

**Clipboard unreadable / expired** — banner (warning)
`Couldn’t read the Figma selection. Copy it again in Figma.`

Everything supported pastes without any prompt. Don't: paste partially
editable plus partially rasterised without saying so.
