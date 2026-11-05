# Timeline

Panel id `timeline`, title **Timeline**, bottom dock under the left dock and
viewer (default height `pm_design::workspace::DEFAULT_TIMELINE`). Shared
rules: [README.md](README.md). Look: [language.md §11](../language.md#11-timeline).

Behaviour source (wins on behaviour and keys):
`v1.2.0:apps/desktop/src/extensions/timeline/` (`timeline.ts`, `index.ts`,
`comp-tabs.ts`, `markers.ts`, `property-reveal.ts`, `property-values.ts`,
`keyframe-navigation.ts`, `directional-snap.ts`, `track-view.ts`,
`graph-*.ts`). Workflows W051–W062, W079–W083 in
[w1-workflows.md](../../w1-workflows.md).

## 1. Model contract (proposed names)

| State | Values | Notes |
|---|---|---|
| `TimelineMode` | `Layers` (After Effects) · `Tracks` (Premiere) | persisted per project |
| `TimelineView` | `DopeSheet` · `DopeSheetAndGraph` · `Graph` | v1.2.0 three view buttons |
| `GraphType` | `Value` · `Speed` | graph options menu |
| `graph_merge` | bool | "Merge Selected Curves" |
| `snap` | bool | magnet; Shift inverts during a gesture |
| `open_comps` / `active_comp` | ids | composition tabs |
| `rows` | `Layer{id, depth, open}` · `Property{layer, path, axes}` · `Track{id, kind}` | flattened, virtualised |
| `selection` | `layers: Set`, `keys: Set<(layer, path, key)>`, `clips: Set` | ids, not row flags |
| `playhead` | frame (composition time base) | |
| `work_area` | `(in, out)` frames, ≥ 1 frame | |
| `markers` | `{id, frame, name}` | |
| `render_bar` | segments of `BarState` `Missing` · `Rendering` · `Rendered` · `Kept` · `Stale` | see promedia review |
| `viewport` | `scroll_time`, `pps` (points per second, clamp 4…4000), `scroll_y`, `layer_column_width` | persisted per project |
| `columns` | `show_layer_numbers`, `show_switches` (workspace `chrome.timeline`, as main) | Columns menu |
| `gesture` | `None` · `Scrub` · `MoveClips` · `Trim{edge}` · `MoveKeys` · `Marquee` · `Stagger` · `PickWhip` · `WorkArea{handle}` · `Pan` | drives cursor + live preview |

## 2. Anatomy

```
┌ Timeline panel ────────────────────────────────────────────────────────────────────┐
│ ▶ 00:00:04:15  30 fps │ Launch Film × │ Endcard × │      [Layers|Tracks] [▤|▤∿|∿] ⋯ ⊓ │ header  m.header
├──────────────────────────┬─────────────────────────────────────────────────────────┤
│ (ruler gutter)           │ 0:00   0:01   0:02 ▼ 0:03          ruler m.ruler        │
│                          │▔▔▔▔render bar 3 pt▔▔▔▔                                   │
├──────────────────────────┼──────────────────────────┊──────────────────────────────┤
│ 01 [T] Kicker        👁   │   ▭▭▭▭ Kicker ▭▭▭▭▭▭▭▭▭▭▭┊◆                              │ layer row m.track_row
│ 02 [T] Headline   ⛓  👁   │ ▭▭▭▭▭ Headline ▭▭▭▭▭▭▭▭▭▭┊▭▭▭▭▭                          │ (selected: ring)
│   ‹ ◆ ›  Scale  100, 100 % │ ◆   ◆   ◆                ┊       ◆                      │ property row m.property_row
│   ‹ ◆ ›  Opacity    100 %  │ ◆   ◆                    ┊                              │
│ 03 [◎] Orb           👁   │      ▭▭▭▭ Orb ▭▭▭▭▭▭▭▭▭▭▭┊▭▭▭                            │
└──────────────────────────┴──────────────────────────┊──────────────────────────────┘
  layer column (c.panel_secondary)                  playhead (c.accent)
```

### Header (`m.header`)

Left to right; leading inset `m.inset`, items separated by `m.gap`.

| Element | Spec |
|---|---|
| Play/Pause button | borderless icon button, `play.fill` / `pause.fill` at `m.symbol_panel`, `c.text` (not secondary: it is the primary control here). Tooltip `Play / Pause (Space)`. |
| Current time | `t.timecode` (13, `t.medium_weight`, tabular), `c.text`, format `HH:MM:SS:FF` at the composition rate. **Neutral** (language §17.2). Fixed width = width of `00:00:00:00` in that font, so it never reflows. Click → becomes a text field (see §5 *Go to time*). |
| Frame rate | `t.minimum`, `c.muted`, `30 fps` / `29.97 fps` (integer rates without decimals, others to 2 decimals). |
| Separator | `m.symbol_panel`-tall vertical hairline, `m.border` `c.separator`, `m.gap` either side. Shown only when tabs follow. |
| Composition tabs | One tab per open composition (v1.2.0 `comp-tabs.ts`). Tab: label `t.body`, `c.secondary`; active tab `c.text` with a 2 pt `c.accent_text` underline inset `m.inset − m.inner_radius` from each tab edge, bottom-aligned to the header. Height `m.header`. Max tab width `m.toolbar_title_width + m.hit + m.inset` (196 pt); longer names truncate with `…` and a tooltip. Close glyph `xmark` at `m.symbol_inline` at the **leading** edge of the tab (Mac convention), hit `m.hit`, shown on hover, on the active tab and on keyboard focus; hidden (and not focusable) when only one composition is open. Tabs scroll horizontally (no scrollbar; overflow fades with a 16 pt `c.panel` gradient edge — the only gradient allowed in the timeline). |
| Spacer | flexible |
| Mode control | GPUI segmented control (README §2 tab group recipe), labels `Layers` · `Tracks`, `t.minimum` `t.medium_weight`. Tooltips `Use layer timeline (After Effects)` / `Use track timeline (Premiere)`. |
| View control | GPUI segmented control, three icon segments at `m.symbol_inline`: Dope Sheet (`chart.bar.doc.horizontal`), Dope Sheet and Graph (`rectangle.split.1x2`), Graph Editor (custom `bezier` symbol). Selected segment per README; **not** an accent fill (v1.2.0 filled the active view in accent — dropped, accent is a mark). Tooltips `Dope Sheet`, `Dope Sheet and Graph`, `Graph Editor (⇧F3)`. |
| Graph options | borderless icon button `ellipsis.circle`, shown only when view ≠ Dope Sheet. Opens an `NSMenu`: `Value Graph` ✓ / `Speed Graph` ✓ (radio), separator, `Merge Selected Curves` ✓ (disabled with < 2 curves), `Fit Selected Curves` (disabled with no graph). Tooltip `Graph Options`. |
| Snap | borderless toggle button, custom `magnet` symbol at `m.symbol_panel`; off `c.secondary`, on `c.accent` (active-state glyph rule, language §8). Tooltip `Snap Clips to Edges, Markers and the Playhead` + detail line `Hold Shift to invert`. |
| Trailing inset | README §2 trailing rule |

### Layer column (`m.layer_column` default, `c.panel_secondary`)

Resizable: drag its trailing edge (the column/track boundary, hit `m.hit`
centred on it, cursor `resizeLeftRight`); min `m.layer_column`, max 50 % of the
panel width; double-click the edge resets to `m.layer_column`. Width persists
per project. Keyboard: the edge is a focusable splitter (`←`/`→` by
`m.keyboard_step`, `Return` resets).

The top `m.ruler` of the layer column (the **ruler gutter**) is
`c.panel_secondary` (v1.2.0 placed transport here; native moved it to the
header). Dragging in the gutter does nothing; it is not a scrub area.

**Column headings** sit in the gutter in Layers mode, so main's heading strip adds no
height. In Tracks mode the gutter stays empty.

| Heading | Spec |
|---|---|
| `Layer` | `t.minimum` `t.medium_weight` `c.muted`, at the Name column's x |
| Visibility | `eye` at `m.symbol_inline` `c.muted`, centred over the Visibility slot |
| `Parent` | Same text style, at the Parent column's x, only while that column shows |

Right-click the gutter for the **Columns** `NSMenu`: `Layer Name` ✓ (disabled), `Layer Numbers`
✓, `Switches` ✓, `Parent` ✓ (disabled), separator, `Show All Columns`. Hiding `Switches` hides
every switch in Layers mode except a set one.

**Layer row** (`m.track_row`):

| Column | Spec |
|---|---|
| Leading inset | `m.inset` |
| Index | `t.minimum`, tabular, `c.muted`, right-aligned in a column whose width = width of `9` × (digits of the largest index) in that font. Group children show no index. |
| Gap | `m.gap` |
| Depth indent (Layers mode, grouped layers) | `depth × m.tree_indent`, inserted here |
| Disclosure | `chevron.right` at `m.symbol_inline`, `c.secondary`, rotates 90° open; only on rows that can open (groups, or layers with revealed/keyframed properties); hit `m.hit` overlapping the index/badge columns is allowed because neither is interactive. |
| Kind badge | `m.kind_badge` square, `m.inner_radius`, fill = the layer's label colour `c.labels.*`, glyph at `m.symbol_inline` in `c.text`: text `textformat`, shape `square.on.circle`, video `film`, image `photo`, audio `waveform`, adjustment `slider.horizontal.3`, null `circle.dashed`, camera `camera`, group `square.stack`, precomp `rectangle.stack`, captions `captions.bubble`, solid `square.fill`, shader/extension `wand.and.rays`. |
| Gap | `m.gap` |
| Name | `t.body`, `c.text`, truncating tail with `…` and a tooltip of the full name. |
| Trailing switches | right-aligned at `m.inset` from the column edge, each in an `m.hit` square, glyph `m.symbol_inline`, in fixed slots. Order trailing→leading: **Visibility** (`eye` / `eye.slash`), **Lock** (`lock` / `lock.open`), **Solo** (`s.circle`), **Parent** (`link`, transformable layers only), **Audio** (`speaker.wave.2` / `speaker.slash`, only a video with sound or a precomp that carries sound), **3D** (`cube` / `cube.fill`; not audio, adjustment or always-3D layers), **Raster** (`sun.max` / `sun.max.fill`, precomps only, `Continuously Rasterize`). A switch is drawn when its state is non-default or when the row is hovered/focused (default-state glyphs then appear in `c.muted`). Set states draw in `c.secondary`; solo set draws `c.accent` (it is an active state that changes what plays). Audio, 3D and Raster show their default state on hover only when the column is at least `m.layer_column + 3 × m.hit`; below that they appear only when set, and stay reachable from the layer menu and Properties. Empty slots give their width to the name. |
| Parent column | From `m.layer_column + m.value_column` wide, the Parent slot becomes a column of width `m.value_column`: `link` glyph (pick whip) then the parent's name or `None`, `t.minimum` `c.secondary`, truncating. It sits in the same place in the slot order. |

**Property row** (`m.property_row`):

| Column | Spec |
|---|---|
| Previous keyframe | `chevron.left` at `m.symbol_inline`, `m.hit` square starting at `m.inset`. `c.secondary`; `c.muted` + disabled when no earlier key. Shown only on row hover or focus, as main; space kept. Hidden when the property has no keyframes. Tooltip `Previous Keyframe (⇧J)`. |
| Keyframe toggle | custom `diamond` at `m.keyframe`, next `m.hit` square. Outline `c.muted` = not animated; filled `c.accent` = key at playhead; half-filled `c.accent` = animated, no key at playhead (same semantics as Properties). Tooltip `Add Keyframe` / `Remove Keyframe`. |
| Next keyframe | `chevron.right`, next `m.hit` square, rules as previous. Tooltip `Next Keyframe (⇧K)`. |
| Section heading row | `m.property_row`, as main's `property-sections.ts`: `Content`, `Transform`, each effect's name, the mask's name, `Layer`. `t.body` `t.medium_weight` `c.secondary` at the label's x. No diamond, no value, not a focus stop. |
| Gap | `m.gap` |
| Label | starts at `m.inset + 3 × m.hit + m.gap` (plus `depth × m.tree_indent`); `t.body`, `c.secondary`, truncating, tooltip = full property path (`Transform ▸ Scale`). |
| Value readout | trailing, right edge at `m.inset` from the column edge (Scale: link glyph occupies the last `m.hit`, readout before it). One `m.field`-high well, `c.field`, `m.control_radius`, padding `m.inset − m.gap` (6) each side, `t.body` tabular `c.text`, unit `c.muted` after a space. Axes join with `, ` (`960, 540 px`, `100, 100 %`). Width fits content, min `m.value_column ÷ 2`, max `m.value_column`. **Shown only when the label keeps ≥ `m.value_column ÷ 2 − m.inset` points** (≈ 52); otherwise hidden — the value stays editable in Properties — and shown again when the column widens. |
| Scale link | `link` glyph, `m.hit` square; linked `link` in `c.secondary`; unlinked custom `link.slash` (same grid as `link`) in `c.muted`. Tooltip `Lock Aspect Ratio` / `Unlock Aspect Ratio`. |

**Track header row** (Tracks mode, `m.track_row`): track name `V1`, `V2`,
`A1` … in `t.body` `t.medium_weight` `c.text` at `m.inset`; trailing switches
Visibility (video) / Mute (`speaker.wave.2` / `speaker.slash`, audio) and Lock.
No index, no badge (language §15: track names stay).

### Ruler (`m.ruler`)

- Background `c.panel` (the track area colour), bottom hairline `m.border`
  `c.separator` across the track area only.
- Major ticks: 7 pt lines `c.secondary` at labelled times; labels `t.minimum`
  tabular `c.muted`, 4 pt right of the tick, baseline 4 pt below the ruler top
  + cap height. Label format: `0:05` under a minute, `1:05` beyond, frames
  (`0:05:12`) only when one frame ≥ 40 pt.
- Minor ticks: 3 pt `c.separator`, at the largest of {1 frame, ¼ s, 1 s, 5 s}
  that keeps ≥ 8 pt between ticks. Label spacing ≥ 60 pt.
- **Work area**: `c.accent_wash` bed spanning the ruler height between in/out;
  handles 6 pt wide × ruler height, `c.accent`, `m.inner_radius` on the outer
  corners. When the work area equals the whole composition, draw no bed and
  no handles (show handles on ruler hover at the composition ends in `c.muted`).
- **Markers**: 9 pt tall pin (custom `marker` shape: rounded-top rectangle
  with a point), `c.secondary`, at the marker frame; a hairline guide
  `c.separator` drops through the track area. Named markers show the name in
  `t.minimum` `c.muted` to the right when there is room.
- **Playhead head**: 11 pt wide, `m.ruler − 4` tall, `c.accent`, radius 3 top
  / 6 bottom (shape `ph-head` in the v3 mockup), centred on the playhead x.
- **Render bar**: 3 pt strip at the very top of the ruler (under the header
  edge), states per [reviews/na-cloud-promedia.md](../reviews/na-cloud-promedia.md):
  `Missing`/`Stale` nothing; `Rendering` `c.muted` with moving 45° stripes
  (static 50 % `c.muted` under Reduce Motion); `Rendered` `c.success` at
  70 %; `Kept` `c.success` + 1 pt `c.text` top keyline.

### Track area (`c.panel`)

| Element | Spec |
|---|---|
| Row separators | none in normal contrast; Increase Contrast draws an `m.border` `c.separator` line under each layer row. |
| Hovered layer row | `c.hover` across layer column **and** track area. |
| Selected layer row | `c.selected_fill` across layer column and track area (neutral, language §17.3). |
| Clip | top/bottom inset `m.inner_radius`, height `m.track_row − 2 × m.inner_radius`, radius `m.inner_radius`, fill = layer label colour `c.labels.*`. Name inside at `m.inset − m.inner_radius` from the clip's leading visible edge (sticky: stays visible when the clip's start is scrolled off), `t.body` `c.text`, truncating. Audio clips draw the waveform in `c.text` at 75 % instead of a name, centred, ≤ `m.track_row − 4 × m.inner_radius` peak-to-peak. Video clips: name only (no filmstrip in v1.2.0). |
| Selected clip | 1.5 pt (`m.playhead`) `c.text` ring outside the clip shape + fill lifted one step (brightness +12 % in dark, −6 % in light; implement as the label colour mixed 12 % toward `c.text` in dark, 6 % toward `c.text` in light). **Never** an accent fill. |
| Clip beyond composition end / out of work area | unchanged; the region outside the work area gets a 30 % `c.panel_secondary` veil over the track area. |
| Offline media clip | fill `c.labels.gray`, name `c.danger` with `exclamationmark.triangle.fill` before it. |
| Locked layer clip | unchanged fill; a `lock` glyph `m.symbol_inline` `c.muted` after the name; pointer gestures on it show the "not allowed" cursor. |
| Collapsed key summary | on the layer row, at each key time of any property: diamond `m.keyframe − 2`, `c.text` at 90 %, 1 pt `c.panel` keyline, vertically at the clip's bottom edge. Selected keys: `c.accent`. A group's row shows its descendants' keys the same way. |
| Keyframe (property row) | `m.keyframe` diamond centred in the row. Unselected `c.text` at 90 %; selected `c.accent` + 1 pt `c.panel` keyline (shape change by keyline, not colour only). Interpolation shape: linear = diamond, ease/bezier = circle, hold = square-ish hourglass (custom `keyframe.hold`), as v1.2.0. |
| Playhead line | `m.playhead` width `c.accent`, from the ruler head through all rows. Over a selected clip it stays `c.accent`. |
| Snap guide | while snapping, a 1 pt `c.accent` vertical line through ruler and tracks at the latched time; disappears on release. |
| Marquee | 1 pt `c.secondary` stroke, fill `c.text` at 6 %, square corners. |
| Pick whip | 1.5 pt `c.accent` line from the Parent glyph to the pointer with a 6 pt `c.accent` circle at the pointer; the target row draws the drop-target treatment (README §3). |
| Live drag ghost (clip move between tracks) | the clip renders at its new place at 100 %; the original place shows an outline (1 pt `c.secondary` dashed 3/3). |
| Stagger band (text animator) | as v1.2.0 shape, fill `c.labels.violet`, handles `c.text`. |

