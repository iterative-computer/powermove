# Design review: `feat/na-studio3d-ui` (native 3D Studio UI)

Reviewer: design lead (self-check of its own lane; needs a second reviewer's
sign-off) · 2026-11-11. Spec: [surfaces/3d-studio.md](../surfaces/3d-studio.md),
[language.md](../language.md), [tokens.md](../tokens.md). Evidence (outside
Git): `/tmp/pm-shots/na-studio3d-ui/` with `captions.md`.

Scope: Properties for 3D layers (header, 3D transform, World, Fog, Terrain,
Scatter, Material with Finish shelf, Texture Maps, Material Options, inline
layer errors), the composition's 3D Scene section (Sky, Render Quality, Shot,
Set Up Studio), Layer ▸ Add 3D Layer (including Import 3D Model…), the viewer
bar's Fast | Quality control, Export's 3D Quality row and estimate, timeline
kind badges, agent Undo names. The disclosure-group fix applies to every
inspector section.

**Result: passes for the surfaces in scope, with the open items below.**

## 0. Evidence

| Item | Result |
|---|---|
| Light and dark captures of every changed surface, 2×, real app (`screencapture -l`) | ✅ |
| Increase Contrast + Reduce Transparency | ✅ `after-12-*` (cube dark, World light) |
| Before/after pairs | ✅ `before-0*` / `after-*` |
| Interaction states that change appearance | ✅ hover (swatch tooltip), pressed/selected (Fast/Quality, pop-up menus), error, open/closed disclosure. Keyboard focus ring on the new controls not captured: ❌ |
| Keyboard walkthrough | Round 2 found K1/K2 app-wide; fixed in round 3 (see Round 3 · Keyboard) |
| `cargo test -p pm-design`; no token literals added to views | ✅ no token literals in views (swatch colours are content, per §8) |

## 1. Tokens and colour

| Item | Result |
|---|---|
| All metrics/colours from tokens | ✅ (`metrics.row`, `metrics.gap`, `metrics.inset`, `colors.danger`, `colors.muted`, …) |
| Neutral grey dark mode, floating surfaces lifted | ✅ unchanged surfaces; pop-up menus are system menus |
| Accent only as a mark | ✅ selected segment is `c.floating`, not accent; swatches have no selected state |
| Semantic colour paired with symbol and words | ✅ layer error: `exclamationmark.triangle.fill` + danger text with the fix |
| Timeline kind badges per §1 | ✅ model `cube` on blue, light `lightbulb` and camera `camera` on gray |

## 2. Materials

✅ No glass added. Properties stays solid; menus are `NSMenu`.

## 3. Layout and density

| Item | Result |
|---|---|
| One value column, row height, insets | ✅ all new rows use `InspectorRow` |
| Nothing clipped | ✅ after fixes: the quality detail moved from a status row to a full-width section note; the finish shelf wraps into balanced rows (the kit has no `flexWrap`); 3D Scale drops the 2D link button so three values fit; Export's 3D help shortened to one line |
| Panel matches spec | ⚠️ §8 asks for a push button for Set Up Studio and a pull-down for Add Shot; the kit has neither in the inspector, so Set Up Studio is a section action and Add Shot a pop-up whose first row is a disabled `Add Shot…` title |
| Error icon baseline | ✅ the triangle is centred on the first line of the message |

## 4. Typography and numbers

✅ Tabular wells unchanged; units after a space (`6 °`, `100 %` is carried by
the `Scale %` label on the three-axis row); no all-caps.

## 5. HIG placement

