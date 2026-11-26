# Powermove Native design tokens

`apps/native/crates/design` (pm-design) is the single source of every colour,
size, type value and motion value. The system they encode is
[language.md](language.md); this page is the value table.

- `colors.*` come from `Colors::of`: seven OKLCH roles per appearance, the rest
  ink over a ground at an alpha step. Derived steps (`fill`, `fill_strong`,
  `hairline`, `raised`, `faint`) follow a theme's colour patch.
- Two surface tones per appearance. **Chrome** is `colors.window`: the title
  bar, the sidebar and the ground around the content are one tint over the
  behind-window blur (`translucency.window`), laid once: the title bar paints
  nothing of its own and the sidebar column gets the same ground, not a glass
  of its own. **Content** is `colors.panel`: Home's sheet, docked panels and
  the viewer's surround; in light it is solid (`translucency.panel` 1), as a
  Mac's light content areas. Wells, the viewer's picture ground and
  selections sit inside a content surface, never a third surface beside it.
- `space.*` and `radius.*` are the scales; the older `metrics.*` names that
  panels and themes use (`gap`, `inset`, `control_radius`, `panel_radius`, …)
  are set from them.
- Token names are the contract of built-in panels, user extensions and themes
  (`Tokens::PATCHABLE`): add names, never rename one.
- A theme's `metrics.gap` also sets `metrics.dock_gap` (v1 themes have one gap).
- `text_size.*` are the text steps, like a utility scale (`xs` 11, `sm` 12,
  `base` 13 = the macOS body, `lg` 15, `xl` 17, `x2l` 20, `x3l` 22, `x4l` 26);
  `ink.*` are the alpha steps for `text` or `accent` ink over a surface
  (`wash`, `well`, `edge`, `tint`). Views pick a step, never a number.
- `settings.*` is the Settings window at the macOS settings scale (13 pt rows,
  11 pt secondary, 30 pt sidebar rows with 20 pt tiles, a 700 pt window), its
  section tiles' material (`settings.tile.*`) and the visual pickers' art
  (`settings.picker.*`, a dusk wallpaper with a mini window).
  `welcome.*` places the welcome window and the sign-in sheet.
- `metrics.list_gap` keeps two highlights of any hoverable or selectable list
  apart: a row keeps its pitch and its highlight is `list_gap` shorter, centred
  (Settings' and Home's sidebars, kit rows, v4 list and tree items).
- The Settings, sign-in, welcome and picker views name no colour and no text
  size of their own: `crates/design/tests/view_tokens.rs` fails on a hex
  colour, an RGB literal, an ink alpha byte or a numeric font size there.

To change a value: edit `crates/design/src/lib.rs`, then regenerate the table
and the snapshot with `PM_UPDATE_DESIGN_SNAPSHOT=1 cargo test -p pm-design`.
Every token needs a role sentence in `crates/design/tests/spec.rs`.