### Graph area (view ≠ Dope Sheet)

- Shares the track area's horizontal axis; in Dope Sheet and Graph the graph
  occupies the lower half (splitter between, `m.keyboard_step`).
- Grid: horizontal value lines `c.separator`; value labels `t.minimum`
  tabular `c.muted` at the leading edge of the graph area.
- Curves: 1.5 pt. No per-axis hues. The focused curve is `c.text`, others
  `c.secondary`; each curve is named by a `t.minimum` label at its right end
  (`X`, `Y`, `Opacity`), so colour is never the cue.
- Points: `m.keyframe − 2` squares, `c.text`; selected `c.accent` + `c.panel`
  keyline. Bézier handles: 1 pt `c.secondary` line, 6 pt circle `c.text`;
  active handle `c.accent`.
- Graph selection transform box: 1 pt `c.secondary` rectangle with 6 pt
  square handles `c.text`.

## 3. States

| State | Visual |
|---|---|
| **Empty (no project)** | README empty state: symbol `film.stack`, `No Composition`, `Open a project to start editing.`, button `Open…` (`open`). Header shows only the disabled Play button. |
| **Empty (composition, no layers)** | Layer column and ruler render; track area centred text `t.reading` `c.secondary`: `Drag media here or import from your computer.` with a borderless button `Import…` below. Drop target = whole track area. |
| **Loading (project restoring)** | Rows render from the model immediately; clips whose media is restoring show their label fill with a `t.minimum` `c.muted` `Loading…` instead of the name. No spinner. |
| **Selected** | §2 (row fill, clip ring, key accent). |
| **Hover** | Row `c.hover`; clip edge (within 5 pt of an edge, scaled: `m.drag_threshold + 2`) shows a 2 pt `c.text` bar inside the edge + `resizeLeftRight` cursor; clip body → `openHand` cursor; key → pointing hand; ruler → `resizeLeftRight`; work-area handle → `resizeLeftRight`, bed → `openHand`; marker → tooltip `{name} · {timecode}` + detail `Right-click to remove`. |
| **Focus** | The timeline has one focus stop (the track area) plus the header controls and the layer-column splitter. Focused row = ring inset 1 pt inside the row (both columns). Focused key (after keyboard key navigation) = 2 pt `c.focus` ring around the diamond. |
| **Disabled** | Locked layers: clips and keys don't respond (cursor "not allowed"); property values show `c.muted` and are not editable. Composition tabs: none. |
| **Error** | Offline media clip (above). A failed edit (collision, invalid time) shakes nothing; the drag snaps back with `mo.exit` and VoiceOver announces the reason (copy below). |
| **Drag/drop in** | Media/effect drags from other panels: the row under the pointer gets the drop-target treatment (layer) or an insertion line between rows (new layer at that index) plus a 1 pt `c.accent` vertical line at the drop time. Option held: insertion line label `As Folder` (`t.minimum`) per v1.2.0 B144. |
| **Multi-select** | Rows, clips and keys select independently per kind; status bar shows `{n} layers selected`. Moving one selected clip moves all (rigid, spacing preserved). |
| **Playing** | Playhead moves; current-time readout updates every frame at fixed width; the play glyph becomes `pause.fill`. Only the visible viewport repaints (W055 B172). Rows do not re-layout. |
| **60 k rows** | README §4. Index column width from digit count. Collapsed key summaries are computed per visible row from a per-layer key index (never by scanning properties on paint). |
| **Graph empty** | Graph area centred `t.reading` `c.secondary`: `Select an animated property to see its curve.` |