| Item | Result |
|---|---|
| Every command in the main menu | ✅ Layer ▸ Add 3D Layer (also in the viewer's context menu) |
| Menus are `NSMenu` | ✅; the submenu parent was disabled by a cleared action in an auto-enabling menu, fixed for View ▸ Tools too |
| Pop-up buttons for value choices | ✅ Sky, Render Quality, World Lighting/Preset, Fog Shape, Casts Shadows, Texture Maps, Export 3D Quality |
| Open panel | ✅ system `NSOpenPanel` for Import 3D Model… |

## 6. Keyboard

See Round 2 · Keyboard.

## 7. Copy

✅ Title Case for controls and menus (`Add 3D Layer`, `Import 3D Model…`,
`Set Up Studio`, `Show in Camera`), sentence case for notes and errors, `…`
on commands that ask for more, no developer words. Undo names: `Add <thing>`,
`Import <file>`, `Finish: <name>`, `Sky`, `3D Quality: <preset>`,
`Add Shot: <name>`, `Set Up Studio`, `Casts Shadows`, `Natural Ground`…

## 8. Motion

N/A — no new animation.

## 9. Accessibility

✅ Increase Contrast strengthens swatch, well and segment borders. Every new
control has a role and label (swatches are labelled actions with tooltips;
the layer error is `role="status"`). Information never by colour alone. VoiceOver: see Round 2.

## Round 2 (viewer 3D interaction, keyboard and VoiceOver) · 2026-11-11

Evidence (outside Git): `/tmp/pm-shots/na-studio3d-ui/round2/` with
`captions.md`, keyboard frames in `kb/`, accessibility trees `ax-tree.txt`
(before) and `ax-tree-after.txt`.

Scope: the viewer's 3D group (§2: Camera | Free View, Move, Rotate, Scale,
Orbit, Pan, Zoom, Light, Frame, Fast | Quality), navigation (§4), gizmos
(§5), the Light tool's click-to-light (§6), selection by the engine's
surface pick, and the round-1 review notes (postcard Pine Trees, scatter
pickers, Finish spheres, lit first shape).

**Result: passes for pointer and VoiceOver; keyboard Tab traversal fails
app-wide (not 3D-specific), see K1.**

### Interaction

| Item | Result |
|---|---|
| Click selects what's drawn | ✅ engine `pick_surface` on the displayed frame, including in Free View (the pick worker uses the same view). Lights fall back to a bounds pick. Empty click deselects |
| Navigation never edits the shot | ✅ ⌥-drag / middle-drag orbit, ⇧⌥-drag pan, ⌥-scroll zoom, ⇧ fine; the first gesture switches to Free View; no Undo entry |
| Gizmos per §5 | ✅ projected bounds, axis colours X/Y/Z, Move arrows, Rotate rings with back halves hidden, Scale tips; only the active handle during a drag; ⌃ snaps, ⇧ fine, Esc restores. Undo `Move/Rotate/Scale 3D Layer(s)`. ⚠️ axis letters at the tips and plane squares not drawn yet |
| Light tool | ✅ click a model adds a softbox there (`Add Softbox`) or re-aims the selected light (`Aim Light`), clamped above the floor, rounded values. ❌ the §6 Light panel (Aim mode, sliders, kind) not built |
| Frame | ✅ toolbar Frame and F fit the selection, or everything |
| Arrow nudges | ✅ fixed: 3D layers move 0.1 scene units, 1 with ⇧, up is up (was a whole metre per press) |

### Keyboard (checklist §6)

| Item | Result |
|---|---|
| Ctrl-Tab / Ctrl-⇧-Tab between panels with a visible ring | ✅ Project → Viewer → Properties → Timeline → Effects |
| K1 · Full Keyboard Access: Tab reaches every control | ❌ **app-wide**. With Full Keyboard Access on, Tab from a focused panel, or from an editing number field, leaves focus where it is (AX focus stays on the panel; screenshots show no ring move). The shell's key monitor sends `shellFocusNext`, which calls `window.focus_next`, but focus doesn't move in the running app. The shell unit test that cycles stops passes, so the live tab-stop map differs from the test's. Owner: shell lane |
| K2 · Focus ring drawn on controls | ❌ follows from K1, plus: the shell swallows Tab before GPUI sees it, so GPUI's input modality stays "mouse" and `focus_visible_style` rings never draw even when focus does move. Fix in the shell: mark keyboard modality when it handles Tab (needs a small setter in vendored GPUI) |
| Segmented controls: arrows move the choice | ✅ in the kit (`ArrowLeft/Right/Home/End`); not reachable by keyboard until K1 is fixed |
| Arrow keys go to the focused control, else nudge | ⚠️ arrows reach a control only while the shell's `keyboard_control_focus` is set; one of three presses was lost during the nudge test |
| Esc cancels the innermost transient | ✅ a gizmo drag restores its values; navigation restores the starting view |

### VoiceOver (checklist §9, AX tree and AXPress)

