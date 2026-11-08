# Powermove Native design tokens

`apps/native/crates/design` (pm-design) is the single source of every colour,
metric, type size and motion value in Powermove Native. Views read
`pm_design::Tokens`; they never write a literal. This page explains the
tokens; the table at the end is **generated** from the crate and checked by
`apps/native/crates/design/tests/spec.rs`, so it cannot drift.

The language behind these values is [language.md](language.md). The gate a UI
lane must pass is [review-checklist.md](review-checklist.md).

## How to change a token

1. Edit `Tokens::new` (or add a field) in `apps/native/crates/design/src/lib.rs`.
2. Give a new field a role sentence in `ROLES` in `tests/spec.rs`.
3. Regenerate this table and the JSON snapshot:
   `PM_UPDATE_DESIGN_SNAPSHOT=1 flock /tmp/pm-build.lock cargo test -p pm-design`
4. Run the theme pipeline tests (`cd apps/native/js/themes && npm test`) if the
   field is theme-patchable, and regenerate the compat editions.
5. Explain the change in the commit and, if it changes the look, in language.md.

The tests also enforce the language's rules, not only the values:

| Rule | Test |
|---|---|
| Text ≥ 7:1, secondary and muted ≥ 4.5:1 on panel, field, floating and window; secondary ≥ 7:1 under Increase Contrast | `contrast_rules_from_language` |
| Accent text, danger, warning, success ≥ 4.5:1 on panel | same |
| Label on an accent fill ≥ 3:1 (13 pt semibold) and the accent mark ≥ 3:1 on panel | same |
| Text on every label colour and on the selected row ≥ 7:1 | same |
| Elevation order: floating > panel > window ≥ viewer (dark) | same |
| Dark neutrals are pure grey (R = G = B) | `dark_is_neutral_grey` |
| Radii are concentric (control = panel − gap), capsules are height ÷ 2, metrics sit on the 2 pt grid, type steps ascend | `concentric_radii_and_grid` |
| Glass only on toolbar, floating bars, popovers, viewer overlays, sheets, menus and the palette; solid under content | `materials_follow_hig` |
| Increase Contrast beats a theme patch; a bad patch changes nothing | `theme_patch_cannot_beat_increase_contrast` |
| Every JS theme mapping targets a patchable pm-design field | `theme_pipeline_targets_exist` |

## Fonts

| Constant | Value | Use |
|---|---|---|
| `TEXT_FONT` | `.SystemUIFont` (SF Pro, automatic optical size) | All UI text |
| `DISPLAY_FONT` | `SF Pro Display` | Display sizes ≥ 20 pt only |
| `MONO_FONT` | `SF Mono` | Code, expressions, shader source, logs. **Not** numbers: values and timecode use SF Pro with tabular figures (`monospacedDigitSystemFont`). |

## Theme mapping

The theme pipeline (`apps/native/js/themes/tokens.mjs`) projects v1–3 theme
tokens onto these fields. Only values a theme, workspace or extension
**changed** from the frozen v1.2.0 foundation are projected; an untouched
v1.2.0 value means "the default", so pm-design's native default stands. Lengths
move relative to the native value when the host passes its current tokens
(`environment.nativeTokens`). Translucent colours are composited over the
theme's panel colour. Increase Contrast is re-applied after every patch
(`Tokens::apply_patch`).