## 4. Interactions

| Gesture | Result | Feedback |
|---|---|---|
| Click / drag in ruler | Move playhead (frame-snapped) | playhead follows pointer at sub-frame paint, readout shows the snapped frame (B177) |
| Shift during ruler drag | directional latch on clip edges, keys, markers (B052/B119) | snap guide while latched |
| Click empty track area | deselect clips/keys; **playhead stays** (B117) | — |
| Drag empty track area | marquee select keys (and clips in Tracks mode); Shift/⌘ adds | marquee rectangle; autoscroll at edges |
| Middle-drag / two-finger horizontal scroll | pan time | — |
| ⌥ scroll / pinch | zoom time around pointer | — |
| Drag clip body | move in time (live composition update, one undo) | clip moves; Shift toggles snapping; Escape restores (B180) |
| Drag clip vertically (Tracks mode) | move to another track, live stacking preview | ghost per §2 |
| Drag clip edge | trim in/out (minimum one frame) | edge bar; source continuity preserved |
| ⌘⌥ drag across selected layers | quick stagger in selection order (B127) | clips offset live |
| Click key | select key (Shift toggles, ⌘ adds) | key → accent |
| Drag key(s) | move selected keys together (one undo); Shift snaps by the anchored key (B247) | snap guide |
| Double-click key | **Edit Value…** dialog (`Edit Value…` sheet: time and value fields, `Cancel` / `Apply`) | sheet |
| Click diamond (property row) | toggle key at playhead (B158) | diamond fill changes |
| Click ‹ / › | go to previous/next key of that property | playhead jumps |
| Drag value readout | scrub (axis by which number is under the pointer), all selected layers, one undo (B345) | language §9 scrub rules |
| Click value readout | typed edit (arithmetic allowed); Return commits, Esc cancels | AppKit dense field |
| Click disclosure | open/close layer strip; shows keyframed properties immediately (B164) | rows animate (README transitions) |
| Double-click layer name | inline rename; Return commits, Esc cancels, blank ignored (B124) | AppKit field in the name slot |
| Click switch | toggle visibility/lock/solo (undoable `Toggle visibility` etc.) | glyph state |
| Drag Parent glyph to a row | set parent | pick whip |
| Click Parent glyph | parent pop-up menu: `None`, separator, every eligible layer (checkmark on current) | NSMenu |
| Drag work-area handle / bed | resize / move work area (≥ 1 frame) | handles |
| Right-click ruler | menu: `Add Marker Here` (disabled if one exists at that frame), separator, `Set Work Area Start`, `Set Work Area End`, `Reset Work Area`, separator, `Switch to Track Timeline` / `Switch to Layer Timeline` | NSMenu |
| Right-click marker | menu: `Remove Marker` | |
| Right-click property row | `Add Keyframe Here`, `Paste Keyframes Here`, `Clear All Keyframes` (disabled without keys); on Position, separator, `Separate Dimensions` ✓ | |
| Right-click ruler gutter | Columns menu | |
| Click 3D, Raster or Audio switch | toggle; undo `3D Layer`, `Continuously Rasterize`, `Toggle Audio` | glyph state |
| Right-click selected keys | `Edit Value…`, `Keyframe Velocity…`, `Scale Keyframes…`, separator, `Easy Ease` (F9), `Easy Ease In` (⇧F9), `Easy Ease Out` (⌘⇧F9) | |
| Right-click layer | the shared layer context menu (owned by the layer-commands lane; same items as the Layer main menu) | |
| Drop files | import at drop time/index (Option = as folder) | drop treatment |
| Tab close (×) / ⌘W on a focused tab | close composition (not available with one tab) | |

