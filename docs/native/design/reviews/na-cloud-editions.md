# Design review: `feat/na-cloud-editions` (compat theme editions)

Reviewer: design lead · 2026-11-01 · Commit reviewed: `dafd65f5e`
Scope: the eight generated `editions/v1/*.css` stylesheets that give compat
(WKWebView) content Powermove Native's look, `editions.mjs`, and the
`native-tokens-v1.json` fixture. Read against [language.md](../language.md).

**Result: changes required before merge.**

## Must-fix

1. **Rebase onto `feat/na-design` and regenerate.** `FOUNDATION_TOKENS[].native`
   is now an **array** of targets (one token may feed several native roles),
   so `nativeValues` must iterate it (`spec.native[0]` for the CSS value).
   Regenerate `fixtures/native-tokens-v1.json` from the new
   `crates/design/tests/tokens.snapshot.json` (new fields: `accent_text`,
   `accent_wash`, semantic colours, labels, typography roles, metrics).
2. **Floating surfaces must stay lifted.** `APP_ROOT_CSS` (v1.2.0 `app.css`)
   carries `:root[data-theme="dark"]{--bg-float:var(--bg-panel)}` and the
   generator deliberately keeps it, so dark menus/popovers in compat content
   are the same grey as the panel they float over. Main removed that rule on
   2026-11-01 (`ba5a8742c`). Always emit `--bg-float` = `colors.floating`.
3. **`--r-pill` must stay `999px`.** The editions emit `--r-pill:17px`, so every
   pill taller than 34 px (tags, large chips, progress tracks) loses its
   capsule shape. Fixed upstream by unmapping `r-pill`; regenerate.
4. **Density blocks must be relative to native.** `:root[data-density=compact]`
   is copied verbatim from v1.2.0 (`--gap:8px`, `--pad:9px`), which makes
   compact *looser* than native normal (`--gap:6px`). Emit
   `native + (density − v1.2.0 normal)` for every mapped length, as the theme
   pipeline now does (compact: row 24, gap 4, header 26, control 24, pad 9).
5. **Compat content sits on solid panels.** `--bg-window` is emitted as
   `rgb(17 17 17 / .86)` (the Electron window material). In native, compat
   documents are hosted inside solid panels/sheets; the window material is
   AppKit's. Emit the opaque `colors.window` in every edition, not only the
   `-solid` ones, unless a specific compat surface is documented as sitting on
   the window ground.

## Should-fix

6. **Type roles.** After rebase `--fs-md` becomes 13 (`reading`), `--fs-lg` 15
   (`heading`) and `--fs-xl` stays 18. Check the regenerated editions emit
   these; the current ones shrink legacy content (`--fs-md:12px`,
   `--fs-lg:13px`) and blow up `--fs-xl` to 22.
7. **`--bg-row`** is mapped to `panel_secondary` (`#161616`), making list
   rows and Settings cards *darker* than the panel. v1.2.0/main rows are one
   tonal step *towards text* from the panel (dark `#222222` on `#191919`).
   Emit `color-mix(in srgb,var(--tx) 4%,var(--bg-panel))` (main's `--sg-row`).
8. **Keep `--accent-dim` translucent** in editions
   (`color-mix(... 12%, transparent)`) rather than the opaque native
   `accent_wash`: compat DOM stacks it on many backgrounds.
9. **`--hairline`** returns to the foundation `0.5px` after rebase; the current
   `1px` doubles every compat border on Retina.

## Nice-to-have

10. Add `font-variant-numeric: tabular-nums` to the edition's base `:root`
    rule for inputs/numbers so compat values match native number rendering.
11. Name the editions after the four appearance axes in the manifest, with a
    contrast check per edition (text/secondary/muted ≥ 4.5:1) mirroring
    `pm-design/tests/spec.rs`.

## Checklist status

Evidence (screenshots of a compat panel in each edition) is missing — supply
light/dark/IC/RT captures of one legacy extension panel per the
[review checklist](../review-checklist.md) §0 after regenerating.
