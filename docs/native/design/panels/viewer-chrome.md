# Viewer chrome

Panel id `viewer`, centre of the window; cannot be closed. This spec covers
everything the app draws **around and over** the composition image: the
viewer bar, the surround, the frame, gizmos, the source preview mode and the
Focus/full-screen overlay. The image itself is the renderer's.
Shared rules: [README.md](README.md). Look:
[language.md §10](../language.md#10-viewer).

Behaviour source: `v1.2.0:apps/desktop/src/extensions/viewer/viewer.ts`
(zoom menu `L855`, preview resolution `L877`), `source-preview.ts`,
`GizmoToolbar.svelte`, `text-editor.ts`, `path-editing.ts`,
`layer-locks.ts`. Workflows W045–W049, W021, B001–B004, B020/B021,
B061/B062, B090–B095, B109/B110, B115/B116.

## 1. Model contract (proposed names)

| State | Values |
|---|---|
| `ViewerMode` | `Composition` · `Source{asset}` · `NoProject` |
| `ZoomMode` | `Fit` · `Fixed(f32)` (0.02…8.0) |
| `PreviewResolution` | `Auto` · `Full` · `Half` · `Quarter`; plus `effective: Full/Half/Quarter` when `Auto` |
| `CanvasLost` | bool — explicit pan moved the composition fully out of view (B020) |
| `Presentation` | `Focus` · `FullScreen` · `Docked` |
| `Gizmo` | `None` · `Transform{layers}` · `Anchor` · `Path` · `Text{editing}` · `Crop` |
| `ShowControls` | bool (⌘⇧H "Show/Hide Layer Controls") |
| `Banner` | `None` · `Info{text, action?}` · `Warning{text, action?}` |

## 2. Anatomy

```
┌ viewer bar (m.header) ────────────────────────────────────────────────────┐
│ [Source: VO_take03.wav  ‹ Back to Composition]      [Fit (54 %) ⌄] [Auto (Half) ⌄] │
├────────────────────────────────────────────────────────────────────────────┤
│ surround c.viewer                                                          │
│        ┌──────────── composition frame ────────────┐                       │
│        │   image (renderer)                          │                       │
│        │      ┌─────── gizmo box (c.accent) ──┐      │                       │
│        │      ■             ■               ■      │                       │
│        └────────────────────────────────────────────┘                       │
│                         [ banner ]                                          │
└────────────────────────────────────────────────────────────────────────────┘
```

### Viewer bar (`m.header`)

The viewer panel has **no title**. The bar is part of the panel (solid
`c.panel`), not glass.

| Slot | Spec |
|---|---|
| Leading (Composition mode) | empty, unless the composition has 3D layers: then the 3D toolbar in [3d-studio.md §2](../surfaces/3d-studio.md#2-viewer-3d-toolbar). |
| Leading (Source mode) | `Source:` `t.body` `c.muted` + ` ` + asset name `t.body` `c.text` (truncating), then `m.gap`, then a borderless text button `chevron.left` + `Back to Composition` (`t.body` `c.accent_text`). Esc does the same (W021). |
| Spacer | flexible |
| **Zoom pop-up** | trailing, first. `NSPopUpButton`-style GPUI pop-up **in the bar** (not over the image): borderless at rest (`t.body` `c.secondary`, chevron `chevron.up.chevron.down` `m.symbol_inline` `c.muted`), `c.hover` fill on hover, `m.field` tall, `m.control_radius`. Title: `Fit (54 %)` when fitting, `54 %` when fixed. Fixed width = width of `Fit (100 %)` + chevron, so it never jumps. Menu (real `NSMenu`): `Fit` ✓, separator, `5 %`, `12.5 %`, `25 %`, `50 %`, `100 %`, `200 %`, `400 %`, `800 %` (✓ on the matching fixed value; none when custom), separator, `Zoom In` ⌘=, `Zoom Out` ⌘−, `Actual Size` ⌘1, `Fit Composition` ⌘0. Tooltip `Zoom`. |
| **Preview resolution pop-up** | trailing, second, `m.gap` after zoom. Same recipe. Title: `Auto (Half)` when Auto has chosen a lower effective resolution, `Auto` when effective = Full, otherwise `Full` / `Half` / `Quarter`. Fixed width = width of `Auto (Quarter)` + chevron. Menu: `Auto` ✓ with a detail line `Lowers resolution while playing to keep up` (small `NSMenuItem` subtitle), separator, `Full`, `Half`, `Quarter`. Tooltip `Preview Resolution`. Also in `View ▸ Preview Resolution` submenu with the same items. |
| Trailing inset | README §2 |

**Decision (open item "quality pop-up placement"):** the preview-resolution
pop-up lives in the viewer bar, trailing, immediately after zoom — never over
the image, never in the timeline header or status bar. Reason: it changes
what the viewer shows (B109: it relayouts the viewer), it pairs with zoom as
"how big / how sharp", and both follow the `Fit (54 %)` title pattern so the
automatic choice is visible without opening the menu. In Focus and full
screen the two move together into the overlay capsule (§2 *Overlay*).

No safe-area, grid or guides toggle: v1.2.0 has none (B115). Do not add one.

### Surround and frame

| Element | Spec |
|---|---|
| Surround | `c.viewer`, edge to edge below the bar, no radius except the panel's own bottom corners. Light: `#E4E4E4` neutral (token change, README §6). |
| Composition frame | drawn at its real bounds; inside it the comp background (or checkerboard: 8 pt squares, `c.field`/`c.panel` in light, `c.field`/`c.hover` in dark, scaled with zoom ≥ 100 %, fixed 8 pt below). A 1 px (device pixel) `c.separator` outline is drawn **only** when the background is transparent or within ΔE 3 of the surround. |
| Preview-corner preference (`square` / `rounded`) | affects the surround mask only (language §10). |
| Lost canvas | when `CanvasLost`: a centred glass `ViewerOverlay` capsule button `Return to Composition` (`viewfinder` glyph) — one click fits (B020/B067). |
| Out-of-frame content | layers outside the frame are drawn at 35 % over the surround while selected (pasteboard), not drawn otherwise — v1.2.0 behaviour. |

### Gizmos (drawn over the image)

Scale with the display, not with zoom. Accent marks need a keyline because
they sit on arbitrary pixels.

| Element | Spec |
|---|---|
| Bounding box | 1 pt `c.accent`, plus a 1 pt `#000` 40 % outer keyline (`c.media_badge`) so it reads on orange or white footage |
| Handles | `m.keyframe − 2` (7 pt) squares, `c.accent` fill, 1 pt `c.viewer`-dark (`#0E0E0E`) keyline in both schemes |
| Rotation handle | 18 pt stem above top-centre, 9 pt circle 1.5 pt `c.accent` stroke, `#0E0E0E` fill |
| Anchor point | 11 pt crosshair circle, 1.5 pt `c.accent`, white 1 pt inner ring |
| Centre guides (alignment) | full-width/height 1 pt `c.accent` lines while snapping + fixed bullseye at composition centre (B061) |
| Smart guides / snapping | 1 pt `c.accent`; distance labels in a `c.media_badge` capsule, `t.minimum` `c.selected_text`, tabular (`24 px`) |
| Path editing | vertices 7 pt squares (`c.floating` fill, 1 pt `c.accent`), selected vertex filled `c.accent`; tangents 1 pt `c.secondary` with 6 pt circles |
| Text editing caret/selection | system insertion point colour (orange via AccentColor); selection `c.accent` at 30 % |
| Locked layers | no handles; hovering a locked layer shows the "not allowed" cursor and tooltip `Layer is locked` |
| Marquee (Select tool) | 1 pt `c.selected_text` 70 % stroke with a 1 pt `#000` 40 % outer stroke; no fill |
| Zoom tool rectangle | same as marquee |
| Hidden controls (⌘⇧H off) | no box, handles, guides or path chrome; selection still exists |

### Gizmo toolbar (v1.2.0 `GizmoToolbar.svelte`)

Contextual tool options (shape kind, pen options) appear as a **glass
`FloatingBar` capsule** pinned to the top-centre of the viewer, `m.gap`
below the bar, only while a tool with options is active. Height
`m.capsule_radius × 2`, items `m.hit`, `t.body`. It never covers the image's
top edge by more than its height; when the frame's top is within its band,
the capsule still floats (it is glass over content — allowed, language §3).

### Overlay (Focus and full screen)

In Focus workspace or full screen the viewer bar hides; a glass
`ViewerOverlay` capsule (`Tokens::material(Surface::ViewerOverlay)`) holds
`[Fit (54 %) ⌄] [Auto (Half) ⌄]` at the bottom-centre, `m.inset` above the
viewer's bottom edge. It fades in on pointer movement (`mo.enter`), fades out
`mo.exit` after 2 s still (not while the pointer is over it or a menu is
open). Under Reduce Motion: cross-fade `mo.reduced`. **Overlay content uses
the dark scheme in both appearances** (language §4 *Over the image*).

### Banners

Transient information about the viewer (`Preview resolution lowered while
playing`, `Media offline in this frame`) appears as a glass capsule at the
bottom-centre (above the overlay capsule if both), `t.body`, symbol + text +
optional borderless action. One at a time; auto-dismiss after 4 s unless it
has an action; never stacks. Dark scheme in both appearances.

## 3. States

| State | Visual |
|---|---|
| **Empty (no project)** | Bar hidden. Surround `c.viewer` with README empty state in **viewer-local** colours: `No Composition`, `Open a project to start editing.`, button `Open…`. |
| **Empty composition** | Frame drawn (background / checkerboard); centred inside the frame `t.reading` `c.muted`-on-dark: `Drop media here` — shown only when the composition has zero layers; drop target = frame. |
| **Loading (first frame)** | Frame outline + background only; after 0.5 s a small spinner centred in the frame (dark scheme). The last good frame stays visible across reloads and zoom (B066/B069) — never blank to show loading. |
| **Selected** | gizmos as above. |
| **Hover** | layer under the pointer (Select tool) gets a 1 pt `c.accent` 60 % outline (hover hint); handles show resize/rotate cursors. Pop-ups `c.hover`. |
| **Focus** | the canvas is one Tab stop: focus ring `c.focus` drawn **inside** the viewer surround edge, inset 2 pt (never over the image). Pop-ups take the system ring. |
| **Disabled** | pop-ups disabled with no project. |
| **Error (render failure)** | frame keeps the last good image; a warning banner `Couldn’t draw this frame. Showing the last good frame.` with `Retry`. |
| **Drag/drop** | dragging media/effects over the frame: frame gets the drop-target stroke (1.5 pt `c.accent`, no wash over the image); drop places at composition coordinates (B019). Effect drop targets the layer under the pointer (that layer's outline highlights). |
| **Multi-select** | one bounding box around the selection + individual 1 pt `c.accent` 60 % outlines per layer. |
| **Source preview** | §2 bar leading content; the frame shows the source fitted; transport for audio/video is a glass `ViewerOverlay` capsule at bottom-centre: play/pause, a scrub slider (time readout hidden visually but exposed to VoiceOver, B093), no transport for stills (B092); audio shows `waveform` symbol at 4 × `m.symbol_panel` and the filename once below it (B091). |
| **Playing** | no gizmos change; resolution title may change `Auto` ↔ `Auto (Half)` (fixed width, no jump). |
| **60 k rows** | N/A. |

## 4. Interactions (W045)

| Gesture | Result |
|---|---|
| Scroll wheel | zoom around the pointer (anchor stays under the cursor, B021) |
| Trackpad two-finger scroll | pan; ⇧ redirects vertical to horizontal |
| Pinch | zoom around the pinch centre |
| Middle-drag, or Space-drag, or Hand tool drag | pan (temporary Hand restores the previous tool) |
| Zoom tool click / ⌥-click / drag | ×1.25 / ×0.8 / frame rectangle (B002) |
| ⌘0, ⇧/ | fit; ⌘1, / actual size; ., ⌘= zoom in; ,, ⌘− zoom out |
| Double-click text layer | edit text in place (W049); Esc or ⌘Return commits |
| Drag layer (Select tool) | move; snapping on both axes, ⌘ bypasses (B062) |
| Press with the Anchor Point tool | selects the layer under the pointer, narrowing a multi-selection to it. Dragging past `m.drag_threshold` in the same gesture moves that layer's anchor to the pointer, one undo `Move Anchor Point`; Esc restores. A click with no drag only selects, so it never moves an anchor (main `5458b5449`) |
| Pop-ups | open `NSMenu`; arrow keys, Return, Esc, type-ahead (system) |

## 5. Keyboard and focus

Tab order inside the viewer panel: `Back to Composition` (Source mode) →
zoom pop-up → resolution pop-up → canvas. With the canvas focused, all
w1-keymap viewer/tool bindings apply (V H Z W Y Q G, arrows nudge, ⌘⌥Home
centre anchor, ⌘⇧H controls). Esc cancels a gizmo drag, then exits text
edit, then Source mode, then deselects.

## 6. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Canvas | group `Viewer` with custom rotor `Layers` | value `Launch Film, 54 percent, Half resolution, 2 layers selected` |
| Selected layer (rotor item) | `AXLayoutItem` | `Headline, text layer, position 960 by 540 pixels`; actions `Move Left/Right/Up/Down` (nudge) |
| Zoom pop-up | pop-up button | `Zoom, Fit, 54 percent` |
| Resolution pop-up | pop-up button | `Preview resolution, Auto, currently Half` |
| Back to Composition | button | `Back to composition` |
| Source transport | group `Source preview` with `Play`, slider `Source time` (value in seconds) | |
| Return to Composition | button | |

## 7. Appearance

- **Dark**: surround `#0E0E0E`; bar `c.panel`.
- **Light**: bar `c.panel` (light), surround neutral `#E4E4E4`. Everything
  drawn **over the image** — gizmo keylines, distance labels, overlay
  capsule, banners, source transport, the lost-canvas button, empty-state
  text inside the frame — uses the **dark-scheme** token set, whatever the
  app appearance (language §4 *Over the image*). The bar and surround follow
  the app appearance.
- **Increase Contrast**: frame outline always drawn; gizmo keylines 2 pt.
- **Reduce Transparency**: overlay/banners/gizmo toolbar solid dark
  `c.floating` (`#2C2C2C`).
- **Reduce Motion**: overlay cross-fades; zoom changes jump (no animated zoom).

## 8. Copy

| Where | String |
|---|---|
| Zoom titles | `Fit ({n} %)`, `{n} %` (one decimal only for 12.5) |
| Zoom menu | `Fit`, `5 %`, `12.5 %`, `25 %`, `50 %`, `100 %`, `200 %`, `400 %`, `800 %`, `Zoom In`, `Zoom Out`, `Actual Size`, `Fit Composition` |
| Resolution | `Auto`, `Auto ({Full/Half/Quarter})`, `Full`, `Half`, `Quarter`; Auto subtitle `Lowers resolution while playing to keep up` |
| Tooltips | `Zoom`, `Preview Resolution` |
| Source mode | `Source:` · `Back to Composition` |
| Lost canvas | `Return to Composition` |
| Empty composition | `Drop media here` |
| Banners | `Couldn’t draw this frame. Showing the last good frame.` + `Retry` |
| Locked | `Layer is locked` |

## 9. Don't

- Don't put zoom/resolution chips over the image in the docked viewer.
- Don't tint, round or add material to the image.
- Don't draw gizmos in the light-scheme colours over footage.
- Don't add guides/safe-area/grid toggles (not in v1.2.0).
- Don't show a viewer title.
- Don't animate zoom changes from the menu.
