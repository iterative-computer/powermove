# Powermove Native design tokens

`apps/native/crates/design` (pm-design) is the single source of every colour,
size, type value and motion value. The system they encode is
[language.md](language.md); this page is the value table.

- `colors.*` come from `Colors::of`: seven OKLCH roles per appearance, the rest
  ink over a ground at an alpha step. Derived steps (`fill`, `fill_strong`,
  `hairline`, `raised`, `faint`) follow a theme's colour patch.
- `space.*` and `radius.*` are the scales; the older `metrics.*` names that
  panels and themes use (`gap`, `inset`, `control_radius`, `panel_radius`, …)
  are set from them.
- Token names are the contract of built-in panels, user extensions and themes
  (`Tokens::PATCHABLE`): add names, never rename one.
- A theme's `metrics.gap` also sets `metrics.dock_gap` (v1 themes have one gap).

To change a value: edit `crates/design/src/lib.rs`, then regenerate the table
and the snapshot with `PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design`.
Every token needs a role sentence in `crates/design/tests/spec.rs`.

<!-- tokens:begin -->
| Token | Role | Light | Dark | Light, Increase Contrast | Dark, Increase Contrast |
|---|---|---|---|---|---|
| `colors.accent` | Powermove orange: primary button, playhead, selection marks | `#F0580C` | `#FA6310` | `#F0580C` | `#FA6310` |
| `colors.accent_text` | Accent as text or a thin mark on panels (4.5:1) | `#B93800` | `#FF8B4D` | `#9A2F00` | `#FFA073` |
| `colors.accent_wash` | Opaque accent tint: drop targets, active guide bed | `#FCEDE6` | `#402617` | `#FCEDE6` | `#402617` |
| `colors.channel_blue` | Blue channel ink on panels (curves, channel values) | `#3263C3` | `#70ADFB` | `#3263C3` | `#70ADFB` |
| `colors.channel_green` | Green channel ink on panels (curves, channel values) | `#1D7D3E` | `#6FD087` | `#1D7D3E` | `#6FD087` |
| `colors.channel_red` | Red channel ink on panels (curves, channel values) | `#BA3630` | `#F47B74` | `#BA3630` | `#F47B74` |
| `colors.danger` | Destructive and error text, offline media | `#C22A1F` | `#F36357` | `#C22A1F` | `#F36357` |
| `colors.faint` | Placeholders, shortcut hints, disabled labels (derived) | `#9FA0A3` | `#6A6867` | `#9FA0A3` | `#6A6867` |
| `colors.field` | Value wells, text fields, segmented control track | `#F3F3F4` | `#282523` | `#F3F3F4` | `#282523` |
| `colors.fill` | Soft neutral fill: segmented tracks, search fields (derived) | `#F3F3F4` | `#292624` | `#EBEBED` | `#322F2D` |
| `colors.fill_strong` | Selected tab or segment over a raised track, never the accent (derived) | `#EFEFF0` | `#3D3A39` | `#E4E4E5` | `#4A4645` |
| `colors.floating` | Solid fallback for floating surfaces; lifted above panel | `#FFFFFF` | `#292524` | `#FFFFFF` | `#292524` |
| `colors.focus` | Keyboard focus ring | `#898A8D` | `#9A9997` | `#383A3F` | `#DBDBD9` |
| `colors.hairline` | Opaque control and card edge (derived) | `#EBEBED` | `#2B2826` | `#86878B` | `#807E7D` |
| `colors.hover` | Pointer hover fill on rows and borderless buttons | `#ECECED` | `#2E2C2A` | `#ECECED` | `#2E2C2A` |
| `colors.labels.blue` | Label: default for text and shape layers | `#C9D4E8` | `#3B4250` | `#C9D4E8` | `#3B4250` |
| `colors.labels.gray` | Label: guides, nulls, cameras | `#DCDCE0` | `#3A3A3A` | `#DCDCE0` | `#3A3A3A` |
| `colors.labels.green` | Label: audio | `#C5E3D1` | `#24503A` | `#C5E3D1` | `#24503A` |
| `colors.labels.orange` | Label: generated / solid layers | `#F7CFB6` | `#7A3A1F` | `#F7CFB6` | `#7A3A1F` |
| `colors.labels.red` | Label: user colour | `#F2CAC7` | `#6E2B2B` | `#F2CAC7` | `#6E2B2B` |
| `colors.labels.teal` | Label: video and image media | `#C2E2E3` | `#1F4B52` | `#C2E2E3` | `#1F4B52` |
| `colors.labels.violet` | Label: adjustment and effect layers | `#D8CFF0` | `#4A3B6B` | `#D8CFF0` | `#4A3B6B` |
| `colors.labels.yellow` | Label: user colour | `#EFE0B2` | `#5C4A17` | `#EFE0B2` | `#5C4A17` |
| `colors.media_badge` | Badge fill over media (duration, offline); always dark | `#000000` | `#000000` | `#000000` | `#000000` |
| `colors.muted` | Captions, metadata, placeholder, units | `#66676B` | `#9C9B99` | `#57595D` | `#BDBCBB` |
| `colors.panel` | Panel material: every docked work surface and sheet content | `#FDFDFE` | `#1C1917` | `#FDFDFE` | `#1C1917` |
| `colors.panel_secondary` | Recessed band inside a panel: timeline layer column, ruler | `#F5F5F7` | `#151311` | `#F5F5F7` | `#151311` |
| `colors.raised` | Raised neutral surface: selected segment or tab, neutral button, card (derived) | `#FFFFFF` | `#373433` | `#FFFFFF` | `#373433` |
| `colors.scope_blue` | Blue channel scope trace on the dark viewer | `#6BA6FF` | `#6BA6FF` | `#6BA6FF` | `#6BA6FF` |
| `colors.scope_green` | Green channel scope trace on the dark viewer | `#4FD58A` | `#4FD58A` | `#4FD58A` | `#4FD58A` |
| `colors.scope_red` | Red channel scope trace on the dark viewer | `#FF6B62` | `#FF6B62` | `#FF6B62` | `#FF6B62` |
| `colors.secondary` | Row labels, secondary text | `#515257` | `#B4B3B2` | `#3C3E43` | `#D7D6D5` |
| `colors.selected_danger` | Danger words on a selected row | `#A82319` | `#FF8A80` | `#A82319` | `#FF8A80` |
| `colors.selected_fill` | Neutral selected row / segment fill | `#E6E6E7` | `#363332` | `#E6E6E7` | `#363332` |
| `colors.selected_secondary` | Metadata on a selected row (muted fails there) | `#515257` | `#B4B3B2` | `#3C3E43` | `#D7D6D5` |
| `colors.selected_text` | Text and glyphs on an accent fill | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |
| `colors.separator` | Hairline separators and panel edges | `#DCDDDE` | `#363332` | `#86878B` | `#807E7D` |
| `colors.success` | Completed, online, rendered | `#107A52` | `#40CF8E` | `#107A52` | `#40CF8E` |
| `colors.text` | Primary text and glyphs | `#1D1F25` | `#F5F5F4` | `#1D1F25` | `#F5F5F4` |
| `colors.viewer` | Viewer surround; the darkest neutral so footage reads true | `#E6E6EA` | `#090807` | `#E6E6EA` | `#090807` |
| `colors.viewer_grid` | The viewer's grid over the picture | `#D8D8D8` | `#D8D8D8` | `#D8D8D8` | `#D8D8D8` |
| `colors.viewer_guide` | Ruler guides over the viewer's picture | `#0097BC` | `#2CCCEB` | `#0097BC` | `#2CCCEB` |
| `colors.warning` | Warnings, missing-but-recoverable states | `#A15C01` | `#F59E0A` | `#A15C01` | `#F59E0A` |
| `colors.window` | Window ground behind panels, toolbar band and status bar | `#EEEEF1` | `#0C0A09` | `#EEEEF1` | `#0C0A09` |
| `metrics.agent_gap` | Space between turns in the Agent conversation | 16 | 16 | 16 | 16 |
| `metrics.agent_height` | Agent popover height when the window has room | 680 | 680 | 680 | 680 |
| `metrics.agent_width` | Default agent panel width | 440 | 440 | 440 | 440 |
| `metrics.anchor_mark` | Anchor point crosshair circle | 11 | 11 | 11 | 11 |
| `metrics.border` | Separator thickness (Increase Contrast doubles it) | 1 | 1 | 2 | 2 |
| `metrics.capsule_radius` | Toolbar capsule radius (height ÷ 2) | 17 | 17 | 17 | 17 |
| `metrics.channel_strip` | Mixer channel strip width | 72 | 72 | 72 | 72 |
| `metrics.checker` | Transparency checkerboard square | 8 | 8 | 8 | 8 |
| `metrics.clip_radius` | Timeline clip bar corner radius | 6 | 6 | 6 | 6 |
| `metrics.composer_max_lines` | Agent composer grows to this many lines, then scrolls | 10 | 10 | 10 | 10 |
| `metrics.control` | Standalone control height (buttons, popups) | 28 | 28 | 28 | 28 |
| `metrics.control_radius` | Control radius inside a panel (panel radius − gap) | 8 | 8 | 8 | 8 |
| `metrics.curve_editor` | The curve editor's least graph height | 200 | 200 | 200 | 200 |
| `metrics.curve_editor_max` | The curve editor's greatest graph height in a wide panel | 360 | 360 | 360 | 360 |
| `metrics.curve_editor_wide` | The curve editor's greatest width in a wide panel | 640 | 640 | 640 | 640 |
| `metrics.dock_gap` | Space between docked panels and to the window edge | 8 | 8 | 8 | 8 |
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
| `metrics.grade_field` | A colour wheel's master field under the wheel | 72 | 72 | 72 | 72 |
| `metrics.grade_pair` | Least width of two colour wheels side by side on their plates | 254 | 254 | 254 | 254 |
| `metrics.grade_wheel` | A colour wheel's least diameter | 112 | 112 | 112 | 112 |
| `metrics.grade_wheel_max` | A colour wheel's greatest diameter in a wide panel | 220 | 220 | 220 | 220 |
| `metrics.group_radius` | Settings grouped-form card corner | 12 | 12 | 12 | 12 |
| `metrics.header` | Panel header, viewer bar and timeline header height | 38 | 38 | 38 | 38 |
| `metrics.hit` | Minimum pointer target | 24 | 24 | 24 | 24 |
| `metrics.inner_radius` | Control nested in a control (segment in a track) | 6 | 6 | 6 | 6 |
| `metrics.inset` | Content inset inside a panel | 12 | 12 | 12 | 12 |
| `metrics.keyboard_step` | Splitter arrow-key step | 10 | 10 | 10 | 10 |
| `metrics.keyframe` | Keyframe diamond size | 9 | 9 | 9 | 9 |
| `metrics.kind_badge` | Layer-kind badge square in the timeline and Project list | 18 | 18 | 18 | 18 |
| `metrics.label_column` | Most an inspector row's label takes before its value | 120 | 120 | 120 | 120 |
| `metrics.layer_column` | Timeline layer list column width | 320 | 320 | 320 | 320 |
| `metrics.mark_bracket` | Width of In and Out brackets on rulers and scrubbers | 6 | 6 | 6 | 6 |
| `metrics.media_badge_opacity` | Opacity of the media badge fill | 0.55 | 0.55 | 0.55 | 0.55 |
| `metrics.menu_inset` | Menu and palette row inset | 12 | 12 | 12 | 12 |
| `metrics.menu_row` | Palette row height | 30 | 30 | 30 | 30 |
| `metrics.message_inset` | Leading room beside a person's message in the Agent conversation | 52 | 52 | 52 | 52 |
| `metrics.min_center` | Smallest viewer extent | 180 | 180 | 180 | 180 |
| `metrics.min_height` | Minimum window height | 640 | 640 | 640 | 640 |
| `metrics.min_panel` | Smallest panel extent | 72 | 72 | 72 | 72 |
| `metrics.min_width` | Minimum window width | 980 | 980 | 980 | 980 |
| `metrics.palette_height` | Command palette maximum height | 360 | 360 | 360 | 360 |
| `metrics.palette_width` | Command palette width | 520 | 520 | 520 | 520 |
| `metrics.panel_radius` | Panel corner radius | 12 | 12 | 12 | 12 |
| `metrics.pasteboard_opacity` | Opacity of selected layers drawn outside the frame | 0.35 | 0.35 | 0.35 | 0.35 |
| `metrics.pill_inset` | Track-to-segment padding and segment gap | 2 | 2 | 2 | 2 |
| `metrics.pill_radius` | Segmented track and tab-strip corner | 8 | 8 | 8 | 8 |
| `metrics.playhead` | Playhead line width | 1 | 1 | 1 | 1 |
| `metrics.popover_radius` | Corner of popovers Powermove frames itself (agent popover) | 16 | 16 | 16 | 16 |
| `metrics.popover_width` | Default popover width | 260 | 260 | 260 | 260 |
| `metrics.property_row` | Timeline property sub-row | 22 | 22 | 22 | 22 |
| `metrics.rotation_stem` | Rotation handle stem length | 18 | 18 | 18 | 18 |
| `metrics.row` | Inspector and list row height | 28 | 28 | 28 | 28 |
| `metrics.ruler` | Timeline ruler height | 24 | 24 | 24 | 24 |
| `metrics.section_gap` | Space above each Settings section after the first | 40 | 40 | 40 | 40 |
| `metrics.segment_radius` | Segment and tab corner inside a pill (concentric) | 6 | 6 | 6 | 6 |
| `metrics.settings_height` | Settings window height | 520 | 520 | 520 | 520 |
| `metrics.settings_sidebar` | Settings sidebar width | 200 | 200 | 200 | 200 |
| `metrics.settings_width` | Settings window width | 760 | 760 | 760 | 760 |
| `metrics.sheet_body_max` | Tallest sheet option body before it scrolls | 440 | 440 | 440 | 440 |
| `metrics.sheet_inset` | Content inset inside sheets and Settings panes | 20 | 20 | 20 | 20 |
| `metrics.sheet_value_column` | Shared value column in sheets | 240 | 240 | 240 | 240 |
| `metrics.sheet_width` | Default sheet width | 480 | 480 | 480 | 480 |
| `metrics.sheet_width_wide` | Width of wide sheets: Export, Expression | 600 | 600 | 600 | 600 |
| `metrics.sidebar` | Sidebar width of Home and Settings | 224 | 224 | 224 | 224 |
| `metrics.status` | Status bar height | 24 | 24 | 24 | 24 |
| `metrics.status_dot` | Attention dot and drop insertion-line end circle | 6 | 6 | 6 | 6 |
| `metrics.symbol_action` | SF Symbol size of a surface's one primary action glyph | 20 | 20 | 20 | 20 |
| `metrics.symbol_inline` | SF Symbol size inside rows | 12 | 12 | 12 | 12 |
| `metrics.symbol_panel` | SF Symbol size in panel headers | 14 | 14 | 14 | 14 |
| `metrics.symbol_toolbar` | SF Symbol size in toolbar items | 14 | 14 | 14 | 14 |
| `metrics.sync_badge` | Linked-clip sync badge height | 16 | 16 | 16 | 16 |
| `metrics.tab` | Text tab and full-height segmented control | 28 | 28 | 28 | 28 |
| `metrics.tangent_end` | Bézier tangent end circle | 6 | 6 | 6 | 6 |
| `metrics.text_padding` | Vertical text padding in bubbles, the composer and code blocks | 8 | 8 | 8 | 8 |
| `metrics.thumbnail_min` | Minimum Project grid thumbnail width | 120 | 120 | 120 | 120 |
| `metrics.titlebar` | Title bar band of a plain panel window; content starts below it | 28 | 28 | 28 | 28 |
| `metrics.toolbar` | Unified toolbar height | 52 | 52 | 52 | 52 |
| `metrics.toolbar_title_line` | Document title line height | 14 | 14 | 14 | 14 |
| `metrics.toolbar_title_width` | Document title slot width | 160 | 160 | 160 | 160 |
| `metrics.track_row` | Timeline layer row | 30 | 30 | 30 | 30 |
| `metrics.tree_indent` | Indent per outline level | 16 | 16 | 16 | 16 |
| `metrics.value_column` | Least width of an inspector row's value column | 128 | 128 | 128 | 128 |
| `metrics.viewer_3d_collapse` | Viewer width below which the 3D toolbar becomes one pull-down | 560 | 560 | 560 | 560 |
| `metrics.viewer_3d_labels` | Minimum viewer width for labels in the 3D toolbar | 860 | 860 | 860 | 860 |
| `metrics.viewer_ruler` | The viewer's ruler bands | 16 | 16 | 16 | 16 |
| `metrics.welcome_height` | Welcome window height | 640 | 640 | 640 | 640 |
| `metrics.welcome_width` | Welcome window width | 900 | 900 | 900 | 900 |
| `metrics.window_height` | Default window height | 900 | 900 | 900 | 900 |
| `metrics.window_width` | Default window width | 1440 | 1440 | 1440 | 1440 |
| `space.hair` | Spacing scale: pill inset, icon-to-edge nudge | 2 | 2 | 2 | 2 |
| `space.lg` | Spacing scale: panel and card inset | 12 | 12 | 12 | 12 |
| `space.md` | Spacing scale: between panels, control padding | 8 | 8 | 8 | 8 |
| `space.section` | Spacing scale: between sections | 32 | 32 | 32 | 32 |
| `space.sm` | Spacing scale: between related controls | 6 | 6 | 6 | 6 |
| `space.xl` | Spacing scale: between groups | 16 | 16 | 16 | 16 |
| `space.xs` | Spacing scale: icon to label, stacked text | 4 | 4 | 4 | 4 |
| `space.xxl` | Spacing scale: screen margins | 24 | 24 | 24 | 24 |
| `radius.base` | Radius scale base | 4 | 4 | 4 | 4 |
| `radius.lg` | Panels, cards, menus | 12 | 12 | 12 | 12 |
| `radius.md` | Buttons, fields, tabs, pill tracks | 8 | 8 | 8 | 8 |
| `radius.sm` | Wells, segments, menu rows: the smallest control corner | 6 | 6 | 6 | 6 |
| `radius.smoothing` | Continuous-corner smoothing of `lg` and larger surfaces | 0.6 | 0.6 | 0.6 | 0.6 |
| `radius.window` | The window's own corner (system-owned) | 26 | 26 | 26 | 26 |
| `radius.xl` | Popovers, sheets, the palette | 16 | 16 | 16 | 16 |
| `radius.xs` | Checkbox, kind badge (base) | 4 | 4 | 4 | 4 |
| `typography.body` | Dense UI text: values, list rows, menus in panels | 12 | 12 | 12 | 12 |
| `typography.dense_leading` | Line height multiplier for dense UI | 1.3 | 1.3 | 1.3 | 1.3 |
| `typography.display` | Empty-state and onboarding display | 24 | 24 | 24 | 24 |
| `typography.display_optical_size` | Optical size for display | 24 | 24 | 24 | 24 |
| `typography.display_tracking` | Tracking of screen titles (pt) | -0.2 | -0.2 | -0.2 | -0.2 |
| `typography.display_weight` | Weight of Heading and Display, the only semibold text | 600 | 600 | 600 | 600 |
| `typography.heading` | Sheet and Settings pane headings (semibold) | 15 | 15 | 15 | 15 |
| `typography.heading_tracking` | Tracking of headings (pt) | -0.1 | -0.1 | -0.1 | -0.1 |
| `typography.label` | Inspector row labels | 12 | 12 | 12 | 12 |
| `typography.medium_weight` | Medium weight: tabs, timecode, values being edited | 500 | 500 | 500 | 500 |
| `typography.minimum` | Smallest text anywhere: captions, ruler, status bar | 11 | 11 | 11 | 11 |
| `typography.reading` | Prose: agent replies, sheet explanations, Settings | 13 | 13 | 13 | 13 |
| `typography.reading_leading` | Line height multiplier for prose | 1.45 | 1.45 | 1.45 | 1.45 |
| `typography.regular_weight` | Regular weight | 400 | 400 | 400 | 400 |
| `typography.semibold_weight` | Emphasis weight of panel titles and section headings (medium) | 500 | 500 | 500 | 500 |
| `typography.text_optical_size` | Optical size for text | 12 | 12 | 12 | 12 |
| `typography.timecode` | Transport timecode (medium, tabular figures) | 13 | 13 | 13 | 13 |
| `typography.title` | Panel titles, section headings, window title (semibold) | 13 | 13 | 13 | 13 |
| `motion.bounce` | Spring bounce for appearing surfaces | 0.15 | 0.15 | 0.15 | 0.15 |
| `motion.enter` | Spring response for appearing surfaces (s) | 0.32 | 0.32 | 0.32 | 0.32 |
| `motion.exit` | Fade-out for dismissed surfaces (s) | 0.12 | 0.12 | 0.12 | 0.12 |
| `motion.interaction` | Hover, press and selection feedback (s) | 0.075 | 0.075 | 0.075 | 0.075 |
| `motion.note_hold` | Time a status note holds before the status line returns (s) | 2.2 | 2.2 | 2.2 | 2.2 |
| `motion.overlay_idle` | Still time before the Focus overlay capsule fades (s) | 2 | 2 | 2 | 2 |
| `motion.reduced` | Cross-fade used under Reduce Motion (s) | 0.15 | 0.15 | 0.15 | 0.15 |
| `motion.start_scale` | Scale an appearing surface starts from | 0.96 | 0.96 | 0.96 | 0.96 |
| `motion.tooltip_delay` | Delay before a tooltip appears for hover or keyboard focus (s) | 0.38 | 0.38 | 0.38 | 0.38 |
| `motion.tooltip_warm` | Time window for an immediate tooltip swap to a neighbouring control (s) | 0.6 | 0.6 | 0.6 | 0.6 |
| `motion.wheel_rest` | Pointer rest before the scroll wheel edits a hovered value (s) | 0.5 | 0.5 | 0.5 | 0.5 |
| `translucency.edge` | Panel hairline edge in `text` ink | 0.08 | 0.07 | 0.08 | 0.07 |
| `translucency.faint` | Third text step, as `text` ink | 0.42 | 0.36 | 0.42 | 0.36 |
| `translucency.glass_menu` | Floating colour under a palette's or menu's rows on system glass | 0.84 | 0.72 | 0.84 | 0.72 |
| `translucency.glass_tint` | Window-colour tint on system glass that carries labels | 0.6 | 0.9 | 0.6 | 0.9 |
| `translucency.grid` | Time grid under the timeline tracks, as `text` ink | 0.04 | 0.04 | 0.04 | 0.04 |
| `translucency.ground` | Window tint over the behind-window blur, darker than `window` so panels step above the composite | `#EEEEF1` | `#0C0A09` | `#EEEEF1` | `#0C0A09` |
| `translucency.hover` | Hover wash and soft fills, as `text` ink | 0.045 | 0.06 | 0.08 | 0.1 |
| `translucency.line` | Header, column and status dividers in `text` ink | 0.08 | 0.07 | 0.08 | 0.07 |
| `translucency.offline_hatch` | Media Offline hatch on timeline clips, as `danger` ink | 0.28 | 0.28 | 0.28 | 0.28 |
| `translucency.panel` | Shared panel material over the window tint (1 = solid) | 0.7 | 0.72 | 0.7 | 0.72 |
| `translucency.pressed` | Pressed wash, as `text` ink | 0.1 | 0.14 | 0.16 | 0.2 |
| `translucency.selected` | Neutral on state of a tab or segment, as `text` ink | 0.07 | 0.1 | 0.12 | 0.16 |
| `translucency.tick_major` | Labelled ruler tick, as `text` ink | 0.3 | 0.3 | 0.3 | 0.3 |
| `translucency.tick_minor` | Minor ruler tick, as `text` ink | 0.14 | 0.14 | 0.14 | 0.14 |
| `translucency.window` | Window tint over the behind-window blur (1 = solid, Reduce Transparency) | 0.62 | 0.9 | 0.62 | 0.9 |
<!-- tokens:end -->