## 5. Keyboard and focus

All w1-keymap bindings apply when the timeline (or the editor) has focus;
the timeline adds no new chords. The ones the timeline must render feedback
for:

| Keys | Command |
|---|---|
| Space | play/pause |
| Home / End, ⇧Home / ⇧End | start/end, work in/out |
| PageUp/PageDown, ⌘←/⌘→ (⇧ = 10) | frame step |
| J / K, ⇧J / ⇧K | previous/next visible event, previous/next keyframe |
| I / O, [ / ], ⌥[ / ⌥], ⌥Home / ⌥End | layer in/out navigation, move, trim, boundary retime |
| B / N | work area in/out |
| ⌘B, ⌘⇧D | split (selects the right-hand part) |
| ⌥← / ⌥→ (⇧ = 10) | nudge keys |
| ← → ↑ ↓ (⇧ = 10) | nudge selection (viewer semantics) |
| P S R T A U M F E L (+⇧, double-tap) | property reveal (w1-keymap §Property-reveal sequences) |
| ⌘\` | reveal all properties |
| ⇧F3 | graph editor |
| F9 / ⇧F9 / ⌘⇧F9 | easy ease / in / out |
| ⌘↑ / ⌘↓ (⇧ extends) | select previous/next layer |
| Return (layer focused) | rename layer |
| Esc | cancel the active gesture → close field → deselect (innermost first) |
| Shift (held during a gesture) | invert snapping live (B054) |

**Focus order (Tab):** Play → current time → composition tabs (one stop; ←/→
moves between tabs, Return/Space opens, w1 local route) → mode control → view
control → graph options (if shown) → snap → layer-column splitter → track
area (one stop; ↑/↓ moves the focused row, → opens a strip, ← closes it,
Return renames, Space plays) → back to Play. ⌃Tab leaves the panel.

**Go to time:** clicking the current time (or Tab to it and Return) turns it
into a text field; accepts timecode, frames (`120`) or seconds (`4.5s`);
Return seeks, Esc restores.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Panel | group | `Timeline` |
| Play | button | `Play` / `Pause` |
| Current time | text field | label `Current time`, value spoken as `0 hours 0 minutes 4 seconds 15 frames` |
| Tabs | tab group `Open compositions`; tabs | composition name; selected; close button `Close {composition}` |
| Mode, View | radio group | `Timeline mode` (`Layers`, `Tracks`); `Timeline view` (`Dope Sheet`, `Dope Sheet and Graph`, `Graph Editor`) |
| Snap | toggle button | `Snapping`, on/off |
| Track area | outline `Layers` (`AXRowCount` = all rows) | layer row: `Headline, text layer, 0:00 to 0:10` + `selected`, `locked`, `hidden`, `soloed`; property row: `Scale, 100 by 100 percent, animated, keyframe at playhead` |
| Clip (Tracks mode) | cell | `Headline, V1, 0:00 to 0:10` |
| Keyframe | button | `Keyframe at 0:00:12, Scale, selected` ; actions `Press` (select), `Delete` |
| Ruler | slider `Playhead` | value = current time; increment/decrement = one frame |
| Work area | group `Work area` with two sliders `Work area start` / `Work area end` | |
| Render bar | static text | model summary string (`Rendered 0:00 to 0:12, rendering 0:12 to 0:14`) |
| Value readout | text field (adjustable) | `Opacity, 100 percent`; increment/decrement = step |

Announcements: `Added marker at {timecode}`, `Removed marker`,
`Keyframe time is taken by another keyframe`, `That time is outside the composition`,
`Layer is locked`.

## 7. Appearance

- **Dark**: layer column `c.panel_secondary` (`#161616`) beside track area
  `c.panel` (`#1A1A1A`); labels at their dark fills. Clip text `c.text` on
  label fills is ≥ 7:1 (tested).
