# 3D Studio

Row S098. Grammar: [README](README.md) §1–5. Source: `origin/feat/na-3d-source`,
ported to Rust per the owner decision of 2026-11-04. Files: `docs/3d-studio-redesign.md`,
`apps/desktop/docs/STUDIO.md`, `E/layers-3d/{LayerInspector,SceneField}.svelte`,
`E/layers-3d/{scene-model,scene-actions}.ts`,
`E/viewer/{GizmoToolbar.svelte,light-panel.ts,viewer-chrome.ts}`, the Studio pill in
`E/viewer/viewer.ts`, and `core/scene3d/{gizmo,navigation,light-tool,migration}.ts`.

This is the only 3D system. There's no compatibility mode, no draft renderer choice
beyond Fast and Quality, and no second app. It replaces every older 3D surface.

The target is the Apple-style product launch film: one object on a clean ground under
soft light, with a slow, exact camera. The tools have to feel like that film looks.
What's below refines what the source already does. Where the redesign proposal
describes something the source didn't build, the source wins, and §12 lists the gaps.

## 1. Rules for this family

| Rule | Spec |
|---|---|
| Quiet at rest | Nothing appears unless it's useful now: the 3D group shows only in a composition with 3D layers, the status capsule only while it has news, the Light panel only while the Light tool is on |
| Exact under the hand | Handles draw on the frame they're pressed and never wait for the renderer. The engine answers picks and projected points; the host draws every handle (`docs/3d-studio-redesign.md` §4.1a) |
| One colour language | Accent is selection. Axis colours are data. Material swatches and light colours are content. Nothing else in this family has colour |
| Numbers stay put | Values are the inspector's tabular wells. No value floats beside the pointer |
| One gesture, one Undo | Every drag, slider pull, finish, shot and import is one undo step with the name listed here |