| v1–3 token | pm-design field(s) |
|---|---|
| `--accent` | `colors.accent` |
| `--accent-dim` | `colors.accent_wash` (composited over `--bg-panel`) |
| `--accent-tx` | `colors.accent_text` |
| `--on-accent` | `colors.selected_text` |
| `--bg-window` / `--bg-panel` / `--bg-panel-2` | `colors.window` / `colors.panel` / `colors.panel_secondary` |
| `--bg-sunken` | `colors.viewer` |
| `--bg-field` / `--bg-float` / `--bg-row-hi` | `colors.field` / `colors.floating` / `colors.hover` |
| `--line` | `colors.separator` (composited) |
| `--tx` / `--tx-2` / `--tx-3` | `colors.text` / `colors.secondary` / `colors.muted` (composited) |
| `--danger` / `--warning` / `--success` | `colors.danger` / `colors.warning` / `colors.success` |
| `--gap` / `--pad` | `metrics.gap` / `metrics.inset` |
| `--hdr-h` / `--row-h` / `--ctl-h` | `metrics.header` / `metrics.row` / `metrics.control` |
| `--r-lg` / `--r-sm` / `--r-xs` | `metrics.panel_radius` / `metrics.control_radius` / `metrics.inner_radius` |
| `--val-w` | `metrics.value_column` |
| `--fs-xs` | `typography.minimum` |
| `--fs-sm` | `typography.body`, `typography.label` |
| `--fs-md` | `typography.title`, `typography.reading` |
| `--fs-lg` | `typography.heading` |

Deliberately unmapped: `--r-pill` (a capsule is a shape, height ÷ 2, not a
radius), `--hairline` (a CSS device-pixel workaround; separator width belongs to
pm-design and Increase Contrast), `--fs-xl` (v1.2.0's 18 px heading has no
native role; `display` is native-only), shadows, focus-ring stacks, fonts and
easing curves (native materials and springs replace them).

## Generated table

Colours are sRGB. Sizes are points. Durations are seconds. Reduce Transparency
does not change any value; it changes `Tokens::material` to `Solid`.

