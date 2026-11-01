# Design review: theme pipeline (`apps/native/js/themes`, merged)

Reviewer: design lead · 2026-11-01 · Base: `origin/feat/native-engine`
Scope: the colour/metric/type projection from v1–3 theme tokens onto
pm-design (`tokens.mjs` `native` map, `designTokens`, `evaluateTheme`). The
3,329 catalogued theme occurrences route through this map, so a wrong mapping
is a wrong colour everywhere.

**Result: fixed in this branch** (`feat/na-design`). The lane needs no further
action; the editions lane must rebase (see [na-cloud-editions.md](na-cloud-editions.md)).

## Findings

| # | Severity | Finding | Fix (done) |
|---|---|---|---|
| T1 | Must-fix | With **no theme**, `evaluateTheme` projected all 24 mapped v1.2.0 foundation values onto pm-design. Dark mode became v1.2.0's **warm Stone** (`#0C0A09` window, `#1C1917` panel), accent `#FF6B1A`/`#F0580A` instead of `#FF6019`, 38 pt headers, 30 pt rows, 10 pt gaps, 13 pt body, 18 pt display — i.e. the native look was silently replaced by the old Electron defaults. | Only values that differ from the frozen foundation (same scheme, normal density) are projected. Untouched values are listed in `nativeDefaults` and pm-design's defaults stand. Test: "no theme leaves pm-design's native defaults alone". |
| T2 | Must-fix | `--r-pill` (999 px, "fully round") → `metrics.capsule_radius` (17 pt, a concrete toolbar capsule radius). A default projection set toolbar capsules to 999. | Unmapped: a capsule is height ÷ 2, a shape not a radius. |
| T3 | Must-fix | `--hairline` (0.5 px CSS device-pixel workaround) → `metrics.border`, which Increase Contrast sets to 2. A theme (or the default) could undo Increase Contrast. | Unmapped. Separately, `Tokens::apply_patch` now re-applies Increase Contrast after any patch, so no theme can lower contrast. |
| T4 | Must-fix | Type roles shifted: `--fs-md` (13, Electron body) → `typography.body` (12, dense UI); `--fs-lg` (15) → `title` (13, panel titles); `--fs-xl` (18) → `display` (22). | `fs-sm` → body + label; `fs-md` → title + reading; `fs-lg` → heading (new); `fs-xl` unmapped. |
| T5 | Should-fix | Density was absolute: compact projected v1.2.0's 26 px rows and **8 px gaps — larger than the native 6 pt gap**. | Lengths move relative to the native value when the host passes `environment.nativeTokens` (compact: row 28 → 24, gap 6 → 4). |
| T6 | Should-fix | Translucent theme colours (`--line`, `--tx-2`, `--tx-3`, alpha accents) could never project, so themes could not change separators or secondary text. | Composited over the theme's `--bg-panel` before projection. |
| T7 | Should-fix | No native target for `--accent-tx`, `--accent-dim`, `--danger`, `--warning`, `--success`, `--r-xs`, `--val-w`. | pm-design gained `accent_text`, `accent_wash`, `danger`, `warning`, `success`, `inner_radius`, `value_column`; mapped. |
| T8 | Nice-to-have | `--bg-sunken` → `colors.viewer` is correct for the stage surround but v1.2.0 also uses sunken for wells. | Kept; wells are `field` natively. Documented in tokens.md. |
| T9 | Nice-to-have | Mapping targets were only checked against struct field names, not against what a theme may actually change. | `Tokens::PATCHABLE` lists patchable fields; `pm-design/tests/spec.rs` fails if `tokens.mjs` targets anything else. |

Verified: `npm test` (3,403 pass), `cargo test -p pm-design`, `cargo clippy -p pm-design -D warnings`, `cargo check -p pm-shell`.

Host integration still required (shell lane): call `Tokens::apply_patch` with
`nativePatch` and pass the current tokens as `environment.nativeTokens` when
evaluating a theme.
