# Design review: `feat/na-cloud-p-settings`

Reviewer: design lead · 2026-11-01 · Lane head `5480630dd` (diff vs
`origin/feat/native-engine`). Read against [panels/settings.md](../panels/settings.md),
[panels/README.md](../panels/README.md), the UI-kit rules and the op-tree
snapshots (`panels/settings/__snapshots__/panel.test.tsx.snap`).

**Verdict: APPROVE as an integration candidate — merge
`origin/feat/na-design-fix-p-settings` (`5f0dfdce4`), not the lane head.**
It is the lane plus the small fixes below. Settings is **not** shippable UI
until items 8–10 land (switches, secure fields, colour wells and group cards
are still absent by design); the lane was right not to substitute them.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §1 form | Separate window descriptor, continuous detail, title = active section, no Done, Esc/⌘,/⌘W | APPROVE |
| 2 | §3 row | Control sat right after the label, not trailing | **Fixed** (`justifyContent: space-between`) |
| 3 | §4 Accounts | Subscriptions published `Restart needed` as a default — an unobserved state | **Fixed** (no status until the service publishes one) |
| 4 | §7 VoiceOver | Extension type pop-up was a second `Filter extensions` | **Fixed** → `Extension Type` |
| 5 | §5 | A filter with no matches showed `No Extensions` | **Fixed** → `No Results` / `No extensions match this filter.` |
| 6 | §5 | `No settings found.` rendered as an empty-state *title* | **Fixed** → centred `t.reading` `c.secondary` text |
| 7 | §4 Project | Size presets incomplete | **Fixed** → shipped order (TODO 5 below) |
| 8 | §3 controls | Switch, secure field, minimal colour well, push button missing (`ToolbarItem` stand-ins) | CHANGES REQUIRED — kit lane, then this lane swaps them in |
| 9 | §3 group cards, inset | No group card recipe or sheet tokens | CHANGES REQUIRED — tokens added below; kit `FormGroup` recipe |
| 10 | §3 search highlight | Run highlight absent | CHANGES REQUIRED — kit text-run emphasis (TODO 7) |
| 11 | Extension detail | `About`/`Includes`/… headings are `t.body`; `Delete…` not danger | CHANGES REQUIRED (next pass): sub-headings `t.title`; `Delete…` push button with destructive text, NSAlert per [surfaces/authority.md S012](../surfaces/authority.md) |
| 12 | §2 | Density row disabled placeholder | CHANGES REQUIRED — see TODO 3 |
| 13 | Copy | Every other string matches §4/§9 exactly; Export-defaults details match export-sheet.md | APPROVE |
| 14 | Safety | No secret reaches the tree; archive-only actions; profile writes acknowledged | APPROVE |

## DESIGN-TODO(opus) answers

1. **Recipes.** settings.md §3 is the recipe: `NSSwitch` regular (trailing),
   `NSSecureTextField`, `NSColorWell` `.minimal`, `NSButton` push at
   `m.control` (role `default` / `normal` / `destructive` → system red text),
   source-list sidebar with neutral selection, `NSSearchField` at the sidebar
   top. The kit adds them as AppKit-backed components `Switch`,
   `SecureField`, `ColorWell{minimal}`, `PushButton{role}`, `FormGroup{title}`,
   `SourceList`, `SearchField`. `Switch` is valid **only** under a
   `hostWindow: "settings"` root (kit validator rule). Until then keep the
   placeholders; don't style them.
2. **Tokens.** Added to [panels/README §6](../panels/README.md#6-token-additions-and-changes-requested):
   `metrics.group_radius` (10 = control_radius + inner_radius),
   `metrics.form_row` (40 = row + inset), `metrics.section_gap`
   (40 = 2 × sheet_inset), plus the existing requests `metrics.sheet_inset`
   and `metrics.sheet_value_column`. The op tree references names only; no
   expression path is needed.
3. **Density.** It is a native addition (v1.2.0 has no Density setting), so it
   needs real backing: profile key `density`, values `compact` · `normal` ·
   `comfortable`, default `normal`, read by pm-design to scale `m.row`,
   `m.header`, `m.inset`. **Hide the row** until pm-design exposes density
   variants — a disabled control that can never work is not shown.
4. **Copy.** Profile read failed: row disabled, Error line
   `Couldn’t load this setting.`; write failed: Error line
   `Couldn’t save this setting. Try again.` (value reverts to the last
   acknowledged one). Extensions filter with no matches: `No Results` /
   `No extensions match this filter.` (done). Transcription: none offered →
   `No speech models are available.`; unavailable on this Mac →
   `Transcription isn’t available on this Mac.`; catalogue failed →
   `Couldn’t load speech models. Check your internet connection.` + `Try Again`.
5. **Presets.** `4K UHD · 3840×2160`, `HD 1080p · 1920×1080`,
   `HD 720p · 1280×720`, `Vertical HD · 1080×1920`, `Portrait 4:5 · 1080×1350`,
   `Square · 1080×1080`, separator, `Custom…` (done). A non-matching size shows
   `Custom · {w}×{h}`. `Custom…` reveals rows `Width` and `Height` (number
   wells, `px`, 16–8192, no detail line; Error
   `Enter a size from 16 to 8192 pixels.`). Colour depth: options the engine
   can't render are **omitted**; an authored unsupported value stays selected
   with detail `{depth} isn’t supported on this Mac yet. The project keeps the setting.`
6. **Loading placement.** settings.md §5 is a deliberate exception to
   README §3: a *status check* (subscriptions, Test and Connect) replaces the
   trailing control with a small indeterminate spinner + `Checking…`. A
   whole list loading (Extensions) uses README §3 (centred spinner after
   0.5 s, reason `Loading extensions…`). `ProgressRow` is acceptable as the
   interim op for both. Speech downloads keep a determinate ring + `Cancel`.
7. **Search.** The kit gets a text-run emphasis (`runs: [{start, end, emphasis: "match"}]`)
   painted as `c.accent_wash` behind the run + `t.semibold_weight`. The
   Settings query and the Extensions type filter intersect; the announced count
   and the sidebar dimming reflect what is visible (`{n} settings found`).

## Not design (for the lane)

`bun test` on macOS: `candidate eval resolves from paths with spaces and URL
delimiters` fails on a `/private/tmp` checkout (path handling); pre-existing,
also on the lane head.
