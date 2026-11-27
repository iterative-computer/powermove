# Design review: `feat/na-cloud-uikit` (v4 native UI kit)

Reviewer: design lead · 2026-11-01 · Lane head `3eec8d797`.
Read against [language.md](../language.md), [tokens.md](../tokens.md),
[review-checklist.md](../review-checklist.md) and the panel specs
([panels/README.md](../panels/README.md)). Reviewed from code and docs:
`crates/surface/src/kit/*.rs`, `crates/surface/tests/ui_kit.rs`,
`js/v4/src/components/index.ts`, `docs/native/ui-kit.md`. No app build: the
kit has no AppKit/GPUI adapter yet, so nothing renders to capture.

## Result

**APPROVE — merge `feat/na-design-fix-uikit`, not the lane head.** The fix
branch is the lane plus the seven fixes below and the DESIGN-TODO
resolutions. With it, the kit's contracts (recipes, wire components,
validation, accessibility) match the language. Visual sign-off of the
rendered controls is **not** part of this approval: it happens when the
AppKit/GPUI adapter lands, with the checklist §0 evidence.

Verification on the fix branch (this Mac): `cargo test -p pm-surface`
(standalone manifest) 74 passed (35 + 39, one new test); clippy
`-D warnings` clean; rustfmt clean; `bun test` in `js/v4` 81 passed with
the wire examples; `tsc --noEmit` and Prettier clean.

## Fixed on `feat/na-design-fix-uikit`

| # | Finding | Rule | Fix |
|---|---|---|---|
| M1 | `Toggle` rendered as an `NSSwitch` and announced as a switch | Panels use checkboxes; switches are Settings-only (panels/README §5.5, HIG inspectors) | `Toggle → Widget::Checkbox`, AX `Role::CheckBox`, JSX `role="checkbox"`. API name `Toggle`/`ToggleField` kept for compatibility. |
| M2 | `PopUpButton` was `control` (28 pt) everywhere, so inside a 28 pt inspector row it filled the row | language §9: a `field` (22) sits centred in a `row`; pop-ups in rows are value wells | JSX uses `field` inside an `InspectorRow` (Label context), `control` standalone; `Panel::prepare` narrows the recipe the same way. Radius `control_radius`. |
| M3 | `SheetAnchor` declared `data-pm-system-placement: "below-toolbar"`; ui-kit.md said "placement below toolbar" | language §9 (updated): the system places sheets — centred on macOS 26+/27 | Hint is now `"system"`; doc updated. |
| M4 | AX values read raw unit abbreviations (`10.0 px`) | language §16: values read with units (`100 percent`) | `spoken_unit()` maps px/%/°/s/ms/fps/pt to words; test updated (`10.0 pixels`). |
| S1 | `ColourWell` had no shape (height/radius `None`) | inspector.md §2 *Colour*: `field` square swatch, `control_radius` | Recipe and JSX: `field` height, `control_radius`. |
| S2 | List/tree rows had no selection shape | panels/README §3 *Selected*: neutral fill in a `control_radius` shape | `ListItem`/`TreeItem` radius `control_radius`. |
| S3 | Segmented-track padding left unspecified (DESIGN-TODO 1) | concentric radii (language §7) | `Recipe::track_padding = control_radius − inner_radius`. |

## Checklist

