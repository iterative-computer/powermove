# Pro editing tools

Row N006. Grammar: [README](README.md) §1–5. Contract:
[pro-editing.md](../../pro-editing.md), W079–W083, model
`pm-kernel::commands::{pro_transport, pro_edit, pro_source, pro_links}`. Drawing not
restated here stays in [timeline.md](../panels/timeline.md) and
[viewer-chrome.md](../panels/viewer-chrome.md). These tools are native additions; v1.2.0
has none.

**Shortcuts.** No new binding shadows a w1-keymap row. `J`, `K`, `L` keep their fall-through
from W079. `I`, `O`, `,`, `.` are taken by shipped commands, so this file uses the free
`⌥` chords below.

Two new tokens for the pm-design lane:

| Token | Value | Role |
|---|---|---|
| `metrics.mark_bracket` | 6 (= `status_dot`) | Width of In and Out brackets on rulers and scrubbers |
| `metrics.sync_badge` | 16 (= `track_row − 12`) | Linked-clip sync badge height |

## Shuttle and Play Around

| Key | Result |
|---|---|
| `J`, `L` during playback | reverse or forward 1×, a second press 2× (capped); switching direction starts at 1× |
| `K` during playback | stop, snapped to a frame |
| `J`, `K`, `L` while paused or unavailable | decline: shipped event navigation and property reveal run once |
| `⌃J` | reverse from paused |
| `⌃⇧Space` | Play Around: 2 s before and after the playhead, then return |

**Indicator.** After the timecode in the timeline header: a rate readout `t.timecode`
tabular `c.secondary`, `◀◀ 2×`, `◀ 1×`, `▶▶ 2×`, shown only when the rate isn't +1×, in a
fixed slot so the header doesn't shift. AX `Playing in reverse at 2 times speed`, posted
`.medium` on change. Under Focus or full screen the indicator joins the overlay capsule.

Menu: `Timeline ▸ Reverse`, `Timeline ▸ Play Around Current Frame`. Play Around's
range shows as a `c.accent_wash` band in the ruler while it plays, and clears on return.

## Timeline edit tools

Edit Tool pop-up in the timeline header, Tracks mode only, before Snap; `t.minimum`, fixed
width, tooltip `Edit Tool`, AX `Edit tool, {name}`. Also `Timeline ▸ Edit Tool`.

| Tool | Key | Cursor | Dragging an edge or clip |
|---|---|---|---|
| `Trim` (default) | `⌥V` | existing | existing trim, no shifting |
| `Ripple` | `⌥R` | `resizeLeftRight` + ripple glyph | shifts everything downstream on targeted tracks |
| `Roll` | `⌥L` | same + roll glyph | moves the cut between two clips |
| `Slide` | `⌥S` | same + slide glyph | moves the clip; neighbours compensate |
| `Slip` | `⌥Y` | same + slip glyph | changes the source inside a fixed clip |

Tool cursors are custom symbols on the SF template grid (language §8). The tool stays until
changed; Esc during a drag cancels it. A hold-to-temporarily-use mode isn't included.

**During a drag.** Affected clips draw a `m.drop_stroke` `c.accent` outer ring, with a
`t.minimum` tabular capsule above the pointer, dark-scheme, reading the signed delta
`+00:00:00:12`. Slip adds a two-frame preview in the viewer: the new In and Out frames
side by side, dark-scheme labels `In`, `Out`. Roll and Slide outline both neighbours. On
release one undo step: `Ripple Trim`, `Roll Edit`, `Slide Edit`, `Slip Edit`.

**Ripple Delete.** `Edit ▸ Ripple Delete`, `⇧⌫`. One undo step.

**Refusals** are atomic: nothing moves. Viewer banner, plus the pointer snaps back. Handles
are never quietly clamped.

| Condition | Copy |
|---|---|
| Handle exhausted | `There’s no more media in this clip to extend it.` |
| One-frame minimum | `A clip must stay at least one frame long.` |
| Locked clip in the way | `A locked clip is in the way.` |
| Past composition end | `This would go past the end of the composition. Extend it first.` |
| Not adjacent | `Roll needs two clips that touch.` / `Slide needs a clip with a neighbour on each side.` |
| Clip intersects interval, ambiguous gap | `Ripple Delete can’t work out what should move. Select clips next to each other.` |
| Time remap or source keys | `This edit doesn’t work on a clip with time remapping or animated source time.` |
| Unknown media length | `Powermove doesn’t know this clip’s media length yet.` |
| Broken link | `This clip’s link is damaged. Unlink it first.` |