<!-- tokens:begin -->
| Token | Role | Light | Dark | Light, Increase Contrast | Dark, Increase Contrast |
|---|---|---|---|---|---|
| `colors.accent` | Powermove orange: primary button, playhead, selection marks | `#F0580A` | `#FF6019` | `#F0580A` | `#FF6019` |
| `colors.accent_text` | Accent as text or a thin mark on panels (4.5:1) | `#B93800` | `#FF8A55` | `#9A2F00` | `#FFA073` |
| `colors.accent_wash` | Opaque accent tint: drop targets, active guide bed | `#FBE9E1` | `#3C241A` | `#FBE9E1` | `#3C241A` |
| `colors.danger` | Destructive and error text, offline media | `#C22A1F` | `#F36357` | `#C22A1F` | `#F36357` |
| `colors.field` | Value wells, text fields, segmented control track | `#F3F3F6` | `#262626` | `#F3F3F6` | `#262626` |
| `colors.floating` | Solid fallback for floating surfaces; lifted above panel | `#FFFFFF` | `#2C2C2C` | `#FFFFFF` | `#2C2C2C` |
| `colors.focus` | Keyboard focus ring | `#B93800` | `#FF8A55` | `#9A2F00` | `#FFA073` |
| `colors.hover` | Pointer hover fill on rows and borderless buttons | `#EBEBEF` | `#232323` | `#EBEBEF` | `#232323` |
| `colors.labels.blue` | Label: default for text and shape layers | `#C9D4E8` | `#3B4250` | `#C9D4E8` | `#3B4250` |
| `colors.labels.gray` | Label: guides, nulls, cameras | `#DCDCE0` | `#3A3A3A` | `#DCDCE0` | `#3A3A3A` |
| `colors.labels.green` | Label: audio | `#C5E3D1` | `#24503A` | `#C5E3D1` | `#24503A` |
| `colors.labels.orange` | Label: generated / solid layers | `#F7CFB6` | `#7A3A1F` | `#F7CFB6` | `#7A3A1F` |
| `colors.labels.red` | Label: user colour | `#F2CAC7` | `#6E2B2B` | `#F2CAC7` | `#6E2B2B` |
| `colors.labels.teal` | Label: video and image media | `#C2E2E3` | `#1F4B52` | `#C2E2E3` | `#1F4B52` |
| `colors.labels.violet` | Label: adjustment and effect layers | `#D8CFF0` | `#4A3B6B` | `#D8CFF0` | `#4A3B6B` |
| `colors.labels.yellow` | Label: user colour | `#EFE0B2` | `#5C4A17` | `#EFE0B2` | `#5C4A17` |
| `colors.media_badge` | Badge fill over media (duration, offline); always dark | `#000000` | `#000000` | `#000000` | `#000000` |
| `colors.muted` | Captions, metadata, placeholder, units | `#62646A` | `#949494` | `#414148` | `#BFBFBF` |
| `colors.panel` | Panel material: every docked work surface and sheet content | `#FDFDFE` | `#1A1A1A` | `#FDFDFE` | `#1A1A1A` |
| `colors.panel_secondary` | Recessed band inside a panel: timeline layer column, ruler | `#F5F5F7` | `#161616` | `#F5F5F7` | `#161616` |
| `colors.scope_blue` | Blue channel scope trace on the dark viewer | `#6BA6FF` | `#6BA6FF` | `#6BA6FF` | `#6BA6FF` |
| `colors.scope_green` | Green channel scope trace on the dark viewer | `#4FD58A` | `#4FD58A` | `#4FD58A` | `#4FD58A` |
| `colors.scope_red` | Red channel scope trace on the dark viewer | `#FF6B62` | `#FF6B62` | `#FF6B62` | `#FF6B62` |
| `colors.secondary` | Row labels, secondary text | `#52545A` | `#B4B4B4` | `#303036` | `#DDDDDD` |
| `colors.selected_fill` | Neutral selected row / segment fill | `#D5D5DA` | `#3A3A3A` | `#D5D5DA` | `#3A3A3A` |
| `colors.selected_text` | Text and glyphs on an accent fill | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |
| `colors.separator` | Hairline separators and panel edges | `#D4D4D8` | `#303030` | `#686868` | `#888888` |
| `colors.success` | Completed, online, rendered | `#117A52` | `#3FCF8E` | `#117A52` | `#3FCF8E` |
| `colors.text` | Primary text and glyphs | `#1B1D23` | `#F2F2F2` | `#1B1D23` | `#F2F2F2` |
| `colors.viewer` | Viewer surround; the darkest neutral so footage reads true | `#E4E4E4` | `#0E0E0E` | `#E4E4E4` | `#0E0E0E` |
| `colors.warning` | Warnings, missing-but-recoverable states | `#A15C00` | `#F59E0B` | `#A15C00` | `#F59E0B` |
| `colors.window` | Window ground behind panels, toolbar band and status bar | `#EEEEF1` | `#111111` | `#EEEEF1` | `#111111` |
| `metrics.agent_width` | Default agent panel width | 360 | 360 | 360 | 360 |
| `metrics.anchor_mark` | Anchor point crosshair circle | 11 | 11 | 11 | 11 |
| `metrics.border` | Separator thickness (Increase Contrast doubles it) | 1 | 1 | 2 | 2 |
| `metrics.capsule_radius` | Toolbar capsule radius (height ÷ 2) | 17 | 17 | 17 | 17 |
| `metrics.channel_strip` | Mixer channel strip width | 72 | 72 | 72 | 72 |
| `metrics.checker` | Transparency checkerboard square | 8 | 8 | 8 | 8 |
| `metrics.composer_max_lines` | Agent composer grows to this many lines, then scrolls | 10 | 10 | 10 | 10 |
| `metrics.control` | Standalone control height (buttons, popups) | 28 | 28 | 28 | 28 |
| `metrics.control_radius` | Control radius inside a panel (panel radius − gap) | 6 | 6 | 6 | 6 |
| `metrics.drag_opacity` | Opacity of a drag source left in place | 0.5 | 0.5 | 0.5 | 0.5 |
| `metrics.drag_threshold` | Pointer travel before a drag starts | 3 | 3 | 3 | 3 |
| `metrics.drop_stroke` | Drop-target inner stroke | 1.5 | 1.5 | 2 | 2 |
| `metrics.fader_knob` | Mixer fader knob height | 14 | 14 | 14 | 14 |
| `metrics.field` | Value well height inside a row | 22 | 22 | 22 | 22 |
| `metrics.field_menu_height` | Field menu maximum height | 280 | 280 | 280 | 280 |
| `metrics.field_menu_width` | Field menu width | 300 | 300 | 300 | 300 |
| `metrics.form_row` | Minimum Settings row height | 40 | 40 | 40 | 40 |
| `metrics.gap` | Gap between panels and to the window edge | 6 | 6 | 6 | 6 |
| `metrics.gizmo_handle` | Transform handle and path vertex square | 7 | 7 | 7 | 7 |
| `metrics.gizmo_keyline` | Gizmo stroke and keyline width over the image | 1 | 1 | 1 | 1 |
| `metrics.group_radius` | Settings grouped-form card corner | 10 | 10 | 10 | 10 |
| `metrics.header` | Panel header, viewer bar and timeline header height | 38 | 38 | 38 | 38 |
| `metrics.hit` | Minimum pointer target | 24 | 24 | 24 | 24 |
| `metrics.inner_radius` | Control nested in a control (segment in a track) | 4 | 4 | 4 | 4 |
| `metrics.inset` | Content inset inside a panel | 12 | 12 | 12 | 12 |
| `metrics.keyboard_step` | Splitter arrow-key step | 10 | 10 | 10 | 10 |
| `metrics.keyframe` | Keyframe diamond size | 9 | 9 | 9 | 9 |
| `metrics.kind_badge` | Layer-kind badge square in the timeline and Project list | 18 | 18 | 18 | 18 |
| `metrics.layer_column` | Timeline layer list column width | 266 | 266 | 266 | 266 |
| `metrics.mark_bracket` | Width of In and Out brackets on rulers and scrubbers | 6 | 6 | 6 | 6 |
| `metrics.media_badge_opacity` | Opacity of the media badge fill | 0.55 | 0.55 | 0.55 | 0.55 |
| `metrics.menu_inset` | Menu and palette row inset | 12 | 12 | 12 | 12 |
| `metrics.menu_row` | Palette row height | 30 | 30 | 30 | 30 |
| `metrics.min_center` | Smallest viewer extent | 180 | 180 | 180 | 180 |
| `metrics.min_height` | Minimum window height | 640 | 640 | 640 | 640 |
| `metrics.min_panel` | Smallest panel extent | 72 | 72 | 72 | 72 |
| `metrics.min_width` | Minimum window width | 980 | 980 | 980 | 980 |
| `metrics.palette_height` | Command palette maximum height | 360 | 360 | 360 | 360 |
| `metrics.palette_width` | Command palette width | 520 | 520 | 520 | 520 |
| `metrics.panel_radius` | Panel corner radius | 8 | 8 | 8 | 8 |
| `metrics.pasteboard_opacity` | Opacity of selected layers drawn outside the frame | 0.35 | 0.35 | 0.35 | 0.35 |
| `metrics.playhead` | Playhead line width | 1.5 | 1.5 | 1.5 | 1.5 |
| `metrics.popover_width` | Default popover width | 260 | 260 | 260 | 260 |
| `metrics.property_row` | Timeline property sub-row | 22 | 22 | 22 | 22 |
| `metrics.rotation_stem` | Rotation handle stem length | 18 | 18 | 18 | 18 |
| `metrics.row` | Inspector and list row height | 28 | 28 | 28 | 28 |
| `metrics.ruler` | Timeline ruler height | 24 | 24 | 24 | 24 |
| `metrics.section_gap` | Space above each Settings section after the first | 40 | 40 | 40 | 40 |
| `metrics.settings_height` | Settings window height | 520 | 520 | 520 | 520 |
| `metrics.settings_sidebar` | Settings sidebar width | 180 | 180 | 180 | 180 |
| `metrics.settings_width` | Settings window width | 760 | 760 | 760 | 760 |
| `metrics.sheet_inset` | Content inset inside sheets and Settings panes | 20 | 20 | 20 | 20 |
| `metrics.sheet_value_column` | Shared value column in sheets | 240 | 240 | 240 | 240 |
| `metrics.sheet_width` | Default sheet width | 480 | 480 | 480 | 480 |
| `metrics.sheet_width_wide` | Width of wide sheets: Export, Expression | 600 | 600 | 600 | 600 |
| `metrics.status` | Status bar height | 24 | 24 | 24 | 24 |
| `metrics.status_dot` | Attention dot and drop insertion-line end circle | 6 | 6 | 6 | 6 |
| `metrics.symbol_inline` | SF Symbol size inside rows | 11 | 11 | 11 | 11 |
| `metrics.symbol_panel` | SF Symbol size in panel headers | 13 | 13 | 13 | 13 |
| `metrics.symbol_toolbar` | SF Symbol size in toolbar items | 15 | 15 | 15 | 15 |
| `metrics.sync_badge` | Linked-clip sync badge height | 16 | 16 | 16 | 16 |
| `metrics.tangent_end` | Bézier tangent end circle | 6 | 6 | 6 | 6 |
| `metrics.text_padding` | Vertical text padding in bubbles, the composer and code blocks | 8 | 8 | 8 | 8 |
| `metrics.thumbnail_min` | Minimum Project grid thumbnail width | 120 | 120 | 120 | 120 |
| `metrics.toolbar` | Unified toolbar height | 52 | 52 | 52 | 52 |
| `metrics.toolbar_title_line` | Document title line height | 14 | 14 | 14 | 14 |
| `metrics.toolbar_title_width` | Document title slot width | 160 | 160 | 160 | 160 |
| `metrics.track_row` | Timeline layer row | 28 | 28 | 28 | 28 |
| `metrics.tree_indent` | Indent per outline level | 16 | 16 | 16 | 16 |
| `metrics.value_column` | Inspector value column width shared by every row | 128 | 128 | 128 | 128 |
| `metrics.viewer_3d_collapse` | Viewer width below which the 3D toolbar becomes one pull-down | 560 | 560 | 560 | 560 |
| `metrics.viewer_3d_labels` | Minimum viewer width for labels in the 3D toolbar | 860 | 860 | 860 | 860 |
| `metrics.welcome_height` | Welcome window height | 640 | 640 | 640 | 640 |
| `metrics.welcome_width` | Welcome window width | 900 | 900 | 900 | 900 |
| `metrics.window_height` | Default window height | 900 | 900 | 900 | 900 |
| `metrics.window_width` | Default window width | 1440 | 1440 | 1440 | 1440 |
| `typography.body` | Dense UI text: values, list rows, menus in panels | 12 | 12 | 12 | 12 |
| `typography.dense_leading` | Line height multiplier for dense UI | 1.3 | 1.3 | 1.3 | 1.3 |
| `typography.display` | Empty-state and onboarding display | 22 | 22 | 22 | 22 |
| `typography.display_optical_size` | Optical size for display | 22 | 22 | 22 | 22 |
| `typography.heading` | Sheet and Settings pane headings (semibold) | 15 | 15 | 15 | 15 |
| `typography.label` | Inspector row labels | 12 | 12 | 12 | 12 |
| `typography.medium_weight` | Medium weight: tabs, timecode, values being edited | 500 | 500 | 500 | 500 |
| `typography.minimum` | Smallest text anywhere: captions, ruler, status bar | 11 | 11 | 11 | 11 |
| `typography.reading` | Prose: agent replies, sheet explanations, Settings | 13 | 13 | 13 | 13 |
| `typography.reading_leading` | Line height multiplier for prose | 1.45 | 1.45 | 1.45 | 1.45 |
| `typography.regular_weight` | Regular weight | 400 | 400 | 400 | 400 |
| `typography.semibold_weight` | Semibold weight: titles, headings, primary buttons | 600 | 600 | 600 | 600 |
| `typography.text_optical_size` | Optical size for text | 12 | 12 | 12 | 12 |
| `typography.timecode` | Transport timecode (medium, tabular figures) | 13 | 13 | 13 | 13 |
| `typography.title` | Panel titles, section headings, window title (semibold) | 13 | 13 | 13 | 13 |
| `motion.bounce` | Spring bounce for appearing surfaces | 0.15 | 0.15 | 0.15 | 0.15 |
| `motion.enter` | Spring response for appearing surfaces (s) | 0.32 | 0.32 | 0.32 | 0.32 |
| `motion.exit` | Fade-out for dismissed surfaces (s) | 0.12 | 0.12 | 0.12 | 0.12 |
| `motion.interaction` | Hover, press and selection feedback (s) | 0.075 | 0.075 | 0.075 | 0.075 |
| `motion.overlay_idle` | Still time before the Focus overlay capsule fades (s) | 2 | 2 | 2 | 2 |
| `motion.reduced` | Cross-fade used under Reduce Motion (s) | 0.15 | 0.15 | 0.15 | 0.15 |
| `motion.start_scale` | Scale an appearing surface starts from | 0.96 | 0.96 | 0.96 | 0.96 |
| `motion.tooltip_delay` | Delay before a tooltip appears for hover or keyboard focus (s) | 0.38 | 0.38 | 0.38 | 0.38 |
| `motion.tooltip_warm` | Time window for an immediate tooltip swap to a neighbouring control (s) | 0.6 | 0.6 | 0.6 | 0.6 |
| `motion.wheel_rest` | Pointer rest before the scroll wheel edits a hovered value (s) | 0.5 | 0.5 | 0.5 | 0.5 |
| `translucency.edge` | Panel hairline edge in `text` ink | 0.08 | 0.06 | 0.08 | 0.06 |
| `translucency.line` | Header, column and status dividers in `text` ink | 0.09 | 0.07 | 0.09 | 0.07 |
| `translucency.panel` | Shared panel material over the window tint (1 = solid) | 0.72 | 0.7 | 0.72 | 0.7 |
| `translucency.window` | Window tint over the behind-window blur (1 = solid, Reduce Transparency) | 0.8 | 0.82 | 0.8 | 0.82 |
<!-- tokens:end -->