**Words.** The source uses two sets of light names. Native uses one everywhere: `Softbox`,
`Spotlight`, `Bulb`, `Sun`. The stored types stay `area`, `spot`, `point`, `sun`, so
projects and agent operations don't change. `Intensity` becomes `Brightness` and `Cone
Angle` becomes `Spread`, because they are the same values the Light panel shows. Spelling
follows the app's existing `Color`: `Aluminum`, not `Aluminium`.

Layer kinds: model `cube` on `c.labels.blue`, light `lightbulb` on `c.labels.gray`, camera
`camera` on `c.labels.gray`, in the kind badge (language §8). Camera layers keep their
pinned place at the top of the timeline.

## 2. Viewer 3D toolbar

Ports `GizmoToolbar.svelte`. Electron's strip sits under the viewer; native keeps its
approved viewer bar at the top ([viewer-chrome.md](../panels/viewer-chrome.md)), so the
group occupies the bar's **leading** slot and the zoom and preview-resolution pop-ups keep
their trailing place.

```
┌ viewer bar (m.header) ──────────────────────────────────────────────────────────────┐
│ [Camera|Free View] [Move Rotate Scale ┊ Orbit Pan Zoom ┊ Light] [Scene Axes] [Frame] │
│ [Fast|Quality]                                    ···spacer···  [Fit (54 %) ⌄] [Auto ⌄] │
└─────────────────────────────────────────────────────────────────────────────────────┘
```

(Drawn on two lines only to fit this page; it's one row.)

**Shown** when the active composition has at least one 3D layer, in Composition mode
only. In Source mode the slot keeps the source label. With no 3D layers the bar is exactly
as in viewer-chrome.md.

Order, leading to trailing, with `m.gap` between groups:

| # | Control | Items | AX group |
|---|---|---|---|
| 1 | Segmented | `Camera` (`camera`), `Free View` (`view.3d`) | `Look Through` |
| 2 | Segmented, icon only | `Move` (`move.3d`), `Rotate` (`rotate.3d`), `Scale` (`scale.3d`) · separator · `Orbit` (the capsule's Orbit glyph), `Pan` (`hand.raised`), `Zoom` (`plus.magnifyingglass`) · separator · `Light` (`lightbulb`) | `Tool` |
| 3 | Borderless toggle, only while Move, Rotate or Scale is active | `Scene Axes` (`globe`) ⇄ `Object Axes` (`cube`) | name of the current state |
| 4 | Borderless icon button | `Frame` (`viewfinder`) | `Frame` |
| 5 | Segmented | `Fast` (`bolt`), `Quality` (`camera.aperture`) | `Preview` |

Recipe: language §9 segmented control, track `c.field`, `m.field` tall, padding
`m.control_radius − m.inner_radius`, segments `m.inner_radius`, selected segment `c.floating`
with `c.text`, others `c.secondary`. Label `t.body` `t.medium_weight` beside the glyph at
`m.symbol_inline`. An icon segment is `m.hit` wide, and every segment's hit area is the
full `m.header` height. Group separators are `m.border` `c.separator`, `m.symbol_panel`
tall. It's solid `c.panel` like the rest of the bar: no glass here. Accent is not used:
the selected segment's fill says which tool is on, as in every other segmented control.

| Width of the viewer | Behaviour |
|---|---|
| ≥ `m.viewer_3d_labels` | Items 1, 3 and 5 show labels |
| Below it | Icons only, each with its tooltip |
| Below `m.viewer_3d_collapse` | The whole group becomes one `3D` pull-down (`cube`), with the same items as a checkmarked `NSMenu` in the same order and groups |

**Focus and full screen.** The group joins the glass `ViewerOverlay` capsule, leading of
the zoom pop-up, icons only, dark-scheme tokens, borderless segments (no glass on glass).

**Tooltips** (line 1 name with shortcut, line 2 sentence):

| Item | Line 2 |
|---|---|
| `Camera` | `See exactly what the shot renders.` |
| `Free View` | `Look around without changing the shot.` |
| `Move` | `Drag the selection, or one arrow for one axis.` |
| `Rotate` | `Turn the selection by its rings.` |
| `Scale` | `Resize the selection.` |
| `Orbit` | `Drag to circle the view. Anytime: ⌥-drag.` |
| `Pan` | `Drag to slide the view. Anytime: ⇧⌥-drag.` |
| `Zoom` | `Drag to move closer. Anytime: ⌥-scroll.` |
| `Light` | `Click a model to put light there.` |
| `Scene Axes` | `Handles line up with the scene. Click for the object’s.` |
| `Object Axes` | `Handles line up with the object. Click for the scene’s.` |
| `Frame` (F) | `Fit the selection, or everything.` |
| `Fast` | `Real-time preview. Exports stay full quality.` |
| `Quality` | `Path-traced, as it will export.` |

Behaviour is the source's. Picking a transform tool leaves Light and navigation; picking
Orbit, Pan or Zoom keeps the transform tool underneath; `Light` toggles. The window tool
capsule's Orbit and this `Orbit` are one tool, so selecting either shows both as active.
All items also appear in `View ▸ 3D`, with the same names and checkmarks (language §9:
the menu bar is the complete command surface). The choice of Fast or Quality is remembered.

A tool that doesn't apply to the selection is disabled, with the tooltip `Not available
for this layer.`: cameras don't scale, a `Sun` only rotates, a `Bulb` only moves.

## 3. Fast, Quality and Final

| Tier | What it is | How you get it |
|---|---|---|
| `Fast` | Real-time renderer with the studio set's backdrop, reflections and a soft contact shadow | `Preview` segment |
| `Quality` | Studio's path tracer: Live while dragging, scrubbing or playing, then Refine until clean | `Preview` segment |
| `Final` | Fixed samples per frame, motion blur, depth of field, denoise. Same pixels every render | Always used by export and agent frames. It isn't a viewer mode, because a clean Refine already shows it. Its cost is set by `Final Quality` (§8) |

The preview-resolution pop-up is unchanged and independent of this. It says how sharp;
`Preview` says how it's drawn. There's no Draft or Rendered switch, and no shading modes.

**Status capsule.** Ports the Studio pill, made calmer.

| Part | Spec |
|---|---|
| Place | Bottom-trailing of the viewer stage, `m.inset` from both edges, over the image. Never in the export |
| Material | Glass `ViewerOverlay`, dark-scheme tokens, height `m.field`, capsule radius, no pointer events |
| Ring | Determinate circle `m.symbol_panel`, track `c.separator`, arc `c.accent`, drawn from the model's real sample count |
| Text | `t.body` `c.secondary`, tabular, fixed width per state so it doesn't jitter |

| State | Capsule |
|---|---|
| Live (dragging, scrubbing, playing) | Hidden. The source shows `Live` here; in native it would flash on every drag |
| Refining | `Refining · {n} %`, after the same 0.5 s as Loading, so a quick settle shows nothing |
| Clean | `Clean` with `checkmark` for `mo.overlay_idle`, then fades out in `mo.exit` |
| Preparing a large model | `Preparing {n} triangles` (`1.0M`, `640K`); ring is real progress. Beats Refining |
| Fast | Hidden, except Preparing |

Reduce Motion: no fade, just the `mo.reduced` cross-fade. VoiceOver: `Refining` and `Clean`
post once each per refine at `.medium`; the percentage isn't announced.

Fallbacks. An orthographic camera can't path-trace, so `Quality` is disabled with the
tooltip `Orthographic cameras draw in Fast.` and `Fast` shows as selected; the stored
choice returns with a perspective camera. A model too large for Studio's GPU buffer draws
in the real-time renderer: §10.

## 4. Navigation

Unchanged from `navigation.ts`. Navigation only ever moves the transient Free View, and
never the shot. If the view is `Camera`, the first navigation gesture switches to `Free
View`, and that's the only side effect.

| Gesture | Result |
|---|---|
| ⌥-drag or middle-drag | Orbit |
| ⇧⌥-drag | Pan |
| Scroll | Zoom, in a navigation tool or with ⌥ held |
| ⇧ during Orbit or Zoom | Fine, one tenth speed |
| Trackpad pinch | Zoom, as the 2D viewer |
| F or Home | Frame the selection, or everything when nothing's selected |

Framing eases with the `mo.enter` spring; under Reduce Motion it jumps. Cursors: Orbit and
Pan use the open and closed hand, Zoom the magnifier. Navigation never adds an undo step.
There's no view cube and no axis ball: the source's toolbar replaced them.

## 5. Gizmos

Drawn by the host over the image, sized by the display and not by zoom or depth, with
dark-scheme tokens in both appearances. Geometry comes from the engine. They replace the 2D
handles when every selected layer is 3D, and the 2D rules in viewer-chrome.md
(*Hidden controls*, *Locked layers*) still apply.

| Element | Spec |
|---|---|
| Selection outline | Viewer-chrome bounding box recipe, following the object's projected bounds: `m.gizmo_keyline` `c.accent` with the `c.media_badge` keyline. Multi-select: one outline each, handles at the shared pivot |
| Hover hint | Viewer-chrome hover outline, on the part under the pointer |
| Axis line | `m.gizmo_keyline × 2`, length `m.rotation_stem × 4`, from the pivot |
| Axis colours | X `c.scope_red`, Y `c.scope_green`, Z `c.scope_blue`. Data, so conventional colours stand. Each axis also carries its letter at `t.minimum` beyond the tip, so colour isn't the only cue |
| Move | Arrow tip `m.gizmo_handle` on each axis; plane squares `m.inset`, axis colour at `m.pasteboard_opacity` |
| Rotate | One ring per axis, radius `m.rotation_stem × 4`, stroke as the axis line, back half hidden |
| Scale | Cube tip `m.gizmo_handle` on each axis, centre cube for uniform scale |
| Hover on a handle | That handle turns `c.text`, `mo.interaction` |
| During a drag | Only the active handle stays, plus a `m.gizmo_keyline` guide through the pivot along the active axis in its colour. Everything returns on release |
| Hit area | At least `m.hit` across any line or tip; a press elsewhere selects or navigates as usual |

Modifiers as `gizmo.ts`: ⌃ snaps (1 unit, 15°, 0.1), ⇧ is fine. Esc cancels a drag and
restores the values. Undo names: `Move 3D Layer`, `Rotate 3D Layer`, `Scale 3D Layer`, plural
with several. The drag edits ordinary channels, so an animated property gets a keyframe at the
playhead, as any inspector edit does.

Gizmos aren't exposed to VoiceOver. Every transform is reachable from Properties.

## 6. Light tool

Ports `light-panel.ts` and `light-tool.ts`. Choosing `Light` opens the **Light panel**.

It's a glass child panel, not an `NSPopover`, because a popover dismisses on the click the
tool needs. It uses the kit's `ViewerOverlay` frame with solid content (language §3, dense
content in a floating surface): `c.floating`, dark-scheme tokens in both appearances, since
it sits over the image. Placed at the viewer stage's top-leading corner, `m.gap` from both
edges. Width `m.popover_width`, inset `m.inset`, height fits content. If the stage is
shorter than the panel, the content scrolls. It enters with the `mo.enter` spring and leaves
with `mo.exit`. In Focus and full screen it stays, like the tool. AppKit controls throughout.

```
┌ Light ─────────────────────┐
│ [ Reflect | Direct | Orbit ]  │  Aim, NSSegmentedControl, full width
│ [ Add ]  [ Softbox        ⌄ ] │  button + pop-up
│ Softbox 2            Softbox  │  name t.title, kind t.minimum c.muted
│ Brightness  ───●────  6.00    │
│ Size        ─●──────  1.20    │  row m.row
│ Distance    ────●───  3.40    │
│ Spread      ──●─────  45°     │  Spotlight only
└────────────────────────────┘
```

| Row | Spec |
|---|---|
| Aim | `Reflect` (`The highlight lands where you click.`), `Direct` (`The light faces where you click.`), `Orbit` (`Drag to circle the light around its target.`) as tooltips |
| Add | Toggle push button `Add`, tooltip `Then click a model to put it there.` Pop-up `Softbox`, `Spotlight`, `Bulb`, `Sun`; choosing one also arms Add |
| Slider rows | Label `t.body` `c.secondary`, width `m.hit × 3`. `NSSlider`, small, tinted by AccentColor. Readout `t.body` tabular `c.secondary`, **left-aligned**, width `m.control + m.inset`, unit `°` in `c.muted`. Gaps `m.gap`. Readout format as source: 2 decimals under 10, 1 under 100, none above |
| Ranges | Brightness 0.05–2000 log; Size 0.02–40 log (hidden for a `Bulb` and `Sun`); Distance 0.1–200 log (hidden for `Sun`); Spread 1–89° linear (`Spotlight` only) |
| Tooltips | Brightness `How much light it gives.` Size `Bigger softboxes give softer light and broader highlights.` Distance `How far it sits from what it lights. Brightness on the product stays the same.` Spread `How wide the spotlight’s beam is.` |
| Hint (no light selected, or Add armed) | Replaces the name and sliders, `t.body` `c.muted`: `Select a light to aim it, or Add one.` / `Click a model to add a {kind} there.` |

| Gesture in the viewer | Result |
|---|---|
| Click a model | Aims the selected light by the Aim mode. With no light selected, or Add armed, adds one there |
| Keep dragging | The highlight slides with the pointer, live |
| Scroll with a softbox selected | Resizes it, ×1.12 per notch, undo name `Softbox Size` |
| ⌥⇧-click, any tool | Places a softbox's highlight under the pointer |
| Esc | Leaves the tool |

Cursor: crosshair over a model, standard elsewhere. A release, or the end of a slider pull,
is one Undo: `Aim Light`, `Add {kind}`, `Light {row}`. The light's reflection is the
feedback, so nothing else draws at the click. While a light moves, Quality runs Live and
refines after release.

VoiceOver: `AXGroup` `Light Tool`, controls in reading order. Sliders read `Brightness, 6.00`.
Full Keyboard Access reaches the panel with Tab from the viewer; Esc returns focus to it.

## 7. Add and import

**Add 3D Layer** menu: `Layer ▸ Add 3D Layer`, timeline and viewer context menus. Palette
entries `Add 3D {primitive}`, `Add {light}`, `Add Camera`.

`Cube` · `Sphere` · `Plane` · `Cylinder` · `Cone` · `Torus` · `Capsule` · `Icosphere` ·
separator · `Import 3D Model…` · separator · `Softbox` · `Spotlight` · `Bulb` · `Sun` ·
`Camera`

Other commands keep their names: `New 3D Model`, `Add 3D Studio Cube`, `Duplicate 3D
Layers`, `Delete 3D Layers`.

**Import.** `NSOpenPanel` as a sheet, multiple selection. Dropping a supported file on the
viewer, timeline or Project does the same. Types: `.glb`, `.gltf`, `.obj` (with `.mtl`),
`.step`, `.stp`, `.iges`, `.igs`, `.brep`, `.ply`, `.splat`, and `.png`, `.jpg`, `.jpeg`,
`.webp` as textures selected with a model.

| Step | Where | Copy |
|---|---|---|
| Reading | Background Tasks row, kind `Import`, after 0.5 s | `Importing {file}…` |
| CAD conversion | same row; parts, names and colours are kept | `Converting {file}…` |
| Layer appears | viewer draws at once | Fast renderer until Studio is ready |
| Large model | status capsule (§3) | `Preparing {n} triangles` |
| Empty composition | One Undo adds the model on the floor, a framing camera, the three softboxes and the seamless set | undo name `Import {file}` |
| Composition already has a camera or light | Adds the model only | undo name `Import {file}` |
| Splat | Stands upright and is framed on its dense core. No Studio prep | as above |

Limits: GLB 512 MB; 10 million triangles; 65,536 parts; 16 million splats.

Splats aren't path traced. They composite by layer order and don't show in reflections.
In Properties a splat layer shows the `Model` section and this note, `t.body` `c.muted`,
under `Source`: `Keep this layer above the products it surrounds. Splats don’t appear in
reflections.`

## 8. 3D section in Properties

One `3D` section after content, single selection only, as source. With several layers
selected the section is absent and the transform rows work as usual.

Rows follow the inspector grammar exactly: 9 pt keyframe diamond column, label, shared
`m.value_column`, left-aligned tabular values, scrubbable wells, row height `m.row`,
checkboxes at the leading edge of the value column. A field with a range is a number well,
not a slider: native inspectors have no sliders (panels/README §5.5), and the range clamps
the well. Section titles are the inspector's. Field names are Title Case.

Everything keyframeable uses `SceneField`: AX `Add keyframe for {label}` or `Remove keyframe
for {label}`, at the same state as any inspector diamond.

### Model

| Row | Control |
|---|---|
| `Drop to Floor` | Push button, leading edge of the value column, label column empty |
| `Source` | Read-only `t.body`: asset name, primitive name, `Lathe`, `Extrusion` or `Mesh` |
| `Model Materials` | Checkbox, imported models only |
| `Cast Shadows`, `Receive Shadows` | Checkboxes, last in the section, after Material |

### Material (absent while `Model Materials` is on)

**Finish shelf.** Directly under the heading, spanning the panel between `m.inset`
edges: a grid of `m.row` squares, `m.gap` apart, `m.control_radius`, with a hairline
`c.separator` edge. Each tile is a swatch rendered by Studio. Until it arrives it's the
finish's flat colour, swapped in place without animation. Swatches are content, so
they're the one coloured thing in this panel.

`Anodized Aluminum`, `Brushed Metal`, `Polished Steel`, `Gold`, `Clear Glass`, `Frosted
Glass`, `Gloss Lacquer`, `Soft-Touch Plastic`, `Glazed Ceramic`, `Matte Rubber`, `Display
Under Glass`.

Tile states: tooltip and AX are the name. Hover adds a `m.drop_stroke` `c.secondary` ring
outside the tile; focus is the standard ring. Click applies, one Undo named for the finish.
There's no selected state: a finish is a starting point, and the fields below it may have
moved on.

**Rows**, top to bottom. Disclosure groups use the inspector's section disclosure and keep
their state per project. A group opens on its own if a row inside is animated.

| Group | Rows |
|---|---|
| (always shown) | `Screen` pop-up: `None`, then the project's compositions. `Color`, `Roughness` 0.02–1, `Metalness` 0–1, `Opacity` 0–1 |
| `Coat and Glass` | `Clear Coat` 0–1, `Coat Roughness` 0.002–1, `Transmission` 0–1, `Index of Refraction` 1–2.6, `Specular` 0–1, `Dispersion` 0–1 |
| `Surface` | `Sheen` 0–1, `Thin Film` 0–1200 `nm`, `Water Droplets` 0–1, `Brushed` −1–1, `Subsurface` 0 and up |
| `Emission` | `Emission` colour, `Emission Strength` 0 and up |
| `Texture Maps` | `Color Map`, `Normal Map`, `Roughness Map`, `Metalness Map`, `Emissive Map`, `Occlusion Map` |

`Screen` plays a composition on the surface, and it's the first-priority 3D feature.
Choosing one sets `Emission Strength` to its existing value or 1, as source. In the menu, the
current composition and any that contain this layer are disabled, with the tooltip
`A composition can’t play inside itself.` If the chosen composition is deleted, the pop-up
returns to `None`.

Texture slot: value-column pop-up-style button showing the file name or `Choose…`; `NSMenu`
`Choose Image…`, `Remove`. Undo names `Set Texture`, `Remove Texture`.

### Light

`Type` pop-up (`Softbox`, `Spotlight`, `Bulb`, `Sun`), `Color`, `Brightness`. `Softbox`:
`Width`, `Height`. `Bulb` and `Spotlight`: `Range`, `Falloff`. `Spotlight`: `Spread` 1–89°,
`Softness` 0–1. Then `Cast Shadows`. Position and target live in the transform rows.

### Camera

`Projection` pop-up (`Perspective`, `Orthographic`); `Field of View` 1–150° or `Zoom`;
`Clip Start`, `Clip End`; `f-Stop` 0–64, tooltip `0 keeps everything sharp.`; `Focus`,
tooltip `0 focuses on the aim point.`

**Shot** section, under Camera:

| Row | Control |
|---|---|
| `Set Up Studio` | Push button, same placement as `Drop to Floor`. Undo `Set Up Studio` |
| `Add Shot` | Pull-down in the value column, title `Add Shot`, four-second shot as source. Items, with subtitle: `Turntable` `The selected model or group turns 360°.` · `Push-In` `The camera moves 30 % closer.` · `Orbit Reveal` `The camera orbits 90° around its aim.` · `Hero Tilt` `The camera rises into a low hero angle.` · `Rack Focus` `Focus moves from near to far.` |
| `Final Quality` | Pop-up: `Draft · 16`, `Standard · 64`, `High · 256`, `Best · 1024`. Tooltip `Samples per frame for exports.` Disabled for an orthographic camera |
| `Studio Set` | Checkbox. Undo `Studio Set` |

The source's row of chips becomes one pull-down, since these are commands and language
§9 rules out chip rows. Shots write ordinary keyframes, editable in the timeline.

**Environment** section: `Ambient`, `Ambient Color`, `Exposure`, `Shadows` checkbox,
`Background` checkbox with a colour well after it when on.

## 9. Upgrading old projects

Old 3D layers (`Legacy 3D Scene`, the earlier model layers) upgrade automatically when
the project opens, in memory, with no prompt, as one step before the first frame. There's
no compatibility mode and no way to keep the old look. The file on disk changes only at the
person's next save, and from then on it uses the new format, so it won't open in the
original app (N003).

| Outcome | Surface |
|---|---|
| All layers upgraded | Viewer banner, info, once per project: `3D layers were upgraded, so they may look different.` |
| Nothing to upgrade | Nothing |
| A layer can't upgrade | That layer keeps its data and draws nothing. Properties shows Error `This 3D layer couldn’t be upgraded. {reason}` with `Try Again`. The project stays editable, and nothing is dropped from the file |
| Reasons (from `migration.ts`) | `The model file is missing.` (and the offline `Relink…`) · `It’s animated for more than 2,398 frames. Split or trim it, then try again.` · `Its background is animated. Move that to a separate layer, then try again.` |

Upgrade ignores layer locks. It isn't an undo step, because no edit was made.
`Convert to 3D Layers` is removed as a command.

## 10. Progress and failure

Inline Error rules (panels/README §3) apply. Viewer banners use README §3 and read like a
person; raw engine text goes to the Error Log (S053).

| Situation | Surface | Copy |
|---|---|---|
| Wrong file type | Viewer banner, error | `Choose an OBJ, GLB, glTF, STEP, IGES, BREP, PLY or .splat file.` |
| GLB too big | banner | `This GLB is over the 512 MB limit.` |
| Too many triangles | banner | `This model has more than 10 million triangles, so it wasn’t imported.` |
| Too many parts | banner | `This model has more than 65,536 parts, so it wasn’t imported.` |
| Too many splats | banner | `This file has more than 16 million splats, so it wasn’t imported.` |
| No usable geometry | banner | `This model has no geometry Powermove can use.` |
| CAD unreadable | banner | `This CAD file couldn’t be read. Export it again as STEP and try again.` |
| CAD conversion failed | banner | `This CAD model couldn’t be converted.` |
| Bad texture | banner | `A selected texture couldn’t be read, so the model wasn’t imported.` |
| Bad splat or PLY | banner | `This file isn’t a splat or point cloud Powermove can read.` |
| Failure in Background Tasks | row Failed | same copy, `Retry` |
| Model too large for Studio | banner, info, once per project | `This model is too large for Quality, so it draws in Fast.` |
| Quality stops (GPU lost, out of memory) | banner, warning, with `Try Again` | `Quality preview stopped, so the viewer is showing Fast.` |
| Scene won't load | Properties Error; viewer keeps the last frame | `This 3D layer couldn’t be loaded.` |
| Missing model file | Offline treatment | media-browser.md `Relink…` |
| Refused edit | banner | the refusal, rewritten to the copy style |
| Locked layer | no handles | `Layer is locked` tooltip |
| Export waits for a model | Export task detail | `Preparing 3D model…` |
| Export frame fails | Export task Failed | `Couldn’t render 3D at {timecode}. Your project wasn’t changed.` |

Exports always wait for Studio and never fall back to Fast, except a model too large for
Studio, or an orthographic camera, which render as they preview.

## 11. States and accessibility

| State | Spec |
|---|---|
| Empty scene | No special state: a composition with no 3D layers shows nothing of this spec |
| Loading | Viewer-chrome Loading; the last good frame stays visible |
| Light and dark | Bar and Properties follow the app. Handles, capsule and Light panel use dark-scheme tokens in both |
| Increase Contrast | Axis lines and handles gain a `c.separator` keyline; capsule and Light panel take the system border |
| Reduce Transparency | Capsule and Light panel frame go solid through `Tokens::material` |
| Reduce Motion | Framing jumps; capsule and Light panel cross-fade with `mo.reduced`. Refining itself continues, because it's content |
| VoiceOver | Segments are radio groups named in §2; the `3D` pull-down announces the current tool. Selected tool changes post nothing extra |

## 12. Not in the source, so not in native yet

The proposal describes these, and the branch doesn't have them. Lanes don't build them.
`explode`, `place_on` and `soften` stay available to agents through `edit_3d`.

Human controls for exploded views and place-on; a Focus tool; ⌥-click rim and ⇧-click fill
lights; dragging a swatch onto a part, and hover preview; per-preset simple sliders; USD
import; the corner axis cluster; a light ball; `Look` presets. See the owner question in
[README §6](README.md#6-owner-questions-raised-by-these-specs).

## Don't

- Don't show Live as a status, or any percentage the model doesn't report.
- Don't put glass in the viewer bar or inside Properties.
- Don't use `c.accent` for a selected segment or a finish tile.
- Don't use a popover for the Light panel.
- Don't draw handles from the renderer, or let them wait on it.
- Don't add a compatibility mode, a Blender route or `.blend` import.