## Linked audio and video

| Item | Spec |
|---|---|
| Linked Selection | header toggle next to Snap, `link` glyph, on `c.accent` (active-state rule), tooltip `Linked Selection`. On by default |
| Bypass | hold `⌥` while clicking or dragging: acts on one half only |
| Commands | `Timeline ▸ Link Audio and Video` (one video, one audio selected), `Timeline ▸ Unlink Audio and Video` |
| Linked clip | small `link` glyph after the name at `m.symbol_inline`, `c.secondary` |
| Out of sync | capsule at the clip's leading end, `m.sync_badge` tall, `c.media_badge` fill, `t.minimum` tabular dark-scheme `c.warning` text with the signed offset at composition rate `+00:00:00:04`. Both halves show it, with opposite signs |
| Sync badge menu | `Move into Sync`, `Slip into Sync` |

Unknown links never disappear silently: they report the Broken link refusal. VoiceOver appends
`linked, out of sync by {n} frames` to the clip's label. Link identity is independent of
Parent and Group, and the UI never calls it either.

## Source monitor

Extends viewer-chrome's Source mode. Applies to video and audio sources.

| Control | Key | Spec |
|---|---|---|
| Mark In, Mark Out | `⌥I`, `⌥O` | in Source mode sets source marks, in Composition mode timeline marks |
| Clear In, Clear Out | `⌥⇧I`, `⌥⇧O` | same scope |
| Insert | `⌥,` | ripples targeted tracks |
| Overwrite | `⌥.` | replaces, no ripple |

Menu: `Timeline ▸ Mark In`, and so on. Keys are free chords, and work with the viewer or timeline
focused.

**Marks on the source transport**: `m.mark_bracket` brackets on the scrubber in dark-scheme
`c.accent`, band between in `c.accent_wash` dark value. **On the timeline ruler**: the same
brackets in `c.accent`, distinct from the work area because they're brackets, not a bed.

**Source bar additions** (Source mode, viewer bar):

| Item | Spec |
|---|---|
| Read-out | after the asset name, `t.minimum` tabular `c.muted`: `In 00:00:01:00 · Out 00:00:03:00 · 2.00 s` |
| Targets | pop-up `Targets`: `Video` submenu of video tracks and `None`; `Audio` likewise. Title reads `V1 · A1`, `V1`, or `A1`. Video without embedded audio has `Audio` disabled |
| Insert, Overwrite | borderless buttons with `arrow.right.to.line` and `square.and.pencil`; added to the transport capsule in Focus mode |

**Three-point resolution.** Status follows the marks, in the read-out, so the result is
known before committing:

| Marks | Read-out |
|---|---|
| Fewer than needed | `Set one more mark.` |
| Source In and Out, no timeline marks | `{duration} at the playhead` |
| Three or four consistent | `{duration} into {track} at {timecode}` |
| Durations disagree | `The marks don’t match in length.` (Error; buttons disabled) |

No fit-to-fill and no rate change, ever. Resolved edits are one undo step, `Insert` or
`Overwrite`, and leave the playhead at the end of the edit, with source marks cleared.

Refusals, viewer banner, atomic: `Choose a track for this edit.`, `This video has no
audio.`, `The target track is locked.`, `Linked clips on both tracks must be targeted.`,
`Powermove doesn’t know this source’s length yet.`, `This would go past the end of the
composition. Extend it first.`

VoiceOver: marks announce `In set at {timecode}`; targets read as pop-ups; refusals `.high`.
Keyboard: all commands from the menu. Reduce Motion: playback previews still play; drag
rings appear without spring.

## Don't

- Don't bind plain `I`, `O`, `,` or `.`, or break the `J`/`K`/`L` fall-through.
- Don't clamp a refused edit into a partial result.
- Don't use the accent for an out-of-sync warning.
- Don't add fit-to-fill.
