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
| Keyboard walkthrough | Round 2: done, see Round 2 · Keyboard (K1/K2 fail app-wide) |
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

## Open items

1. Shell keyboard: K1 (Tab traversal) and K2 (focus-visible modality), above.
2. Viewer polish: Scene/Object Axes toggle, axis letters, plane handles, hover
   hints on parts, tooltips' second line (§2), View ▸ 3D menu, the collapsed
   `3D` pull-down below `m.viewer_3d_collapse`, the status capsule
   (Refining · n %, Clean) and the Light panel (§6).
3. Waiting on engine data (Sol's queue): per-layer failure isolation, text
   extrusion (Front, Bevel, Side, Back), layers/groups as materials, 1/2/4
   views, the black studio backdrop, import placement; then Screen materials
   can be re-enabled.
4. Engine findings: fog darkens the foreground in Quality; the shadow catcher
   draws a black void (Set Up Studio's seamless set); Fast draws terrain
   darker than Quality.
