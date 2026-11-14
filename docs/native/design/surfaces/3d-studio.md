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
| Quiet at rest | A 3D composition adds almost nothing to the 2D editor (owner decision, 2026-11-13): no 3D toolbar, no camera inset, no lighting panel. The status capsule shows only while it has news; `View camera` only while looking around |
| Exact under the hand | Handles draw on the frame they're pressed and never wait for the renderer. The engine answers picks and projected points; the host draws every handle (`docs/3d-studio-redesign.md` §4.1a) |
| One colour language | Accent is selection. Axis colours are data. Material swatches and light colours are content. Nothing else in this family has colour |
| Numbers stay put | Values are the inspector's tabular wells. No value floats beside the pointer |
| One gesture, one Undo | Every drag, slider pull, finish, shot and import is one undo step with the name listed here |

**Words.** The source uses two sets of light names. Native uses one everywhere: `Softbox`,
`Spotlight`, `Bulb`, `Sun`. The stored types stay `area`, `spot`, `point`, `sun`, so
projects and agent operations don't change. `Intensity` becomes `Brightness` and `Cone
Angle` becomes `Spread`, as Properties shows them. Spelling
follows the app's existing `Color`: `Aluminum`, not `Aluminium`.

Layer kinds: model `cube` on `c.labels.blue`, light `lightbulb` on `c.labels.gray`, camera
`camera` on `c.labels.gray`, in the kind badge (language §8). Camera layers keep their
pinned place at the top of the timeline.

## 2. The viewer in a 3D composition

**Owner decision, 2026-11-13** (supersedes the earlier 3D toolbar): the 3D experience
adds the least new UI over 2D. It uses the existing viewer, the window's tools and the
existing Properties panel. There is no 3D toolbar group in the viewer bar, no camera
inset, no split and no lighting panel.

| State | Viewer |
|---|---|
| At rest: the shot | The composition at its aspect, with the pasteboard, as any 2D composition. The camera's view is what renders |
| Looking around: the editor view | The first orbit, pan or zoom leaves the shot. The editor view fills the **whole** viewer body, edge to edge (cover fit, never letterboxed, inset or split). It never edits the shot camera's position, rotation or keyframes |
| Return | While away from the shot one control sits at the stage's **top-left**, `m.inset` from both edges: `View camera` (`camera`), glass `ViewerOverlay`. It eases back to the shot and goes away. The zoom pop-up's slot reads `Editor View` meanwhile |

**Transport.** The viewer's play controls and timecode show only while no timeline is in
the workspace (another panel maximised, a workspace without a timeline). With a timeline
on screen its header owns play and the time; the viewer shows neither, paused or playing.

The 3D tools stay available without a toolbar: `View ▸ 3D` lists Camera, Free View, Move,
Rotate, Scale, Orbit, Pan, Zoom, Light, the axes, Frame, Views and Fast or Quality, with
their checkmarks (language §9: the menu bar is the complete command surface). The choice
of Fast or Quality is remembered.

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

Navigation only ever moves the transient editor view, never the shot, and adds no undo
step. The first gesture leaves the shot for the editor view (§2).

| Gesture | Result |
|---|---|
| Two-finger trackpad scroll | Orbit |
| ⇧ two-finger scroll | Pan |
| Pinch, mouse wheel, ⌥ or ⌘ scroll | Zoom (dolly) |
| ⌥-drag or middle-drag | Orbit; ⇧⌥-drag pans |
| The window's Hand tool | Drag orbits, ⇧-drag pans |
| The window's Zoom tool | Drag dollies |
| ⇧ during Orbit or Zoom | Fine, one tenth speed |
| F or Home | Frame the selection, or everything when nothing's selected |
| `View camera` | Back to the shot |

While looking around the shot camera draws as its frustum and the frame it renders, in
`c.secondary`, so the editor view shows where the shot is. There's no view cube and no
axis ball.

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

**Lights** (as Electron's light markers). Every light has a marker, always: a ring of
`MARKER` (9.5 pt) with its kind's glyph (softbox, spotlight cone, bulb, sun rays) in
`c.text` over a dark keyline. A light outside the view, or behind the eye, is pinned to
the edge of what the viewer shows, with a chevron pointing to it; a pinned marker only
selects. A press on a marker selects its light (⇧ toggles) and dragging moves it in the
view plane; markers win over the transform handles. The selected light adds a
`c.accent` ring, its softbox outline or spotlight cone, a dashed aim line and a ◆ at its
target: dragging the ◆ moves the target in the view plane. Rotate swings a light's aim
about its position (the renderer aims lights by their target, never by rotation). ⌥ while
moving a light carries its target with it. A `Sun` only rotates, a `Bulb` only moves, as
before; any light's marker can be dragged. Undo names: `Move Light`, `Aim Light`.

Modifiers as `gizmo.ts`: ⌃ snaps (1 unit, 15°, 0.1), ⇧ is fine. Esc cancels a drag and
restores the values. Undo names: `Move 3D Layer`, `Rotate 3D Layer`, `Scale 3D Layer`, plural
with several. The drag edits ordinary channels, so an animated property gets a keyframe at the
playhead, as any inspector edit does.

Gizmos aren't exposed to VoiceOver. Every transform is reachable from Properties.

## 6. Light tool

There is no Light panel (owner decision, 2026-11-13: "no lighting panel. properties should
be in properties"). The Light tool stays in `View ▸ 3D`: with it, a click on a model
aims the selected light so its highlight lands there, or adds a softbox there when no
light is selected. Everything the panel held lives in Properties (§8): Brightness, Size,
Distance, Spread, Temperature and Aim At. Its presets are `Layer ▸ Add 3D Layer` items
(§7). Agents keep `light_preset` and `aim_light`.

## 7. Add and import

**Add 3D Layer** menu: `Layer ▸ Add 3D Layer`, timeline and viewer context menus. Palette
entries `Add 3D {primitive}`, `Add {light}`, `Add Camera`.

`Cube` · `Sphere` · `Plane` · `Cylinder` · `Cone` · `Torus` · `Capsule` · `Icosphere` ·
separator · `Import 3D Model…` · separator · `Softbox` · `Spotlight` · `Bulb` · `Sun` ·
`Camera` · separator · `Rim Strip Left` · `Rim Strip Right` · `Top Softbox` ·
`Edge Highlight Card` · `Strip Light` (presets, placed for the selected model and the shot)

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
`Softness` 0–1. Then `Temperature` (K, writes `Color`), `Distance` (slides the light along
its aim, keeping its brightness on what it lights; not for `Sun`), `Aim At` (a model; not
for `Bulb`), and `Cast Shadows`. Transform shows `Position` and `Target`; a light's
rotation, which the renderer ignores, shows only while it holds a value or keyframe.

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
| Light and dark | Bar and Properties follow the app. Handles, capsule and `View camera` use dark-scheme tokens in both |
| Increase Contrast | Axis lines and handles gain a `c.separator` keyline; capsule and `View camera` take the system border |
| Reduce Transparency | Capsule and `View camera` go solid through `Tokens::material` |
| Reduce Motion | Framing jumps; capsule and `View camera` cross-fade with `mo.reduced`. Refining itself continues, because it's content |
| VoiceOver | `View camera` is a button named for its label; tool changes from `View ▸ 3D` post nothing extra |

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
- Don't add a lighting panel, popover or 3D toolbar; light values belong in Properties.
- Don't draw handles from the renderer, or let them wait on it.
- Don't add a compatibility mode, a Blender route or `.blend` import.
