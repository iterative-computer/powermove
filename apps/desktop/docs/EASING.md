# Easing

Add **Easing** from the panel library. It docks anywhere and follows the app's
light or dark appearance.

Select timeline keyframes, choose a preset, or edit a curve and press **Apply**.
A standard curve changes interpolation without changing keyframe values or
times. Multi-keyframe curves rebuild the selected span, keeping the endpoints
and creating intermediate keys for overshoot, anticipation and bounce. Both
operations support project Undo/Redo and respect locked layers.

## Graph

- Drag handles to shape a curve; drag interior points to move them.
- Option-click adds or removes an interior point. Command-drag pulls out or
  separates handles. Shift constrains movement. Option-Command-drag mirrors
  the opposite handle of a standard curve.
- Double-click the background to reset. Enter cycles points; arrow keys adjust
  the active handle, with Shift for larger steps.
- **Input coordinates** exposes precise handle coordinates, including a point
  selector for multi-keyframe curves.
- Graph Undo/Redo is separate from project history. **Auto apply** makes a graph
  drag one undoable project edit. **Mirror** reverses the normalized curve.
- Copy selected timeline keys to load their curve. When several properties are
  selected, Copy offers a choice of which curve to show.

## Copy, paste and targeting

**Copy** captures selected keys. **Paste easing** preserves destination values
and times; **Paste values** preserves destination interpolation and times.
Source keys repeat when pasting to a larger selection. Right-click Copy to
**Duplicate at playhead** or **Mirror at playhead**, across layers/properties.

For standard curves and easing paste on macOS:

- Shift targets the incoming section of the selection.
- Option targets the middle section.
- Command targets the outgoing section.
- Combine modifiers to target multiple sections; no modifiers targets all.

The equivalent combinations are available in the Apply/preset context menus.
Multi-keyframe curves always replace the middle span, leaving external
interpolation unchanged. Select consecutive keys for these operations.

## Preset library

The panel includes 12 standard and 12 multi-keyframe presets. Save your edited
curve with **Save easing preset**. Add or rename groups, drag groups and presets
to reorder them, and drag presets onto another group to move them. Right-click
presets to rename, recolor, duplicate, move, export or delete them.

Panel settings choose simultaneous or toggled graph/library views, fixed or
automatic columns (0), preset size, labels, preview animation and five curve
colors. Preset hover previews respect Reduce Motion. Preferences and library
changes persist in the local app profile. Export a backup to move them between
computers.

Import/export the library, a group, or a single preset as Easing JSON. Import
also accepts scalar and vector `.kease` simple/extended preset files and unzipped
preset folders. Vector components become separately named X/Y/Z/W curves (or
R/G/B/A for named color presets), with independent handles. Constant components
import as linear easing; moving components use the same different-endpoints
requirement as scalar curve copying. Files are parsed as data, never executed. ZIP archives must be
unzipped first. Imports merge into new groups and cannot overwrite existing
presets. Invalid files are rejected before the library changes.

## Other tools

The tools menu includes editable overshoot/bounce expressions, removal of
expressions, linear/continuous/auto Bezier interpolation, motion-path cleanup,
and a position controller.

Motion-path cleanup separates timing from geometry. The original curved
position keys and native Bezier handles are preserved; they are never baked
into a polyline. Only provably redundant knots on straight, monotonic paths are
removed. An arc-length timing map changes the speed while every output
position is evaluated on the original curve. The map uses numerical distance
estimation; it does not approximate the route's geometry. Existing unrelated
position expressions must be removed first.

Cleanup selects the **Path progress** keys automatically. Apply new easing to
those selected keys to change the speed while preserving the route. Editing
the original position keys adjusts the geometry; the timing map updates after
edits, undo, redo and reloading the project. Repeating cleanup reuses the
existing controller and resets its timing to linear. Other animation outside
a cleaned selection's time range stays unchanged.

A position controller creates a named **Path progress** null layer. Its Opacity
channel acts as progress from 0–100; animate those keys to retime the path.
The original position channels describe the route. The bounded
`sample(layerId, channel, localTime)` helper reads raw channels;
`motionPath(layerId, channel, progress, start, end)` evaluates the original
position geometry at a travelled-distance fraction. Neither helper recursively
runs source expressions. Controllers and geometry are saved in the project
and work during playback, rendering and portable web export.

## Compatibility scope

This is an independent Powermove implementation of the publicly documented
standard and paid multi-keyframe workflows. It does not execute After Effects
scripts/expressions, reproduce the vendor's bundled preset collection, or
certify identical output to a particular After Effects/plugin version. Value
paste also supports color, text and boolean channels while preserving
interpolation. Curve operations require numeric channels. Position/scale
dimensions are separate numeric channels in Powermove; imported vector
components remain independently editable.

References: [feature overview](https://davideboscolo.com/product/kease-extension-for-after-effects/),
[graph editing](https://davideboscolo.com/graph-editor/),
[preset library](https://davideboscolo.com/easing-curves-presets/),
[modifier targeting](https://davideboscolo.com/easing-and-modifier-keys/),
[extra tools](https://davideboscolo.com/extra-tools/).
