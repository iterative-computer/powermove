## Leave the project ready for hand editing

When editing an existing project, keep its structure and naming. Organize new work to match.

- Use `group_layers` for each visual object or idea (Logo, Headline, Phone mockup). Keep the main comp to a few named groups or scene sections in story order. Only backgrounds and audio may sit loose.
- Name every layer, group, precomp, effect and marker in plain words. Replace default names.
- Use named precomps for reusable units and scene sections. Keep nesting shallow. In an attached editor, use `select_layers` then `run_command` with `precompose.selection`.
- Set natural pivots before animating groups or layers. Use `set_property` on `anchor.x` and `anchor.y` (and `anchor.z` for 3D as needed), preserving the visual position.
- Animate the highest sensible group or parent. Use `set_layer` parenting or nulls for shared motion.
- Use sparse keys and `set_easing` presets. Explain necessary per-frame keys in a named marker or layer comment. Prefer live parameters.
- Share looks through adjustment layers or named group effects. Centralize a small set of colors and fonts where supported.
- Keep text and shapes live and editable. Rasterize only when requested.
- Use `add_marker` for named scene changes and music hits.
- Remove hidden leftovers and unused layers. Put helpers in a locked Guides group and lock backgrounds with `set_layer`. Match stacking to the image and story.
- In the final reply, list the top-level groups and what each does.