## Surface token additions — 2026-11-04

Added the nine Opus-specified tokens in [surfaces/README.md §7](surfaces/README.md#7-new-tokens-requested-by-these-specs),
with their exact values and roles. Decision (N1, N3): scope traces are data
colours, identical in both schemes and drawn with dark-scheme viewer tokens as
[colour.md](surfaces/colour.md) specifies; their contrast floor is 3:1 on that
viewer. Decision (N3, N6): these native surface metrics and data colours do not
extend the theme patch contract, so theme mappings and compat editions are
unchanged. Existing values and consumers remain in place (prune rule).

This adds tokens for the approved surface specs; it does not implement those
surfaces or establish macOS visual acceptance. Actual Mac output remains
provisional until Opus design review.

## Additional surface tokens — 2026-11-04

Added `metrics.viewer_3d_labels` (860 pt), `metrics.viewer_3d_collapse` (560 pt),
`motion.tooltip_delay` (0.38 s), and `motion.tooltip_warm` (0.6 s) from the
updated [surface token requests](surfaces/README.md#7-new-tokens-requested-by-these-specs).
Decision (N3): preserve Opus's exact thresholds and timings from
[3D Studio §2](surfaces/3d-studio.md#2-viewer-3d-toolbar) and
[Tooltips §6](surfaces/main-delta-ui.md#6-tooltips-u09), including under Reduce
Motion; these are geometry and interaction delays, not animation durations.
Decision (N3, N6): keep these native tokens outside the theme patch contract.
The spec test now covers all thirteen requests across both appearances and
every accessibility combination. Mac presentation and interaction still need
verification and Opus review.
