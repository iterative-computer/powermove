# Mixer and 3D

Rows S097 S098. Grammar: [README](README.md) §1–5. Sources:
`R/panels/MixerPanel.svelte`, `R/mixer/{MixerStrip,Fader,LevelReadout}.svelte`,
`R/mixer/{levels,strips,level-control}.ts`. Timeline waveforms are not part of this
file. 3D is in [3d-studio.md](3d-studio.md).

Two new tokens, listed here for the pm-design lane:

| Token | Value | Role |
|---|---|---|
| `metrics.channel_strip` | 72 (= `min_panel`) | Mixer channel strip width |
| `metrics.fader_knob` | 14 (= `keyframe + 5`, rounded to the grid) | Fader knob height |

## S097 Mixer panel

Dockable panel `mixer`, title `Mixer`, default extent 300 pt, minimum 150 pt, no
panel scroll: strips scroll horizontally inside it.

```
┌ Mixer ─────────────────────────────────────────┐
│ ┌Master┐│┌──────┐┌──────┐┌──────┐               │ strips scroll; Master pinned
│ │  ◇   ││ │  ◇   ││  ◇   ││  ◇   │              │ level keyframe
│ │-3.0  ││ │ 0.0  ││-6.0  ││-inf  │              │ readout
│ │ ▐▌ ▮ ││ │ ▐▌ ▮ ││ ▐▌ ▮ ││ ▐▌ ▮ │              │ meters, scale, fader
│ │  ●   ││ │  ●   ││  ●   ││  ●   │              │ clip indicator
│ │ M S  ││ │ M S  ││ M S  ││ M S  │              │ mute, solo
│ │Master││ │Music ││Voice ││Foley │              │ name
│ └──────┘│└──────┘└──────┘└──────┘               │
└────────────────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Strip | width `m.channel_strip`, `c.panel_secondary`, `m.control_radius`, gap `m.gap`. Selected strip (selected layer): name on `c.selected_fill` |
| Master | pinned at the leading edge, hairline `c.separator` after it, no Solo. It writes the composition output level |
| Level keyframe | `m.keyframe` diamond, inspector semantics; AX `Add {name} level keyframe` / `Remove {name} level keyframe`, tooltip `Add Keyframe` / `Remove Keyframe`. None on Master |
| Readout | `NumberField`, `t.body` tabular, `dB` in `c.muted`, fixed width. `-inf` for silence. Tooltip `Drag to adjust · Click to type`. Accepts `-6`, `+2.5`, `-inf`. AX `{name} level in decibels` |
| Meter | stereo pair, each `m.gap` wide. Scale `c.success` below −12 dB, `c.warning` −12 to 0 dB, `c.danger` above 0 dB; `c.field` track; peak-hold tick 1 pt `c.text`. Moves with audio, including under Reduce Motion |
| Scale | ticks and labels between meter and fader at the console-law marks +12, +6, 0, −6, −12, −20, … −60: `t.minimum` tabular `c.muted`; 0 in `c.text` |
| Fader | custom GPUI control, track 2 pt `c.separator`, knob `m.hit` × `m.fader_knob`, `c.floating`, `m.inner_radius`, `m.border` `c.separator`, centre line `c.text`. Pointer target is the full column. Range −100 to +12 dB |
| Clip indicator | `m.status_dot` circle; off `c.field`; on `c.danger` plus `exclamationmark.triangle.fill` over it at `m.symbol_inline`. Tooltip `Clip Indicator`; on: `Clipped · Click to Clear`. AX `{name} clipped. Clear` / `{name} has not clipped` |
| Mute, Solo | `m.hit` square toggles lettered `M` and `S`, `t.body` `t.semibold_weight`. Mute on: `c.selected_fill`. Solo on: `c.selected_fill` + `c.warning` letter. Letter shape carries the state as well, never colour alone |
| Name | `t.minimum`, truncates with a tooltip; click selects the layer |

v1.2.0 has no pan control; none is added. The inventory's *pan* refers to nothing
in the shipped strip.

| State | Spec |
|---|---|
| Empty | `EmptyState` `speaker.slash`, title `No Audio`, line `No audio in this composition`. Master is hidden |
| Quiet strip | meter and name drop to `c.muted` with tooltip naming why: `Muted`, `Switched off on the timeline`, `Time remapping mutes the sound`, `Silenced by a soloed layer`, `Silenced by solo` |
| Locked layer | readout, fader and diamond disabled; meter still moves |
| Animated level | fader knob follows the live value during playback; dragging writes at the playhead |
| Animated mute | Mute tooltip `Mute is animated on the timeline`; click still toggles at the playhead |
| Edit refused | viewer banner, once per gesture: the refusal, or `Couldn’t change the level` |
| Overflow | panels/README overlay scroller; leading and trailing edges gain a 1 pt `c.separator` line when strips are clipped |

Gestures: drag the knob or anywhere on the rail (one undo step named for the level).
⌥-drag fine. Double-click the rail returns to 0 dB as one undo step. Solo click adds
to the solo set; ⌥-click hears only this strip. Solo is monitoring only: it isn't
saved to the project and clears when the Mixer closes or the strip leaves.

Keyboard on a focused fader: ↑/↓ ±0.5 dB, PageUp/PageDown ±6 dB; ⌥ ÷5, ⇧ ×6. Result
rounds to 0.1 dB. Tab order per strip: diamond, readout, fader, clip, Mute, Solo,
name. Strips are reached left to right; Master first.

VoiceOver: each strip is an `AXGroup` named for the layer. The fader is `AXSlider`,
label `{name} level`, value `{n} dB`, range −100 to 12, with increment and decrement
actions. Mute and Solo are toggle buttons, `Mute {name}`, `Solo {name}`, and the Solo
hint `Option-click to hear only this`. Meters are not exposed as live regions;
clipping posts one `.medium` announcement `{name} clipped`.

Differentiate Without Colour: the clip glyph and the numeric readout carry clipping;
meters add a 1 pt `c.text` tick at 0 dB. Increase Contrast: strips draw their edge.

## S098 3D

The old 3D surfaces are retired. The new 3D system ships with the rewrite, and its whole
spec is [3d-studio.md](3d-studio.md): viewer toolbar, preview tiers, navigation, gizmos, light
tool, inspector, import, materials, progress and failure.

Old projects upgrade automatically when opened, with no compatibility mode
([3d-studio.md §9](3d-studio.md#9-upgrading-old-projects)). A lane builds none of the
earlier controls: not the `Global`/`Local` or `Camera`/`Editor` segments, the `3D view`
pop-up, or `Intensity` and `Cone Angle` as labels.

## Don't

- Don't add pan, sends, EQ or inserts to the Mixer.
- Don't move the zoom or quality pop-ups to make room for 3D controls.
