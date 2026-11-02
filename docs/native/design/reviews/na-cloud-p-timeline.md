# Design review: `feat/na-cloud-p-timeline`

Reviewer: design lead · 2026-11-01 · Lane head `4b88b53b6` (diff vs
`origin/feat/native-engine`). Read against [panels/timeline.md](../panels/timeline.md),
[panels/README.md](../panels/README.md) (owner decision 1: `m.layer_column` 266),
[ui-kit.md](../../ui-kit.md), [review-checklist.md](../review-checklist.md) and
`panels/timeline/test/__snapshots__/wire.test.tsx.snap`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-timeline` (`0479b8f74`), not the lane head.** That
branch is the lane plus the **Fixed** items, merged cleanly with
`origin/feat/native-engine`. `bun test panels/timeline` passes on macOS (54/54,
snapshots regenerated). TypeScript and Prettier are clean. This is the
skeleton of the timeline, not the timeline: ruler, clips, keys, graph and
playhead are host-painted (T4). Visual acceptance of the track area waits for
that renderer and a screenshot review.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §2 header order | Play (`c.text`, `Play / Pause (Space)`), time, rate, tabs, mode, view, graph options (only off Dope Sheet), snap | APPROVE |
| 2 | §2 header layout | Leading and trailing items were one run with a `flexible-spacer` placeholder; no trailing inset | **Fixed**: leading cluster + trailing cluster (`justifyContent: space-between`, which the validator accepts); trailing `m.inset` (optical rule F6) |
| 3 | §2 current time | `t.timecode` medium tabular `c.text`, neutral; click → field; spoken value | APPROVE (fixed width T2) |
| 4 | §2 frame rate | `t.minimum` `c.muted` | APPROVE |
| 5 | §2 separator | The hairline between rate and tabs is missing | CHANGES REQUIRED (F3) |
| 6 | §2 tabs | `Open compositions` tablist, roving stop, active `c.text` / others `c.secondary`, close only with > 1 tab and at the leading edge | APPROVE (underline/scroll T3) |
| 7 | §2 tab close | AX label was the composition name, the same as the tab | **Fixed** (T6): `Close {composition}`, tooltip `Close {composition} (⌘W)` |
| 8 | §2 mode / view | `Timeline mode` `Layers · Tracks` at `t.minimum` medium; `Timeline view` three labelled segments, not accent-filled | APPROVE (icon segments T1) |
| 9 | §2 snap | Toggle `Snapping`, active state, tooltip | APPROVE (detail line `Hold Shift to invert`, F1) |
| 10 | §2 layer row | Index/disclosure/name and the switches were one run; switches weren't at the trailing inset | **Fixed**: leading cluster (index, disclosure, name) + trailing switch cluster |
| 11 | §2 disclosure | Labelled with the bare layer name | **Fixed**: `Expand {layer}` / `Collapse {layer}` |
| 12 | §2 switches | Order Parent, Solo, Lock, Visibility (trailing→leading Visibility first); shown when set or on hover/focus; solo active; Parent tooltip copy exact | APPROVE |
| 13 | §2 kind badge | No badge at all (not even a placeholder) | CHANGES REQUIRED (F2) |
| 14 | §2 property row | The diamond was always `diamond` | **Fixed**: `diamond` / `diamond.lefthalf.filled` / `diamond.fill` (static / animated / key at playhead), same as Properties |
| 15 | §2 property row | ‹ › are omitted for unanimated properties, so the label shifts. The spec says "hidden, space kept" | CHANGES REQUIRED (F4) |
| 16 | §2 readout | Single-axis `NumberField`; compound Scale is one semantic readout + link toggle with the right tooltips | APPROVE (T5) |
| 17 | §2 track rows | Name leading, Visibility/Mute + Lock trailing | **Fixed** (clusters); glyph set correct |
| 18 | §2 splitter | Focusable separator labelled `Timeline` | **Fixed**: `Layer column`; width token `m.layer_column` (266) |
| 19 | §3 empty states | `No Composition` + `Open…`; empty composition text + `Import…`; `Select an animated property to see its curve.` | APPROVE (symbol `film.stack` via T1) |
| 20 | §3 multi-select / playing | Selection count text; rows memoised and unchanged during playback (W055) | APPROVE |
| 21 | §6 VoiceOver | Panel `Timeline`, ruler slider `Playhead` with spoken time, render-bar summary, tree with total row count metadata | APPROVE (AXRowCount T4) |
| 22 | Checklist §1, §7 | Tokens only; no accent fills; copy verbatim | APPROVE |

## DESIGN-TODO(opus) answers

- **T1–T5** are kit/host gaps for already-decided design, as the lane says.
  They are confirmed as follow-ups below. One correction to T4: flexible
  spacers are **not** needed. A row is a leading cluster and a trailing cluster
  under `justifyContent: "space-between"` (validator-accepted). That is now
  how the header and every row are built. Use the same pattern everywhere a
  spec says "trailing".
- **T6: close-tab label.** VoiceOver `Close {composition}`; tooltip
  `Close {composition} (⌘W)`. Added to timeline.md §6/§8. Done.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F1 | kit | Symbol slots for `ToolbarItem` and segmented segments (play/pause, `xmark`, view icons incl. custom `bezier`, `magnet`, switch glyphs, chevrons, diamonds); two-line tooltips (snap detail). Shared with every panel. |
| F2 | pm-design + kit | `metrics.kind_badge` (18) square badge with the layer label colour and the kind glyph (`textformat`, `film`, …, table in §2). |
| F3 | timeline lane | Header separator: `m.border` `c.separator` vertical hairline, `m.gap` either side, only when tabs follow, height `m.symbol_panel` (13 pt, the timecode size; replaces "1 × line-height" in §2 so no new token is needed). |
| F4 | kit | `visibility: hidden` (layout kept, not painted, not focusable, not in AX) so ‹ › keep their space on unanimated properties; the label column then starts at `m.inset + 3 × m.hit + m.gap` (token arithmetic). |
| F5 | kit/host | T2 fixed-width timecode field + parser; T3 tab recipe (underline `c.accent_text` 2 pt, leading close, horizontal scroll with the `c.panel` edge fade); T5 dense Scale well with axis hit-mapping and width-aware hiding. |
| F6 | kit | Header trailing optical inset `m.inset − (m.hit − m.symbol_panel) ÷ 2` (token arithmetic, all panels). |
| F7 | host | The whole §2 track-area/ruler/graph renderer (T4), AXRowCount and virtual row navigation, announcements. Then a screenshot review of light/dark/Increase Contrast before the timeline is accepted. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-timeline | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-timeline` `0479b8f74`; F1–F7 open |