| Item | Result |
|---|---|
| Viewer 3D group | ✅ `AXGroup «3D»` › `AXRadioGroup «Look Through»` (Camera, Free View), `«Tool»` (Move … Light), `AXButton «Frame»`, `«Preview»` (Fast, Quality) |
| Checked state | ✅ fixed: v4 views now forward AccessKit's toggled state to GPUI, so radio buttons and keyframe toggles read selected/unselected app-wide (they had no value before) |
| Operable without a pointer | ✅ AXPress on Quality, Light, Frame, a Finish swatch (Polished Steel) and the Coat and Glass disclosure all acted, and the tree updated |
| Copy read aloud | ✅ fixed `1 layer selected` (was `1 layers`) and `Cube, 3D shape layer` (was `3d`). Finish swatches are named buttons; transform wells read `Position X`, `Rotation X = 0 degrees`; ⚠️ `Scale % X` reads the percent sign as part of the label |
| Disclosure expanded state | ❌ AccessKit 0.26's macOS adapter maps disclosure triangles to plain buttons and has no expanded attribute, so VoiceOver can't say collapsed/expanded. Library limitation; the Effects tree works around it by putting the state in the row label |
| Off-screen rows | ⚠️ Layer rows (Parent, Track Matte, 3D, Solo, Visible, Motion Blur) expose only their labels while scrolled out of view: their native controls exist only on screen. App-wide panel behaviour |
| Pop-up order | ⚠️ native pop-ups (Screen, Zoom, Preview resolution) appear at the end of the window's AX children, not inside their rows. App-wide |
| Gizmos | ✅ not exposed, as §5 says; every transform is reachable from Properties |

## Round 3 (extrusion, layers as materials, views, polish, keyboard) · 2026-11-11

