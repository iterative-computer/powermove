# Design review: `feat/na-cloud-kit2`

Reviewer: design lead · 2026-11-01 · Lane head `9996854b4`, read against [reviews/na-cloud-p-settings.md](na-cloud-p-settings.md) TODO 1–2, [panels/settings.md §3](../panels/settings.md), [viewer-chrome.md](../panels/viewer-chrome.md), [surfaces/README §3](../surfaces/README.md) and [panels/README §6](../panels/README.md#6-token-additions-and-changes-requested)
and the lane's own tests (no app build, no captures). macOS check on the fix
branch: `cargo test -p pm-surface -p pm-design -p pm-shell -p pm-panel-*` and `-p pm-native-app` pass; `bun test` 629/630 (the one failure is `test/portable-paths.test.ts`, a Bun 1.3.14 internal error on `/private/tmp` paths — environment, not lane).

**Verdict: APPROVE — merge `origin/feat/na-design-fix-kit2` (`f2b7ec380`), not the lane head.**
It is the lane + `origin/feat/native-engine` (`e493e794d`) + the fixes below.
Merge this **first**: the surface lanes' sheet geometry, viewer gizmos and banners depend on its tokens and components.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | settings §3 | `Switch` (settings-root only), `SecureField` (binding only, no value ever on the surface), `ColorWell{minimal}`, `PushButton{role}`, `FormGroup`/`FormRow`, `SettingsRoot` | APPROVE |
| 2 | settings §3 group | Title outside the card, `t.title` **semibold**; row padding `m.inset`, detail `t.minimum` `c.muted`, control trailing; Increase Contrast outline | APPROVE — semibold matches the system grouped form; recorded in settings.md |
| 3 | inspector §7 | `ColourPopover` recipe; preview starts one transaction, outside click commits, Esc cancels, untouched colour strings never rewritten | APPROVE |
| 4 | viewer-chrome §2 | `ViewerGizmo` kinds, authored positions kept, `m.hit` targets, dark palette over media incl. theme dark patch | APPROVE |
| 5 | viewer-chrome *Banners*, surfaces §3 | `Banner` one slot, 4 s unless action, trailing `xmark` `Dismiss` only with an action, `.medium`/`.high` announcements, inline = solid `c.panel_secondary` | APPROVE |
| 6 | §6 wheel | `NumericWheel` rest + no-scroll gate, Option on trackpad, ⇧ ×10, one transaction per gesture | APPROVE |
| 7 | §6 tokens | pm-design had none of the §6 tokens; the kit correctly refused to substitute | **Fixed** — design lead added all §6 tokens to pm-design (incl. `colors.media_badge` + `metrics.media_badge_opacity` 0.55, light `colors.viewer` `#E4E4E4`, `drop_stroke` 2 under Increase Contrast); tokens.md regenerated; missing-token tests now use an older host table |
| 8 | merge | Engine-base panel snapshots lacked the kit's additive pop-up `pmKit.menu` metadata | **Fixed** — snapshots updated (additions only) |

## DESIGN-TODO(opus) answers

None raised. The token dependency is closed by item 7.

## Sol follow-ups

1. macOS adapters exactly as the lane lists (secure field + Keychain, picker + `NSColorSampler`, menu measurement, wheel units/momentum, overlay idle fade). Then light/dark × Increase Contrast captures for the next design pass.
2. Swap the Settings placeholders (p-settings review items 8–10) for these components.

## Integration note (all eight lanes)

Merge order: kit2 fix, then the seven surface fixes in any order. Authority,
media, recovery and documents each **create** `apps/native/crates/app/src/surfaces/mod.rs`,
and every lane adds lines to `main.rs` and `docs/native/surfaces.md`. Those
conflicts are add/add: keep the union of all module lines and table rows.
After merging, run `bun test` once to refresh any snapshot that picks up
kit2's menu metadata.