<!-- tokens:begin -->
| Token | Role | Light | Dark | Light, Increase Contrast | Dark, Increase Contrast |
|---|---|---|---|---|---|
| `colors.accent` | Powermove orange: primary button, playhead, selection marks | `#F0580C` | `#FA6310` | `#F0580C` | `#FA6310` |
| `colors.accent_text` | Accent as text or a thin mark on panels (4.5:1) | `#B93800` | `#FF8B4D` | `#9A2F00` | `#FFA073` |
| `colors.accent_wash` | Opaque accent tint: drop targets, active guide bed | `#FCEDE6` | `#402617` | `#FCEDE6` | `#402617` |
| `colors.black` | Pure black: veils, a clear window's backing | `#000000` | `#000000` | `#000000` | `#000000` |
| `colors.brand_claude` | Anthropic's mark colour for the Claude Code mark | `#D97757` | `#D97757` | `#D97757` | `#D97757` |
| `colors.channel_blue` | Blue channel ink on panels (curves, channel values) | `#3263C3` | `#70ADFB` | `#3263C3` | `#70ADFB` |
| `colors.channel_green` | Green channel ink on panels (curves, channel values) | `#1D7D3E` | `#6FD087` | `#1D7D3E` | `#6FD087` |
| `colors.channel_red` | Red channel ink on panels (curves, channel values) | `#BA3630` | `#F47B74` | `#BA3630` | `#F47B74` |
| `colors.composer_send_idle` | Agent composer Send circle with nothing to send (derived) | `#FAFAFA` | `#2F2B2A` | `#F6F6F6` | `#332F2E` |
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
| `colors.panel` | Content tone: every docked work surface, Home's sheet and sheet content | `#FDFDFE` | `#1C1917` | `#FDFDFE` | `#1C1917` |
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
| `colors.timeline_selected` | Accent tint for selected timeline layer rows | `#FBDFD2` | `#452818` | `#FBDFD2` | `#452818` |
| `colors.timeline_selected_hover` | Stronger accent tint for hovered selected timeline layer rows | `#FAD5C3` | `#522D18` | `#FAD5C3` | `#522D18` |
| `colors.verified` | A verified publisher's seal (Store), the system blue | `#007AFF` | `#0A84FF` | `#007AFF` | `#0A84FF` |
| `colors.viewer` | Viewer surround; the darkest neutral so footage reads true | `#E6E6EA` | `#090807` | `#E6E6EA` | `#090807` |
| `colors.viewer_grid` | The viewer's grid over the picture | `#D8D8D8` | `#D8D8D8` | `#D8D8D8` | `#D8D8D8` |
| `colors.viewer_guide` | Ruler guides over the viewer's picture | `#0097BC` | `#2CCCEB` | `#0097BC` | `#2CCCEB` |
| `colors.warning` | Warnings, missing-but-recoverable states | `#A15C01` | `#F59E0A` | `#A15C01` | `#F59E0A` |
| `colors.welcome_field_deep` | Welcome stage: the orange field, deep end | `#A3301A` | `#A3301A` | `#A3301A` | `#A3301A` |
| `colors.welcome_field_light` | Welcome stage: the orange field, light end | `#FF7A2E` | `#FF7A2E` | `#FF7A2E` | `#FF7A2E` |
| `colors.welcome_ground` | Welcome stage: the ground left of the window | `#131212` | `#131212` | `#131212` | `#131212` |
| `colors.white` | Pure white: marks over the welcome field | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |
| `colors.window` | Chrome tone: the window ground under the toolbar, sidebar and status bar, around the content | `#EEEEF1` | `#0C0A09` | `#EEEEF1` | `#0C0A09` |
| `metrics.agent_attachment` | Square draft attachment thumbnail edge | 56 | 56 | 56 | 56 |
| `metrics.agent_attachment_sent` | Square sent attachment thumbnail edge | 40 | 40 | 40 | 40 |
| `metrics.agent_gap` | Space between turns in the Agent conversation | 16 | 16 | 16 | 16 |
| `metrics.agent_height` | Agent popover height when the window has room | 680 | 680 | 680 | 680 |
| `metrics.agent_width` | Default agent panel width | 440 | 440 | 440 | 440 |
| `metrics.align_well` | Least width of an alignment well: three cells at the least hit target | 72 | 72 | 72 | 72 |
| `metrics.anchor_mark` | Anchor point crosshair circle | 11 | 11 | 11 | 11 |
| `metrics.band_least` | Least height of the timeline's dock | 120 | 120 | 120 | 120 |
| `metrics.band_room` | Height the timeline's dock leaves above it | 160 | 160 | 160 | 160 |
| `metrics.border` | Separator thickness (Increase Contrast doubles it) | 1 | 1 | 2 | 2 |
| `metrics.capsule_radius` | Toolbar capsule radius (height ÷ 2) | 17 | 17 | 17 | 17 |
| `metrics.channel_strip` | Mixer channel strip width | 72 | 72 | 72 | 72 |
| `metrics.checker` | Transparency checkerboard square | 8 | 8 | 8 | 8 |
| `metrics.chooser_height` | Agent chooser height | 360 | 360 | 360 | 360 |
| `metrics.chooser_list` | Agent chooser model list width | 320 | 320 | 320 | 320 |
| `metrics.chooser_mark` | Agent chooser mark size (rail, rows) | 16 | 16 | 16 | 16 |
| `metrics.chooser_rail` | Agent chooser harness rail width | 36 | 36 | 36 | 36 |
| `metrics.clip_radius` | Timeline clip bar corner radius | 6 | 6 | 6 | 6 |
| `metrics.color_picker_plane_height` | Height of the colour saturation plane and vertical tracks | 256 | 256 | 256 | 256 |
| `metrics.color_picker_plane_width` | Width of the colour saturation and brightness plane | 230 | 230 | 230 | 230 |
| `metrics.color_picker_track` | Width of the colour hue and opacity tracks | 14 | 14 | 14 | 14 |
| `metrics.color_picker_width` | Width of the shared anchored colour picker | 535 | 535 | 535 | 535 |
| `metrics.column_least` | Narrowest side column | 200 | 200 | 200 | 200 |
| `metrics.column_most` | Widest side column | 760 | 760 | 760 | 760 |
| `metrics.comp_tab_close` | Composition tab close button width | 16 | 16 | 16 | 16 |
| `metrics.comp_tab_content_gap` | Gap between composition tab label and close button | 4 | 4 | 4 | 4 |
| `metrics.comp_tab_gap` | Gap between composition tabs | 2 | 2 | 2 | 2 |
| `metrics.comp_tab_height` | Composition tab height | 22 | 22 | 22 | 22 |
| `metrics.comp_tab_leading` | Composition tab leading padding | 10 | 10 | 10 | 10 |
| `metrics.comp_tab_trailing` | Composition tab trailing padding | 6 | 6 | 6 | 6 |
| `metrics.composer_add_symbol` | Agent composer thin + glyph square | 16 | 16 | 16 | 16 |
| `metrics.composer_control` | Agent composer control row height, its + button and Send circle | 22 | 22 | 22 | 22 |
| `metrics.composer_max_lines` | Agent composer grows to this many lines, then scrolls | 10 | 10 | 10 | 10 |
| `metrics.composer_popover_radius` | Agent composer corner, concentric in the popover | 8 | 8 | 8 | 8 |
| `metrics.composer_radius` | Agent composer corner, concentric in its panel | 6 | 6 | 6 | 6 |
| `metrics.composer_symbol` | Agent composer Send arrow, Stop square and model chevron | 15 | 15 | 15 | 15 |
| `metrics.control` | Standalone control height (buttons, popups) | 28 | 28 | 28 | 28 |
| `metrics.control_radius` | Control radius inside a panel (panel radius − gap) | 8 | 8 | 8 | 8 |
| `metrics.curve_editor` | The curve editor's least graph height | 200 | 200 | 200 | 200 |
| `metrics.curve_editor_max` | The curve editor's greatest graph height in a wide panel | 360 | 360 | 360 | 360 |
| `metrics.curve_editor_wide` | The curve editor's greatest width in a wide panel | 640 | 640 | 640 | 640 |
| `metrics.dock_gap` | Space between docked panels and to the window edge | 8 | 8 | 8 | 8 |
| `metrics.dock_reach` | How far outside a dock a dragged panel still drops into it | 28 | 28 | 28 | 28 |
| `metrics.drag_opacity` | Opacity of a drag source left in place | 0.5 | 0.5 | 0.5 | 0.5 |
| `metrics.drag_threshold` | Pointer travel before a drag starts | 3 | 3 | 3 | 3 |
| `metrics.drop_stroke` | Drop-target inner stroke | 1.5 | 1.5 | 2 | 2 |
| `metrics.easing_preset_columns` | Equal-width columns in each Easing preset family | 3 | 3 | 3 | 3 |
| `metrics.edge_strip` | Width of a side drop strip where no dock stands | 96 | 96 | 96 | 96 |
| `metrics.edge_strip_bottom` | Height of the bottom drop strip where no dock stands | 56 | 56 | 56 | 56 |
| `metrics.export_job_progress_height` | Inline export job progress stroke | 2 | 2 | 2 | 2 |
| `metrics.export_job_progress_width` | Inline export job progress width | 80 | 80 | 80 | 80 |
| `metrics.export_job_row` | Two-line export job row height | 44 | 44 | 44 | 44 |
| `metrics.export_preview` | Export progress sheet's picture of the frame last encoded | 315 | 315 | 315 | 315 |
| `metrics.fader_knob` | Mixer fader knob height | 14 | 14 | 14 | 14 |
| `metrics.field` | Value well height inside a row | 22 | 22 | 22 | 22 |
| `metrics.field_menu_height` | Field menu maximum height | 280 | 280 | 280 | 280 |
| `metrics.field_menu_width` | Field menu width | 300 | 300 | 300 | 300 |
| `metrics.form_row` | Minimum Settings row height | 40 | 40 | 40 | 40 |
| `metrics.gap` | Gap between panels and to the window edge | 6 | 6 | 6 | 6 |
| `metrics.gizmo_handle` | Transform handle and path vertex square | 7 | 7 | 7 | 7 |
| `metrics.gizmo_keyline` | Gizmo stroke and keyline width over the image | 1 | 1 | 1 | 1 |
| `metrics.grade_field` | Room for a colour wheel master's number beside its strip | 72 | 72 | 72 | 72 |
| `metrics.grade_pair` | Least width of two colour wheels side by side on their plates | 254 | 254 | 254 | 254 |
| `metrics.grade_pair_max` | Most width of two colour wheel plates side by side | 446 | 446 | 446 | 446 |
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
| `metrics.list_gap` | Space between adjacent hoverable or selectable list rows: highlights never touch | 2 | 2 | 2 | 2 |
| `metrics.mark_bracket` | Width of In and Out brackets on rulers and scrubbers | 6 | 6 | 6 | 6 |
| `metrics.media_badge_height` | Height of a Project tile's duration and status badges | 16 | 16 | 16 | 16 |
| `metrics.media_badge_opacity` | Opacity of the media badge fill | 0.55 | 0.55 | 0.55 | 0.55 |
| `metrics.media_comp_inset` | A composition's frame inside its Project tile | 8 | 8 | 8 | 8 |
| `metrics.media_folder_glyph` | Folder glyph on a Project folder tile | 30 | 30 | 30 | 30 |
| `metrics.media_frame_radius` | Corner radius of the composition frame inside a Project tile | 2 | 2 | 2 | 2 |
| `metrics.media_glyph` | Media kind glyph on a Project tile without a picture | 22 | 22 | 22 | 22 |
| `metrics.media_inset` | Project tile badges and actions from the tile edge; badge padding; tile-to-name gap | 4 | 4 | 4 | 4 |
| `metrics.media_loading_opacity` | A Project tile's picture while its media loads | 0.55 | 0.55 | 0.55 | 0.55 |
| `metrics.media_offline_opacity` | A missing file's remembered picture under its badge | 0.32 | 0.32 | 0.32 | 0.32 |
| `metrics.media_open_dot_inset` | Inset of the open-composition indicator from the tile corner | 11.586 | 11.586 | 11.586 | 11.586 |
| `metrics.media_ring` | Selection ring inside a Project tile | 2 | 2 | 2 | 2 |
| `metrics.media_row_thumb_height` | Height of a Project list row's leading thumbnail | 18 | 18 | 18 | 18 |
| `metrics.media_row_thumb_width` | Width of a Project list row's leading thumbnail | 32 | 32 | 32 | 32 |
| `metrics.media_small_radius` | Corner radius of compact Project thumbnails | 4 | 4 | 4 | 4 |
| `metrics.media_tint_opacity` | Red or accent wash over a Project tile needing attention | 0.1 | 0.1 | 0.1 | 0.1 |
| `metrics.menu_inset` | Menu and palette row inset | 12 | 12 | 12 | 12 |
| `metrics.menu_row` | Palette row height | 30 | 30 | 30 | 30 |
| `metrics.message_inset` | Leading room beside a person's message in the Agent conversation | 52 | 52 | 52 | 52 |
| `metrics.min_center` | Smallest viewer extent | 180 | 180 | 180 | 180 |
| `metrics.min_height` | Minimum window height | 640 | 640 | 640 | 640 |
| `metrics.min_panel` | Smallest panel extent | 72 | 72 | 72 | 72 |
| `metrics.min_width` | Minimum window width | 980 | 980 | 980 | 980 |
| `metrics.nav_row` | A source-list row's pitch in Home and utility sidebars | 32 | 32 | 32 | 32 |
| `metrics.palette_height` | Command palette maximum height | 360 | 360 | 360 | 360 |
| `metrics.palette_width` | Command palette width | 520 | 520 | 520 | 520 |
| `metrics.panel_drag_threshold` | Travel before a panel header or tab drags | 5 | 5 | 5 | 5 |
| `metrics.panel_least` | Least height of a docked panel | 88 | 88 | 88 | 88 |
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
| `metrics.settings_column` | Settings form column's greatest width, insets included | 680 | 680 | 680 | 680 |
| `metrics.settings_height` | Settings window height | 520 | 520 | 520 | 520 |
| `metrics.settings_number_field` | Settings row number field width, its unit included | 96 | 96 | 96 | 96 |
| `metrics.settings_poster_height` | Settings project header poster height | 54 | 54 | 54 | 54 |
| `metrics.settings_poster_width` | Settings project header poster width | 96 | 96 | 96 | 96 |
| `metrics.settings_sidebar` | Settings sidebar width | 200 | 200 | 200 | 200 |
| `metrics.settings_text_field` | Settings row text field width | 220 | 220 | 220 | 220 |
| `metrics.settings_width` | Settings window width | 760 | 760 | 760 | 760 |
| `metrics.sheet_body_max` | Tallest sheet option body before it scrolls | 440 | 440 | 440 | 440 |
| `metrics.sheet_inset` | Content inset inside sheets and Settings panes | 20 | 20 | 20 | 20 |
| `metrics.sheet_value_column` | Shared value column in sheets | 240 | 240 | 240 | 240 |
| `metrics.sheet_width` | Default sheet width | 480 | 480 | 480 | 480 |
| `metrics.sheet_width_wide` | Width of wide sheets: Export, Expression | 600 | 600 | 600 | 600 |
| `metrics.sidebar` | Sidebar width of Home and Settings | 224 | 224 | 224 | 224 |
| `metrics.splitter_dead_zone` | Travel before a splitter between panels resizes | 2 | 2 | 2 | 2 |
| `metrics.splitter_glow` | Length of the glow that follows the pointer along a splitter | 180 | 180 | 180 | 180 |
| `metrics.splitter_glow_width` | Thickness of the splitter glow | 4 | 4 | 4 | 4 |
| `metrics.splitter_reach` | How far a dock divider's grab reaches into the panels beside its gap | 4 | 4 | 4 | 4 |
| `metrics.status` | Status bar height | 24 | 24 | 24 | 24 |
| `metrics.status_dot` | Attention dot and drop insertion-line end circle | 6 | 6 | 6 | 6 |
| `metrics.store_column` | Store content column's greatest width | 960 | 960 | 960 | 960 |
| `metrics.symbol_action` | SF Symbol size of a surface's one primary action glyph | 20 | 20 | 20 | 20 |
| `metrics.symbol_inline` | SF Symbol size inside rows | 12 | 12 | 12 | 12 |
| `metrics.symbol_panel` | SF Symbol size in panel headers | 14 | 14 | 14 | 14 |
| `metrics.symbol_toolbar` | SF Symbol size in toolbar items | 14 | 14 | 14 | 14 |
| `metrics.sync_badge` | Linked-clip sync badge height | 16 | 16 | 16 | 16 |
| `metrics.tab` | Text tab and full-height segmented control | 28 | 28 | 28 | 28 |
| `metrics.tangent_end` | Bézier tangent end circle | 6 | 6 | 6 | 6 |
| `metrics.text_padding` | Vertical text padding in bubbles, the composer and code blocks | 8 | 8 | 8 | 8 |
| `metrics.thumbnail_min` | Minimum Project grid thumbnail width | 120 | 120 | 120 | 120 |
| `metrics.timeline_indent` | Timeline layer nesting indent per depth, capped at four levels | 12 | 12 | 12 | 12 |
| `metrics.titlebar` | Title bar band of a plain panel window; content starts below it | 28 | 28 | 28 | 28 |
| `metrics.toolbar` | Unified toolbar height | 52 | 52 | 52 | 52 |
| `metrics.toolbar_title_line` | Document title line height | 14 | 14 | 14 | 14 |
| `metrics.toolbar_title_width` | Document title slot width | 160 | 160 | 160 | 160 |
| `metrics.track_row` | Timeline layer row | 30 | 30 | 30 | 30 |
| `metrics.tree_indent` | Indent per outline level | 16 | 16 | 16 | 16 |
| `metrics.value_column` | Least width of an inspector row's value column | 128 | 128 | 128 | 128 |
| `metrics.value_column_most` | Most width of a capped inspector row's value column (Effect Controls) | 240 | 240 | 240 | 240 |
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
| `motion.menu_fade` | Palette and pop-up menu fade in and out (s) | 0.1 | 0.1 | 0.1 | 0.1 |
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
| `translucency.glass_tint` | Window-colour tint on system glass that carries labels | 0.9 | 0.9 | 0.9 | 0.9 |
| `translucency.grid` | Time grid under the timeline tracks, as `text` ink | 0.04 | 0.04 | 0.04 | 0.04 |
| `translucency.ground` | Window tint over the behind-window blur, darker than `window` so panels step above the composite | `#EEEEF1` | `#0C0A09` | `#EEEEF1` | `#0C0A09` |
| `translucency.hover` | Hover wash and soft fills, as `text` ink | 0.045 | 0.06 | 0.08 | 0.1 |
| `translucency.line` | Header, column and status dividers in `text` ink | 0.08 | 0.07 | 0.08 | 0.07 |
| `translucency.offline_hatch` | Media Offline hatch on timeline clips, as `danger` ink | 0.28 | 0.28 | 0.28 | 0.28 |
| `translucency.panel` | Content panel over the chrome (1 = solid: light content is solid paper) | 1 | 0.72 | 1 | 0.72 |
| `translucency.pressed` | Pressed wash, as `text` ink | 0.1 | 0.14 | 0.16 | 0.2 |
| `translucency.selected` | Neutral on state of a tab or segment, as `text` ink | 0.07 | 0.1 | 0.12 | 0.16 |
| `translucency.tick_major` | Labelled ruler tick, as `text` ink | 0.3 | 0.3 | 0.3 | 0.3 |
| `translucency.tick_minor` | Minor ruler tick, as `text` ink | 0.14 | 0.14 | 0.14 | 0.14 |
| `translucency.window` | Chrome tint over the behind-window blur: title bar, sidebar and the ground around the content, one tone (1 = solid) | 0.9 | 0.9 | 0.9 | 0.9 |
| `settings.account_row` | Settings account row height | 40 | 40 | 40 | 40 |
| `settings.account_weight` | Settings account name and initial weight | 600 | 600 | 600 | 600 |
| `settings.app_icon` | Settings › Agents app mark side | 20 | 20 | 20 | 20 |
| `settings.app_icon_gap` | Space after a Settings › Agents app mark | 10 | 10 | 10 | 10 |
| `settings.avatar` | Settings account avatar side | 26 | 26 | 26 | 26 |
| `settings.column` | Settings form column's greatest width | 560 | 560 | 560 | 560 |
| `settings.credits_air` | Air above the credits meter and the daily chart | 8 | 8 | 8 | 8 |
| `settings.credits_bar` | A day's bar width in the daily credits chart, at most | 6 | 6 | 6 | 6 |
| `settings.credits_bar_gap` | Least space between the daily credits chart's bars | 2 | 2 | 2 | 2 |
| `settings.credits_bar_radius` | The daily credits chart's bar caps' corner | 2 | 2 | 2 | 2 |
| `settings.credits_chart` | Settings › Account daily credits chart height | 52 | 52 | 52 | 52 |
| `settings.credits_meter` | Settings › Account credits meter thickness | 6 | 6 | 6 | 6 |
| `settings.credits_today` | Today's marker under the daily credits chart | 3 | 3 | 3 | 3 |
| `settings.dark_veil` | Dark Settings sidebar veil over the system material | 0.37 | 0.37 | 0.37 | 0.37 |
| `settings.filter_width` | A Settings filter field's least width | 175 | 175 | 175 | 175 |
| `settings.group_radius` | A Settings group card's corner | 10 | 10 | 10 | 10 |
| `settings.group_weight` | Settings sidebar group name weight | 600 | 600 | 600 | 600 |
| `settings.header_height` | Settings page header band and sidebar group header row | 30 | 30 | 30 | 30 |
| `settings.header_tile` | Settings page header's tile side | 20 | 20 | 20 | 20 |
| `settings.header_weight` | Settings page header weight | 700 | 700 | 700 | 700 |
| `settings.heading_weight` | Settings group heading and chosen picker label weight | 600 | 600 | 600 | 600 |
| `settings.help_delay` | Seconds the pointer rests before an ⓘ popover opens | 0.3 | 0.3 | 0.3 | 0.3 |
| `settings.help_width` | The ⓘ popover's text width | 260 | 260 | 260 | 260 |
| `settings.hover_opacity` | A hovered sidebar row's wash relative to the selection | 0.5 | 0.5 | 0.5 | 0.5 |
| `settings.page_bottom` | Air under the last Settings card | 20 | 20 | 20 | 20 |
| `settings.page_inset` | Settings page side inset | 20 | 20 | 20 | 20 |
| `settings.page_top` | Air above the first Settings card | 4 | 4 | 4 | 4 |
| `settings.path_width` | A Settings folder path pop-up's greatest width | 210 | 210 | 210 | 210 |
| `settings.picker.close` | Picker art: mini window close light | `#FF5F57` | `#FF5F57` | `#FF5F57` | `#FF5F57` |
| `settings.picker.fold` | Picker art: dark fold colour | `#2E2F8A` | `#0D0B2A` | `#2E2F8A` | `#0D0B2A` |
| `settings.picker.fold_opacity` | Picker art: dark fold opacity | 0.35 | 0.5 | 0.35 | 0.5 |
| `settings.picker.glow` | Picker art: low warm glow colour | `#FFB36B` | `#FF8A3D` | `#FFB36B` | `#FF8A3D` |
| `settings.picker.glow_opacity` | Picker art: low warm glow opacity | 0.5 | 0.5 | 0.5 | 0.5 |
| `settings.picker.minimize` | Picker art: mini window minimise light | `#FEBC2E` | `#FEBC2E` | `#FEBC2E` | `#FEBC2E` |
| `settings.picker.sheen` | Picker art: sheen colour along the fold | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |
| `settings.picker.sheen_opacity` | Picker art: sheen opacity | 0.22 | 0.14 | 0.22 | 0.14 |
| `settings.picker.sky_bottom` | Picker art: wallpaper sky at the horizon | `#FF9A5A` | `#F0703A` | `#FF9A5A` | `#F0703A` |
| `settings.picker.sky_mid` | Picker art: wallpaper sky middle | `#9B5CC4` | `#5B2A86` | `#9B5CC4` | `#5B2A86` |
| `settings.picker.sky_top` | Picker art: wallpaper sky top | `#4C4FC4` | `#1E1B4B` | `#4C4FC4` | `#1E1B4B` |
| `settings.picker.tile_edge` | Picker art: unselected tile's edge opacity | 0.1 | 0.12 | 0.1 | 0.12 |
| `settings.picker.window_edge` | Picker art: mini window edge opacity | 0.12 | 0.16 | 0.12 | 0.16 |
| `settings.picker.window_shadow` | Picker art: mini window shadow opacity | 0.3 | 0.45 | 0.3 | 0.45 |
| `settings.picker.zoom` | Picker art: mini window zoom light | `#28C840` | `#28C840` | `#28C840` | `#28C840` |
| `settings.picker_column` | A picker option's column: every picker is one left-aligned grid | 120 | 120 | 120 | 120 |
| `settings.picker_height` | A visual picker tile's height | 64 | 64 | 64 | 64 |
| `settings.picker_radius` | A picker tile's corner | 9 | 9 | 9 | 9 |
| `settings.picker_ring` | A picker selection ring's width | 2 | 2 | 2 | 2 |
| `settings.picker_ring_gap` | Gap between a picker tile and its selection ring | 3 | 3 | 3 | 3 |
| `settings.picker_width` | A visual picker tile's width | 96 | 96 | 96 | 96 |
| `settings.picker_window_radius` | A picker's mini window corner | 4 | 4 | 4 | 4 |
| `settings.progress_width` | A Settings progress bar width | 96 | 96 | 96 | 96 |
| `settings.row` | A Settings row's least height | 38 | 38 | 38 | 38 |
| `settings.row_inset` | A Settings row's side inset | 12 | 12 | 12 | 12 |
| `settings.row_padding` | A Settings row's vertical padding | 6 | 6 | 6 | 6 |
| `settings.section_gap` | Space between Settings groups | 14 | 14 | 14 | 14 |
| `settings.secure_width` | A Settings secure field width | 160 | 160 | 160 | 160 |
| `settings.sidebar` | Settings sidebar width | 210 | 210 | 210 | 210 |
| `settings.sidebar_inset` | Sidebar highlight's side inset | 10 | 10 | 10 | 10 |
| `settings.sidebar_row` | Settings sidebar row highlight height | 30 | 30 | 30 | 30 |
| `settings.sidebar_tile` | Settings sidebar section tile side | 20 | 20 | 20 | 20 |
| `settings.tile.account_glyph` | Section tile: signed-out account glyph | `#6A6A70` | `#98989F` | `#6A6A70` | `#98989F` |
| `settings.tile.body_bottom` | Section tile: body gradient bottom | `#E2E2E6` | `#26262B` | `#E2E2E6` | `#26262B` |
| `settings.tile.body_top` | Section tile: body gradient top | `#FFFFFF` | `#55555C` | `#FFFFFF` | `#55555C` |
| `settings.tile.edge` | Section tile: inner edge colour | `#000000` | `#FFFFFF` | `#000000` | `#FFFFFF` |
| `settings.tile.edge_bottom` | Section tile: inner edge opacity at the bottom | 0.2 | 0.06 | 0.2 | 0.06 |
| `settings.tile.edge_top` | Section tile: inner edge opacity at the top | 0.08 | 0.42 | 0.08 | 0.42 |
| `settings.tile.glyph_bottom` | Section tile: glyph gradient bottom | `#232327` | `#D4D4DA` | `#232327` | `#D4D4DA` |
| `settings.tile.glyph_top` | Section tile: glyph gradient top | `#55555B` | `#FFFFFF` | `#55555B` | `#FFFFFF` |
| `settings.tile.light` | Section tile: top light colour | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` | `#FFFFFF` |
| `settings.tile.light_opacity` | Section tile: top light opacity | 0.9 | 0.24 | 0.9 | 0.24 |
| `settings.tile.press` | Section tile: glyph's pressed-in shadow colour | `#FFFFFF` | `#000000` | `#FFFFFF` | `#000000` |
| `settings.tile.press_opacity` | Section tile: glyph's pressed-in shadow opacity | 0.9 | 0.5 | 0.9 | 0.5 |
| `settings.tile.shadow` | Section tile: drop shadow colour | `#000000` | `#000000` | `#000000` | `#000000` |
| `settings.tile.shadow_blur` | Section tile: drop shadow blur | 0.7 | 0.7 | 0.7 | 0.7 |
| `settings.tile.shadow_offset` | Section tile: drop shadow offset | 0.6 | 0.6 | 0.6 | 0.6 |
| `settings.tile.shadow_opacity` | Section tile: drop shadow opacity | 0.2 | 0.6 | 0.2 | 0.6 |
| `settings.tile_radius` | A section tile's corner | 5 | 5 | 5 | 5 |
| `settings.title_air` | Air under a Settings row's second line | 4 | 4 | 4 | 4 |
| `settings.value_least` | A Settings shortcut value's least width | 88 | 88 | 88 | 88 |
| `settings.window_default_height` | Settings window first height | 600 | 600 | 600 | 600 |
| `settings.window_height` | Settings window least height | 500 | 500 | 500 | 500 |
| `settings.window_width` | Settings window width (fixed) | 700 | 700 | 700 | 700 |
| `text_size.base` | Text size step 13 pt: the macOS body | 13 | 13 | 13 | 13 |
| `text_size.lg` | Text size step 15 pt: page headers | 15 | 15 | 15 | 15 |
| `text_size.sm` | Text size step 12 pt: small labels | 12 | 12 | 12 | 12 |
| `text_size.x2l` | Text size step 20 pt: sheet titles | 20 | 20 | 20 | 20 |
| `text_size.x3l` | Text size step 22 pt: code digits | 22 | 22 | 22 | 22 |
| `text_size.x4l` | Text size step 26 pt: the welcome title | 26 | 26 | 26 | 26 |
| `text_size.xl` | Text size step 17 pt | 17 | 17 | 17 | 17 |
| `text_size.xs` | Text size step 11 pt: secondary text, group names | 11 | 11 | 11 | 11 |
| `ink.edge` | Ink alpha: a well's or hairline's edge | 0.12 | 0.12 | 0.12 | 0.12 |
| `ink.tint` | Ink alpha: an accent tint behind a mark | 0.15 | 0.15 | 0.15 | 0.15 |
| `ink.wash` | Ink alpha: a hovered row's faint wash | 0.03 | 0.03 | 0.03 | 0.03 |
| `ink.well` | Ink alpha: a field's or cell's quiet fill | 0.05 | 0.05 | 0.05 | 0.05 |
| `welcome.actions_room` | Room left above the welcome actions | 20 | 20 | 20 | 20 |
| `welcome.actions_top` | Where the welcome actions start | 510 | 510 | 510 | 510 |
| `welcome.cell_height` | A code cell height | 48 | 48 | 48 | 48 |
| `welcome.cell_width` | A code cell width | 38 | 38 | 38 | 38 |
| `welcome.column` | The welcome column's width | 330 | 330 | 330 | 330 |
| `welcome.field_height` | The welcome email well's height | 40 | 40 | 40 | 40 |
| `welcome.field_radius` | The welcome well's and cells' corner | 10 | 10 | 10 | 10 |
| `welcome.half` | Each half of the welcome window | 450 | 450 | 450 | 450 |
| `welcome.icon_tile` | A welcome list icon's tile | 32 | 32 | 32 | 32 |
| `welcome.list_fade` | The welcome list's edge fade | 18 | 18 | 18 | 18 |
| `welcome.list_inset` | A welcome list row's side inset | 16 | 16 | 16 | 16 |
| `welcome.list_max_height` | The welcome list's greatest height before it scrolls | 224 | 224 | 224 | 224 |
| `welcome.list_radius` | The welcome list's corner | 12 | 12 | 12 | 12 |
| `welcome.list_row` | A welcome list row's least height | 50 | 50 | 50 | 50 |
| `welcome.list_widen` | How much wider than the column the welcome list runs | 20 | 20 | 20 | 20 |
| `welcome.logo_top` | The welcome mark's height from the top | 104 | 104 | 104 | 104 |
| `welcome.primary_height` | The welcome primary action's height | 36 | 36 | 36 | 36 |
| `welcome.primary_width` | The welcome primary action's width | 280 | 280 | 280 | 280 |
| `welcome.sheet_control` | Sign-in sheet controls' height | 36 | 36 | 36 | 36 |
| `welcome.sheet_foot` | Cancel's line above the sheet's bottom | 22 | 22 | 22 | 22 |
| `welcome.sheet_foot_gap` | Cancel's line under the last control | 20 | 20 | 20 | 20 |
| `welcome.sheet_height` | Sign-in sheet first height | 428 | 428 | 428 | 428 |
| `welcome.sheet_padding` | Sign-in sheet column inset | 32 | 32 | 32 | 32 |
| `welcome.sheet_tile` | Sign-in sheet tile side | 48 | 48 | 48 | 48 |
| `welcome.sheet_width` | Sign-in sheet width | 400 | 400 | 400 | 400 |
| `welcome.window_left` | The welcome window picture's left | 32 | 32 | 32 | 32 |
| `welcome.window_scale` | The welcome window picture's scale | 0.88 | 0.88 | 0.88 | 0.88 |
| `welcome.window_top` | The welcome window picture's top | 104 | 104 | 104 | 104 |
<!-- tokens:end -->