Evidence (outside Git): `/tmp/pm-shots/na-studio3d-ui/round3/` with
`captions.md`. The before shots come from merge 7aeeb8a4e (Sol's items 3–7),
built once before the round-3 code; the afters come from ec51f5791.

**Result: passes for the surfaces in scope. K1/K2 are fixed app-wide.
Open items are below.**

### Correction to round 2's K1

Round 2 drove Tab with `cliclick kp:tab`, which never arrives as a Tab key
event. With a real Tab (System Events key code 48), focus did move, but
only between panel containers and splitters, never into a control. So the
defect was real, and the diagnosis is now exact. Every v4 view tracked a
plain `FocusHandle`. A tracked handle's own `tab_stop`/`tab_index` decide
tab order (GPUI ignores the element's), so no kit control was ever a tab
stop.

### Keyboard (checklist §6)

| Item | Result |
|---|---|
| K1 · Tab reaches every control in order | ✅ fixed: v4 handles carry the node's `tabIndex`. `-1` stays focusable but out of the order (a segmented control's other segments). Regression test `tab_reaches_kit_controls_and_skips_roving_segments` |
| K2 · Focus ring visible | ✅ fixed: the shell marks its Tab/Ctrl-Tab as keyboard input (`Window::note_keyboard_input`, a small vendored-GPUI addition), so `focus_visible_style` draws. The shell's Tab test asserts it |
| Space / Return on a focused control | ✅ Space toggles Scene / Object Axes |
| Arrows inside a focused segmented control | ❌ the arrow reaches GPUI (no nudge), but the kit's keydown doesn't change the segment. App-wide; next for the kit/v4 owner |
| Native pop-ups (Views, Fit, Auto, Properties pop-ups) in the Tab order | ⚠️ AppKit controls sit outside GPUI focus; Tab passes them. They're reachable with VoiceOver and the mouse |

### Surfaces

| Item | Result |
|---|---|
| Extrude (text and shapes) | ✅ one click: depth is about a third of the type size (or the shape's smaller side), with a rounded bevel. The four surfaces start from the layer's colour: Front satin, Bevel brighter and partly metallic, Side and Back darker. An unlit composition also gets a key light and a sky that only lights. Labels: Extrude, Depth, Bevel (None / Angular / Rounded / Inset), Bevel Size, Inner Bevel, Detail (Draft / Standard / High); then Front, Bevel, Side, Back, each with Color, Roughness, Metalness and Color Map. Stored values are keyframeable; values not yet stored show what they start from. One Undo: `Extrude` |
| Layers and groups as materials (AE 27) | ✅ Color and Emissive maps list the composition's 2D layers and groups, and other compositions, on models and on each extrusion surface. Screen is enabled again (capture works). The source stays visible in the 2D comp, as in AE |
| 1, 2 and 4 views | ✅ the main viewer is pane 1 (Look Through, tools, gizmos, Preview). The other panes are embedded viewers, each with a corner pull-down for its camera (Shot Camera, Top, Front, Right, Left, Back, Bottom) and Fast or Quality. Two views add Top; four add Top, Front and Right. Views never edit the shot or Undo |
| Viewer polish | ✅ Scene / Object Axes (toolbar, menu, pull-down); axis letters; plane squares (drag in a plane); hover turns a handle to text colour; two-line tooltips; View ▸ 3D; the collapsed `3D` pull-down with a checkmark per setting; the Light panel |
| Light panel (§6) | ⚠️ built with Add, kind, name and kind, and Brightness / Size / Distance / Spread (one Undo each). Deviations: it uses the kit's number wells, because the kit has no `NSSlider`, so dragging is linear rather than log. There's no Aim segment (clicks aim as Reflect), and no enter/exit spring |
| Status capsule (§3) | ❌ not built: the engine reports no refine sample progress, and the spec asks for a real ring. Handed to Sol |
| Copy | ✅ the status line names the layer ("“Rounded” isn’t drawn. Select it to see why in Properties."), not the engine reason. "Pine Tree", not "Pine Tree Source" |
| Finish shelf | ✅ shaded spheres ship (round 2's after-05a was captured before that change) |

## Round 4 (first-time user walkthrough, estimate, capsule, views) · 2026-11-11

Evidence (outside Git): `/tmp/pm-shots/na-studio3d-ui/round4/` and the
captioned sequence in `round4/walkthrough/` (`captions.md`). Built from
merges of `feat/na-studio3d` up to df2d8073b.

**Result: the product-shot path works end to end for a first-time user,
from an empty project to an exported film. Each rough edge found on the way
is fixed or listed below.**

### Review notes from round 3

| Note | Result |
|---|---|
| Free View showed a flat 2D box offset from extruded letters | ✅ extruded text and shapes get a 3D bounds outline and the 3D move/rotate/scale handles in every view; moves write the layer's pixel channels |
| 2D layers covered the orthographic panes | ✅ AE-like: 2D layers draw only in the Shot Camera pane. Top/Front/Right hide the outermost 2D layers and groups (a group that holds 3D stays) |
| Pane 1 letterboxed, the others filled | ✅ every pane aspect-fits the composition inside the same inset, on the same backdrop |
| A broken layer in demo shots | ✅ the walkthrough scene has none |

### Status and estimates

| Item | Result |
|---|---|
| Status capsule (§3) | ✅ Quality only, bottom-trailing on the stage: "Refining · n %", "Refining · Denoising", then "✓ Clean" for 2 s. Fed by the engine's refine progress |
| Export estimate | ✅ the sheet asks the viewer to measure whenever size, length or scene changes; the footer says "measuring…" and then the measured range ("12–60 min", "38 min–3.2 h", number and unit on one line). A measurement superseded by the viewer's own redraw is asked again. It is usually ready before the sheet opens |
| Export progress | ✅ "About N min left" after 8 frames (the native export never sent it before) |

### Round-3 leftovers

| Item | Result |
|---|---|
| Arrow keys in a focused segmented control (app-wide) | ✅ the v4 key listener is on header-slot roots too, so ← → move the segment (Move → Rotate in the 3D bar) |
| Native pop-ups in the Tab order | ⚠️ still outside GPUI focus; reachable with the mouse and VoiceOver |
| Light panel Aim | ✅ Reflect / Direct / Orbit segment; Orbit drags the light around its target as one Undo ("Orbit Light") |
| Slider kit component | ⚠️ not built (the surface schema is frozen); the panel keeps number wells |
| 3D handles for extruded layers | ✅ (see above) |
| Navigation and selection in extra panes | ✅ scroll zooms and drag pans a pane; a click selects through that pane's camera |
| View ▸ 3D checkmarks | ✅ |

### Walkthrough fixes

| # | Rough edge | Result |
|---|---|---|
| R1 | An empty project said nothing about 3D | ✅ Properties shows a 3D Scene section with a way in and Set Up Studio |
| R2 | An imported model landed unlit | ✅ the first model in an unlit composition comes with a key light, sky and floor |
| R3 | The layer was named "Bottle.obj" | ✅ "Bottle" |
| R6 | Set Up Studio failed silently ("geometry not loaded") | ✅ works from retained bounds; framing targets the hero model, not the floor (R7); running it again replaces its own rig (R6b) |
| R11 | "Add Shot…" carried a checkmark | ✅ none |
| R12 | Shortening the composition left the ruler and bars at the old length | ✅ a view showing the whole old length refits; a zoomed or scrolled one stays |
| E1 | Export Video greyed the sheet and nothing appeared | ✅ the save panel was begun as a second sheet on the main window and AppKit queued it; it now attaches to the Export sheet |
| E2 | A dying render worker showed engine wording | ✅ "The renderer stopped unexpectedly…" |
| R4 | A model's selection box can be about twice its size (unit bounds before geometry loads) | ⚠️ engine: needs retained model bounds |
| R9 | Default white text is invisible on the white studio set | ⚠️ left as AE does it; noted |
| R10 | The first extrusion of a session once broke the render until reopen | ⚠️ not reproducible; for the engine lane |
| E3 | No time left during a long export | ✅ "About N min left" |
| E4 | A 26-minute export ended with a 2.2 s note | ✅ the status line keeps "Exported <name>"; the Dock icon bounces if Powermove is in the background |
| F1 | Fog blacked out Quality at any density while Fast showed light haze (uniform fog filling the schema's 10,000 units) | ✅ adding a terrain seeds the World's fog shape (Height Falloff 0.06, Visibility 400; Density stays 0), so raising Density gives the same aerial haze in both |
| F2 | Quality showed a silent dark stage before its first sample | ✅ the capsule says "Rendering…", then Refining · n % |

### Landscape walkthrough

Sky (the "World" layer), Terrain (with an overview camera), Pine Trees (320
on the terrain) and Atmosphere work from Layer ▸ Add 3D Layer and Properties.
With the fog fix, Fast and Quality agree. Found on the way (outside 3D):

| # | Rough edge | Owner |
|---|---|---|
| L1 | Project cards on Home show a grey "1920×1080" placeholder, never a thumbnail | Home |
| L2 | With two project windows, closing one with its close button and then choosing Quit stops with "Couldn’t quit: Close every project before releasing the library."; the other window drops to Home (reproduced twice) | lifecycle |
| L3 | The project left open by L2 (then force-stopped) won't open again: "The operation couldn’t be completed.", with no detail | lifecycle / recovery |
| A1 | A text field whose value is set through accessibility keeps drawing the old value (the stored value is right) | kit |
| A2 | New Project sheet: accessibility order lists Height before Width | Home |

## Round 5 (export promises, kit and accessibility leftovers, extruded surfaces) · 2026-11-12

Evidence (outside Git): `~/Developer/pm-shots/na-studio3d-ui/round5/` with
`captions.md` (before and after for each visible change).

### Export sheet

| Item | Result |
|---|---|
| Per-preset promises (owner decision 2026-11-11) | ✅ the 3D Quality row's second line states the chosen preset's promise in plain words. Draft: "A quick check of motion and timing, about 1 s a frame. It can look soft." Production: "A clean, final-looking picture: about 3 s a frame, up to 8 s with glass or chrome." Final: "Reference quality. It takes as long as it needs." The menu rows carry short subtitles (Check motion and timing · Clean, final-looking · Reference quality) |
| Estimate kept prominent | ✅ a Render Time row under 3D Quality ("Measured on this Mac for this scene and size", "Measuring on this Mac…", or "A rough guess…"), and the footer's first line is the time in the text colour, semibold ("12–60 min to render"), so it stays visible while the form scrolls |
| Render at 75% Size | ⚠️ a secondary checkbox, off by default ("Faster, slightly softer. Full size is always the default."), stored with the sheet's options and presets. The engine's export request has no 3D render-size field yet, so it renders at full size; the coordinator says so where the request is built. For Sol: a `studio_render_scale` (0.75) on the export request |

### Kit and accessibility

| Item | Result |
|---|---|
| Native pop-ups in the Tab order | ✅ every native control (pop-ups, checkboxes, switches, buttons, sliders) is a Tab stop with the panel's focus ring. Space, Return or ↑/↓ open a pop-up's menu (AppKit then runs the menu's own keys); Space toggles a checkbox or switch and presses a button; arrows step a slider |
| Tab trapped in multiline fields (found walking Tab through Properties) | ✅ Tab in the Text box typed tab characters into the layer's text and never left it. A plain multiline field now commits on Tab / Shift-Tab and focus moves on (Option-Tab types a tab; the code editor keeps Tab). A small vendored gpui-macos addition: the text slot reports `tab`/`backtab` |
| Slider component | ✅ `Slider` in the v4 kit: a native NSSlider, linear or logarithmic, one change (one Undo) per pull, arrow keys step it, and VoiceOver reads the value (not the log position). The Light panel uses it for Brightness, Size, Distance (log) and Spread, each with its reading beside |
| Pop-up row subtitles | ✅ an option's `detail` shows as the menu item's subtitle (macOS 14.4+), here and in every v4 pop-up |
| Home thumbnails | ✅ cards show a poster frame: a small Draft-quality still at the playhead, rendered off the main thread after a save or when the project closes (`<profile>/thumbnails`) |
| "Accessibility-edited field keeps its old value" (A1) | ✅ the field was right; the reading was wrong: a number field without a declared precision showed 0.002 as "0". Small values now keep two significant digits |
| New Project: Width before Height (A2) | ✅ the pair is built in reading order (Width, ×, Height), so VoiceOver and the key-view loop follow it |

### Extruded surfaces (Fable's list)

| Item | Result |
|---|---|
| Per-surface Emission and Opacity | ✅ the data model and the engine already had `opacity` (percent), `emissionColor` and `emissionIntensity` per surface, and the commands accepted them; Properties now shows Opacity, Emission and Emission Strength on Front, Bevel, Side and Back, plus an Emission Map beside the Color Map (a live layer, group or composition can make a surface glow) |

## Round 6 (Light panel motion, posters, showcase) · 2026-11-13

Evidence (outside Git): `~/Developer/pm-shots/na-studio3d-ui/round6/` and
`~/Developer/pm-shots/showcase/` (both with `captions.md`). The Mac sat at the
lock screen the whole round, so nothing could be activated or read through
accessibility. Items 1–2 are covered by unit tests, and the showcase was
built and rendered entirely through MCP.

| Item | Result |
|---|---|
| Light panel enter/exit (§6) | ✅ springs in (Enter: opacity and a short rise) and out (Exit) while it stays mounted for the exit. Any v4 node can opt in with `pmKit.presence` |
| Deleting a project deletes its poster | ✅ Move to Trash (Home or an agent) removes the Home poster; Put Back renders a new one at the next save or close. Delete Forever is still unsupported in the kernel |
| Showcase | ✅ `opus-showcase.mp4`, iPhone 18 Pro, dark studio, macro-to-hero camera move, Production at 1080p, plus 3 stills. The band is avoided by keeping the camera pitched down more than half the field of view (noted in its captions). Sol's far-clip catcher fix (039ef4e22) is merged but wasn't used for this render |

Found while making the showcase (for the engine/MCP lanes): `auto_studio`
needs loaded geometry, so it can't run headless; `light_highlight` can place
a light below the floor for a vertical surface seen from above; and
Production ran at about 13–22 s a frame for this glass-and-titanium phone.

## Open items

2. Export: wire Render at 75% Size once the engine takes a 3D render size.
3. Fog in a studio scene (no terrain) still uses the schema's 10,000-unit
   shape, which goes dark at any density; the landscape migration covers
   terrains only.
4. Engine: a model's selection box is about twice its size until geometry
   loads (unit bounds, R4); the first extrusion of a session once broke the
   render (R10, not reproduced); Fast's fog ignores shadowing in the
   atmosphere, so a thick fog that is black in Quality looks light in Fast;
   scenes saved with the old fog shape keep it.
5. Lifecycle and Home: L1–L3 and A1–A2 above.
