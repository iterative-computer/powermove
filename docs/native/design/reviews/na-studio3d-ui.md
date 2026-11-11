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
| Keyboard walkthrough | ❌ not recorded. New controls are kit controls (PopUpButton, SegmentedControl with arrow keys, ToolbarItem actions, Checkbox), so they inherit kit focus; not verified live |
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
| Error icon baseline | ⚠️ the triangle sits a point high against the first text line |

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

❌ Not walked through (see §0). Segmented control supports arrow keys; pop-ups
and checkboxes are system-standard.

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
the layer error is `role="status"`). Information never by colour alone. ❌
VoiceOver rotor not recorded.

## Open items (not built tonight)

1. Viewer 3D toolbar beyond Fast | Quality (Camera/Free View, transform and
   navigation tools, Frame), gizmos, the Light tool and Light panel (lights
   by pointing), and the Refining/Clean status capsule. A CPU ray query exists
   in `pm-geom3d::query`; the engine's GPU `pick_surface` is not routed
   through the viewer worker.
2. Text/shape extrusion (Front, Bevel, Side, Back), layers/groups as material
   sources and 1/2/4 views: no native document data or commands yet.
3. Engine: one bad layer blanks the whole 3D set; the seamless studio set
   draws as a black void; imports land at the origin (no floor drop or studio
   set on import, which needs the scene import archive in the app).
