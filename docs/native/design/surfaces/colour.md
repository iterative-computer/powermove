# Colour: project depth, scopes and LUTs

Rows S068 N004. Grammar: [README](README.md) §1–5. Sources:
`R/legacy/ui/project-settings.ts`, `R/core/color-depth.ts`,
`engine/crates/pm-color/src/{scopes,lut}.rs`, [pro-media.md](../../pro-media.md). N004
isn't a shipped surface: v1.2.0 has no scopes. Timeline and Mixer waveforms are audio
and are not scopes.

Three new tokens for the pm-design lane. They are data colours, not chrome, always on
`c.viewer`, and identical in both schemes because scopes are drawn over-the-image.

| Token | Value | Role |
|---|---|---|
| `colors.scope_red` | `#FF6B62` | Red channel trace (≥ 3:1 on `viewer`) |
| `colors.scope_green` | `#4FD58A` | Green channel trace (≥ 3:1 on `viewer`) |
| `colors.scope_blue` | `#6BA6FF` | Blue channel trace (≥ 3:1 on `viewer`) |

## S068 Project Settings, colour and validation

Completes settings.md *Project*, whose row list stands with these corrections from
v1.2.0: the preset row is `Resolution`, with `4K UHD · 3840×2160`, `HD 1080p ·
1920×1080`, `HD 720p · 1280×720`, `Vertical HD · 1080×1920`, `Portrait 4:5 · 1080×1350`,
`Square · 1080×1080`, `Custom…`; and `Color Depth` reads `8-bit`, `16-bit float`,
`32-bit float`.

| Row | Rule | Behaviour |
|---|---|---|
| `Size` (shown for `Custom…`), detail `Width and height of every frame, in pixels.` | 16–8192 px, step 2 | A value that doesn't match a preset shows `Custom…` |
| `Frame Rate` | 1–240 fps, whole numbers | |
| `Duration`, detail `Total length of the composition, in seconds.` | 0.1–3600 s, 0.5 step, stored to 0.001 | |
| Any number | Blank restores the previous value. Out of range clamps on commit and the row shows `t.minimum` `c.warning` with symbol `Changed to {value}, the {nearest limit}.` until the next edit. Never an alert, never a blocked field |
| `Color Depth` | applies to every composition | Takes effect at once; viewer re-renders, render bar goes `Missing` as for any graph change |
| Export defaults | same clamps as the Export sheet; `Width`, `Height`, `Frame Rate` rows appear for a custom scale | |

| State | Spec |
|---|---|
| No project | section absent, as settings.md |
| Read-only project | controls disabled; group note `This project is read-only.` |
| Agent or second window changed a value | row updates live unless it has focus |
| Save failed | handled by S003; the value stays in the open project |

Every number has AX units: `Width, 1920 pixels`. Undo: edits are ordinary project edits
named for the row.

## HDR and camera log

Project Settings, group `Color`: `HDR` switch, then `Mastering Peak` (600, 1000, 2000, 4000
nits), disabled while HDR is off. Turning HDR on sets `Color Depth` to `32-bit float` in the
same Undo step. Export adds `Dynamic Range` (`SDR`, `HDR · PQ`, `HDR · HLG`) for ProRes and MP4
when the project is HDR. View adds `Viewer Output ▸ HDR EDR`, `Apply LUT…` and `Interpret
Selection As ▸`. Behaviour: [hdr.md](../../../engine/hdr.md).

## N004 Scopes panel

Dockable panel `scopes`, title `Scopes`, not in any default workspace. `Window ▸ Scopes`.