- **Light**: same structure (`#F5F5F7` / `#FDFDFE`); label fills are the
  light pastel set. The playhead uses light `c.accent` (`#F0580A`).
- **Increase Contrast**: row separators drawn; clip outlines 1 pt
  `c.separator` on every clip; selected clip ring stays 1.5 pt `c.text`;
  keyframes get a 1 pt `c.text` keyline when unselected.
- **Reduce Motion**: rendering stripes static; disclosure appears without the
  spring; playhead and playback still move.
- **Reduce Transparency**: no change (no glass in the timeline).

## 8. Copy

Tooltips and menu items are listed in §2/§4. Additional:

| Where | String |
|---|---|
| Undo names | `Move Layers`, `Trim Layer`, `Split Layer`, `Move Keyframes`, `Add Keyframe`, `Clear Keyframes`, `Rename Layer`, `Toggle Visibility`, `Toggle Lock`, `Toggle Solo`, `3D Layer`, `Continuously Rasterize`, `Toggle Audio`, `Separate Dimensions`, `Join Dimensions`, `Set Parent`, `Add Marker`, `Remove Marker`, `Set Work Area`, `Reset Work Area`, `Stagger Layers`, `Link Scale Axes` |
| Empty composition | `Drag media here or import from your computer.` · button `Import…` |
| Graph empty | `Select an animated property to see its curve.` |
| Tab close tooltip | `Close {composition} (⌘W)` |
| Parent tooltip | `Parent: {name} · Drag to a layer` (`None` when unparented; `Missing Layer` when broken) |
| Status bar (selection) | `{n} layers selected` · `{n} keyframes selected` |

## 9. Don't

- Don't fill the selected clip, the active view segment or the timecode with
  the accent.
- Don't animate row reflow during playback, and don't repaint offscreen rows.
- Don't draw per-row separators in normal contrast.
- Don't put transport controls in the ruler gutter (v1.2.0) — they live in
  the header.
- Don't show Scale as two readouts in the timeline; it is one readout with
  axis-specific scrubbing.
- Don't add a heading strip under the ruler; headings live in the gutter.
- Don't label parenting `Link Layers`; it's `Parent`.
- Don't use letters (`V`, `A`, `T`) as layer badges; track names `V1`/`A1`
  stay only in Tracks mode headers.