| § | Item | Result |
|---|---|---|
| 0 | Evidence (screenshots, IC/RT captures, keyboard log) | N/A — no renderer in this lane. Required again at adapter integration. |
| 0 | `cargo test -p pm-design` passes; no literals in views | ✅ kit tests include source guards and mutation probes against literal colours/sizes/radii/type/durations |
| 1 | Every visual value from `pm_design::Tokens` | ✅ recipes read tokens only; validator flags `nonTokenValue` and wrong token families |
| 1 | Neutral selection, accent only as a mark | ✅ `selected_fill` for rows; accent only for the active toolbar glyph |
| 1 | Text steps, not opacity | ✅ text / secondary / muted |
| 2 | Glass only on permitted surfaces; solid content inside floating frames | ✅ `Tokens::material(surface)`, `content_material: Solid`, popover content `floating`, sheet content `panel` |
| 2 | Reduce Transparency → solid | ✅ via `Tokens::material`; covered by the 16 appearance combinations |
| 3 | One shared value column; label flexible with ellipsis + tooltip | ✅ `InspectorRow` fixes the value slot at `value_column`, label gets the rest |
| 3 | Hit targets ≥ `hit` | ✅ validator checks real layout rectangles (`smallHitTarget`) |
| 3 | Empty states centred, heading/reading, one action | ✅ (see F5 for the symbol slot) |
| 4 | Tabular figures, left-aligned values, muted units | ✅ `NumberField` recipe + JSX |
| 4 | No text under `minimum`, no caps | ✅ sizes are token roles |
| 5 | Pop-ups for choices; system controls first | ✅ (M2 fixed the in-row height) |
| 5 | Sheets system-placed | ✅ after M3 |
| 5 | Popovers anchored, Esc/outside dismiss, live edits | ✅ contract declared (`PopoverAnchor`), host enforces |
| 6 | Keyboard: segmented ←/→/Home/End skipping disabled, one Tab stop; list Enter/Space; tree ←/→ | ✅ |
| 6 | Focus ring token | ✅ `focus` in every recipe |
| 7 | No product copy authored by the kit | ✅ callers supply all strings |
| 8 | Motion tokens; Reduce Motion cross-fade | ✅ springs only on floating surfaces, `reduced` cross-fade under Reduce Motion |
| 9 | Roles, labels, values with units, mixed state | ✅ after M4; `missingLabel` diagnostic |
| 9 | Increase Contrast | ✅ tokens; see F6 for well borders |

## DESIGN-TODO(opus) resolutions

All five are resolved (rules in [panels/README.md §5](../panels/README.md#5-resolutions-of-the-ui-kit-design-todos);
the kit's comments and ui-kit.md now point here):

1. **Segmented-track padding** — `control_radius − inner_radius`; implemented (S3).
2. **Wheel editing** — designed: edit only after the pointer rests on the
   well for `motion.wheel_rest` (0.5 s, new token) with no container scroll
   in that time; discrete wheel, or trackpad with ⌥; one undo step. Left
   unbound in code until the token exists (F1). Safe as shipped.
3. **Colour-picker contents** — [panels/inspector.md §7](../panels/inspector.md#7-colour-popover);
   the host owns the popover, the kit's `ColourWell` anchors it.
4. **Tree indentation** — `metrics.tree_indent` (16, new token), disclosure
   `chevron.right` at `symbol_inline` in a `hit` square. Hosts use the system
   outline indentation (also 16) until the token lands (F2).
5. **Checkbox/switch placement** — checkbox at the leading edge of the value
   column, untitled, AX label from the row; implemented (M1).

## Follow-ups (not blocking this merge)

| # | Owner | Item |
|---|---|---|
| F1 | pm-design + kit | Add `motion.wheel_rest` (0.5) and implement the wheel gate in `NumericScrub` (or a sibling `WheelEdit`), one undo step closing `wheel_rest` after the last event. |
| F2 | pm-design + kit | Add `metrics.tree_indent` (16) and `Recipe::indent` for `TreeItem`; plus the other tokens listed in panels/README §6. |
| F3 | kit | `ToolbarItem` recipes report `Glass` per item; the doc says the toolbar host owns glass once. Report the item's material as solid/none so a host cannot stack glass per item. |
| F4 | kit / v4 API | JSX `well` style sets `height: field` **and** `minHeight: hit`; a CSS-like host would draw 24 pt wells. Hosts must draw `Recipe::visual_height` and hit-test `Recipe::hit`; in the next API revision rename the hit hints (`hitMinWidth`/`hitMinHeight`) so they can't be read as visual size. |
| F5 | kit | `EmptyState` needs an optional `symbol` prop (SF Symbol at 2 × `symbol_panel`, `muted`), as every panel empty state has one. |
| F6 | kit | Under Increase Contrast, value wells and segmented tracks draw a `border` × `separator` outline (inspector.md §8). Expose `draws_border` in the recipe from `accessibility.increase_contrast`. |
| F7 | v4 API lane | `js/v4/src/ui.ts` still carries an older `DESIGN-TODO(opus)` for control renderers; it is superseded by this kit and the panel specs — remove it when `powermove/ui` is routed to the kit. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| `feat/na-cloud-uikit` via `feat/na-design-fix-uikit` | design lead | 2026-11-01 | APPROVE (contracts); rendering review pending adapter |