```
┌ Scopes ──────────── [Waveform ⌄] [⋯] ┐ header
│┌────────────────────────────────────┐│
││ 100 ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄ ││ plot, c.viewer
││  50 ┄┄┄┄ density trace ┄┄┄┄┄┄┄┄┄┄ ││
││   0 ┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄┄ ││
│└────────────────────────────────────┘│
│ Output · Rec. 709     Over 0.2 %     │ footer, t.minimum
└───────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Scope pop-up | `Histogram`, `Waveform`, `RGB Parade`, `Vectorscope`, AX `Scope` |
| More menu | `Analysis Stage ▸` `Working Color`, `Output` · `Matrix ▸` `Rec. 709`, `Rec. 2020` · `Range ▸` `Standard`, `Extended` |
| Plot | fills the panel, `m.control_radius`, solid `c.viewer` in both appearances. No glass |
| Density | sample counts mapped to brightness, log scale, in dark-scheme `c.text` for luma |
| Histogram | R, G, B overlaid in `c.scope_*` plus luma outline |
| RGB Parade | three planes side by side, `c.scope_*`, `R`, `G`, `B` labels `t.minimum` |
| Vectorscope | square plot, Cb horizontal, Cr vertical; centre cross and circle in `c.separator` dark-scheme value |
| Graticule | hairlines dark-scheme `c.separator`; labels `t.minimum` tabular dark-scheme `c.muted`: 0, 25, 50, 75, 100 |
| Footer | `{stage} · {matrix}`; trailing, only when non-zero, `Under {n} %`, `Over {n} %`, or for the vectorscope `Outside {n} %` |

**Stage is always stated.** The model refuses to guess, so the footer always names what
is being measured. Default: `Output`, `Rec. 709`, `Standard`. A Rec. 2020 project defaults
to `Rec. 2020`. Settings are per panel instance, and saved with the workspace.

**HDR.** `Range ▸ Extended` reads BT.2100 PQ, so the axis carries cd/m² up to 10000, with a
line at 203 labelled `SDR White`. There is no tone mapping in the scope: values
above range stay counted, and appear in `Over`. Extended is disabled with tooltip `This
project has no values above SDR.` for 8-bit projects. An HDR project defaults to Extended
and `Rec. 2020`.

| State | Spec |
|---|---|
| No project | `EmptyState` `waveform.path.ecg`, `No Picture`, `Open a composition to see its scopes.` |
| Paused | analyses the frame at the playhead |
| Playing | analyses the latest frame at most 10 times a second; dropped frames are skipped, and the panel never slows playback |
| Waiting | plot keeps the previous result; no spinner under 0.5 s |
| Hidden | analysis stops |
| Can't read frame | Error centred in the plot, dark-scheme: `Scopes can’t read this frame.` Plot clears |
| Non-finite pixels | same Error, `This frame has invalid values.`, with detail in the Error Log |

Keyboard: ⌃Tab reaches the panel; pop-up then menu. ⌘1 to ⌘4 aren't bound: those belong
to the shell. VoiceOver: plot is an image labelled `{scope} of the current frame`, with
value `Peak {n} percent, minimum {n} percent`, updated only when focus lands, never live.
Differentiate Without Colour: RGB Parade's lettered planes and the Histogram toggle (`R`,
`G`, `B` segmented, in More) let each channel be read alone. Reduce Motion: scopes keep
updating; they're content. Increase Contrast: graticule to `m.border`.

## N004 LUTs

A LUT is an effect, so it appears in the Effects browser and Effect Controls.

| Row | Control | Notes |
|---|---|---|
| `LUT File` | value-column button with the file name, else `Choose…` | `NSOpenPanel` for `.cube`; pop-up adds `Reveal in Finder`, `Remove` |
| `Input Range` | read-only text `{min} to {max}` | from the file's domain |
| `Clamped` line | `t.minimum` `c.muted` | shown when the file's domain clips input: `Values outside this range are clamped before the LUT.` |

| State | Spec |
|---|---|
| No file | effect passes through; row `Choose…`; label in `c.muted` |
| Unreadable | Error under the row `This LUT couldn’t be read. {parser reason}` |
| Missing | S006 Offline wording: `{file} is offline.` + `Relink…`; effect passes through |
| Size limit | Error `This LUT is too large.` |

The LUT bytes are embedded in the project's media records, so moving a project keeps it.

Advanced colour inspection beyond scopes isn't designed: nothing in the model needs it.

## Don't

- Don't tone map scope input or hide what stage is measured.
- Don't reuse label colours for traces.
- Don't let scopes slow playback.
