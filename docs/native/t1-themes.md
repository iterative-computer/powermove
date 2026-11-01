# T1: theme selectors, attributes and native dispositions

Baseline: `v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`), read with `git show` from this repository. This is a **source inventory and proposed disposition**, not a visual-parity result. No build or app launch was performed. Installed-extension observations below publish counts only; no extension identifiers, source text, private selectors or names are included.

`R` = `apps/desktop/src/renderer/src/`, `X` = `apps/desktop/src/extensions/`. Source lines refer to the tag. The exact-selector catalog below covers all seven global sheets imported by `R/main.ts:4–12`, the shared token sheet, renderer source/onboarding CSS, component `<style>` blocks, the entry page's boot style, four built-in timeline/viewer stylesheet constants, and Store/visual-reply document defaults. Generated Svelte hash suffixes are implementation details; source selectors and semantic surfaces are the migration contract. Arbitrary installed-extension CSS has an unbounded selector vocabulary and cannot be exhaustively inferred from core source; its disposition must be checked against the actual installed stylesheet.

## Decision classes

- **T — translate:** preserve intended appearance through v4 tokens, semantic surface rules and native state/layout rules, then verify visually. This includes **all shell chrome**: titlebar, project tabs, toolbars, docking, panel frames, splitters, drop targets, layout, loading state and popout chrome. The shell has no compat edition. A CSS property unsupported by a simple surface rule needs a semantic implementation, not a silently ignored declaration.
- **C — compat surface:** keep a non-shell surface in its compat edition/WKWebView where arbitrary DOM structure, CSS cascade/layout, content HTML, pseudo-elements, masks, filter/compositing or extension drawing cannot yet be expressed equivalently. Translate the shell part of a mixed theme and use compat only for the affected content. Do not route the whole shell to a browser.

These are proposed mappings. T does not certify that every CSS declaration already has a native implementation. CSS specificity/source-order, descendant/child/id/class/role/attribute/state matching and custom-property dependency resolution must be represented where they encode intended design. Browser font rasterization, Skia/ANGLE bugs, generated Svelte class names and Electron drag CSS are not native design requirements. Preserve authored geometry/text placement/colour; use the specification for browser quirks. Native scrolling, keyboard focus/accessibility and OS material fallbacks retain their platform behavior.

## What a theme can change

| Input / API | Shipped behavior and source | Surfaces affected | Proposed disposition |
|---|---|---|---|
| `api.theme.register({id,name,scheme,tokens,darkTokens,css,rootAttributes})` | `R/kernel/api.ts:387–415`; `theme-apply.ts:50–134` writes `--*` inline, then arbitrary HTML root attributes and one `style[data-powermove-theme]`; unregister/reload clears tracked keys | Entire same-document shell and panels; not restricted to colour | T tokens/known semantic rules; C for unmatched non-shell DOM rules; shell always T |
| `activate`, `active`, `list`, `scheme`, `setScheme` | Same API; fixed light/dark overrides user/system mode, auto uses media query; `theme:changed` event. `theme-apply.ts:16–41,66–99,143–164`; `R/legacy/app.ts:143–174` owns `data-theme` | Every token consumer, JS/canvas consumer and system-appearance dependent surface | T preference/event contract; preserve fixed/auto/system distinction |
| Workspace `theme` | `R/layout/install.ts:38–74`: accent/bg/panel/text/line/font/mono → `--accent/--bg-window/--bg-panel/--tx/--line/--f-ui/--f-mono`; accent derives dim/text colours; radius → `--r-base=max(2,radius)/3 px` | Whole active workspace, panel chrome, controls and shell spacing/rounding | T workspace layer, applied after kernel theme |
| Workspace `density`, `chrome`, dock/panel geometry | `R/layout/install.ts:131`; `R/legacy/core/workspace.ts:539–547`; `R/kernel/workspace-appearance.ts:26–28,150–151` | Row/header/control size, gap/padding, panel positions/sizes, viewer corners, timeline colours | T typed layout/style rules; geometry is not just a palette |
| Appearance ownership | `extensionWorkspaceAppearance` / `extensionThemeAppearance`, `R/kernel/install.ts:749–762`, `workspace-appearance.ts`, `theme-state.ts` | Active and inactive saved workspaces; overlapping mods, user overrides | T owner/base/stack/tombstone semantics, not a flattened current value |
| Trusted legacy DOM / `PM` access | Same live document; arbitrary selectors, `!important`, style/class/attribute mutations and injected nodes can affect all chrome | Any same-document element, including shell | Preserve extension behavior: translate shell effects; C content needing its exact DOM; audit cleanup of side effects |
| Store sandbox theme registration | `R/kernel/sandbox-host.ts:129–141` allows CSS containing **only custom properties** on `:root`, `html`, `body`; blocks other declarations/URL syntax and strips `rootAttributes` | Global tokens from registered theme; sandbox view inherits computed inline workspace/theme tokens and scheme (`:114–123,616`) | T token-only global contribution; C view DOM where still legacy. Do not broaden Store permissions |
| Popouts | `R/layout/popout.ts:12–18` clones links/styles and copies density/theme/inline style once; viewer/timeline/toolbar are dock-only (`:3`) | Detached panel content + its own chrome | T native popout shell, same theme snapshot; C detached legacy content. Arbitrary extra root attrs are not copied by this helper |

### Root attributes and classes

Four shipped **appearance attributes**; five additional audio diagnostic attributes; three shell/environment classes. `style` and arbitrary extension `rootAttributes` are open namespaces. Standard `lang='en'` at entry is document metadata, not a theme switch.

| Root selector / attribute | Values / writer → consumer | Affected surfaces | Class |
|---|---|---|---|
| `:root`, `html`, `body`, root `style` | Foundation/theme/workspace custom properties; `packages/tokens/tokens.css`, `theme-apply.ts`, `layout/install.ts` | Every inheriting native surface and compat panel; alias graph and colour scheme | T; C for private DOM properties without a semantic equivalent |
| `:root[data-theme="dark"]` and its negation | `data-theme=dark` or absent/light; `R/legacy/app.ts:153`, onboarding welcome `:25` → all dark rules, canvas observers | App/materials, tabs, menus/scrims, Settings selects, Store artwork, account logos, pickers, timeline, mixer, agent effects/visual replies | T resolved appearance; C for legacy effect/HTML body until translated |
| `:root[data-density="compact"]`, `:root[data-density="comfy"]` (`normal` default) | Workspace → `R/layout/install.ts:131`; token overrides | Shell padding/gaps, rows/headers/controls/type/scroll geometry | T; recompute actual layout/hit bounds |
| `[data-preview-corners="rounded"]` (`square` otherwise) | Workspace `chrome.previewCornerRadius` → `workspace.ts:546`, `X/viewer/index.ts:21` | Viewer stage surround/clipping; does not change project pixels | T surface corner rule |
| `[data-timeline-surfaces="reversed"]` (`normal` otherwise) | Workspace `chrome.timelineSurfaceOrder` → `workspace.ts:547`; `X/timeline/timeline.ts:1144–1167` | Swaps panel/panel-2 between track bed and gutter/ruler, including canvas drawing | T painter palette input, not only CSS |
| `data-audio-engine`, `data-audio-state`, `data-audio-voices`, `data-audio-monitor-solo`, `data-audio-source-offset` | Diagnostics `R/legacy/core/audio.ts:577–586` | No built-in theme selector found; trusted CSS may select playback/voice/solo state | T expose equivalent state to rules; never import stale live values as user preferences |
| `.native-app` | Electron shim `R/legacy/host/electron-shim.ts:604` | Platform/shell drag regions and native-only chrome | T environment/state predicate |
| `.remote-app`, `.host-disconnected` | `R/host/web-bridge.ts:633–637,683` | Traffic-light spacing and remote connection state | T native environment/state rule; remote serve is outside cutover scope |
| Arbitrary root attributes and `class` | `ThemeDefinition.rootAttributes` permits any key, not an allowlist; trusted theme CSS may combine attributes/descendants/pseudo states | Entire shell or any content | T shell predicate/rule translation; C untranslatable non-shell content. Unknown attrs are preserved, not discarded |
| `html[data-noscroll]` in a Store view document | `apps/desktop/src/renderer/sandbox/boot.ts:151` | Separate sandbox view scroll container, not editor root | C view while DOM-hosted; T equivalent scroll rule for a native view |

### Selector semantics and surface mapping

| Selector family (exact instances in catalog) | Concrete affected surface | Class / treatment |
|---|---|---|
| `#app`, `#titlebar`, `#tabs`, `#doc-strip`, `.project-*`, `.tab-ghost`, `.tb-*`, `.titlebar-*` | Document tabs/dirty/rename/drag/overflow/focus/close, traffic-light reservation, window controls | **T mandatory**, including `:has`, narrow-tab `@container`, drag state and generated decoration |
| `#toolbar*`, `.iconbtn`, button/selected/disabled variants | Tool strip, tool selection, hit targets and hover/press states | T mandatory; AppKit where appropriate |
| `#body`, `#dock-*`, `.dock*`, `.panel*`, `.splitter*`, `.area-*`, `.drop-*`, `.pop-*`, `#workspace-editbar` | Dock geometry, collapse/hide/resize, moving, drag previews, split/join arrows, popout chrome | **T mandatory**; do not implement only colour tokens |
| `.sg-*`, `.settings-*`, `.st-*`, `.acct-*`, `.pub-*`, transcription selectors | Settings navigation/rows/extensions, Store, sign-in/publish/transcription | T native built-in surfaces; C old extension-provided DOM section where necessary |
| `.ir*`, `.layer*`, `.tl-*`, `.ch*`, viewer/stage and panel ids | Inspector rows, layer list, timeline chrome, viewer surround | T native primitives/painters; C legacy forks requiring unchanged DOM |
| `.spatial-*`, `.agent-*`, code/markdown/iframe/content selectors | Agent rail/composer/questions/trace/artifacts/visual-reply content | T agent chrome/composer/trace; C arbitrary authored HTML/iframe content |
| `.drop`, `.pm-menu`, `.modal*`, `#scrim`, palette, toast, picker/listbox, pseudo-elements | Menus/sheets/floats/overlays and their visibility/focus | T native child surfaces; C legacy content in a compat overlay with correct hit-test holes |
| `input`, `textarea`, `[contenteditable]`, field classes, `:focus*`, `::placeholder`, `::selection`, disabled/ARIA/data variants | Native text and shared controls | T native input/state; respect IME/selection/accessibility. The shipped blanket focus-ring suppression is not a requirement to remove native accessible focus |
| `::-webkit-scrollbar*`, prefix-only `user-select`, `-webkit-app-region` | Browser scroll chrome, selection workaround, Electron draggable areas | T semantic scroll/select/drag behavior; discard browser workaround mechanics |
| `::before`, `::after`, `:is`, `:where`, `:not`, `:has`, child/descendant/nth selectors | Decorations and conditional matching across all listed surfaces | T where mapped to semantic nodes/state/decorations; C arbitrary legacy content where matching cannot be lowered |
| Media/container conditions, keyframes, reduced motion/transparency, glass/masks/blends | Appearance variants, adaptive tabs/Settings, animation, compositing | T native condition/motion/material rules for shell; C non-shell CSS effects lacking an equivalent. Solid fallback below glass OS support and under Reduce Transparency |

### Non-CSS consumers: copying variables alone is insufficient

- Timeline reads computed `--accent`, `--tx`, `--tx-2/3/4`, `--bg-panel`, `--bg-panel-2`, `--bg-float`, `--line`, `--on-accent` (`timeline.ts:1765,2406`), plus reversed-surface attribute; it invalidates on `theme:changed` or root mutation (`X/timeline/timeline.ts:1141–1167`). Map these into Rust timeline painter inputs (**T**), preserving gutter/track/ruler geometry.
- 3D gizmos resolve `--accent` through `R/core/scene3d/theme.ts:3–10`, used by `gizmo.ts:144,332`; generated curve controls and workspace ease canvases resolve text/subtext/line/accent/panel tokens (`R/panels/generated/CurveControl.svelte:54–57,103`, `R/legacy/core/workspace.ts:736–762`). Squircle/rounded-scroll geometry and dock hit/drop bounds consume smoothing/radius/gap tokens (`R/layout/rounded-scroll.ts:49`, `area-drag.ts:181`, `R/settings/squircle.ts:76`). Map all to native painter/layout inputs (**T**), not stale CSS strings.
- Mixer observes root `data-theme/style/class` (`R/panels/MixerPanel.svelte:198,253`); Store observes `data-theme` (`R/store/StoreScreen.svelte:622`); Settings observes it (`R/settings/SettingsScreen.svelte:290`). These need native reactive updates (**T**).
- Agent atmosphere/glow/ghost frames branch on root dark state (`R/panels/agent/AgentAtmosphereFrame.svelte:10`, `PromptGlowFrame.svelte:33`, `GhostFieldFrame.svelte:27`, `GhostCard.svelte:114`). Preserve the effect's intended output through native drawing (**T**) or its legacy surface (**C**) until translation is verified.
- Visual replies copy a selected computed palette into a sandboxed HTML document (`R/panels/agent/VisualReply.svelte:20–32,68,111`; `visual-reply.ts:52–62`). The container/composer is T; arbitrary authored HTML and its own selectors are **C**. HTML is an authored artifact, not interchangeable with a static screenshot.

## Foundation tokens

Every unique token declared by `packages/tokens/tokens.css` is enumerated below. All are **T** aliases/typed design inputs. Preserve dependency chains, light/dark overrides and workspace priority. Base tokens affect shell **and** panels; do not scope them to the compat page alone. App-local tokens (glass, titlebar, boot, content, etc.) are recorded with their selectors in the full catalog.

| Family | Tokens | Native effect |
|---|---|---|
| Colour/surfaces/ink/text/status | `--accent`, `--accent-dim`, `--accent-hover`, `--accent-tx`, `--bg-field`, `--bg-float`, `--bg-panel`, `--bg-panel-2`, `--bg-rail`, `--bg-row`, `--bg-row-hi`, `--bg-sunken`, `--bg-window`, `--blue`, `--blue-deep`, `--cream`, `--danger`, `--gray`, `--green`, `--ink-1`, `--ink-2`, `--ink-3`, `--ink-rgb`, `--line`, `--line-2`, `--line-strong`, `--on-accent`, `--paper-rgb`, `--red`, `--sand`, `--section-line`, `--success`, `--tx`, `--tx-2`, `--tx-3`, `--tx-4`, `--violet`, `--warning`, `--yellow` | T: colour/surfaces/ink/text/status throughout shell and content |
| Geometry/metrics | `--ctl-h`, `--gap`, `--gut`, `--hairline`, `--hdr-h`, `--hit`, `--pad`, `--panel-edge-inset`, `--r-base`, `--r-lg`, `--r-md`, `--r-pill`, `--r-sm`, `--r-xl`, `--r-xs`, `--row-h`, `--val-w` | T: geometry/metrics throughout shell and content |
| Typography | `--f-mono`, `--f-ui`, `--fs-lg`, `--fs-md`, `--fs-sm`, `--fs-xl`, `--fs-xs`, `--fw-medium`, `--fw-regular`, `--fw-semibold`, `--lh`, `--lh-tight` | T: typography throughout shell and content |
| Control material/shadows/focus | `--ctl-edge`, `--ctl-edge-inset`, `--ctl-raised`, `--focus-ring`, `--shadow-float`, `--shadow-lift`, `--shadow-panel`, `--shadow-raise` | T: control material/shadows/focus throughout shell and content |
| Motion and disabled state | `--disabled`, `--dur-1`, `--dur-2`, `--dur-3`, `--ease`, `--ease-io` | T: motion and disabled state throughout shell and content |

Density overrides are not new tokens: compact changes row/header/control/font/gutter/padding/gap; comfy changes the same inputs. `--r-base` scales the radius family. Legacy `--ctl-edge`, `--ctl-edge-inset`, `--shadow-raise` names remain accepted even where shipped values are flat/none. Native material choices are tokens, not copies of Chromium backdrop-filter internals.

## Installed-theme count and limits

Read-only immediate-child manifest/source survey of the installed extension folder: **44 installed extension directories; 1 declares a theme through the theme API, and 1 global theme-like extension under the root-attribute/token/global-CSS criterion** (the same installation). Panel-local style injection or merely consuming `--bg-panel` is not a theme. The profile is mutable and includes engine-era data; it is not the original frozen corpus. The earlier baseline's theme-like count of **3** is therefore **not reproduced by this survey**. Keep both observations distinct; do not substitute the live count into performance gates without re-freezing the actual corpus. No installed identifiers or private selectors are included here.

There are **2 built-in theme definitions** at the tag (default and high contrast, `X/theme-default/index.ts:3–42`). Both propose T: the latter changes 8 colour/focus tokens per scheme and the former uses defaults. They are not counted as user-installed extensions.

For each actual installed/forked/Store theme before cutover: lower its shell rules, identify non-shell DOM dependencies, retain compat editions wherever equivalence is unproved, and record a visual disposition. Validate enabled → disabled → enabled, reload, removal, restart, two saved workspaces, overlapping mods disabled in both orders, and a later user change surviving disable (`apps/desktop/docs/EXTENSIONS.md:493–521`). Inventory alone does not pass T1. Freeze performance budgets only after these real dispositions exist.

## Exact source-selector catalog

Each row is **one selector occurrence** (comma lists split only at top level, so commas inside `:is/:where/:not` remain intact). Repeated selectors at different source lines/conditions are intentional. `T` is a proposed semantic native mapping; component-owned arbitrary HTML content is `C`. Conditions are preserved. Declaration names make local custom properties visible without copying an entire stylesheet. All built-in CSS may additionally be applied to a compat edition of its content, but that does not turn shell rows into C.

Catalog totals: **3329 selector occurrences in 74 source files; 3310 T, 19 C; 82 unique foundation tokens; 55 keyframe blocks.** Counts exclude private extension CSS and compiler-generated hash selectors.

### `packages/tokens/tokens.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 6 | `:root` | — | All surfaces / token foundation | T | `color-scheme` |
| 7 | `:root[data-theme="dark"]` | — | All surfaces / token foundation | T | `color-scheme` |
| 8 | `:root` | — | All surfaces / token foundation | T | `--accent`, `--accent-dim`, `--accent-hover`, `--accent-tx`, `--bg-field`, `--bg-float`, `--bg-panel`, `--bg-panel-2`, `--bg-rail`, `--bg-row`, `--bg-row-hi`, `--bg-sunken`, `--bg-window`, `--blue`, `--blue-deep`, `--cream`, `--ctl-edge`, `--ctl-edge-inset`, `--ctl-h`, `--ctl-raised`, `--danger`, `--disabled`, `--dur-1`, `--dur-2`, `--dur-3`, `--ease`, `--ease-io`, `--f-mono`, `--f-ui`, `--focus-ring`, `--fs-lg`, `--fs-md`, `--fs-sm`, `--fs-xl`, `--fs-xs`, `--fw-medium`, `--fw-regular`, `--fw-semibold`, `--gap`, `--gray`, `--green`, `--gut`, `--hairline`, `--hdr-h`, `--hit`, `--ink-1`, `--ink-2`, `--ink-3`, `--ink-rgb`, `--lh`, `--lh-tight`, `--line`, `--line-2`, `--line-strong`, `--on-accent`, `--pad`, `--panel-edge-inset`, `--paper-rgb`, `--r-base`, `--r-lg`, `--r-md`, `--r-pill`, `--r-sm`, `--r-xl`, `--r-xs`, `--red`, `--row-h`, `--sand`, `--section-line`, `--shadow-float`, `--shadow-lift`, `--shadow-panel`, `--shadow-raise`, `--success`, `--tx`, `--tx-2`, `--tx-3`, `--tx-4`, `--val-w`, `--violet`, `--warning`, `--yellow` |
| 118 | `:root[data-density="compact"]` | — | All surfaces / token foundation | T | `--ctl-h`, `--fs-md`, `--gap`, `--gut`, `--hdr-h`, `--pad`, `--row-h` |
| 119 | `:root[data-density="comfy"]` | — | All surfaces / token foundation | T | `--ctl-h`, `--fs-md`, `--gap`, `--gut`, `--hdr-h`, `--pad`, `--row-h` |
| 122 | `:root[data-theme="dark"]` | — | All surfaces / token foundation | T | `--accent`, `--accent-dim`, `--accent-hover`, `--accent-tx`, `--bg-field`, `--bg-float`, `--bg-panel`, `--bg-panel-2`, `--bg-rail`, `--bg-row`, `--bg-row-hi`, `--bg-sunken`, `--bg-window`, `--blue`, `--blue-deep`, `--cream`, `--ctl-edge`, `--ctl-edge-inset`, `--ctl-raised`, `--danger`, `--focus-ring`, `--gray`, `--ink-1`, `--ink-2`, `--ink-3`, `--ink-rgb`, `--line`, `--line-2`, `--line-strong`, `--paper-rgb`, `--sand`, `--section-line`, `--shadow-float`, `--shadow-lift`, `--shadow-panel`, `--shadow-raise`, `--success`, `--tx`, `--tx-2`, `--tx-3`, `--tx-4`, `--violet`, `--warning` |

### `apps/desktop/css/account.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 10 | `.acct-avatar` | — | Account and sign-in UI | T | `--size`, `-webkit-user-select`, `background`, `border-radius`, `color`, `display`, `flex`, `font-size`, `font-weight`, `height`, `letter-spacing`, `line-height`, `place-items`, `user-select`, `width` |
| 19 | `.ps-account` | — | Account and sign-in UI | T | `display`, `flex-direction`, `margin-top` |
| 20 | `.ps-updates:not([hidden])~.ps-account` | — | Account and sign-in UI | T | `margin-top` |
| 23 | `.acct-row.signed-in` | — | Account and sign-in UI | T | `gap`, `padding-left` |
| 24 | `.acct-row.open` | — | Account and sign-in UI | T | `background`, `color` |
| 25 | `.acct-row.open svg` | — | Account and sign-in UI | T | `color` |
| 27 | `.acct-menu-head` | — | Account and sign-in UI | T | `align-items`, `display`, `gap` |
| 28 | `.acct-menu-head>div` | — | Account and sign-in UI | T | `display`, `flex-direction`, `gap`, `min-width` |
| 29 | `.acct-menu-head>div>b` | — | Account and sign-in UI | T | `color`, `font-size`, `font-weight`, `line-height`, `overflow`, `text-overflow`, `white-space` |
| 30 | `.acct-menu-head>div>span` | — | Account and sign-in UI | T | `color`, `font-size`, `font-weight`, `line-height`, `overflow`, `text-overflow`, `white-space` |
| 33 | `.account-modal .mb` | — | Account and sign-in UI | T | `gap`, `padding` |
| 34 | `.acct-sheet:focus` | — | Account and sign-in UI | T | `outline` |
| 35 | `.acct-step` | — | Account and sign-in UI | T | `display`, `flex-direction` |
| 36 | `.acct-mark` | — | Account and sign-in UI | T | `-webkit-user-drag`, `-webkit-user-select`, `display`, `height`, `margin`, `user-select`, `width` |
| 37 | `:root[data-theme="dark"] .acct-mark.is-light` | — | Account and sign-in UI | T | `display` |
| 37 | `:root:not([data-theme="dark"]) .acct-mark.is-dark` | — | Account and sign-in UI | T | `display` |
| 38 | `.acct-head h2` | — | Account and sign-in UI | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `text-wrap` |
| 39 | `.acct-head p` | — | Account and sign-in UI | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 40 | `.acct-head b` | — | Account and sign-in UI | T | `color`, `font-weight`, `overflow-wrap` |
| 44 | `.acct-providers` | — | Account and sign-in UI | T | `display`, `flex-direction`, `gap`, `margin-top` |
| 45 | `.account-modal .btn.acct-provider` | — | Account and sign-in UI | T | `border-radius`, `font-size`, `gap`, `height`, `padding`, `width` |
| 45 | `.account-modal .btn.acct-submit` | — | Account and sign-in UI | T | `border-radius`, `font-size`, `gap`, `height`, `padding`, `width` |
| 46 | `.acct-provider svg` | — | Account and sign-in UI | T | `height`, `width` |
| 46 | `.acct-provider .acct-provider-mark` | — | Account and sign-in UI | T | `height`, `width` |
| 48 | `.acct-provider[data-provider="apple"] svg` | — | Account and sign-in UI | T | `transform` |
| 49 | `.acct-email` | — | Account and sign-in UI | T | `display`, `flex-direction`, `gap`, `margin-top` |
| 50 | `.acct-field` | — | Account and sign-in UI | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `height`, `padding`, `transition`, `width` |
| 51 | `.acct-field::placeholder` | — | Account and sign-in UI | T | `color` |
| 52 | `.acct-field:hover` | — | Account and sign-in UI | T | `box-shadow` |
| 53 | `.acct-field:focus-visible` | — | Account and sign-in UI | T | `box-shadow`, `outline` |
| 54 | `.account-modal .btn.pri:disabled` | — | Account and sign-in UI | T | `background`, `color`, `opacity` |
| 55 | `.account-modal .btn.pri` | — | Account and sign-in UI | T | `transition` |
| 57 | `.acct-foot` | — | Account and sign-in UI | T | `align-items`, `display`, `gap`, `justify-content`, `margin-top` |
| 58 | `.acct-foot p` | — | Account and sign-in UI | T | `color`, `font-size`, `line-height` |
| 59 | `.acct-foot .btn` | — | Account and sign-in UI | T | `height`, `margin-right`, `padding` |
| 59 | `.acct-code-foot .btn` | — | Account and sign-in UI | T | `height`, `margin-right`, `padding` |
| 60 | `.acct-link` | — | Account and sign-in UI | T | `background`, `border`, `color`, `cursor`, `font`, `font-weight`, `padding`, `text-decoration`, `text-decoration-color`, `text-underline-offset`, `transition` |
| 61 | `.acct-link:hover` | — | Account and sign-in UI | T | `color`, `text-decoration-color` |
| 62 | `.acct-link:focus-visible` | — | Account and sign-in UI | T | `border-radius`, `box-shadow`, `outline` |
| 65 | `.acct-back` | — | Account and sign-in UI | T | `align-items`, `align-self`, `border`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `margin`, `padding`, `transition` |
| 66 | `.acct-back svg` | — | Account and sign-in UI | T | `height`, `transform`, `width` |
| 67 | `.acct-back:hover` | — | Account and sign-in UI | T | `background`, `color` |
| 68 | `.acct-back:focus-visible` | — | Account and sign-in UI | T | `box-shadow`, `outline` |
| 71 | `.acct-wait` | — | Account and sign-in UI | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `gap`, `line-height`, `margin`, `min-height`, `padding` |
| 72 | `.acct-wait>span` | — | Account and sign-in UI | T | `flex`, `min-width` |
| 73 | `.acct-wait .btn` | — | Account and sign-in UI | T | `height`, `margin-right`, `padding` |
| 74 | `.acct-spinner` | — | Account and sign-in UI | T | `animation`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-width`, `width` |
| 78 | `.acct-code` | — | Account and sign-in UI | T | `cursor`, `display`, `gap`, `margin-top`, `position` |
| 79 | `.acct-code input` | — | Account and sign-in UI | T | `border`, `caret-color`, `cursor`, `height`, `inset`, `opacity`, `padding`, `position`, `width`, `z-index` |
| 80 | `.acct-cell` | — | Account and sign-in UI | T | `border-radius`, `box-shadow`, `color`, `display`, `flex`, `font-size`, `font-variant-numeric`, `font-weight`, `height`, `line-height`, `min-width`, `place-items`, `transition` |
| 81 | `.acct-cell:nth-of-type(4)` | — | Account and sign-in UI | T | `margin-left` |
| 82 | `.acct-cell.filled` | — | Account and sign-in UI | T | `background`, `box-shadow` |
| 83 | `.acct-cell.active` | — | Account and sign-in UI | T | `box-shadow` |
| 84 | `.acct-cell.active:not(.filled)::after` | — | Account and sign-in UI | T | `animation`, `background`, `border-radius`, `content`, `height`, `width` |
| 86 | `.acct-code.verifying .acct-cell` | — | Account and sign-in UI | T | `color` |
| 87 | `.acct-code.wrong .acct-cell` | — | Account and sign-in UI | T | `background`, `box-shadow`, `color` |
| 88 | `.acct-code-foot` | — | Account and sign-in UI | T | `align-items`, `display`, `gap`, `justify-content`, `margin-top`, `min-height` |
| 89 | `.acct-code-foot .btn` | — | Account and sign-in UI | T | `font-variant-numeric` |
| 90 | `.acct-status` | — | Account and sign-in UI | T | `color`, `font-size`, `line-height`, `min-width`, `text-wrap` |
| 91 | `.acct-status.is-wrong` | — | Account and sign-in UI | T | `color` |
| 94 | `.account-modal` | — | Account and sign-in UI | T | `--sg-row` |
| 95 | `.acct-form.sg-column` | — | Account and sign-in UI | T | `display`, `flex-direction`, `margin`, `max-width`, `width` |
| 96 | `.acct-form .sg-group` | — | Account and sign-in UI | T | `border-radius`, `margin` |
| 97 | `.acct-form .sg-group>.settings-row` | — | Account and sign-in UI | T | `min-height` |
| 98 | `.acct-form .settings-input` | — | Account and sign-in UI | T | `flex`, `width` |
| 98 | `.acct-handle-field` | — | Account and sign-in UI | T | `flex`, `width` |
| 99 | `.acct-handle-field` | — | Account and sign-in UI | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `height`, `padding`, `transition` |
| 100 | `.acct-handle-field input` | — | Account and sign-in UI | T | `background`, `border`, `color`, `flex`, `font-size`, `height`, `min-width`, `padding` |
| 101 | `.acct-handle-field input:focus-visible` | — | Account and sign-in UI | T | `outline` |
| 103 | `.acct-form .settings-input` | — | Account and sign-in UI | T | `text-align` |
| 104 | `.acct-form .settings-input:focus-visible` | — | Account and sign-in UI | T | `box-shadow`, `outline` |
| 105 | `.acct-form .settings-input::placeholder` | — | Account and sign-in UI | T | `color` |
| 106 | `.acct-handle-status` | — | Account and sign-in UI | T | `transition` |
| 107 | `.acct-handle-status[data-state="available"]` | — | Account and sign-in UI | T | `color` |
| 108 | `.acct-handle-status[data-state="error"]` | — | Account and sign-in UI | T | `color` |
| 109 | `.acct-coord` | — | Account and sign-in UI | T | `color`, `font-size`, `line-height`, `margin-top` |
| 110 | `.acct-coord code` | — | Account and sign-in UI | T | `color`, `font` |
| 111 | `.acct-form .acct-submit` | — | Account and sign-in UI | T | `margin-top` |
| 112 | `.account-modal .btn.acct-later` | — | Account and sign-in UI | T | `height`, `margin-top`, `width` |
| 113 | `.acct-email .acct-status` | — | Account and sign-in UI | T | `margin` |
| 114 | `.acct-legal` | — | Account and sign-in UI | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 115 | `.acct-legal .acct-link` | — | Account and sign-in UI | T | `color`, `font-weight` |
| 118 | `.acct-profile` | — | Account and sign-in UI | T | `gap` |
| 119 | `.acct-profile .settings-copy` | — | Account and sign-in UI | T | `padding-left` |
| 120 | `.acct-profile>.acct-avatar` | — | Account and sign-in UI | T | `margin-left` |
| 122 | `.btn.acct-delete:not(:disabled):is(:hover,:focus-visible)` | — | Account and sign-in UI | T | `color` |
| 124 | `.account-modal button` | — | Account and sign-in UI | T | `cursor` |
| 124 | `.acct-sheet label` | — | Account and sign-in UI | T | `cursor` |
| 125 | `.acct-sheet :is(.btn,.acct-link,.acct-back,.acct-code)` | — | Account and sign-in UI | T | `-webkit-user-select`, `user-select` |
| 125 | `.acct-row` | — | Account and sign-in UI | T | `-webkit-user-select`, `user-select` |
| 127 | `.acct-spinner` | @media (prefers-reduced-motion:reduce) | Account and sign-in UI | T | `animation-duration` |
| 128 | `.acct-cell.active:not(.filled)::after` | @media (prefers-reduced-motion:reduce) | Account and sign-in UI | T | `animation` |

### `apps/desktop/css/app.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 1 | `*` | — | Global controls / materials / shell | T | `box-sizing`, `margin`, `padding` |
| 2 | `html` | — | Global controls / materials / shell | T | `height`, `overflow` |
| 2 | `body` | — | Global controls / materials / shell | T | `height`, `overflow` |
| 3 | `:root` | — | Global controls / materials / shell | T | `--ui-corner-smoothing` |
| 8 | `:where(.panel,.drop,.modal,#library-screen,.sg-main,.library-card,.ps-grid:not(.list) .ps-thumb,.spatial-compose,.spatial-outline,.panel-ghost-card,.panel-drop-preview.dock-edge,.pop-mirror,.fill-picker,#workspace-editbar)` | — | Global controls / materials / shell | T | `corner-shape` |
| 18 | `:root` | — | Global controls / materials / shell | T | `--bg-window`, `--bg-window-solid` |
| 19 | `:root[data-theme="dark"]` | — | Global controls / materials / shell | T | `--bg-window`, `--bg-window-solid` |
| 22 | `:root[data-theme="dark"]` | — | Global controls / materials / shell | T | `--bg-float` |
| 25 | `:root` | — | Global controls / materials / shell | T | `--bg-content` |
| 26 | `html` | — | Global controls / materials / shell | T | `background` |
| 34 | `:root` | — | Global controls / materials / shell | T | `--glass-backdrop`, `--glass-edge`, `--glass-edge-hi`, `--glass-fill`, `--glass-fill-hi`, `--glass-rim`, `--glass-rim-hi` |
| 46 | `:root[data-theme="dark"]` | — | Global controls / materials / shell | T | `--glass-edge`, `--glass-edge-hi`, `--glass-fill`, `--glass-fill-hi`, `--glass-rim`, `--glass-rim-hi` |
| 54 | `:is(.glass,#doc-strip,#toolbar-strip,.ps-navbtn.on,.sg-navbtn.on)` | — | Global controls / materials / shell | T | `-webkit-backdrop-filter`, `backdrop-filter`, `background`, `box-shadow`, `isolation`, `position` |
| 61 | `:is(.glass,.ps-navbtn.on,.sg-navbtn.on)::after` | — | Global controls / materials / shell | T | `-webkit-mask`, `-webkit-mask-composite`, `background`, `border-radius`, `content`, `inset`, `mask`, `mask-composite`, `opacity`, `padding`, `pointer-events`, `position`, `transition`, `z-index` |
| 67 | `:is(.glass-hi,#toolbar .iconbtn.tl.on,#toolbar .iconbtn.tg.on,.project-doc.on,.project-strip-btn.on)` | — | Global controls / materials / shell | T | `background`, `box-shadow`, `isolation`, `position` |
| 68 | `.glass-hi::after` | — | Global controls / materials / shell | T | `-webkit-mask`, `-webkit-mask-composite`, `background`, `border-radius`, `content`, `inset`, `mask`, `mask-composite`, `opacity`, `padding`, `pointer-events`, `position`, `transition`, `z-index` |
| 75 | `:is(.glass,#doc-strip,#toolbar-strip,.ps-navbtn.on,.sg-navbtn.on)` | @media (prefers-reduced-transparency: reduce) | Global controls / materials / shell | T | `-webkit-backdrop-filter`, `backdrop-filter`, `background`, `box-shadow` |
| 76 | `:is(.glass,.ps-navbtn.on,.sg-navbtn.on)::after` | @media (prefers-reduced-transparency: reduce) | Global controls / materials / shell | T | `display` |
| 76 | `.glass-hi::after` | @media (prefers-reduced-transparency: reduce) | Global controls / materials / shell | T | `display` |
| 77 | `.glass-hi` | @media (prefers-reduced-transparency: reduce) | Global controls / materials / shell | T | `background`, `box-shadow` |
| 78 | `:is(#toolbar .iconbtn.tl.on,#toolbar .iconbtn.tg.on,.project-doc.on,.project-strip-btn.on)` | @media (prefers-reduced-transparency: reduce) | Global controls / materials / shell | T | `background`, `box-shadow` |
| 80 | `body` | — | Global controls / materials / shell | T | `-webkit-font-smoothing`, `-webkit-user-select`, `background`, `color`, `cursor`, `font-family`, `font-size`, `text-rendering`, `user-select` |
| 88 | `body *` | — | Global controls / materials / shell | T | `-webkit-user-select`, `user-select` |
| 89 | `input:not([type=range]):not([type=color]):not([type=file]):not([type=checkbox]):not([type=radio])` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 89 | `textarea` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 89 | `[contenteditable]:not([contenteditable="false"])` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 89 | `[contenteditable]:not([contenteditable="false"]) *` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 89 | `.editable-text` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 89 | `.selectable-text` | — | Global controls / materials / shell | T | `-webkit-user-select`, `cursor`, `user-select` |
| 92 | `::selection` | — | Global controls / materials / shell | T | `background` |
| 93 | `button` | — | Global controls / materials / shell | T | `-webkit-tap-highlight-color`, `background`, `border`, `color`, `cursor`, `font` |
| 97 | `:focus` | — | Global controls / materials / shell | T | `outline` |
| 97 | `:focus-visible` | — | Global controls / materials / shell | T | `outline` |
| 97 | `:focus-within` | — | Global controls / materials / shell | T | `outline` |
| 98 | `select:focus` | — | Global controls / materials / shell | T | `box-shadow`, `outline` |
| 98 | `select:focus-visible` | — | Global controls / materials / shell | T | `box-shadow`, `outline` |
| 99 | `:is(button,[role="button"]):disabled` | — | Global controls / materials / shell | T | `opacity`, `pointer-events` |
| 99 | `.is-disabled` | — | Global controls / materials / shell | T | `opacity`, `pointer-events` |
| 100 | `input` | — | Global controls / materials / shell | T | `background`, `border`, `color`, `font`, `outline` |
| 100 | `textarea` | — | Global controls / materials / shell | T | `background`, `border`, `color`, `font`, `outline` |
| 101 | `input::placeholder` | — | Global controls / materials / shell | T | `color` |
| 101 | `textarea::placeholder` | — | Global controls / materials / shell | T | `color` |
| 102 | `.pm-icon` | — | Global controls / materials / shell | T | `display`, `fill`, `flex`, `overflow`, `stroke` |
| 103 | `::-webkit-scrollbar` | — | Global controls / materials / shell | T | `height`, `width` |
| 104 | `::-webkit-scrollbar-thumb` | — | Global controls / materials / shell | T | `background`, `background-clip`, `border`, `border-radius` |
| 105 | `::-webkit-scrollbar-thumb:hover` | — | Global controls / materials / shell | T | `background`, `background-clip` |
| 106 | `::-webkit-scrollbar-track` | — | Global controls / materials / shell | T | `background` |
| 110 | `.panel` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.panel *` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.library-content` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.spatial-preview` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.pop-mirror` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.pop-mirror *` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 110 | `.fill-picker` | — | Global controls / materials / shell | T | `scrollbar-width` |
| 111 | `.panel::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.panel *::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.library-content::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.spatial-preview::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.pop-mirror::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.pop-mirror *::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 111 | `.fill-picker::-webkit-scrollbar` | — | Global controls / materials / shell | T | `display` |
| 114 | `#app` | — | Shell titlebar and project tabs | T | `background`, `display`, `flex-direction`, `height` |
| 116 | `#titlebar` | — | Shell titlebar and project tabs | T | `--titlebar-h`, `--traffic-inset`, `-webkit-app-region`, `align-items`, `display`, `flex`, `gap`, `height`, `padding`, `position`, `z-index` |
| 125 | `#tabs` | — | Shell titlebar and project tabs | T | `-webkit-app-region`, `align-items`, `display`, `flex`, `gap`, `height`, `min-width`, `position` |
| 130 | `.project-tablist` | — | Shell titlebar and project tabs | T | `align-items`, `display`, `flex`, `gap`, `min-width` |
| 131 | `.project-tab-scroll` | — | Shell titlebar and project tabs | T | `align-items`, `display`, `gap`, `height`, `min-width`, `overflow-x`, `overflow-y`, `overscroll-behavior-x`, `position`, `scrollbar-width` |
| 135 | `.project-tab-scroll::-webkit-scrollbar` | — | Shell titlebar and project tabs | T | `display` |
| 137 | `.project-tab-scroll.fade-start` | — | Shell titlebar and project tabs | T | `-webkit-mask-image`, `mask-image` |
| 138 | `.project-tab-scroll.fade-end` | — | Shell titlebar and project tabs | T | `-webkit-mask-image`, `mask-image` |
| 139 | `.project-tab-scroll.fade-start.fade-end` | — | Shell titlebar and project tabs | T | `-webkit-mask-image`, `mask-image` |
| 140 | `#tabs :is([role="tab"],button):focus-visible` | — | Shell titlebar and project tabs | T | `outline`, `outline-offset` |
| 142 | `.titlebar-drag` | — | Shell titlebar and project tabs | T | `-webkit-app-region`, `align-self`, `flex`, `min-width`, `pointer-events` |
| 143 | `.titlebar-drag-traffic` | — | Shell titlebar and project tabs | T | `bottom`, `flex`, `left`, `position`, `top`, `width` |
| 145 | `.remote-app #titlebar` | — | Shell titlebar and project tabs | T | `--traffic-inset` |
| 146 | `#titlebar button` | — | Shell titlebar and project tabs | T | `-webkit-app-region` |
| 146 | `#titlebar input` | — | Shell titlebar and project tabs | T | `-webkit-app-region` |
| 146 | `#titlebar a` | — | Shell titlebar and project tabs | T | `-webkit-app-region` |
| 146 | `#titlebar [role="button"]` | — | Shell titlebar and project tabs | T | `-webkit-app-region` |
| 147 | `.project-strip-btn` | — | Shell titlebar and project tabs | T | `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 148 | `.project-strip-btn:hover` | — | Shell titlebar and project tabs | T | `background`, `color` |
| 149 | `.project-strip-btn:active` | — | Shell titlebar and project tabs | T | `transform` |
| 150 | `.project-strip-btn.on` | — | Shell titlebar and project tabs | T | `background`, `box-shadow`, `color` |
| 151 | `.project-strip-btn svg` | — | Shell titlebar and project tabs | T | `height`, `width` |
| 152 | `.project-new` | — | Shell titlebar and project tabs | T | `margin-left` |
| 156 | `.project-doc` | — | Shell titlebar and project tabs | T | `align-items`, `border`, `border-radius`, `color`, `container-type`, `display`, `flex`, `font-size`, `font-weight`, `gap`, `height`, `min-width`, `padding`, `position`, `transition`, `white-space`, `width` |
| 162 | `.project-doc:hover` | — | Shell titlebar and project tabs | T | `background`, `color` |
| 164 | `.project-doc.current:not(.on)` | — | Shell titlebar and project tabs | T | `color` |
| 165 | `.project-doc.on` | — | Shell titlebar and project tabs | T | `background`, `box-shadow`, `color` |
| 167 | `.project-doc.lifted` | — | Shell titlebar and project tabs | T | `background`, `box-shadow`, `color`, `transition`, `z-index` |
| 168 | `.project-tab-scroll.dragging .project-doc:not(.lifted)` | — | Shell titlebar and project tabs | T | `pointer-events` |
| 171 | `.project-doc.torn` | — | Shell titlebar and project tabs | T | `opacity` |
| 172 | `.tab-ghost` | — | Shell titlebar and project tabs | T | `background`, `box-shadow`, `color`, `pointer-events`, `position`, `transform`, `width`, `z-index` |
| 173 | `.project-doc-label` | — | Shell titlebar and project tabs | T | `flex`, `min-width`, `overflow`, `text-overflow` |
| 174 | `.project-doc-close` | — | Shell titlebar and project tabs | T | `border-radius`, `color`, `display`, `flex`, `height`, `margin-left`, `opacity`, `place-items`, `position`, `transition`, `width` |
| 175 | `.project-doc-close svg` | — | Shell titlebar and project tabs | T | `height`, `transition`, `width` |
| 176 | `.project-doc:is(:hover,.on,:focus-visible) .project-doc-close` | — | Shell titlebar and project tabs | T | `opacity` |
| 177 | `.project-doc-close:hover` | — | Shell titlebar and project tabs | T | `background`, `color` |
| 178 | `.project-doc.lifted .project-doc-close` | — | Shell titlebar and project tabs | T | `opacity` |
| 179 | `.project-doc-state` | — | Shell titlebar and project tabs | T | `color`, `flex`, `font-weight` |
| 180 | `.project-doc.on .project-doc-state` | — | Shell titlebar and project tabs | T | `color` |
| 183 | `.project-doc-state` | @container (max-width:150px) | Shell titlebar and project tabs | T | `display` |
| 184 | `.project-doc.dirty .project-doc-label` | @container (max-width:150px) | Shell titlebar and project tabs | T | `font-style` |
| 186 | `.project-doc.renaming` | — | Shell titlebar and project tabs | T | `box-shadow` |
| 187 | `.project-doc-input` | — | Shell titlebar and project tabs | T | `background`, `color`, `font`, `height`, `line-height`, `min-width`, `width` |
| 188 | `.project-doc` | @media (prefers-reduced-motion:reduce) | Shell titlebar and project tabs | T | `transition` |
| 188 | `.project-doc-close` | @media (prefers-reduced-motion:reduce) | Shell titlebar and project tabs | T | `transition` |
| 188 | `.project-doc-close svg` | @media (prefers-reduced-motion:reduce) | Shell titlebar and project tabs | T | `transition` |
| 191 | `.project-doc.pending .project-doc-label` | — | Shell titlebar and project tabs | T | `animation` |
| 192 | `.project-doc.pending:not([data-tab-id])` | — | Shell titlebar and project tabs | T | `pointer-events` |
| 193 | `.tb-project-controls` | — | Shell titlebar and project tabs | T | `display` |
| 194 | `.project-doc.pending .project-doc-label` | @media (prefers-reduced-motion:reduce) | Shell titlebar and project tabs | T | `animation` |
| 196 | `.tb-right` | — | Shell titlebar and project tabs | T | `-webkit-app-region`, `align-items`, `display`, `gap`, `margin-left`, `position` |
| 199 | `.tb-export` | — | Shell titlebar and project tabs | T | `border-radius`, `height`, `margin-left`, `padding` |
| 205 | `.export-modal` | — | Export dialog | T | `max-height`, `max-width`, `top` |
| 210 | `.export-modal>h3` | — | Export dialog | T | `padding-left` |
| 211 | `.export-modal .mb` | — | Export dialog | T | `gap`, `padding` |
| 212 | `.export-form.sg-column` | — | Export dialog | T | `display`, `flex-direction`, `gap`, `margin`, `max-width`, `padding`, `width` |
| 213 | `.export-form [hidden]` | — | Export dialog | T | `display` |
| 214 | `.export-form .settings-section` | — | Export dialog | T | `gap` |
| 215 | `.export-form .sg-group` | — | Export dialog | T | `margin` |
| 218 | `.export-destinations` | — | Export dialog | T | `background`, `border`, `border-radius`, `display`, `gap`, `grid-template-columns`, `margin`, `padding` |
| 219 | `.export-destination` | — | Export dialog | T | `background`, `border`, `border-radius`, `color`, `cursor`, `font-size`, `font-weight`, `height`, `min-width`, `overflow`, `padding`, `text-overflow`, `transition`, `white-space` |
| 220 | `.export-destination:hover` | — | Export dialog | T | `color` |
| 221 | `.export-destination.on` | — | Export dialog | T | `background`, `color` |
| 221 | `.export-destination[aria-pressed=true]` | — | Export dialog | T | `background`, `color` |
| 222 | `.export-destination:focus-visible` | — | Export dialog | T | `box-shadow`, `outline` |
| 223 | `.export-destination-note` | — | Export dialog | T | `color`, `font-size`, `line-height`, `margin`, `min-height` |
| 225 | `.export-about` | — | Export dialog | T | `padding` |
| 226 | `.export-about-copy` | — | Export dialog | T | `display`, `flex-direction`, `gap` |
| 227 | `.export-about-copy>b` | — | Export dialog | T | `color`, `font-size`, `font-weight`, `line-height` |
| 228 | `.export-about-copy>p` | — | Export dialog | T | `color`, `font-size`, `line-height`, `margin`, `text-wrap` |
| 229 | `.export-code-includes` | — | Export dialog | T | `color`, `display`, `flex-wrap`, `font-size`, `gap`, `line-height`, `margin` |
| 230 | `.export-code-includes span+span::before` | — | Export dialog | T | `color`, `content`, `margin-right` |
| 231 | `.export-about-copy .export-code-delivery` | — | Export dialog | T | `color` |
| 233 | `.export-form .sg-group>.settings-row` | — | Export dialog | T | `min-height` |
| 235 | `.export-form .settings-copy` | — | Export dialog | T | `padding` |
| 236 | `.export-form .settings-copy>span` | — | Export dialog | T | `max-width` |
| 237 | `.export-form .pm-select.settings-select` | — | Export dialog | T | `max-width`, `min-width` |
| 238 | `.export-form .settings-input` | — | Export dialog | T | `width` |
| 239 | `.export-form .export-preset-save` | — | Export dialog | T | `align-items`, `display`, `gap` |
| 240 | `.export-form .export-preset-save .settings-input` | — | Export dialog | T | `text-align`, `width` |
| 241 | `.export-form .export-preset-save .btn` | — | Export dialog | T | `height`, `padding` |
| 242 | `.export-modal .mf .btn` | — | Export dialog | T | `border-radius` |
| 243 | `.export-form .settings-note` | — | Export dialog | T | `margin-top` |
| 245 | `.export-summary` | — | Export dialog | T | `color`, `display`, `flex-direction`, `font-size`, `font-variant-numeric`, `gap`, `line-height`, `margin` |
| 246 | `.export-summary>b` | — | Export dialog | T | `color`, `font-weight` |
| 247 | `.export-summary>span:empty` | — | Export dialog | T | `display` |
| 249 | `.export-progress` | — | Export dialog | T | `display`, `flex-direction`, `gap` |
| 250 | `.export-preview-stage` | — | Export dialog | T | `background`, `border-radius`, `overflow`, `position` |
| 251 | `.export-preview` | — | Export dialog | T | `background`, `display`, `max-height`, `object-fit`, `width` |
| 252 | `.export-scan` | — | Export dialog | T | `height`, `inset`, `pointer-events`, `position`, `width` |
| 253 | `.export-progress .bar` | — | Export dialog | T | `background`, `border-radius`, `overflow` |
| 254 | `.export-progress .bar i` | — | Export dialog | T | `background`, `display`, `height`, `transition` |
| 255 | `.export-progress-label` | — | Export dialog | T | `color`, `display`, `font-size`, `font-variant-numeric`, `gap`, `justify-content`, `line-height` |
| 256 | `.export-progress-label>span:last-child` | — | Export dialog | T | `color` |
| 258 | `.iconbtn` | — | Export dialog | T | `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 262 | `.iconbtn:hover` | — | Export dialog | T | `background`, `color` |
| 263 | `.iconbtn:active` | — | Export dialog | T | `background`, `transform` |
| 266 | `.iconbtn.on` | — | Export dialog | T | `background`, `color` |
| 267 | `.iconbtn.dis` | — | Export dialog | T | `opacity`, `pointer-events` |
| 268 | `.iconbtn svg` | — | Export dialog | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 271 | `#toolbar-strip` | — | Shell toolbar | T | `-webkit-app-region`, `align-items`, `border`, `border-radius`, `display`, `flex`, `height`, `margin`, `min-width`, `padding`, `position`, `z-index` |
| 276 | `#toolbar-strip[hidden]` | — | Shell toolbar | T | `display` |
| 277 | `#toolbar` | — | Shell toolbar | T | `align-items`, `display`, `gap`, `width` |
| 278 | `#toolbar .iconbtn` | — | Shell toolbar | T | `border-radius`, `height`, `width` |
| 279 | `#toolbar .tl-group` | — | Shell toolbar | T | `align-items`, `display`, `gap` |
| 280 | `#toolbar .iconbtn.tl-more` | — | Shell toolbar | T | `color`, `padding`, `width` |
| 281 | `#toolbar .tl-more svg` | — | Shell toolbar | T | `height`, `width` |
| 282 | `#toolbar .tl-more:hover` | — | Shell toolbar | T | `color` |
| 283 | `#toolbar .tl-sep` | — | Shell toolbar | T | `background`, `flex`, `height`, `margin`, `width` |
| 284 | `#toolbar .iconbtn.tl.on` | — | Shell toolbar | T | `background`, `box-shadow`, `color` |
| 285 | `#toolbar .iconbtn.tg` | — | Shell toolbar | T | `color` |
| 286 | `#toolbar .iconbtn.tg.on` | — | Shell toolbar | T | `background`, `box-shadow`, `color` |
| 289 | `#panel-preview .body` | — | Viewer surround | T | `align-items`, `display` |
| 290 | `.pv` | — | Viewer surround | T | `align-items`, `display`, `gap`, `padding`, `width` |
| 291 | `.pv-row` | — | Viewer surround | T | `align-items`, `display`, `gap` |
| 292 | `.pv-meta` | — | Viewer surround | T | `align-items`, `color`, `display`, `gap` |
| 293 | `.pv-tc` | — | Viewer surround | T | `color`, `font-size`, `font-variant-numeric` |
| 296 | `#body` | — | Shell docks, panels, splitters, popouts | T | `display`, `flex`, `gap`, `min-height`, `padding` |
| 298 | `#body` | @media (min-width:1200px) | Shell docks, panels, splitters, popouts | T | `transition` |
| 300 | `.dock` | — | Shell docks, panels, splitters, popouts | T | `display`, `flex-direction`, `gap`, `min-height`, `min-width` |
| 301 | `.dock.col` | — | Shell docks, panels, splitters, popouts | T | `flex-direction` |
| 302 | `.dock#dock-center` | — | Shell docks, panels, splitters, popouts | T | `flex`, `min-width` |
| 303 | `.dock#dock-left` | — | Shell docks, panels, splitters, popouts | T | `flex`, `min-width` |
| 304 | `.dock#dock-right` | — | Shell docks, panels, splitters, popouts | T | `flex`, `min-width` |
| 305 | `.dock#dock-bottom` | — | Shell docks, panels, splitters, popouts | T | `flex`, `min-height` |
| 307 | `.dock-stack` | — | Shell docks, panels, splitters, popouts | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `min-width` |
| 308 | `.dock-row` | — | Shell docks, panels, splitters, popouts | T | `display`, `flex`, `gap`, `min-height`, `min-width` |
| 311 | `.dock:is(#dock-left,#dock-right)` | — | Shell docks, panels, splitters, popouts | T | `border-radius`, `corner-shape`, `overflow-y`, `overscroll-behavior-y`, `scrollbar-width` |
| 312 | `.dock:is(#dock-left,#dock-right)::-webkit-scrollbar` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 318 | `.dock:is(#dock-left,#dock-right)` | — | Shell docks, panels, splitters, popouts | T | `--dock-fade`, `mask-image`, `transition` |
| 329 | `.dock:is(#dock-left,#dock-right)[data-overflow-top="1"]` | — | Shell docks, panels, splitters, popouts | T | `--dock-fade-top` |
| 330 | `.dock:is(#dock-left,#dock-right)[data-overflow-bottom="1"]` | — | Shell docks, panels, splitters, popouts | T | `--dock-fade-bottom` |
| 332 | `.dock:is(#dock-left,#dock-right)` | @media (prefers-reduced-motion:reduce) | Shell docks, panels, splitters, popouts | T | `transition` |
| 341 | `.dock:is(#dock-left,#dock-right)>.ui-placement-ghost` | — | Shell docks, panels, splitters, popouts | T | `flex-shrink` |
| 342 | `.dock.hidden` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 345 | `.splitter` | — | Shell docks, panels, splitters, popouts | T | `cursor`, `flex`, `margin`, `position`, `z-index` |
| 346 | `.splitter::after` | — | Shell docks, panels, splitters, popouts | T | `border-radius`, `content`, `inset`, `opacity`, `position`, `transition` |
| 350 | `.splitter:not(.h)::after` | — | Shell docks, panels, splitters, popouts | T | `background` |
| 355 | `.splitter:hover::after` | — | Shell docks, panels, splitters, popouts | T | `opacity`, `transition-duration` |
| 355 | `.splitter.drag::after` | — | Shell docks, panels, splitters, popouts | T | `opacity`, `transition-duration` |
| 356 | `.splitter.h` | — | Shell docks, panels, splitters, popouts | T | `cursor`, `flex`, `margin`, `width` |
| 357 | `.splitter.h::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `inset` |
| 363 | `.splitter.h::before` | — | Shell docks, panels, splitters, popouts | T | `content`, `cursor`, `inset`, `position`, `z-index` |
| 364 | `.panel-dragging .splitter::after` | — | Shell docks, panels, splitters, popouts | T | `opacity` |
| 366 | `.panel` | — | Shell docks, panels, splitters, popouts | T | `background`, `border`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `min-height`, `min-width`, `overflow`, `position` |
| 370 | `.panel.flush` | — | Shell docks, panels, splitters, popouts | T | `background` |
| 371 | `.panel.grow` | — | Shell docks, panels, splitters, popouts | T | `flex` |
| 372 | `.panel.hidden` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 374 | `.panel > header` | — | Shell docks, panels, splitters, popouts | T | `align-items`, `border-bottom`, `color`, `display`, `flex`, `font-size`, `font-weight`, `gap`, `height`, `letter-spacing`, `padding`, `user-select` |
| 379 | `.panel > header::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `content`, `height`, `left`, `pointer-events`, `position`, `right`, `top` |
| 383 | `.panel > header` | — | Shell docks, panels, splitters, popouts | T | `padding-left`, `position` |
| 384 | `.panel > header:not(:has(button:hover))` | — | Shell docks, panels, splitters, popouts | T | `cursor` |
| 385 | `.panel > header:active:not(:has(button:active))` | — | Shell docks, panels, splitters, popouts | T | `cursor` |
| 386 | `.panel > header .grip` | — | Shell docks, panels, splitters, popouts | T | `bottom`, `color`, `display`, `left`, `opacity`, `place-items`, `pointer-events`, `position`, `top`, `transition`, `width` |
| 387 | `.panel > header:hover .grip` | — | Shell docks, panels, splitters, popouts | T | `opacity` |
| 388 | `.panel > header .grip svg` | — | Shell docks, panels, splitters, popouts | T | `height`, `width` |
| 389 | `.panel > header .panel-action` | — | Shell docks, panels, splitters, popouts | T | `color`, `height`, `width` |
| 390 | `.panel > header .panel-action svg` | — | Shell docks, panels, splitters, popouts | T | `height`, `width` |
| 391 | `.panel > header .panel-action:hover` | — | Shell docks, panels, splitters, popouts | T | `color` |
| 392 | `.panel-move-handle` | — | Shell docks, panels, splitters, popouts | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `cursor`, `display`, `height`, `left`, `opacity`, `place-items`, `position`, `top`, `transform`, `transition`, `width`, `z-index` |
| 399 | `.panel-move-handle:hover` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow`, `color`, `opacity` |
| 399 | `.panel-move-handle:focus-visible` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow`, `color`, `opacity` |
| 400 | `.panel-move-handle:active` | — | Shell docks, panels, splitters, popouts | T | `cursor` |
| 401 | `.panel-move-handle svg` | — | Shell docks, panels, splitters, popouts | T | `height`, `pointer-events`, `width` |
| 402 | `.panel-move-handle.inline` | — | Shell docks, panels, splitters, popouts | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `height`, `margin-right`, `opacity`, `position`, `transform`, `width` |
| 406 | `.panel-move-handle.inline:hover` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow`, `color`, `opacity` |
| 406 | `.panel-move-handle.inline:focus-visible` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow`, `color`, `opacity` |
| 407 | `.panel > header .ptitle` | — | Shell docks, panels, splitters, popouts | T | `overflow`, `text-overflow`, `white-space` |
| 408 | `.panel-ghost` | — | Shell docks, panels, splitters, popouts | T | `height`, `left`, `pointer-events`, `position`, `top`, `will-change`, `z-index` |
| 411 | `.panel-ghost-card` | — | Shell docks, panels, splitters, popouts | T | `align-items`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `display`, `gap`, `height`, `opacity`, `padding`, `transform`, `transition` |
| 416 | `.panel-ghost.on .panel-ghost-card` | — | Shell docks, panels, splitters, popouts | T | `opacity`, `transform` |
| 417 | `.panel-ghost-label` | — | Shell docks, panels, splitters, popouts | T | `font-size`, `font-weight`, `white-space` |
| 418 | `.panel-ghost-destination` | — | Shell docks, panels, splitters, popouts | T | `color`, `font-size`, `font-weight`, `white-space` |
| 419 | `.panel > header .sub` | — | Shell docks, panels, splitters, popouts | T | `color`, `font-size`, `font-weight` |
| 420 | `.panel > header .sp` | — | Shell docks, panels, splitters, popouts | T | `flex` |
| 423 | `.panel-options` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 424 | `.panel > .body` | — | Shell docks, panels, splitters, popouts | T | `flex`, `min-height`, `overflow`, `position` |
| 425 | `.panel.noscroll > .body` | — | Shell docks, panels, splitters, popouts | T | `overflow` |
| 429 | `.panel.frame > .body` | — | Shell docks, panels, splitters, popouts | T | `overflow` |
| 430 | `.ext-panel-frame` | — | Shell docks, panels, splitters, popouts | T | `background`, `border`, `display`, `height`, `inset`, `opacity`, `position`, `transition`, `width` |
| 431 | `.ext-panel-frame[data-state]` | — | Shell docks, panels, splitters, popouts | T | `opacity` |
| 432 | `.ext-panel-frame` | @media (prefers-reduced-motion:reduce) | Shell docks, panels, splitters, popouts | T | `transition` |
| 433 | `.panel.headless > header` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 435 | `.chip` | — | Shell docks, panels, splitters, popouts | T | `align-items`, `background`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `isolation`, `justify-content`, `padding`, `position`, `transition`, `white-space` |
| 442 | `.chip::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `border-radius`, `content`, `inset`, `opacity`, `position`, `transition`, `z-index` |
| 442 | `.btn::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `border-radius`, `content`, `inset`, `opacity`, `position`, `transition`, `z-index` |
| 443 | `.chip:hover` | — | Shell docks, panels, splitters, popouts | T | `color` |
| 444 | `.chip:hover::after` | — | Shell docks, panels, splitters, popouts | T | `opacity` |
| 444 | `.btn:hover::after` | — | Shell docks, panels, splitters, popouts | T | `opacity` |
| 445 | `.chip:active` | — | Shell docks, panels, splitters, popouts | T | `transform` |
| 445 | `.btn:active` | — | Shell docks, panels, splitters, popouts | T | `transform` |
| 446 | `.chip:active::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `opacity`, `transition` |
| 446 | `.btn:active::after` | — | Shell docks, panels, splitters, popouts | T | `background`, `opacity`, `transition` |
| 447 | `.chip.ghost` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow` |
| 448 | `.chip.ghost::after` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 448 | `.chip.solid::after` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 448 | `.btn.ghost::after` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 448 | `.btn.pri::after` | — | Shell docks, panels, splitters, popouts | T | `display` |
| 449 | `.chip.ghost:hover` | — | Shell docks, panels, splitters, popouts | T | `background` |
| 450 | `.chip.wide` | — | Shell docks, panels, splitters, popouts | T | `height`, `width` |
| 451 | `.chip.solid` | — | Shell docks, panels, splitters, popouts | T | `background`, `box-shadow`, `color` |
| 452 | `.chip.solid:hover` | — | Shell docks, panels, splitters, popouts | T | `background`, `color` |
| 453 | `.chip svg` | — | Shell docks, panels, splitters, popouts | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 456 | `input[type=range]` | — | Timeline | T | `-webkit-appearance`, `background`, `border-radius`, `height`, `width` |
| 457 | `input[type=range]::-webkit-slider-thumb` | — | Timeline | T | `-webkit-appearance`, `background`, `border-radius`, `box-shadow`, `height`, `width` |
| 459 | `.generated-tool-readout` | — | Inspector and fields | T | `color`, `display`, `font-size`, `max-width`, `overflow`, `text-overflow`, `white-space` |
| 460 | `.generated-tool-preview` | — | Inspector and fields | T | `background`, `border`, `border-radius`, `color`, `font-size`, `margin-top`, `padding` |
| 461 | `.generated-tool-preview[hidden]` | — | Inspector and fields | T | `display` |
| 462 | `.generated-tool-preview strong` | — | Inspector and fields | T | `color`, `display`, `font-size`, `font-weight` |
| 463 | `.generated-tool-preview p` | — | Inspector and fields | T | `color`, `line-height`, `margin` |
| 464 | `.generated-tool-preview ol` | — | Inspector and fields | T | `display`, `flex-direction`, `gap`, `list-style`, `margin`, `padding` |
| 465 | `.generated-tool-preview li` | — | Inspector and fields | T | `align-items`, `display`, `gap`, `justify-content`, `min-width` |
| 466 | `.generated-tool-preview li span` | — | Inspector and fields | T | `overflow`, `text-overflow`, `white-space` |
| 467 | `.generated-tool-preview code` | — | Inspector and fields | T | `color`, `flex`, `font-size` |
| 468 | `.generated-tool-preview li.more` | — | Inspector and fields | T | `color` |
| 469 | `.generated-tool-preview.error` | — | Inspector and fields | T | `background`, `border-color` |
| 470 | `.generated-curve-field` | — | Inspector and fields | T | `background`, `border`, `border-radius`, `display`, `flex-direction`, `gap`, `margin`, `padding` |
| 471 | `.generated-curve-head` | — | Inspector and fields | T | `align-items`, `color`, `display`, `font-size`, `gap`, `justify-content` |
| 472 | `.generated-curve-head code` | — | Inspector and fields | T | `color`, `font-size`, `max-width`, `overflow`, `text-overflow`, `white-space` |
| 473 | `.generated-curve-canvas` | — | Inspector and fields | T | `aspect-ratio`, `background`, `border-radius`, `cursor`, `display`, `height`, `touch-action`, `width` |
| 474 | `.generated-curve-canvas:focus-visible` | — | Inspector and fields | T | `outline` |
| 475 | `.generated-curve-presets` | — | Inspector and fields | T | `display`, `gap`, `grid-template-columns` |
| 476 | `.generated-curve-presets button` | — | Inspector and fields | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `font-size`, `gap`, `height`, `min-width`, `overflow`, `padding` |
| 477 | `.generated-curve-presets button:hover` | — | Inspector and fields | T | `border-color`, `color` |
| 478 | `.generated-curve-presets button i` | — | Inspector and fields | T | `border-bottom`, `border-left`, `border-radius`, `flex`, `height`, `position`, `width` |
| 479 | `.generated-curve-presets button i::after` | — | Inspector and fields | T | `border-radius`, `border-top`, `bottom`, `content`, `height`, `left`, `position`, `transform`, `width` |
| 480 | `.generated-curve-presets button span` | — | Inspector and fields | T | `min-width`, `overflow`, `text-overflow`, `white-space` |
| 483 | `.sec` | — | Inspector and fields | T | `align-items`, `border-top`, `color`, `display`, `font-size`, `font-weight`, `height`, `justify-content`, `letter-spacing`, `margin`, `padding` |
| 485 | `.sec:first-child` | — | Inspector and fields | T | `border-top`, `margin-top` |
| 486 | `.row` | — | Inspector and fields | T | `align-items`, `background`, `border-radius`, `display`, `gap`, `height`, `padding`, `position` |
| 490 | `.row .k` | — | Inspector and fields | T | `color`, `flex`, `font-size`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 491 | `.row .v` | — | Inspector and fields | T | `color`, `font-family`, `font-size`, `font-variant-numeric`, `padding-right`, `text-align` |
| 493 | `.row.split` | — | Inspector and fields | T | `gap`, `height`, `margin`, `padding` |
| 494 | `.row.split .k` | — | Inspector and fields | T | `align-items`, `color`, `display`, `flex`, `height`, `padding`, `width` |
| 495 | `.row.split .vwrap` | — | Inspector and fields | T | `align-items`, `display`, `flex`, `gap`, `height`, `min-width`, `padding`, `width` |
| 498 | `.row.split .vwrap.pair` | — | Inspector and fields | T | `display`, `gap` |
| 499 | `.row.split .pair-group` | — | Inspector and fields | T | `background`, `border-radius`, `display`, `flex`, `gap`, `grid-template-columns`, `height`, `min-width`, `overflow` |
| 503 | `.row.split .pair-group .num` | — | Inspector and fields | T | `background`, `border-radius` |
| 504 | `.row.split .pair-group .num:hover` | — | Inspector and fields | T | `background` |
| 505 | `.row.split .pair-group .num.editing` | — | Inspector and fields | T | `background` |
| 506 | `.row.split .pair-group .well + .well` | — | Inspector and fields | T | `box-shadow` |
| 507 | `.row.split .vwrap > *` | — | Inspector and fields | T | `min-width` |
| 512 | `.num` | — | Inspector and fields | T | `background`, `border-radius`, `color`, `height`, `transition` |
| 512 | `.sel` | — | Inspector and fields | T | `background`, `border-radius`, `color`, `height`, `transition` |
| 512 | `.color-field` | — | Inspector and fields | T | `background`, `border-radius`, `color`, `height`, `transition` |
| 516 | `.num:hover` | — | Inspector and fields | T | `background` |
| 516 | `.sel:hover` | — | Inspector and fields | T | `background` |
| 516 | `.color-field:hover` | — | Inspector and fields | T | `background` |
| 517 | `.num` | — | Inspector and fields | T | `cursor`, `flex`, `font-family`, `font-size`, `font-variant-numeric`, `line-height`, `min-width`, `padding`, `text-align`, `width` |
| 519 | `.num.editing` | — | Inspector and fields | T | `background`, `box-shadow`, `cursor` |
| 520 | `.num.link` | — | Inspector and fields | T | `color` |
| 522 | `.well` | — | Inspector and fields | T | `align-items`, `display`, `flex`, `min-width`, `position` |
| 523 | `.well[data-prefix]::before` | — | Inspector and fields | T | `bottom`, `color`, `content`, `display`, `font-family`, `font-size`, `left`, `place-items`, `pointer-events`, `position`, `top`, `width` |
| 525 | `.well[data-prefix] .num` | — | Inspector and fields | T | `padding-left` |
| 526 | `.kf` | — | Inspector and fields | T | `color`, `display`, `flex`, `height`, `place-items`, `position`, `right`, `top`, `width` |
| 527 | `.kf i` | — | Inspector and fields | T | `background`, `border`, `height`, `transform`, `transition`, `width` |
| 528 | `.kf:hover i` | — | Inspector and fields | T | `border-color` |
| 529 | `.kf.track i` | — | Inspector and fields | T | `border-color` |
| 530 | `.kf.on i` | — | Inspector and fields | T | `background`, `border-color` |
| 531 | `.color-field` | — | Inspector and fields | T | `align-items`, `display`, `flex`, `flex-direction`, `gap`, `justify-content`, `padding` |
| 532 | `.color-field .sw` | — | Inspector and fields | T | `border-radius`, `box-shadow`, `height`, `width` |
| 532 | `.color-dialog` | — | Inspector and fields | T | `display`, `flex-direction`, `gap` |
| 532 | `.color-dialog-value` | — | Inspector and fields | T | `align-items`, `display`, `gap` |
| 532 | `.color-hex` | — | Inspector and fields | T | `background`, `border`, `border-radius`, `box-shadow`, `flex`, `font-family`, `height`, `padding` |
| 532 | `.color-dialog-preview` | — | Inspector and fields | T | `background`, `border-radius`, `height`, `width` |
| 532 | `.color-grid` | — | Inspector and fields | T | `display`, `gap`, `grid-template-columns` |
| 532 | `.color-choice` | — | Inspector and fields | T | `aspect-ratio`, `background`, `border-radius` |
| 532 | `.color-choice.on` | — | Inspector and fields | T | `box-shadow` |
| 535 | `.color-workbench` | — | Inspector and fields | T | `display`, `gap`, `grid-template-columns`, `height` |
| 536 | `.color-side` | — | Inspector and fields | T | `display`, `flex-direction`, `gap`, `min-width` |
| 537 | `.color-compare` | — | Inspector and fields | T | `border-radius`, `display`, `flex`, `height`, `overflow` |
| 538 | `.color-compare-swatch` | — | Inspector and fields | T | `background`, `flex`, `min-width` |
| 539 | `.color-channels` | — | Inspector and fields | T | `display`, `gap` |
| 540 | `.color-channel` | — | Inspector and fields | T | `align-items`, `color`, `display`, `font-size`, `gap`, `grid-template-columns` |
| 541 | `.color-channel input` | — | Inspector and fields | T | `background`, `border-radius`, `color`, `font`, `height`, `min-width`, `padding`, `text-align`, `width` |
| 542 | `.color-channel em` | — | Inspector and fields | T | `font-style` |
| 543 | `.color-channels hr` | — | Inspector and fields | T | `background`, `border`, `height` |
| 544 | `.color-dialog :is(.fill-sv,.fill-hue)` | — | Inspector and fields | T | `touch-action` |
| 545 | `.color-dialog :is(button,[role="slider"]):focus-visible` | — | Inspector and fields | T | `outline`, `outline-offset` |
| 546 | `.sw` | — | Inspector and fields | T | `background`, `border-radius`, `flex`, `height`, `position`, `width` |
| 547 | `.fill-picker-layer` | — | Inspector and fields | T | `background`, `inset`, `position`, `z-index` |
| 548 | `.fill-color-workbench` | — | Inspector and fields | T | `display`, `flex-direction`, `gap` |
| 548 | `.fill-color-main` | — | Inspector and fields | T | `display`, `gap`, `grid-template-columns`, `height` |
| 549 | `.fill-sv` | — | Inspector and fields | T | `background`, `border-radius`, `cursor`, `overflow`, `position` |
| 550 | `.fill-sv i` | — | Inspector and fields | T | `border`, `border-radius`, `box-shadow`, `height`, `pointer-events`, `position`, `transform`, `width` |
| 551 | `.fill-hue` | — | Inspector and fields | T | `background`, `border-radius`, `cursor`, `position` |
| 551 | `.fill-hue i` | — | Inspector and fields | T | `border`, `border-radius`, `box-shadow`, `height`, `left`, `pointer-events`, `position`, `right`, `transform` |
| 552 | `.fill-color-values` | — | Inspector and fields | T | `align-items`, `display`, `gap`, `grid-template-columns` |
| 552 | `.fill-current` | — | Inspector and fields | T | `background`, `border-radius`, `height`, `width` |
| 553 | `.fill-hex-field` | — | Inspector and fields | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `gap`, `grid-template-columns`, `height`, `padding` |
| 553 | `.fill-hex` | — | Inspector and fields | T | `color`, `font`, `min-width` |
| 554 | `.fill-channels-toggle` | — | Inspector and fields | T | `background`, `border-radius`, `color`, `font-size`, `height`, `padding` |
| 554 | `.fill-channels-toggle:hover` | — | Inspector and fields | T | `background`, `color` |
| 554 | `.fill-channels-toggle.on` | — | Inspector and fields | T | `background`, `color` |
| 555 | `.fill-channels` | — | Inspector and fields | T | `background`, `border-radius`, `display`, `gap`, `grid-column`, `grid-template-columns`, `padding` |
| 555 | `.fill-channels[hidden]` | — | Inspector and fields | T | `display` |
| 555 | `.fill-channel` | — | Inspector and fields | T | `align-items`, `color`, `display`, `font-size`, `gap`, `grid-template-columns` |
| 555 | `.fill-channel input` | — | Inspector and fields | T | `background`, `border-radius`, `font`, `height`, `padding`, `width` |
| 556 | `.fill-palette-row` | — | Inspector and fields | T | `align-items`, `display` |
| 556 | `.fill-palette` | — | Inspector and fields | T | `display`, `flex`, `gap`, `grid-template-columns` |
| 556 | `.fill-spectrum-color` | — | Inspector and fields | T | `background`, `border-radius`, `height` |
| 556 | `.fill-spectrum-color:hover` | — | Inspector and fields | T | `transform` |
| 557 | `.toggle` | — | Inspector and fields | T | `background`, `border-radius`, `box-shadow`, `flex`, `height`, `position`, `transition`, `width` |
| 558 | `.toggle i` | — | Inspector and fields | T | `background`, `border-radius`, `box-shadow`, `height`, `left`, `position`, `top`, `transition`, `width` |
| 559 | `.toggle.on` | — | Inspector and fields | T | `background` |
| 560 | `.toggle.on i` | — | Inspector and fields | T | `transform` |
| 561 | `.sel` | — | Inspector and fields | T | `flex`, `font-family`, `font-size`, `max-width`, `padding`, `width` |
| 562 | `.stopwatch` | — | Inspector and fields | T | `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 563 | `.stopwatch:hover` | — | Inspector and fields | T | `background`, `color` |
| 564 | `.stopwatch.on` | — | Inspector and fields | T | `color` |
| 565 | `.stopwatch svg` | — | Inspector and fields | T | `fill`, `height`, `stroke`, `stroke-width`, `width` |
| 566 | `.empty` | — | Inspector and fields | T | `color`, `font-size`, `line-height`, `padding`, `text-align`, `text-wrap` |
| 568 | `.lyr` | — | Layer list | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `gap`, `height`, `padding`, `transition` |
| 569 | `.lyr:hover` | — | Layer list | T | `background` |
| 570 | `.lyr.sel` | — | Layer list | T | `background`, `color` |
| 571 | `.lyr:focus-visible` | — | Layer list | T | `background`, `outline` |
| 572 | `.lyr .sw2` | — | Layer list | T | `border-radius`, `flex`, `height`, `width` |
| 573 | `.lyr .nm` | — | Layer list | T | `flex`, `overflow`, `text-overflow`, `white-space` |
| 574 | `.lyr .idx` | — | Layer list | T | `color`, `flex`, `font-size`, `font-variant-numeric`, `min-width`, `text-align`, `white-space` |
| 576 | `.simple-effect-row .idx` | — | Layer list | T | `font-size`, `opacity`, `text-align`, `transition` |
| 577 | `.simple-effect-row:hover .idx` | — | Layer list | T | `opacity` |
| 577 | `.simple-effect-row:focus-visible .idx` | — | Layer list | T | `opacity` |
| 580 | `.assets-panel-body` | — | Layer list | T | `display`, `flex-direction`, `min-height`, `overflow`, `position` |
| 583 | `.asset-card` | — | Layer list | T | `transition` |
| 584 | `.asset-card.is-dragging` | — | Layer list | T | `opacity`, `transform` |
| 585 | `.asset-drop-overlay` | — | Layer list | T | `backdrop-filter`, `background`, `border`, `border-radius`, `display`, `inset`, `opacity`, `place-items`, `pointer-events`, `position`, `transition`, `z-index` |
| 586 | `.assets-panel-body.is-drop-over .asset-drop-overlay` | — | Layer list | T | `opacity` |
| 587 | `.asset-drop-card` | — | Layer list | T | `align-items`, `background`, `border-radius`, `box-shadow`, `color`, `display`, `flex-direction`, `gap`, `padding`, `text-align`, `transform`, `transition` |
| 588 | `.assets-panel-body.is-drop-over .asset-drop-card` | — | Layer list | T | `transform` |
| 589 | `.asset-drop-card svg` | — | Layer list | T | `color`, `height`, `margin-bottom`, `width` |
| 590 | `.asset-drop-card b` | — | Layer list | T | `color`, `font-size`, `font-weight` |
| 591 | `.asset-drop-card small` | — | Layer list | T | `color`, `font-size` |
| 595 | `.asset-list` | — | Layer list | T | `align-content`, `display`, `flex`, `gap`, `grid-template-columns`, `min-height`, `overflow`, `padding`, `position`, `user-select` |
| 596 | `.asset-card` | — | Layer list | T | `color`, `cursor`, `display`, `flex-direction`, `gap`, `min-width`, `outline`, `position` |
| 597 | `.asset-preview` | — | Layer list | T | `aspect-ratio`, `background`, `border-radius`, `color`, `display`, `overflow`, `place-items`, `position`, `width` |
| 601 | `.asset-preview::after` | — | Layer list | T | `border-radius`, `box-shadow`, `content`, `inset`, `pointer-events`, `position`, `transition`, `z-index` |
| 602 | `.asset-card:focus-visible .asset-preview::after` | — | Layer list | T | `box-shadow` |
| 603 | `.asset-card[aria-selected="true"] .asset-preview::after` | — | Layer list | T | `box-shadow` |
| 604 | `.asset-preview svg` | — | Layer list | T | `height`, `width` |
| 605 | `.asset-preview img` | — | Layer list | T | `height`, `inset`, `object-fit`, `position`, `width` |
| 605 | `.asset-preview video` | — | Layer list | T | `height`, `inset`, `object-fit`, `position`, `width` |
| 606 | `.asset-preview video` | — | Layer list | T | `opacity` |
| 607 | `.asset-preview video[data-ready="true"]` | — | Layer list | T | `opacity` |
| 610 | `.asset-card.is-offline .asset-preview img` | — | Layer list | T | `filter`, `opacity` |
| 610 | `.asset-card.is-offline .asset-preview video` | — | Layer list | T | `filter`, `opacity` |
| 611 | `.asset-card.is-offline .asset-preview` | — | Layer list | T | `background` |
| 612 | `.asset-card.is-offline .asset-preview>svg` | — | Layer list | T | `opacity` |
| 613 | `.asset-offline` | — | Layer list | T | `align-items`, `background`, `border-radius`, `bottom`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `left`, `letter-spacing`, `padding`, `position`, `white-space`, `z-index` |
| 614 | `.asset-offline svg` | — | Layer list | T | `height`, `width` |
| 615 | `.asset-loading` | — | Layer list | T | `align-items`, `background`, `border-radius`, `bottom`, `color`, `display`, `font-size`, `font-weight`, `height`, `left`, `letter-spacing`, `padding`, `position`, `white-space`, `z-index` |
| 616 | `.asset-card.is-loading .asset-preview img` | — | Layer list | T | `opacity` |
| 616 | `.asset-card.is-loading .asset-preview video` | — | Layer list | T | `opacity` |
| 617 | `.asset-copy small.asset-missing` | — | Layer list | T | `color` |
| 618 | `.asset-preview.audio` | — | Layer list | T | `background` |
| 619 | `.asset-wave` | — | Layer list | T | `align-items`, `display`, `gap`, `inset`, `padding`, `position` |
| 620 | `.asset-wave i` | — | Layer list | T | `background`, `flex`, `min-width`, `opacity` |
| 621 | `.asset-badge` | — | Layer list | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `font-variant-numeric`, `height`, `left`, `padding`, `position`, `top`, `transition`, `z-index` |
| 622 | `.asset-copy` | — | Layer list | T | `display`, `flex-direction`, `gap`, `min-width`, `padding` |
| 623 | `.asset-copy b` | — | Layer list | T | `overflow`, `text-overflow`, `white-space` |
| 623 | `.asset-copy small` | — | Layer list | T | `overflow`, `text-overflow`, `white-space` |
| 624 | `.asset-copy b` | — | Layer list | T | `color`, `font-size`, `font-weight` |
| 625 | `.asset-copy small` | — | Layer list | T | `color`, `font-size` |
| 626 | `.asset-actions` | — | Layer list | T | `display`, `gap`, `opacity`, `position`, `right`, `top`, `transition`, `z-index` |
| 627 | `.asset-card:hover .asset-actions` | — | Layer list | T | `opacity` |
| 627 | `.asset-card:focus-within .asset-actions` | — | Layer list | T | `opacity` |
| 628 | `.asset-card:hover .asset-badge` | — | Layer list | T | `opacity` |
| 628 | `.asset-card:focus-within .asset-badge` | — | Layer list | T | `opacity` |
| 629 | `.asset-add` | — | Layer list | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 629 | `.asset-reveal` | — | Layer list | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 629 | `.asset-locate` | — | Layer list | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 629 | `.asset-download` | — | Layer list | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 629 | `.asset-delete` | — | Layer list | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 630 | `.asset-add svg` | — | Layer list | T | `height`, `width` |
| 630 | `.asset-reveal svg` | — | Layer list | T | `height`, `width` |
| 630 | `.asset-locate svg` | — | Layer list | T | `height`, `width` |
| 630 | `.asset-download svg` | — | Layer list | T | `height`, `width` |
| 630 | `.asset-delete svg` | — | Layer list | T | `height`, `width` |
| 631 | `.asset-reveal:hover` | — | Layer list | T | `background`, `color` |
| 632 | `.asset-add:hover` | — | Layer list | T | `background`, `color` |
| 632 | `.asset-locate:hover` | — | Layer list | T | `background`, `color` |
| 632 | `.asset-download:hover` | — | Layer list | T | `background`, `color` |
| 633 | `.asset-delete:hover` | — | Layer list | T | `background`, `color` |
| 634 | `.asset-empty` | — | Layer list | T | `align-items`, `color`, `display`, `flex-direction`, `gap`, `grid-column`, `justify-content`, `min-height`, `padding`, `text-align` |
| 635 | `.asset-empty svg` | — | Layer list | T | `color`, `height`, `width` |
| 636 | `.asset-empty b` | — | Layer list | T | `color`, `font-size`, `font-weight` |
| 637 | `.asset-empty span` | — | Layer list | T | `font-size`, `line-height`, `max-width`, `text-wrap` |
| 640 | `.asset-preview.folder` | — | Layer list | T | `color` |
| 641 | `.asset-preview.folder svg` | — | Layer list | T | `height`, `width` |
| 642 | `.asset-card.is-folder.is-drop-target .asset-preview` | — | Layer list | T | `background`, `color` |
| 643 | `.asset-card.is-folder.is-drop-target .asset-preview::after` | — | Layer list | T | `box-shadow` |
| 644 | `.asset-rename` | — | Layer list | T | `background`, `border-radius`, `box-shadow`, `color`, `font`, `font-size`, `height`, `margin`, `min-width`, `padding` |
| 645 | `.asset-path` | — | Layer list | T | `align-items`, `color`, `display`, `flex`, `font-size`, `gap`, `min-width`, `padding` |
| 646 | `.asset-path-crumb` | — | Layer list | T | `border-radius`, `color`, `height`, `max-width`, `min-width`, `overflow`, `padding`, `text-overflow`, `white-space` |
| 647 | `.asset-path-root` | — | Layer list | T | `align-items`, `display`, `flex`, `gap` |
| 648 | `.asset-path-root svg` | — | Layer list | T | `flex`, `height`, `transform`, `width` |
| 649 | `.asset-path-crumb[aria-current="location"]` | — | Layer list | T | `color`, `font-weight` |
| 650 | `.asset-path-crumb:hover` | — | Layer list | T | `background`, `color` |
| 651 | `.asset-path-crumb.is-drop-target` | — | Layer list | T | `background`, `color` |
| 652 | `.asset-path-sep` | — | Layer list | T | `color`, `flex` |
| 654 | `.asset-preview.comp` | — | Layer list | T | `background`, `color` |
| 655 | `.asset-preview.comp .comp-frame` | — | Layer list | T | `aspect-ratio`, `background`, `border-radius`, `box-shadow`, `height`, `max-height`, `max-width`, `overflow`, `position` |
| 656 | `.asset-preview.comp .comp-frame img.comp-thumb` | — | Layer list | T | `height`, `inset`, `object-fit`, `position`, `width` |
| 657 | `.asset-preview.comp>svg` | — | Layer list | T | `color`, `mix-blend-mode`, `opacity`, `position`, `z-index` |
| 658 | `.asset-preview.comp .comp-open` | — | Layer list | T | `background`, `border-radius`, `bottom`, `height`, `position`, `right`, `width`, `z-index` |
| 659 | `.asset-add:disabled` | — | Layer list | T | `cursor`, `opacity` |
| 663 | `.code` | — | Code/shader editor | T | `background`, `caret-color`, `color`, `font-family`, `font-size`, `height`, `line-height`, `overflow`, `padding`, `resize`, `tab-size`, `white-space`, `width` |
| 665 | `.codebar` | — | Code/shader editor | T | `align-items`, `border-top`, `color`, `display`, `flex`, `font-size`, `gap`, `height`, `padding` |
| 667 | `.codebar .ok` | — | Code/shader editor | T | `color` |
| 667 | `.codebar .bad` | — | Code/shader editor | T | `color` |
| 674 | `.project-ghost` | — | Shell loading placeholders | T | `animation`, `opacity`, `pointer-events`, `position`, `z-index` |
| 675 | `.project-ghost.generic` | — | Shell loading placeholders | T | `display`, `gap`, `grid-template`, `padding` |
| 676 | `body:has(#projects-screen.on) :is(.project-ghost,#boot-ghost)` | — | Shell loading placeholders | T | `visibility` |
| 676 | `body:has(#settings-screen.on) :is(.project-ghost,#boot-ghost)` | — | Shell loading placeholders | T | `visibility` |
| 677 | `.pg-tile` | — | Shell loading placeholders | T | `background`, `border-radius`, `box-shadow`, `corner-shape`, `display`, `flex-direction`, `min-height`, `min-width`, `overflow`, `position` |
| 678 | `.project-ghost.generic .pg-tile` | — | Shell loading placeholders | T | `position` |
| 679 | `.pg-head` | — | Shell loading placeholders | T | `align-items`, `display`, `flex`, `height`, `padding` |
| 680 | `.pg-head::before` | — | Shell loading placeholders | T | `background`, `border-radius`, `content`, `height`, `width` |
| 681 | `.pg-rows` | — | Shell loading placeholders | T | `animation`, `display`, `flex-direction`, `gap`, `padding` |
| 682 | `.pg-row` | — | Shell loading placeholders | T | `background`, `border-radius`, `flex`, `height`, `width` |
| 683 | `.pg-row:nth-child(4n+2)` | — | Shell loading placeholders | T | `width` |
| 684 | `.pg-row:nth-child(4n+3)` | — | Shell loading placeholders | T | `width` |
| 685 | `.pg-row:nth-child(4n)` | — | Shell loading placeholders | T | `width` |
| 687 | `.pg-timeline .pg-rows` | — | Shell loading placeholders | T | `gap` |
| 688 | `.pg-timeline .pg-row` | — | Shell loading placeholders | T | `border-radius`, `height`, `margin-left`, `width` |
| 689 | `.pg-timeline .pg-row:nth-child(4n+2)` | — | Shell loading placeholders | T | `margin-left`, `width` |
| 690 | `.pg-timeline .pg-row:nth-child(4n+3)` | — | Shell loading placeholders | T | `margin-left`, `width` |
| 691 | `.pg-timeline .pg-row:nth-child(4n)` | — | Shell loading placeholders | T | `margin-left`, `width` |
| 692 | `.pg-stage` | — | Shell loading placeholders | T | `animation`, `background`, `border-radius`, `flex`, `margin`, `min-height` |
| 693 | `.project-loading-status` | — | Shell loading placeholders | T | `border`, `clip-path`, `height`, `margin`, `overflow`, `padding`, `position`, `white-space`, `width` |
| 698 | `.project-ghost` | @media (prefers-reduced-motion:reduce) | Shell loading placeholders | T | `animation` |
| 699 | `.pg-rows` | @media (prefers-reduced-motion:reduce) | Shell loading placeholders | T | `animation` |
| 699 | `.pg-stage` | @media (prefers-reduced-motion:reduce) | Shell loading placeholders | T | `animation` |
| 703 | `#scrim` | — | Modal, palette, overlays | T | `backdrop-filter`, `background`, `display`, `inset`, `position`, `z-index` |
| 706 | `.titlebar-overlay-drag` | — | Modal, palette, overlays | T | `-webkit-app-region`, `height`, `left`, `position`, `right`, `top`, `z-index` |
| 707 | `:root[data-theme="dark"] #scrim` | — | Modal, palette, overlays | T | `background` |
| 708 | `#scrim.on` | — | Modal, palette, overlays | T | `display` |
| 709 | `.modal` | — | Modal, palette, overlays | T | `background`, `border`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `overflow`, `position`, `z-index` |
| 711 | `.modal h3` | — | Modal, palette, overlays | T | `font-size`, `font-weight`, `letter-spacing`, `padding` |
| 712 | `.modal .mb` | — | Modal, palette, overlays | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `overflow`, `padding` |
| 713 | `.modal .mf` | — | Modal, palette, overlays | T | `border-top`, `display`, `gap`, `justify-content`, `padding` |
| 714 | `.modal .mf .btn` | — | Modal, palette, overlays | T | `white-space` |
| 718 | `.modal.is-measured` | — | Modal, palette, overlays | T | `transition` |
| 722 | `.modal` | — | Modal, palette, overlays | T | `animation`, `transform-origin` |
| 723 | `#scrim.on` | — | Modal, palette, overlays | T | `animation` |
| 726 | `.modal.is-resizing .mb` | — | Modal, palette, overlays | T | `overflow` |
| 728 | `.modal .mb` | — | Modal, palette, overlays | T | `scrollbar-width` |
| 729 | `.modal .mb::-webkit-scrollbar` | — | Modal, palette, overlays | T | `display` |
| 734 | `.modal .mb` | — | Modal, palette, overlays | T | `-webkit-mask-image`, `animation`, `animation-range`, `animation-timeline`, `mask-image` |
| 736 | `.modal.is-measured` | @media (prefers-reduced-motion:reduce) | Modal, palette, overlays | T | `transition` |
| 736 | `.modal` | @media (prefers-reduced-motion:reduce) | Modal, palette, overlays | T | `animation` |
| 736 | `#scrim.on` | @media (prefers-reduced-motion:reduce) | Modal, palette, overlays | T | `animation` |
| 738 | `.btn` | — | Modal, palette, overlays | T | `align-items`, `background`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `isolation`, `justify-content`, `padding`, `position`, `transition`, `white-space` |
| 739 | `.btn svg` | — | Modal, palette, overlays | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 740 | `.btn.ghost` | — | Modal, palette, overlays | T | `background`, `box-shadow`, `color`, `transition` |
| 741 | `.btn.ghost:hover` | — | Modal, palette, overlays | T | `background`, `color` |
| 742 | `.btn.pri` | — | Modal, palette, overlays | T | `background`, `box-shadow`, `color`, `transition` |
| 743 | `.btn.pri:hover` | — | Modal, palette, overlays | T | `background` |
| 744 | `.btn:not(.pri):not(.ghost):hover` | — | Modal, palette, overlays | T | `background` |
| 745 | `.btn:focus-visible` | — | Modal, palette, overlays | T | `outline`, `outline-offset` |
| 746 | `.btn.danger` | — | Modal, palette, overlays | T | `color` |
| 747 | `.field` | — | Modal, palette, overlays | T | `align-items`, `background`, `border-radius`, `box-shadow`, `display`, `font-size`, `height`, `padding`, `transition`, `width` |
| 749 | `.field:focus-within` | — | Modal, palette, overlays | T | `background`, `box-shadow` |
| 750 | `.field input` | — | Modal, palette, overlays | T | `flex` |
| 754 | `.settings-select` | — | Modal, palette, overlays | T | `background`, `border`, `border-radius`, `color`, `font`, `font-size`, `height`, `padding` |
| 754 | `.settings-input` | — | Modal, palette, overlays | T | `background`, `border`, `border-radius`, `color`, `font`, `font-size`, `height`, `padding` |
| 755 | `.settings-select` | — | Modal, palette, overlays | T | `min-width`, `padding-right` |
| 756 | `.settings-input` | — | Modal, palette, overlays | T | `font-variant-numeric`, `text-align`, `width` |
| 757 | `.settings-input.is-wide` | — | Modal, palette, overlays | T | `text-align`, `width` |
| 758 | `.settings-select:focus-visible` | — | Modal, palette, overlays | T | `box-shadow`, `outline` |
| 758 | `.settings-input:focus-visible` | — | Modal, palette, overlays | T | `box-shadow`, `outline` |
| 759 | `.settings-field-pair` | — | Modal, palette, overlays | T | `align-items`, `color`, `display`, `gap` |
| 760 | `.settings-check` | — | Modal, palette, overlays | T | `accent-color`, `flex`, `height`, `width` |
| 761 | `.settings-swatch` | — | Modal, palette, overlays | T | `background`, `border`, `border-radius`, `height`, `padding`, `width` |
| 762 | `.settings-row>.color-field` | — | Modal, palette, overlays | T | `flex`, `width` |
| 763 | `.settings-row.is-dim` | — | Modal, palette, overlays | T | `opacity`, `transition` |
| 764 | `.settings-row[hidden]` | — | Modal, palette, overlays | T | `display` |
| 765 | `.new-project-form` | — | Modal, palette, overlays | T | `display`, `flex-direction`, `gap` |
| 766 | `.new-project-field` | — | Modal, palette, overlays | T | `align-items`, `display`, `gap`, `justify-content`, `min-height` |
| 767 | `.new-project-field>span` | — | Modal, palette, overlays | T | `color`, `font-size` |
| 769 | `#palette` | — | Modal, palette, overlays | T | `--pal-gutter`, `--pal-r`, `border-radius`, `left`, `max-height`, `padding`, `top`, `transform`, `width` |
| 771 | `#palette` | — | Modal, palette, overlays | T | `animation`, `background` |
| 772 | `body:has(#palette) #scrim.on` | — | Modal, palette, overlays | T | `animation` |
| 773 | `#palette .field` | — | Modal, palette, overlays | T | `background`, `border-radius`, `box-shadow`, `flex`, `font-size`, `gap`, `height`, `padding` |
| 774 | `#palette .field:focus-within` | — | Modal, palette, overlays | T | `background`, `box-shadow` |
| 775 | `#palette .field .psearch` | — | Modal, palette, overlays | T | `color`, `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-width`, `width` |
| 776 | `.plist` | — | Modal, palette, overlays | T | `scrollbar-width` |
| 776 | `.plist::-webkit-scrollbar` | — | Modal, palette, overlays | T | `display` |
| 779 | `.plist` | — | Modal, palette, overlays | T | `corner-shape` |
| 779 | `.pitem` | — | Modal, palette, overlays | T | `corner-shape` |
| 779 | `.pglider` | — | Modal, palette, overlays | T | `corner-shape` |
| 780 | `.plist` | — | Modal, palette, overlays | T | `background`, `border-radius`, `box-shadow`, `min-height`, `overflow`, `padding`, `position` |
| 781 | `.pgroup` | — | Modal, palette, overlays | T | `color`, `font-size`, `padding` |
| 782 | `.pgroup:first-child` | — | Modal, palette, overlays | T | `padding-top` |
| 783 | `.pitem` | — | Modal, palette, overlays | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `gap`, `height`, `padding` |
| 784 | `.pitem` | — | Modal, palette, overlays | T | `position`, `z-index` |
| 785 | `.pglider` | — | Modal, palette, overlays | T | `background`, `border-radius`, `box-shadow`, `height`, `left`, `opacity`, `pointer-events`, `position`, `right`, `top`, `transition` |
| 786 | `.pglider.on` | — | Modal, palette, overlays | T | `opacity` |
| 787 | `.pitem .kb` | — | Modal, palette, overlays | T | `color`, `font-family`, `font-size`, `margin-left` |
| 788 | `.plist .empty` | — | Modal, palette, overlays | T | `padding` |
| 791 | `#status` | — | Shared controls and status | T | `align-items`, `border-top`, `color`, `display`, `flex`, `font-size`, `font-variant-numeric`, `gap`, `height`, `padding` |
| 793 | `#status .sp` | — | Shared controls and status | T | `flex` |
| 794 | `#status b` | — | Shared controls and status | T | `color`, `font-weight` |
| 795 | `#status .live` | — | Shared controls and status | T | `color` |
| 796 | `.toastwrap` | — | Shared controls and status | T | `align-items`, `display`, `flex-direction`, `gap`, `left`, `position`, `top`, `transform`, `z-index` |
| 797 | `.toast` | — | Shared controls and status | T | `align-items`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `font-weight`, `gap`, `max-width`, `padding` |
| 799 | `.toast-icon` | — | Shared controls and status | T | `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 800 | `.toast[data-toast-error] .toast-icon` | — | Shared controls and status | T | `color` |
| 801 | `.toast[data-toast-kind="alert"] .toast-icon` | — | Shared controls and status | T | `color` |
| 802 | `.toast-icon .pm-icon` | — | Shared controls and status | T | `display`, `height`, `width` |
| 803 | `.toast>span:not(.toast-icon)` | — | Shared controls and status | T | `min-width`, `white-space` |
| 804 | `.toast>button` | — | Shared controls and status | T | `border-radius`, `color`, `display`, `flex`, `font-size`, `height`, `line-height`, `margin-left`, `place-items`, `width` |
| 805 | `.toast>button:hover` | — | Shared controls and status | T | `background`, `color` |
| 806 | `.toast>button.toast-action` | — | Shared controls and status | T | `background`, `border-radius`, `color`, `font-size`, `font-weight`, `height`, `margin-left`, `padding`, `width` |
| 807 | `.toast>button.toast-action:hover` | — | Shared controls and status | T | `background` |
| 808 | `.toast-corner` | — | Shared controls and status | T | `align-items`, `display`, `flex-direction`, `gap`, `pointer-events`, `position`, `right`, `top`, `z-index` |
| 809 | `.toast-corner>.toast` | — | Shared controls and status | T | `pointer-events` |
| 810 | `.drop` | — | Shared controls and status | T | `background`, `border`, `border-radius`, `box-shadow`, `max-height`, `min-width`, `overflow`, `padding`, `position`, `z-index` |
| 812 | `.drop .di` | — | Shared controls and status | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `gap`, `height`, `padding`, `position`, `text-align`, `transition`, `white-space`, `width` |
| 813 | `.drop:focus` | — | Shared controls and status | T | `outline` |
| 814 | `.drop .di:hover` | — | Shared controls and status | T | `background`, `outline` |
| 814 | `.drop .di:focus-visible` | — | Shared controls and status | T | `background`, `outline` |
| 815 | `.drop .di.on::before` | — | Shared controls and status | T | `border-bottom`, `border-left`, `content`, `height`, `left`, `position`, `top`, `transform`, `width` |
| 816 | `.drop .di .kb` | — | Shared controls and status | T | `color`, `font-family`, `font-size`, `margin-left`, `padding-left` |
| 817 | `.drop .di.disabled` | — | Shared controls and status | T | `opacity` |
| 818 | `.drop .di.disabled:hover` | — | Shared controls and status | T | `background` |
| 820 | `.drop:has(.di[data-blink]) .di` | — | Shared controls and status | T | `background`, `transition` |
| 821 | `.drop .di[data-blink=on]` | — | Shared controls and status | T | `background` |
| 822 | `.drop .sep` | — | Shared controls and status | T | `background`, `height`, `margin` |
| 823 | `.drop .hd` | — | Shared controls and status | T | `color`, `font-size`, `font-weight`, `padding` |
| 825 | `.font-select` | — | Shared controls and status | T | `max-width`, `min-width`, `overflow`, `text-align`, `text-overflow`, `white-space` |
| 826 | `.font-drop` | — | Shared controls and status | T | `display`, `flex-direction`, `overflow`, `padding` |
| 827 | `.font-search` | — | Shared controls and status | T | `background`, `border-radius`, `box-shadow`, `color`, `flex`, `font-family`, `font-size`, `height`, `padding`, `width` |
| 828 | `.font-search:focus` | — | Shared controls and status | T | `box-shadow`, `outline` |
| 829 | `.font-results` | — | Shared controls and status | T | `margin-top`, `min-height`, `overflow` |
| 830 | `.font-results .font-item` | — | Shared controls and status | T | `cursor`, `font-size`, `text-align`, `width` |
| 831 | `.font-empty` | — | Shared controls and status | T | `color`, `font-size`, `line-height`, `padding` |
| 834 | `#spatial-assistant` | — | Agent rail/composer/content | T | `color`, `cursor`, `inset`, `isolation`, `pointer-events`, `position`, `z-index` |
| 835 | `#spatial-assistant.conversation-mode` | — | Agent rail/composer/content | T | `cursor`, `pointer-events` |
| 836 | `#spatial-assistant.conversation-mode :is(.spatial-ripple-host,.spatial-wash,.spatial-ink,.spatial-outline,.spatial-hint)` | — | Agent rail/composer/content | T | `display` |
| 837 | `#spatial-assistant.conversation-mode .spatial-compose` | — | Agent rail/composer/content | T | `pointer-events` |
| 838 | `#spatial-assistant .spatial-ripple-host` | — | Agent rail/composer/content | T | `height`, `inset`, `position`, `width` |
| 838 | `#spatial-assistant .spatial-wash` | — | Agent rail/composer/content | T | `height`, `inset`, `position`, `width` |
| 838 | `#spatial-assistant .spatial-ink` | — | Agent rail/composer/content | T | `height`, `inset`, `position`, `width` |
| 839 | `#spatial-assistant .spatial-ripple-host` | — | Agent rail/composer/content | T | `pointer-events`, `z-index` |
| 840 | `.spatial-wash` | — | Agent rail/composer/content | T | `background`, `pointer-events`, `transition`, `z-index` |
| 841 | `.spatial-ink` | — | Agent rail/composer/content | T | `overflow`, `pointer-events`, `z-index` |
| 842 | `.spatial-ink path` | — | Agent rail/composer/content | T | `fill`, `filter`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 843 | `.spatial-ink .spatial-shade` | — | Agent rail/composer/content | T | `fill`, `filter`, `stroke` |
| 844 | `#spatial-assistant.target-selected .spatial-wash` | — | Agent rail/composer/content | T | `background` |
| 845 | `.spatial-hint` | — | Agent rail/composer/content | T | `align-items`, `backdrop-filter`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `font-weight`, `height`, `left`, `letter-spacing`, `padding`, `pointer-events`, `position`, `top`, `white-space`, `will-change`, `z-index` |
| 846 | `.spatial-compose` | — | Agent rail/composer/content | T | `--spatial-control-inset`, `--spatial-frame-inset`, `--spatial-input-radius`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `cursor`, `padding`, `position`, `width`, `z-index` |
| 849 | `.spatial-compose.full` | — | Agent rail/composer/content | T | `--bg-field`, `--bg-float`, `--bg-panel`, `--ink-rgb`, `--spatial-frame-inset`, `--tx`, `--tx-2`, `--tx-3`, `--tx-4`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `padding` |
| 850 | `.spatial-compose.full .spatial-input-row` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-radius`, `min-height`, `padding` |
| 851 | `.spatial-compose.full .spatial-input-row:focus-within` | — | Agent rail/composer/content | T | `background`, `box-shadow` |
| 852 | `.spatial-compose.full textarea` | — | Agent rail/composer/content | T | `font-size`, `height`, `line-height`, `min-height`, `padding` |
| 853 | `.spatial-compose.full .spatial-input-row>.spatial-action.spatial-action.pri` | — | Agent rail/composer/content | T | `border-radius`, `height`, `width` |
| 854 | `.spatial-compose.conversation` | — | Agent rail/composer/content | T | `border`, `border-radius`, `overflow`, `padding`, `width` |
| 855 | `.spatial-target` | — | Agent rail/composer/content | T | `color`, `cursor`, `font-family`, `font-size`, `font-weight`, `min-width`, `overflow`, `padding`, `text-overflow`, `user-select`, `white-space` |
| 856 | `.spatial-target:active` | — | Agent rail/composer/content | T | `cursor` |
| 856 | `.spatial-target:focus-visible` | — | Agent rail/composer/content | T | `outline` |
| 857 | `.spatial-compose.full .spatial-target` | — | Agent rail/composer/content | T | `display` |
| 858 | `.spatial-input-row` | — | Agent rail/composer/content | T | `align-items`, `background`, `border`, `border-radius`, `display`, `gap`, `min-height`, `padding`, `transition` |
| 859 | `.spatial-input-row:focus-within` | — | Agent rail/composer/content | T | `background`, `box-shadow` |
| 860 | `.spatial-context-label` | — | Agent rail/composer/content | T | `align-self`, `color`, `flex`, `font-family`, `font-size`, `font-weight`, `line-height`, `white-space` |
| 861 | `.spatial-compose textarea` | — | Agent rail/composer/content | T | `align-self`, `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `height`, `line-height`, `max-height`, `min-height`, `min-width`, `overflow-y`, `padding`, `resize`, `width` |
| 862 | `.spatial-compose textarea::placeholder` | — | Agent rail/composer/content | T | `color` |
| 863 | `.spatial-compose textarea:focus` | — | Agent rail/composer/content | T | `outline` |
| 864 | `.spatial-actions` | — | Agent rail/composer/content | T | `align-items`, `display`, `gap`, `min-height`, `padding-top` |
| 865 | `.spatial-actions .spatial-status` | — | Agent rail/composer/content | T | `color`, `display`, `font-size`, `font-variant-numeric`, `margin-right` |
| 866 | `.spatial-actions .spatial-status.on` | — | Agent rail/composer/content | T | `display` |
| 867 | `.spatial-compose.compact .spatial-actions .spatial-action` | — | Agent rail/composer/content | T | `display` |
| 868 | `.spatial-compose.full .spatial-actions` | — | Agent rail/composer/content | T | `display` |
| 869 | `#spatial-assistant:not(.target-selected) .spatial-focus-host` | — | Agent rail/composer/content | T | `display` |
| 870 | `.spatial-attach` | — | Agent rail/composer/content | T | `align-self`, `background`, `border`, `border-radius`, `color`, `display`, `flex`, `height`, `padding`, `place-items`, `width` |
| 871 | `.spatial-attach svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 872 | `.spatial-attach:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 873 | `.spatial-attach:focus-visible` | — | Agent rail/composer/content | T | `outline`, `outline-offset` |
| 874 | `.spatial-compose.full:has(.spatial-attach) .spatial-input-row` | — | Agent rail/composer/content | T | `padding-left` |
| 875 | `.spatial-attachment-rail` | — | Agent rail/composer/content | T | `display`, `flex-wrap`, `gap`, `padding` |
| 876 | `.spatial-attachment-rail[hidden]` | — | Agent rail/composer/content | T | `display` |
| 877 | `.spatial-compose.full.has-attachments` | — | Agent rail/composer/content | T | `border-radius` |
| 878 | `.spatial-action` | — | Agent rail/composer/content | T | `background`, `border-radius`, `color`, `height`, `padding`, `transition` |
| 879 | `.spatial-action:hover` | — | Agent rail/composer/content | T | `background` |
| 880 | `.spatial-action:active` | — | Agent rail/composer/content | T | `transform` |
| 881 | `.spatial-input-row>.spatial-action.spatial-action.pri` | — | Agent rail/composer/content | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `display`, `flex`, `font-size`, `font-weight`, `height`, `line-height`, `outline`, `padding`, `place-items`, `width` |
| 882 | `.spatial-input-row>.spatial-action.pri svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 882 | `.agent-send svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 883 | `.spatial-send svg[data-icon="return"]` | — | Agent rail/composer/content | T | `transform` |
| 883 | `.agent-send svg[data-icon="return"]` | — | Agent rail/composer/content | T | `transform` |
| 884 | `.spatial-input-row>.spatial-action.pri:hover` | — | Agent rail/composer/content | T | `background` |
| 885 | `.spatial-input-row>.spatial-action.pri:active` | — | Agent rail/composer/content | T | `transform` |
| 887 | `.spatial-actions>.spatial-action.pri` | — | Agent rail/composer/content | T | `background`, `color`, `font-weight` |
| 888 | `.spatial-action:disabled` | — | Agent rail/composer/content | T | `opacity`, `pointer-events` |
| 889 | `.spatial-conversation-head` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-bottom`, `display`, `gap`, `height`, `padding` |
| 890 | `.spatial-conversation-head .spatial-target` | — | Agent rail/composer/content | T | `flex`, `padding` |
| 891 | `.spatial-close` | — | Agent rail/composer/content | T | `border-radius`, `color`, `display`, `font-size`, `height`, `line-height`, `place-items`, `width` |
| 892 | `.spatial-close:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 893 | `.spatial-conversation-log` | — | Agent rail/composer/content | T | `background`, `display`, `flex-direction`, `gap`, `max-height`, `min-height`, `overflow`, `padding` |
| 894 | `.spatial-message` | — | Agent rail/composer/content | T | `border-radius`, `font-size`, `line-height`, `max-width`, `padding`, `user-select`, `white-space` |
| 895 | `.spatial-message.user` | — | Agent rail/composer/content | T | `align-self`, `background`, `border-bottom-right-radius`, `color` |
| 896 | `.spatial-message.assistant` | — | Agent rail/composer/content | T | `align-self`, `background`, `border-bottom-left-radius`, `color` |
| 897 | `.spatial-message.user.is-entering` | — | Agent rail/composer/content | T | `animation`, `transform-origin` |
| 898 | `.spatial-message.pending.is-entering` | — | Agent rail/composer/content | T | `animation` |
| 901 | `.spatial-message.pending` | — | Agent rail/composer/content | T | `align-items`, `color`, `display`, `gap` |
| 902 | `.spatial-message.pending i` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `height`, `width` |
| 904 | `.agent-thinking-copy` | — | Agent rail/composer/content | T | `min-width`, `white-space` |
| 905 | `.agent-thinking-word` | — | Agent rail/composer/content | T | `animation`, `animation-delay`, `display`, `opacity`, `white-space` |
| 907 | `.spatial-followup-row` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-top`, `display`, `gap`, `padding` |
| 908 | `.spatial-compose textarea.spatial-followup` | — | Agent rail/composer/content | T | `background`, `border`, `border-radius`, `height`, `min-height`, `padding` |
| 909 | `.spatial-compose textarea.spatial-followup:focus` | — | Agent rail/composer/content | T | `border-color` |
| 910 | `.spatial-followup-row>.spatial-action.pri` | — | Agent rail/composer/content | T | `background`, `color`, `font-size`, `height`, `padding`, `width` |
| 911 | `.spatial-proposal` | — | Agent rail/composer/content | T | `background`, `border`, `border-radius`, `padding`, `width` |
| 912 | `.spatial-proposal-kicker` | — | Agent rail/composer/content | T | `color`, `font-size`, `font-weight`, `margin-bottom` |
| 913 | `.spatial-proposal h3` | — | Agent rail/composer/content | T | `font-size`, `margin` |
| 913 | `.spatial-proposal p` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height`, `margin` |
| 914 | `.spatial-proposal-actions` | — | Agent rail/composer/content | T | `display`, `gap`, `justify-content`, `padding-top` |
| 915 | `.spatial-preview` | — | Agent rail/composer/content | T | `max-height`, `overflow`, `padding` |
| 916 | `.spatial-preview h3` | — | Agent rail/composer/content | T | `font-size`, `margin` |
| 916 | `.spatial-preview p` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height`, `margin` |
| 917 | `.spatial-preview-control` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-radius`, `display`, `font-size`, `gap`, `margin-top`, `min-height`, `padding` |
| 918 | `.spatial-preview-control>svg` | — | Agent rail/composer/content | T | `color`, `flex`, `height`, `width` |
| 919 | `.spatial-preview-control>span:last-child:not(:first-child)` | — | Agent rail/composer/content | T | `color`, `font-size`, `margin-left`, `max-width`, `overflow`, `text-overflow`, `white-space` |
| 920 | `.spatial-preview-control.panel-action>span:last-child` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height`, `margin-left`, `max-width`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 921 | `.spatial-preview-control i` | — | Agent rail/composer/content | T | `background`, `border-radius`, `height`, `margin-left`, `width` |
| 922 | `.spatial-preview-control i.curve` | — | Agent rail/composer/content | T | `background`, `border-bottom`, `border-left`, `border-radius`, `height`, `position` |
| 923 | `.spatial-preview-control i.curve::after` | — | Agent rail/composer/content | T | `border-radius`, `border-top`, `content`, `inset`, `position`, `transform` |
| 924 | `.spatial-frame-grid` | — | Agent rail/composer/content | T | `display`, `gap`, `grid-template-columns`, `margin-top` |
| 925 | `.spatial-frame-grid figure` | — | Agent rail/composer/content | T | `background`, `border`, `border-radius`, `margin`, `overflow` |
| 926 | `.spatial-frame-grid figure>button` | — | Agent rail/composer/content | T | `background`, `border`, `cursor`, `display`, `width` |
| 927 | `.spatial-frame-grid img` | — | Agent rail/composer/content | T | `aspect-ratio`, `background`, `display`, `object-fit`, `width` |
| 928 | `.spatial-frame-grid figcaption` | — | Agent rail/composer/content | T | `color`, `font-size`, `padding` |
| 929 | `.spatial-preview-control.spatial-review-warning` | — | Agent rail/composer/content | T | `background`, `border-radius`, `color`, `padding` |
| 930 | `.spatial-outline` | — | Agent rail/composer/content | T | `border`, `border-radius`, `box-shadow`, `pointer-events`, `position`, `will-change`, `z-index` |
| 931 | `.spatial-outline.confirmed` | — | Agent rail/composer/content | T | `animation` |
| 938 | `#panel-agent` | — | Agent rail/composer/content | T | `--shadow-float`, `--shadow-lift`, `box-shadow` |
| 939 | `#panel-agent>header::after` | — | Agent rail/composer/content | T | `display` |
| 940 | `.agent-shell .spatial-frame-grid figure` | — | Agent rail/composer/content | T | `border` |
| 941 | `#panel-agent>.body` | — | Agent rail/composer/content | T | `display`, `min-height`, `overflow` |
| 942 | `.agent-panel-body` | — | Agent rail/composer/content | T | `container-type` |
| 943 | `.agent-shell` | — | Agent rail/composer/content | T | `background`, `display`, `flex`, `flex-direction`, `min-height`, `min-width`, `position` |
| 944 | `.agent-scroll` | — | Agent rail/composer/content | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `overflow-y`, `padding`, `scrollbar-width` |
| 945 | `.agent-scroll::-webkit-scrollbar` | — | Agent rail/composer/content | T | `display` |
| 952 | `.agent-scroll` | — | Agent rail/composer/content | T | `mask-image`, `transition` |
| 953 | `.agent-scroll[data-overflow-bottom="1"]` | — | Agent rail/composer/content | T | `--agent-fade-bottom` |
| 954 | `.agent-scroll` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `transition` |
| 955 | `.agent-scroll` | — | Agent rail/composer/content | T | `-webkit-user-select`, `user-select` |
| 955 | `.agent-scroll *` | — | Agent rail/composer/content | T | `-webkit-user-select`, `user-select` |
| 958 | `.agent-shell` | — | Agent rail/composer/content | T | `overflow-wrap` |
| 959 | `.agent-scroll` | — | Agent rail/composer/content | T | `min-width` |
| 960 | `.agent-scroll>*` | — | Agent rail/composer/content | T | `max-width`, `min-width` |
| 960 | `.agent-footer` | — | Agent rail/composer/content | T | `max-width`, `min-width` |
| 960 | `.agent-composer` | — | Agent rail/composer/content | T | `max-width`, `min-width` |
| 960 | `.agent-input-row` | — | Agent rail/composer/content | T | `max-width`, `min-width` |
| 961 | `.agent-shell :is(pre,code,textarea)` | — | Agent rail/composer/content | T | `max-width`, `min-width`, `overflow-wrap`, `white-space` |
| 962 | `.agent-shell :is(.agent-word,.agent-thinking-word)` | — | Agent rail/composer/content | T | `max-width`, `overflow-wrap` |
| 963 | `.agent-shell .agent-option-bar` | — | Agent rail/composer/content | T | `flex-wrap` |
| 964 | `.agent-shell .agent-modelbar` | — | Agent rail/composer/content | T | `flex-wrap`, `max-width`, `overflow` |
| 965 | `.agent-shell .agent-modelbar .pm-select` | — | Agent rail/composer/content | T | `min-width` |
| 966 | `.agent-shell .agent-scroll` | — | Agent rail/composer/content | T | `overflow-x` |
| 967 | `.agent-composer .agent-input-row` | @container (max-width:280px) | Agent rail/composer/content | T | `gap` |
| 968 | `.agent-shell .agent-card-actions` | — | Agent rail/composer/content | T | `flex-wrap` |
| 969 | `.agent-shell .agent-prompt-glow` | — | Agent rail/composer/content | T | `left`, `right` |
| 971 | `.agent-msg` | — | Agent rail/composer/content | T | `flex`, `min-width` |
| 972 | `.agent-msg.user` | — | Agent rail/composer/content | T | `align-items`, `display`, `flex-direction`, `gap` |
| 973 | `.agent-bubble` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height`, `white-space` |
| 975 | `.agent-prompt` | — | Agent rail/composer/content | T | `background`, `border-radius`, `isolation`, `max-width`, `padding`, `position`, `width` |
| 977 | `.agent-bubble` | — | Agent rail/composer/content | T | `max-width` |
| 981 | `.agent-prompt-glow` | — | Agent rail/composer/content | T | `animation`, `inset`, `pointer-events`, `position`, `z-index` |
| 982 | `:root[data-theme="dark"] .agent-prompt-glow` | — | Agent rail/composer/content | T | `mix-blend-mode` |
| 983 | `:root:not([data-theme="dark"]) .agent-prompt-glow` | — | Agent rail/composer/content | T | `mix-blend-mode` |
| 986 | `.agent-prompt-glow::before` | — | Agent rail/composer/content | T | `animation`, `border-radius`, `box-shadow`, `content`, `inset`, `position` |
| 987 | `.agent-prompt-glow[data-glow-renderer="motion-gpu"]::before` | — | Agent rail/composer/content | T | `display` |
| 989 | `.agent-prompt .agent-bubble` | — | Agent rail/composer/content | T | `position`, `z-index` |
| 993 | `.agent-prompt-glow` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 994 | `.agent-prompt-glow::before` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation`, `opacity` |
| 997 | `.agent-msg.assistant` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height` |
| 998 | `.agent-msg.assistant p` | — | Agent rail/composer/content | T | `margin`, `white-space` |
| 999 | `.agent-word` | — | Agent rail/composer/content | T | `animation`, `animation-delay`, `display`, `opacity`, `white-space` |
| 1000 | `.agent-msg-files` | — | Agent rail/composer/content | T | `display`, `flex-wrap`, `gap`, `margin-top` |
| 1001 | `.agent-msg.user .agent-msg-files` | — | Agent rail/composer/content | T | `align-self`, `justify-content`, `max-width` |
| 1003 | `.agent-timeline` | — | Agent rail/composer/content | T | `display`, `flex-direction`, `gap`, `padding-left` |
| 1004 | `.agent-step` | — | Agent rail/composer/content | T | `align-items`, `color`, `display`, `font-size`, `font-weight`, `gap`, `line-height` |
| 1005 | `.agent-step>i` | — | Agent rail/composer/content | T | `border`, `border-radius`, `flex`, `height`, `width` |
| 1006 | `.agent-step.active` | — | Agent rail/composer/content | T | `color` |
| 1007 | `.agent-step.active>i` | — | Agent rail/composer/content | T | `animation`, `border`, `border-top-color` |
| 1008 | `.agent-step.complete>i` | — | Agent rail/composer/content | T | `background`, `border-color` |
| 1009 | `.agent-step.error` | — | Agent rail/composer/content | T | `color` |
| 1009 | `.agent-step.error>i` | — | Agent rail/composer/content | T | `background`, `border-color` |
| 1010 | `.agent-step>em` | — | Agent rail/composer/content | T | `font-size`, `font-style`, `margin-left`, `opacity` |
| 1011 | `.agent-activity` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height` |
| 1012 | `.agent-activity.is-entering` | — | Agent rail/composer/content | T | `animation` |
| 1016 | `.agent-thinking-copy` | — | Agent rail/composer/content | T | `animation`, `color`, `display`, `min-width`, `white-space` |
| 1023 | `.agent-trace` | — | Agent rail/composer/content | T | `display`, `flex-direction`, `gap`, `min-width`, `padding-left` |
| 1024 | `.agent-trace p` | — | Agent rail/composer/content | T | `margin` |
| 1025 | `.agent-trace.is-live>*` | — | Agent rail/composer/content | T | `animation` |
| 1028 | `.agent-trace-prose` | — | Agent rail/composer/content | T | `color`, `display`, `flex-direction`, `font-size`, `gap`, `line-height`, `min-width` |
| 1028 | `.agent-reply` | — | Agent rail/composer/content | T | `color`, `display`, `flex-direction`, `font-size`, `gap`, `line-height`, `min-width` |
| 1029 | `.agent-trace-prose` | — | Agent rail/composer/content | T | `padding` |
| 1031 | `p.is-streaming::after` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `content`, `display`, `height`, `margin-left`, `vertical-align`, `width` |
| 1031 | `.agent-md-li.is-streaming p::after` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `content`, `display`, `height`, `margin-left`, `vertical-align`, `width` |
| 1031 | `.agent-md-quote.is-streaming p::after` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `content`, `display`, `height`, `margin-left`, `vertical-align`, `width` |
| 1035 | `.agent-shell :is(.agent-md-p,.agent-md-li p,.agent-md-quote p)` | — | Agent rail/composer/content | T | `padding-inline-end` |
| 1037 | `.agent-trace-loading` | — | Agent rail/composer/content | T | `align-items`, `display`, `gap`, `min-width`, `padding` |
| 1038 | `.agent-trace-loading .agent-trace-thought` | — | Agent rail/composer/content | T | `color`, `font-size`, `font-weight`, `min-width` |
| 1039 | `.agent-pixel-loader` | — | Agent rail/composer/content | T | `display`, `flex`, `gap`, `grid-template-columns` |
| 1040 | `.agent-pixel-loader>i` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `display`, `height`, `opacity`, `width` |
| 1042 | `.agent-trace-code` | — | Agent rail/composer/content | T | `background`, `border-radius`, `box-shadow`, `font-family`, `font-size`, `padding` |
| 1046 | `.shimmer-text .agent-trace-code` | — | Agent rail/composer/content | T | `-webkit-background-clip`, `background-clip`, `color` |
| 1047 | `.agent-trace.is-archived` | — | Agent rail/composer/content | T | `margin` |
| 1048 | `.agent-msg.assistant.is-error p` | — | Agent rail/composer/content | T | `color` |
| 1050 | `.shimmer-text` | — | Agent rail/composer/content | T | `-webkit-background-clip`, `animation`, `background`, `background-clip`, `background-size`, `color` |
| 1054 | `.shimmer-text::selection` | — | Agent rail/composer/content | T | `-webkit-text-fill-color`, `background`, `color` |
| 1056 | `.panel-library-glider` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `transition` |
| 1056 | `.pglider` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `transition` |
| 1058 | `.shimmer-text` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `-webkit-background-clip`, `animation`, `background`, `background-clip`, `color` |
| 1059 | `.agent-trace.is-live>*` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 1060 | `.agent-pixel-loader>i` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 1061 | `p.is-streaming::after` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 1061 | `.agent-md-li.is-streaming p::after` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 1061 | `.agent-md-quote.is-streaming p::after` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation` |
| 1062 | `.agent-word` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation`, `filter`, `opacity`, `transform` |
| 1062 | `.agent-thinking-word` | @media (prefers-reduced-motion:reduce) | Agent rail/composer/content | T | `animation`, `filter`, `opacity`, `transform` |
| 1065 | `.agent-welcome` | — | Agent rail/composer/content | T | `color`, `flex`, `margin`, `max-width`, `padding`, `text-align`, `width` |
| 1066 | `.agent-welcome-heading` | — | Agent rail/composer/content | T | `align-items`, `display`, `gap`, `margin-bottom`, `min-width` |
| 1067 | `.agent-welcome-heading>div` | — | Agent rail/composer/content | T | `min-width` |
| 1068 | `.agent-welcome b` | — | Agent rail/composer/content | T | `color`, `display`, `font-size`, `font-weight`, `margin-bottom` |
| 1069 | `.agent-welcome-heading span` | — | Agent rail/composer/content | T | `color`, `display`, `font-size`, `line-height` |
| 1070 | `.agent-suggestions` | — | Agent rail/composer/content | T | `display`, `flex-direction`, `gap` |
| 1071 | `.agent-suggestions button` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `gap`, `justify-content`, `line-height`, `min-height`, `min-width`, `padding`, `text-align`, `transition` |
| 1072 | `.agent-suggestions button span` | — | Agent rail/composer/content | T | `min-width` |
| 1073 | `.agent-suggestions button svg` | — | Agent rail/composer/content | T | `fill`, `flex`, `height`, `opacity`, `stroke`, `stroke-width`, `width` |
| 1074 | `.agent-suggestions button:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 1075 | `.agent-suggestions button:focus-visible` | — | Agent rail/composer/content | T | `outline`, `outline-offset` |
| 1077 | `.agent-footer` | — | Agent rail/composer/content | T | `display`, `flex`, `flex-direction`, `gap`, `max-height`, `min-height`, `padding`, `position` |
| 1078 | `.agent-jump` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `box-shadow`, `color`, `display`, `height`, `left`, `place-items`, `position`, `top`, `transform`, `width`, `z-index` |
| 1079 | `.agent-jump:hover` | — | Agent rail/composer/content | T | `color` |
| 1079 | `.agent-jump:active` | — | Agent rail/composer/content | T | `transform` |
| 1080 | `.agent-jump svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 1084 | `.agent-card` | — | Agent rail/composer/content | T | `animation`, `background`, `border-radius`, `box-shadow`, `min-height`, `overflow-y`, `padding` |
| 1086 | `.agent-card-kicker` | — | Agent rail/composer/content | T | `color`, `font-size`, `font-weight`, `margin-bottom` |
| 1087 | `.agent-card h3` | — | Agent rail/composer/content | T | `font-size`, `font-weight`, `margin` |
| 1088 | `.agent-card p` | — | Agent rail/composer/content | T | `color`, `font-size`, `line-height`, `margin` |
| 1089 | `.agent-card-actions` | — | Agent rail/composer/content | T | `display`, `gap`, `justify-content`, `padding-top` |
| 1090 | `.agent-btn` | — | Agent rail/composer/content | T | `border-radius`, `color`, `font-size`, `height`, `padding`, `transition` |
| 1091 | `.agent-btn:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 1092 | `.agent-btn:active` | — | Agent rail/composer/content | T | `transform` |
| 1093 | `.agent-btn.pri` | — | Agent rail/composer/content | T | `background`, `color` |
| 1094 | `.agent-btn.pri:hover` | — | Agent rail/composer/content | T | `background`, `opacity` |
| 1103 | `.agent-composer` | — | Agent rail/composer/content | T | `--composer-inset`, `--composer-pad`, `--composer-tray`, `background`, `border`, `border-radius`, `box-shadow`, `display`, `flex`, `flex-direction`, `gap`, `padding` |
| 1104 | `.agent-composer-field` | — | Agent rail/composer/content | T | `background`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `min-width`, `transition` |
| 1105 | `.agent-composer-field:focus-within` | — | Agent rail/composer/content | T | `box-shadow` |
| 1106 | `.agent-composer.is-dropping .agent-composer-field` | — | Agent rail/composer/content | T | `box-shadow` |
| 1109 | `.agent-input-row` | — | Agent rail/composer/content | T | `align-items`, `display`, `padding` |
| 1110 | `.agent-composer textarea` | — | Agent rail/composer/content | T | `background`, `border`, `color`, `display`, `flex`, `font`, `max-height`, `min-height`, `min-width`, `outline`, `padding`, `resize`, `scrollbar-width` |
| 1110 | `.agent-inline-prompt` | — | Agent rail/composer/content | T | `background`, `border`, `color`, `display`, `flex`, `font`, `max-height`, `min-height`, `min-width`, `outline`, `padding`, `resize`, `scrollbar-width` |
| 1111 | `.agent-composer-actions` | — | Agent rail/composer/content | T | `align-items`, `display`, `gap`, `min-width`, `padding` |
| 1112 | `.agent-composer-actions .sp` | — | Agent rail/composer/content | T | `flex` |
| 1113 | `.agent-composer-foot` | — | Agent rail/composer/content | T | `align-items`, `display`, `min-width`, `padding` |
| 1114 | `.agent-composer-foot .agent-option-bar` | — | Agent rail/composer/content | T | `flex`, `min-width`, `padding` |
| 1115 | `.agent-composer-foot .agent-modelbar` | — | Agent rail/composer/content | T | `color` |
| 1116 | `.agent-composer-foot .agent-modelbar .pm-select` | — | Agent rail/composer/content | T | `padding-right` |
| 1119 | `.agent-composer-foot .agent-modelbar .pm-select-chevron` | — | Agent rail/composer/content | T | `opacity`, `transition` |
| 1120 | `.agent-composer-foot .agent-modelbar .pm-select:hover .pm-select-chevron` | — | Agent rail/composer/content | T | `opacity` |
| 1120 | `.agent-composer-foot .agent-modelbar .pm-select:focus-visible .pm-select-chevron` | — | Agent rail/composer/content | T | `opacity` |
| 1120 | `.agent-composer-foot .agent-modelbar .pm-select[aria-expanded=true] .pm-select-chevron` | — | Agent rail/composer/content | T | `opacity` |
| 1121 | `.agent-composer textarea::-webkit-scrollbar` | — | Agent rail/composer/content | T | `display` |
| 1122 | `.agent-composer textarea::placeholder` | — | Agent rail/composer/content | T | `color` |
| 1123 | `.agent-round` | — | Agent rail/composer/content | T | `border-radius`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 1124 | `.agent-round:active` | — | Agent rail/composer/content | T | `transform` |
| 1125 | `.agent-attach` | — | Agent rail/composer/content | T | `background`, `color` |
| 1126 | `.agent-attach:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 1127 | `.agent-attach svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 1129 | `.agent-composer .agent-send.agent-send` | — | Agent rail/composer/content | T | `background`, `border`, `box-shadow`, `color`, `padding` |
| 1130 | `.agent-composer .agent-send.agent-send.is-sendable` | — | Agent rail/composer/content | T | `background`, `color` |
| 1131 | `.agent-send.is-sendable:hover` | — | Agent rail/composer/content | T | `background` |
| 1132 | `.agent-send:disabled` | — | Agent rail/composer/content | T | `background`, `color` |
| 1133 | `.agent-send svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 1134 | `.agent-send svg[data-icon="return"]` | — | Agent rail/composer/content | T | `transform` |
| 1135 | `.agent-send>i.agent-spin` | — | Agent rail/composer/content | T | `animation`, `border`, `border-radius`, `border-top-color`, `height`, `width` |
| 1136 | `.agent-stop` | — | Agent rail/composer/content | T | `background`, `color` |
| 1137 | `.agent-stop:hover` | — | Agent rail/composer/content | T | `opacity` |
| 1138 | `.agent-stop>i` | — | Agent rail/composer/content | T | `background`, `border-radius`, `height`, `width` |
| 1141 | `.agent-option-bar` | — | Agent rail/composer/content | T | `align-items`, `display`, `flex-wrap`, `gap`, `margin-top`, `max-width`, `min-width`, `overflow`, `padding` |
| 1142 | `.agent-option-bar .sp` | — | Agent rail/composer/content | T | `flex`, `min-width` |
| 1143 | `.agent-modelbar` | — | Agent rail/composer/content | T | `align-items`, `border-radius`, `color`, `display`, `flex`, `flex-wrap`, `font-size`, `font-weight`, `gap`, `max-width`, `min-width`, `overflow`, `padding`, `transition`, `white-space` |
| 1144 | `.agent-modelbar select:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 1144 | `.agent-modelbar select:focus-visible` | — | Agent rail/composer/content | T | `background`, `color` |
| 1145 | `.agent-modelbar select` | — | Agent rail/composer/content | T | `appearance`, `background`, `border`, `border-radius`, `color`, `field-sizing`, `flex`, `font`, `max-width`, `min-width`, `outline`, `overflow`, `padding`, `text-overflow`, `white-space`, `width` |
| 1146 | `.agent-modelbar select.provider` | — | Agent rail/composer/content | T | `flex`, `max-width` |
| 1147 | `.agent-modelbar select.effort` | — | Agent rail/composer/content | T | `flex`, `max-width`, `opacity`, `text-transform` |
| 1148 | `.agent-modelbar select.provider` | — | Agent rail/composer/content | T | `color`, `font-weight` |
| 1149 | `#library-screen.panel-library` | — | Agent rail/composer/content | T | `background`, `height`, `max-height`, `width` |
| 1150 | `.panel-library-head` | — | Agent rail/composer/content | T | `align-items`, `display`, `flex`, `gap`, `padding` |
| 1151 | `.panel-library-head b` | — | Agent rail/composer/content | T | `font-size`, `font-weight` |
| 1152 | `.panel-library-head .sp` | — | Agent rail/composer/content | T | `flex` |
| 1153 | `.panel-library-search` | — | Agent rail/composer/content | T | `align-items`, `background`, `border-radius`, `color`, `display`, `gap`, `margin`, `padding` |
| 1154 | `.panel-library-search svg` | — | Agent rail/composer/content | T | `flex`, `height`, `width` |
| 1154 | `.panel-library-open svg` | — | Agent rail/composer/content | T | `flex`, `height`, `width` |
| 1155 | `.panel-library-search input` | — | Agent rail/composer/content | T | `background`, `border`, `color`, `font`, `min-width`, `outline`, `width` |
| 1156 | `.panel-library-list` | — | Agent rail/composer/content | T | `min-height`, `overflow`, `padding` |
| 1157 | `.panel-library-row` | — | Agent rail/composer/content | T | `align-items`, `border-radius`, `display`, `gap`, `min-height`, `padding` |
| 1158 | `.panel-library-row:hover` | — | Agent rail/composer/content | T | `background` |
| 1158 | `.panel-library-row:focus-within` | — | Agent rail/composer/content | T | `background` |
| 1159 | `.panel-library-open` | — | Agent rail/composer/content | T | `align-items`, `background`, `border`, `color`, `display`, `flex`, `font`, `gap`, `min-width`, `padding`, `text-align` |
| 1160 | `.panel-library-open span` | — | Agent rail/composer/content | T | `overflow`, `text-overflow`, `white-space` |
| 1161 | `.panel-library-status` | — | Agent rail/composer/content | T | `color`, `font-size` |
| 1162 | `.panel-library-empty` | — | Agent rail/composer/content | T | `color`, `padding`, `text-align` |
| 1163 | `.agent-approval` | — | Agent rail/composer/content | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `height`, `padding`, `transition`, `white-space` |
| 1164 | `.agent-approval:hover` | — | Agent rail/composer/content | T | `background`, `color` |
| 1165 | `.agent-approval>i` | — | Agent rail/composer/content | T | `display` |
| 1166 | `.agent-approval[aria-pressed="true"]` | — | Agent rail/composer/content | T | `color` |
| 1168 | `.agent-attachment-rail` | — | Agent rail/composer/content | T | `display`, `flex-wrap`, `gap`, `padding` |
| 1169 | `.agent-attachment` | — | Agent rail/composer/content | T | `align-items`, `animation`, `background`, `border-radius`, `color`, `display`, `font-size`, `gap`, `height`, `max-width`, `min-width`, `padding` |
| 1171 | `.agent-attachment-open` | — | Agent rail/composer/content | T | `align-items`, `color`, `display`, `gap`, `min-width`, `text-align` |
| 1172 | `.agent-attachment-open>img` | — | Agent rail/composer/content | T | `background`, `border-radius`, `flex`, `height`, `object-fit`, `width` |
| 1173 | `.agent-attachment-type` | — | Agent rail/composer/content | T | `background`, `border-radius`, `color`, `display`, `flex`, `font-size`, `height`, `letter-spacing`, `place-items`, `width` |
| 1174 | `.agent-attachment-name` | — | Agent rail/composer/content | T | `max-width`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1175 | `.agent-attachment-remove` | — | Agent rail/composer/content | T | `border-radius`, `color`, `display`, `flex`, `height`, `opacity`, `place-items`, `width` |
| 1176 | `.agent-attachment-open:hover .agent-attachment-name` | — | Agent rail/composer/content | T | `color`, `text-decoration`, `text-underline-offset` |
| 1177 | `.agent-attachment-remove:hover` | — | Agent rail/composer/content | T | `background`, `color`, `opacity` |
| 1178 | `.agent-attachment-remove svg` | — | Agent rail/composer/content | T | `height`, `width` |
| 1179 | `.agent-msg-files .agent-attachment` | — | Agent rail/composer/content | T | `background` |
| 1181 | `.agent-access-warning` | — | Agent rail/composer/content | T | `color`, `display`, `flex-direction`, `font-size`, `gap`, `line-height` |
| 1181 | `.agent-access-warning p:last-child` | — | Agent rail/composer/content | T | `background`, `border-radius`, `color`, `padding` |
| 1182 | `.agent-image-preview` | — | Agent rail/composer/content | T | `background`, `border-radius`, `display`, `max-height`, `min-height`, `overflow`, `place-items` |
| 1183 | `.agent-image-preview img` | — | Agent rail/composer/content | T | `display`, `height`, `max-height`, `max-width`, `object-fit`, `width` |
| 1184 | `.agent-card-actions .sp` | — | Agent rail/composer/content | T | `flex` |
| 1188 | `.panel-drop-preview` | — | Shell dock drag/drop, split/join | T | `background`, `border-radius`, `box-shadow`, `height`, `opacity`, `pointer-events`, `position`, `transform`, `transition`, `z-index` |
| 1193 | `.panel-drop-preview.on` | — | Shell dock drag/drop, split/join | T | `opacity`, `transform` |
| 1194 | `.panel-drop-preview.dock-edge` | — | Shell dock drag/drop, split/join | T | `background`, `border`, `border-radius`, `box-shadow`, `height`, `transform`, `transition` |
| 1195 | `.panel-drop-preview.dock-edge.on` | — | Shell dock drag/drop, split/join | T | `transform` |
| 1196 | `.panel.drag-src` | — | Shell dock drag/drop, split/join | T | `opacity`, `transition` |
| 1198 | `.panel-corner` | — | Shell dock drag/drop, split/join | T | `cursor`, `height`, `position`, `touch-action`, `width`, `z-index` |
| 1199 | `.panel-corner[data-corner="top-left"]` | — | Shell dock drag/drop, split/join | T | `left`, `top` |
| 1200 | `.panel-corner[data-corner="top-right"]` | — | Shell dock drag/drop, split/join | T | `right`, `top` |
| 1201 | `.panel-corner[data-corner="bottom-left"]` | — | Shell dock drag/drop, split/join | T | `bottom`, `left` |
| 1202 | `.panel-corner[data-corner="bottom-right"]` | — | Shell dock drag/drop, split/join | T | `bottom`, `right` |
| 1203 | `.panel-corner::before` | — | Shell dock drag/drop, split/join | T | `background`, `content`, `inset`, `opacity`, `position`, `transition` |
| 1205 | `.panel-corner[data-corner="top-left"]` | — | Shell dock drag/drop, split/join | T | `--corner-angle` |
| 1206 | `.panel-corner[data-corner="top-right"]` | — | Shell dock drag/drop, split/join | T | `--corner-angle` |
| 1207 | `.panel-corner[data-corner="bottom-left"]` | — | Shell dock drag/drop, split/join | T | `--corner-angle` |
| 1208 | `.panel-corner[data-corner="bottom-right"]` | — | Shell dock drag/drop, split/join | T | `--corner-angle` |
| 1209 | `.panel:hover > .panel-corner::before` | — | Shell dock drag/drop, split/join | T | `opacity` |
| 1210 | `.panel > .panel-corner:hover::before` | — | Shell dock drag/drop, split/join | T | `background`, `opacity` |
| 1211 | `.panel.popped > .panel-corner` | — | Shell dock drag/drop, split/join | T | `display` |
| 1211 | `.panel-dragging .panel-corner` | — | Shell dock drag/drop, split/join | T | `display` |
| 1212 | `.area-dragging` | — | Shell dock drag/drop, split/join | T | `cursor` |
| 1212 | `.area-dragging *` | — | Shell dock drag/drop, split/join | T | `cursor` |
| 1213 | `.area-op-shade` | — | Shell dock drag/drop, split/join | T | `border-radius`, `opacity`, `pointer-events`, `position`, `transition` |
| 1213 | `.area-op-zone` | — | Shell dock drag/drop, split/join | T | `border-radius`, `opacity`, `pointer-events`, `position`, `transition` |
| 1213 | `.area-op-line` | — | Shell dock drag/drop, split/join | T | `border-radius`, `opacity`, `pointer-events`, `position`, `transition` |
| 1214 | `.area-op-shade` | — | Shell dock drag/drop, split/join | T | `background`, `display`, `place-items`, `z-index` |
| 1215 | `.area-op-arrow` | — | Shell dock drag/drop, split/join | T | `color`, `font-size`, `line-height`, `text-shadow` |
| 1216 | `.area-op-zone` | — | Shell dock drag/drop, split/join | T | `background`, `box-shadow`, `z-index` |
| 1217 | `.area-op-line` | — | Shell dock drag/drop, split/join | T | `background`, `border-radius`, `box-shadow`, `z-index` |
| 1218 | `.area-op-shade.on` | — | Shell dock drag/drop, split/join | T | `opacity` |
| 1218 | `.area-op-zone.on` | — | Shell dock drag/drop, split/join | T | `opacity` |
| 1218 | `.area-op-line.on` | — | Shell dock drag/drop, split/join | T | `opacity` |
| 1219 | `.panel-popout-body` | — | Shell dock drag/drop, split/join | T | `background`, `color`, `display`, `flex-direction`, `height`, `margin`, `overflow` |
| 1219 | `.pop-bar` | — | Shell dock drag/drop, split/join | T | `-webkit-user-select`, `align-items`, `background`, `border-bottom`, `display`, `flex`, `gap`, `height`, `padding`, `user-select` |
| 1219 | `.pop-title` | — | Shell dock drag/drop, split/join | T | `font-size`, `font-weight` |
| 1219 | `.pop-bar .sp` | — | Shell dock drag/drop, split/join | T | `flex` |
| 1219 | `.pop-mirror` | — | Shell dock drag/drop, split/join | T | `display`, `flex`, `min-height` |
| 1219 | `.pop-mirror>.panel.popped` | — | Shell dock drag/drop, split/join | T | `border`, `border-radius`, `flex` |
| 1219 | `.panel.popped>header` | — | Shell dock drag/drop, split/join | T | `display` |
| 1223 | `#projects-screen` | — | Project library | T | `background`, `display`, `inset`, `min-height`, `padding`, `position`, `z-index` |
| 1224 | `#projects-screen.on` | — | Project library | T | `display` |
| 1228 | `body:has(#projects-screen.on) #body` | — | Project library | T | `visibility` |
| 1228 | `body:has(#projects-screen.on) #status` | — | Project library | T | `visibility` |
| 1229 | `.ps-sidebar` | — | Project library | T | `display`, `flex`, `flex-direction`, `gap`, `padding`, `width` |
| 1230 | `.ps-search` | — | Project library | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `flex`, `gap`, `height`, `padding` |
| 1231 | `.ps-search input::placeholder` | — | Project library | T | `color` |
| 1232 | `.ps-search svg` | — | Project library | T | `flex`, `height`, `width` |
| 1232 | `.ps-search input` | — | Project library | T | `color`, `flex`, `font-size`, `min-width` |
| 1233 | `.ps-search svg` | — | Project library | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 1233 | `.ps-navbtn svg` | — | Project library | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 1233 | `.ps-view svg` | — | Project library | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 1233 | `.ps-more svg` | — | Project library | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 1233 | `.ps-empty > svg` | — | Project library | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 1235 | `.ps-nav` | — | Project library | T | `display`, `flex-direction`, `gap` |
| 1235 | `.ps-navbtn` | — | Project library | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `padding`, `text-align`, `transition`, `width` |
| 1236 | `.ps-navs` | — | Project library | T | `display`, `flex-direction`, `gap`, `position` |
| 1237 | `.ps-navbtn` | — | Project library | T | `position`, `z-index` |
| 1237 | `.ps-navbtn:hover` | — | Project library | T | `background`, `color` |
| 1237 | `.ps-navbtn.on` | — | Project library | T | `background`, `color` |
| 1239 | `.nav-glider` | — | Project library | T | `background`, `border-radius`, `height`, `left`, `opacity`, `pointer-events`, `position`, `top`, `transition`, `width` |
| 1240 | `.nav-glider.on` | — | Project library | T | `opacity` |
| 1241 | `.nav-glider.settling` | — | Project library | T | `transition-duration` |
| 1242 | `.nav-glider` | @media (prefers-reduced-motion:reduce) | Project library | T | `transition` |
| 1242 | `.ps-navbtn.on::after` | — | Project library | T | `display` |
| 1243 | `.ps-navbtn svg` | — | Project library | T | `color`, `flex`, `height`, `transition`, `width` |
| 1243 | `.ps-navbtn:hover svg` | — | Project library | T | `color` |
| 1243 | `.ps-navbtn.on svg` | — | Project library | T | `color` |
| 1243 | `.ps-navbtn span:not(.count)` | — | Project library | T | `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1243 | `.ps-navbtn .count` | — | Project library | T | `color`, `font-size`, `margin-left` |
| 1244 | `.ps-updates[hidden]` | — | Project library | T | `display` |
| 1245 | `.ps-updates` | — | Project library | T | `background`, `border-radius`, `color`, `display`, `flex-direction`, `font-size`, `gap`, `line-height`, `margin-top`, `padding` |
| 1246 | `.ps-updates b` | — | Project library | T | `color`, `display`, `font-weight` |
| 1247 | `.ps-updates .btn` | — | Project library | T | `align-self`, `height`, `padding` |
| 1248 | `.ps-updates+.ps-sidefoot` | — | Project library | T | `margin-top` |
| 1249 | `.ps-updates+.ps-updates` | — | Project library | T | `margin-top` |
| 1250 | `.ps-sidefoot` | — | Project library | T | `color`, `font-size`, `line-height`, `margin-top`, `padding` |
| 1251 | `.ps-main` | — | Project library | T | `background`, `border-radius`, `box-shadow`, `display`, `flex`, `flex-direction`, `min-width`, `overflow` |
| 1251 | `.ps-top` | — | Project library | T | `align-items`, `display`, `flex`, `gap`, `height`, `padding` |
| 1253 | `.ps-top .btn` | — | Project library | T | `height`, `padding` |
| 1254 | `.ps-title` | — | Project library | T | `align-items`, `display`, `flex`, `gap`, `min-width` |
| 1254 | `.ps-title b` | — | Project library | T | `color`, `font-size`, `font-weight`, `letter-spacing` |
| 1254 | `.ps-title span` | — | Project library | T | `color`, `font-size` |
| 1255 | `.ps-view` | — | Project library | T | `align-items`, `display`, `gap` |
| 1255 | `.ps-view button` | — | Project library | T | `border-radius`, `color`, `display`, `height`, `place-items`, `width` |
| 1255 | `.ps-view button:hover` | — | Project library | T | `background`, `color` |
| 1255 | `.ps-view button.on` | — | Project library | T | `background`, `color` |
| 1255 | `.ps-view svg` | — | Project library | T | `height`, `width` |
| 1256 | `.ps-sort` | — | Project library | T | `background`, `border`, `border-radius`, `color`, `font`, `font-size`, `height`, `min-width`, `outline`, `padding` |
| 1256 | `.ps-sort:focus-visible` | — | Project library | T | `box-shadow` |
| 1257 | `.ps-content` | — | Project library | T | `flex`, `min-height`, `overflow`, `padding` |
| 1257 | `.ps-grid` | — | Project library | T | `align-content`, `display`, `gap`, `grid-template-columns`, `min-height` |
| 1257 | `.ps-grid.list` | — | Project library | T | `display`, `flex-direction`, `gap` |
| 1257 | `.ps-grid.empty` | — | Project library | T | `display`, `grid-template-columns`, `place-items` |
| 1261 | `.ps-card` | — | Project library | T | `border`, `border-radius`, `cursor`, `min-width`, `padding`, `position` |
| 1262 | `.ps-thumb` | — | Project library | T | `aspect-ratio`, `background`, `border-radius`, `overflow`, `position`, `transition` |
| 1264 | `.ps-thumb::after` | — | Project library | T | `border-radius`, `box-shadow`, `content`, `corner-shape`, `inset`, `pointer-events`, `position`, `transition` |
| 1265 | `.ps-card:hover .ps-thumb` | — | Project library | T | `box-shadow` |
| 1266 | `.ps-card.active .ps-thumb::after` | — | Project library | T | `box-shadow` |
| 1267 | `.ps-card:hover .ps-thumb::after` | — | Project library | T | `box-shadow` |
| 1268 | `.ps-card:active .ps-thumb` | — | Project library | T | `transform` |
| 1269 | `.ps-thumb-inner` | — | Project library | T | `background`, `border-radius`, `corner-shape`, `height`, `overflow` |
| 1269 | `.ps-thumb img` | — | Project library | T | `display`, `height`, `object-fit`, `width` |
| 1271 | `.ps-thumb-empty` | — | Project library | T | `color`, `display`, `font`, `inset`, `letter-spacing`, `place-items`, `pointer-events`, `position` |
| 1272 | `.library-card-badge` | — | Project library | T | `align-items`, `background`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `font-weight`, `height`, `padding` |
| 1273 | `.ps-meta` | — | Project library | T | `align-items`, `display`, `gap`, `padding` |
| 1273 | `.ps-meta-copy` | — | Project library | T | `flex`, `min-width` |
| 1274 | `.ps-name-row` | — | Project library | T | `align-items`, `display`, `gap`, `min-width` |
| 1274 | `.ps-name` | — | Project library | T | `color`, `font-size`, `font-weight`, `line-height`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1275 | `.ps-tag` | — | Project library | T | `color`, `flex`, `font-size`, `font-weight`, `line-height` |
| 1276 | `.ps-sub` | — | Project library | T | `color`, `font-size`, `line-height`, `margin-top`, `overflow`, `text-overflow`, `white-space` |
| 1277 | `.ps-more` | — | Project library | T | `border-radius`, `color`, `display`, `height`, `opacity`, `place-items`, `position`, `width`, `z-index` |
| 1277 | `.ps-card:hover .ps-more` | — | Project library | T | `opacity` |
| 1277 | `.ps-more:focus-visible` | — | Project library | T | `opacity` |
| 1277 | `.ps-more:hover` | — | Project library | T | `background`, `color` |
| 1277 | `.ps-more svg` | — | Project library | T | `height`, `width` |
| 1278 | `.ps-grid.list .ps-card` | — | Project library | T | `align-items`, `border-radius`, `display`, `height`, `padding`, `transition` |
| 1278 | `.ps-grid.list .ps-card:hover` | — | Project library | T | `background` |
| 1278 | `.ps-grid.list .ps-card.active` | — | Project library | T | `background` |
| 1278 | `.ps-grid.list .ps-card:hover .ps-thumb` | — | Project library | T | `box-shadow` |
| 1278 | `.ps-grid.list .ps-card.active .ps-thumb` | — | Project library | T | `box-shadow` |
| 1278 | `.ps-grid.list .ps-card:hover .ps-thumb::after` | — | Project library | T | `box-shadow` |
| 1278 | `.ps-grid.list .ps-card.active .ps-thumb::after` | — | Project library | T | `box-shadow` |
| 1278 | `.ps-grid.list .ps-thumb` | — | Project library | T | `border-radius`, `flex`, `width` |
| 1278 | `.ps-grid.list .ps-thumb-empty` | — | Project library | T | `display` |
| 1278 | `.ps-grid.list .ps-meta` | — | Project library | T | `align-items`, `flex`, `padding` |
| 1278 | `.ps-grid.list .ps-more` | — | Project library | T | `opacity` |
| 1279 | `.ps-empty` | — | Project library | T | `align-items`, `color`, `display`, `flex-direction`, `grid-column`, `justify-content`, `padding`, `text-align` |
| 1279 | `.ps-empty > svg` | — | Project library | T | `color`, `height`, `margin-bottom`, `width` |
| 1279 | `.ps-empty b` | — | Project library | T | `color`, `font-size`, `font-weight`, `margin-bottom` |
| 1279 | `.ps-empty span` | — | Project library | T | `font-size`, `line-height`, `max-width`, `text-wrap` |
| 1279 | `.ps-empty button` | — | Project library | T | `margin-top` |
| 1283 | `select.pm-select-native` | — | Shared select/listbox/menu | T | `flex`, `height`, `margin`, `min-width`, `opacity`, `padding`, `pointer-events`, `position`, `width` |
| 1284 | `.pm-select` | — | Shared select/listbox/menu | T | `-webkit-appearance`, `align-items`, `appearance`, `background-image`, `color`, `cursor`, `display`, `font`, `gap`, `justify-content`, `padding-right`, `position`, `text-align` |
| 1285 | `.pm-select-label` | — | Shared select/listbox/menu | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1286 | `.pm-select[data-empty]` | — | Shared select/listbox/menu | T | `color` |
| 1287 | `.pm-select-chevron` | — | Shared select/listbox/menu | T | `color`, `flex`, `height`, `margin-top`, `position`, `right`, `top`, `transition`, `width` |
| 1288 | `.pm-select:hover .pm-select-chevron` | — | Shared select/listbox/menu | T | `color` |
| 1288 | `.pm-select[aria-expanded=true] .pm-select-chevron` | — | Shared select/listbox/menu | T | `color` |
| 1289 | `.pm-select:focus-visible` | — | Shared select/listbox/menu | T | `box-shadow`, `outline` |
| 1290 | `.pm-select:disabled` | — | Shared select/listbox/menu | T | `opacity`, `pointer-events` |
| 1292 | `.pm-select.sel` | — | Shared select/listbox/menu | T | `font-family`, `font-size`, `height`, `padding` |
| 1293 | `.pm-select.sel .pm-select-chevron` | — | Shared select/listbox/menu | T | `height`, `margin-top`, `right`, `width` |
| 1295 | `.agent-modelbar .pm-select` | — | Shared select/listbox/menu | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `font`, `max-width`, `min-width`, `padding` |
| 1296 | `.agent-modelbar .pm-select.provider` | — | Shared select/listbox/menu | T | `max-width` |
| 1297 | `.agent-modelbar .pm-select.effort` | — | Shared select/listbox/menu | T | `color`, `flex`, `max-width` |
| 1298 | `.agent-modelbar .pm-select-chevron` | — | Shared select/listbox/menu | T | `height`, `margin-top`, `right`, `width` |
| 1299 | `.agent-modelbar .pm-select:hover` | — | Shared select/listbox/menu | T | `background`, `color` |
| 1299 | `.agent-modelbar .pm-select:focus-visible` | — | Shared select/listbox/menu | T | `background`, `color` |
| 1301 | `.agent-access` | — | Shared select/listbox/menu | T | `align-items`, `border-radius`, `color`, `display`, `flex`, `font-size`, `height`, `padding`, `transition`, `white-space` |
| 1302 | `.agent-access:hover` | — | Shared select/listbox/menu | T | `background`, `color` |
| 1302 | `.agent-access:focus-visible` | — | Shared select/listbox/menu | T | `background`, `color` |
| 1302 | `.agent-access[aria-expanded=true]` | — | Shared select/listbox/menu | T | `background`, `color` |
| 1303 | `.agent-access[data-tone=warning]` | — | Shared select/listbox/menu | T | `color` |
| 1304 | `.pm-select.ps-sort` | — | Shared select/listbox/menu | T | `border`, `box-shadow` |
| 1304 | `.pm-select.settings-select` | — | Shared select/listbox/menu | T | `border`, `box-shadow` |
| 1304 | `.pm-select.settings-appearance` | — | Shared select/listbox/menu | T | `border`, `box-shadow` |
| 1304 | `.pm-select.export-select` | — | Shared select/listbox/menu | T | `border`, `box-shadow` |
| 1306 | `.pm-menu` | — | Shared select/listbox/menu | T | `background`, `border-radius`, `box-shadow`, `color`, `font-size`, `font-weight`, `max-width`, `outline`, `overflow-x`, `overflow-y`, `padding`, `position`, `scrollbar-width`, `z-index` |
| 1307 | `:root[data-theme="dark"] .pm-menu` | — | Shared select/listbox/menu | T | `box-shadow` |
| 1308 | `.pm-menu` | — | Shared select/listbox/menu | T | `-webkit-user-select`, `cursor`, `user-select` |
| 1308 | `.pm-menu *` | — | Shared select/listbox/menu | T | `-webkit-user-select`, `cursor`, `user-select` |
| 1309 | `.pm-menu[data-side=bottom]` | — | Shared select/listbox/menu | T | `transform-origin` |
| 1310 | `.pm-menu[data-side=top]` | — | Shared select/listbox/menu | T | `transform-origin` |
| 1311 | `.pm-menu[data-state=open]` | — | Shared select/listbox/menu | T | `animation` |
| 1312 | `.pm-menu[data-state=closed]` | — | Shared select/listbox/menu | T | `animation`, `pointer-events` |
| 1315 | `.pm-menu[data-side=top]` | — | Shared select/listbox/menu | T | `--pm-menu-slide` |
| 1316 | `.pm-menu-glider` | — | Shared select/listbox/menu | T | `background`, `border-radius`, `height`, `left`, `opacity`, `pointer-events`, `position`, `right`, `top`, `transition` |
| 1317 | `.pm-menu-glider.on` | — | Shared select/listbox/menu | T | `opacity` |
| 1318 | `:has(> [data-blink]) > .pm-menu-glider` | — | Shared select/listbox/menu | T | `transition` |
| 1319 | `:has(> [data-blink=off]) > .pm-menu-glider` | — | Shared select/listbox/menu | T | `opacity` |
| 1320 | `.pm-menu-item` | — | Shared select/listbox/menu | T | `align-items`, `border-radius`, `color`, `display`, `gap`, `height`, `padding`, `position`, `white-space`, `z-index` |
| 1321 | `.pm-menu-item[aria-disabled=true]` | — | Shared select/listbox/menu | T | `color`, `pointer-events` |
| 1322 | `.pm-menu-label` | — | Shared select/listbox/menu | T | `flex`, `min-width`, `overflow`, `text-overflow` |
| 1323 | `.pm-menu-check` | — | Shared select/listbox/menu | T | `color`, `height`, `opacity`, `position`, `right`, `width` |
| 1324 | `.pm-menu-item[aria-selected=true] .pm-menu-check` | — | Shared select/listbox/menu | T | `opacity` |
| 1325 | `.pm-menu[data-state]` | @media(prefers-reduced-motion:reduce) | Shared select/listbox/menu | T | `animation` |
| 1325 | `.pm-menu-glider` | @media(prefers-reduced-motion:reduce) | Shared select/listbox/menu | T | `transition` |
| 1328 | `.pm-popover .pm-menu-item` | — | Shared select/listbox/menu | T | `padding` |
| 1329 | `.pm-popover .pm-menu-item[role=menuitemradio]` | — | Shared select/listbox/menu | T | `padding-right` |
| 1330 | `.pm-menu-item[aria-checked=true] .pm-menu-check` | — | Shared select/listbox/menu | T | `opacity` |
| 1331 | `.pm-menu-header` | — | Shared select/listbox/menu | T | `padding`, `position`, `z-index` |
| 1332 | `.pm-menu-sep` | — | Shared select/listbox/menu | T | `background`, `height`, `margin` |
| 1334 | `.pm-menu-icon` | — | Shared select/listbox/menu | T | `color`, `flex`, `height`, `width` |
| 1335 | `.pm-menu-item.has-detail` | — | Shared select/listbox/menu | T | `align-items`, `height`, `min-height`, `padding-bottom`, `padding-top`, `white-space` |
| 1336 | `.pm-menu-item.has-detail .pm-menu-icon` | — | Shared select/listbox/menu | T | `margin-top` |
| 1337 | `.pm-menu-text` | — | Shared select/listbox/menu | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 1338 | `.pm-menu-detail` | — | Shared select/listbox/menu | T | `color`, `font-size`, `font-weight`, `line-height`, `max-width` |
| 1339 | `.pm-menu-item[data-tone=warning]` | — | Shared select/listbox/menu | T | `color` |
| 1339 | `.pm-menu-item[data-tone=warning] .pm-menu-icon` | — | Shared select/listbox/menu | T | `color` |
| 1339 | `.pm-menu-item[data-tone=warning] .pm-menu-detail` | — | Shared select/listbox/menu | T | `color` |
| 1340 | `.pm-menu-header.agent-access-head` | — | Shared select/listbox/menu | T | `color`, `font-size`, `font-weight`, `padding` |
| 1343 | `.ps-pagination` | — | Shell titlebar tabs | T | `align-items`, `color`, `display`, `font-size`, `gap`, `justify-content`, `padding` |
| 1344 | `.ps-pagination[hidden]` | — | Shell titlebar tabs | T | `display` |
| 1345 | `.ps-pagination .btn:focus-visible` | — | Shell titlebar tabs | T | `outline`, `outline-offset` |
| 1348 | `#library-overlay` | — | Reusable panel/workspace Library | T | `align-items`, `backdrop-filter`, `background`, `display`, `inset`, `justify-content`, `padding`, `position`, `z-index` |
| 1349 | `:root[data-theme="dark"] #library-overlay` | — | Reusable panel/workspace Library | T | `background` |
| 1350 | `#library-overlay.on` | — | Reusable panel/workspace Library | T | `display` |
| 1351 | `#library-overlay.dragging` | — | Reusable panel/workspace Library | T | `backdrop-filter`, `opacity`, `pointer-events` |
| 1352 | `.library-drop-hint` | — | Reusable panel/workspace Library | T | `border-radius`, `outline`, `outline-offset` |
| 1353 | `#library-screen` | — | Reusable panel/workspace Library | T | `--library-card-inset`, `--library-card-radius`, `--library-gutter`, `--library-inner-radius`, `background`, `border`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `height`, `overflow`, `position`, `width`, `z-index` |
| 1354 | `#library-screen.on` | — | Reusable panel/workspace Library | T | `display` |
| 1354 | `.library-head` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `display`, `flex`, `height`, `padding` |
| 1355 | `.library-title` | — | Reusable panel/workspace Library | T | `flex`, `min-width` |
| 1355 | `.library-head .iconbtn svg` | — | Reusable panel/workspace Library | T | `height`, `width` |
| 1356 | `.library-shell` | — | Reusable panel/workspace Library | T | `background`, `display`, `flex`, `min-height` |
| 1356 | `.library-sidebar` | — | Reusable panel/workspace Library | T | `background`, `display`, `flex`, `flex-direction`, `gap`, `padding`, `width` |
| 1357 | `.library-search` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `gap`, `height`, `padding`, `transition` |
| 1357 | `.library-search:focus-within` | — | Reusable panel/workspace Library | T | `box-shadow`, `outline` |
| 1358 | `.library-search svg` | — | Reusable panel/workspace Library | T | `flex`, `height`, `width` |
| 1358 | `.library-search input` | — | Reusable panel/workspace Library | T | `color`, `flex`, `font-size`, `min-width` |
| 1359 | `.library-browse` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap`, `padding` |
| 1359 | `.library-browse>span` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `padding` |
| 1359 | `.library-nav-label` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `padding` |
| 1360 | `.library-scope` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `display`, `grid-template-columns`, `padding` |
| 1360 | `.library-scope button` | — | Reusable panel/workspace Library | T | `border-radius`, `color`, `font-size`, `height`, `padding`, `white-space` |
| 1360 | `.library-scope button.on` | — | Reusable panel/workspace Library | T | `background`, `box-shadow`, `color` |
| 1361 | `.library-nav` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `overflow` |
| 1361 | `.library-nav-label` | — | Reusable panel/workspace Library | T | `margin` |
| 1361 | `.library-nav-label:first-child` | — | Reusable panel/workspace Library | T | `margin-top` |
| 1361 | `.library-navbtn` | — | Reusable panel/workspace Library | T | `align-items`, `border-radius`, `color`, `display`, `flex`, `font-size`, `gap`, `height`, `padding`, `text-align`, `transition`, `width` |
| 1361 | `.library-navbtn>span:not(.library-count)` | — | Reusable panel/workspace Library | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1361 | `.library-navbtn .library-count` | — | Reusable panel/workspace Library | T | `color` |
| 1361 | `.library-navbtn.on .library-count` | — | Reusable panel/workspace Library | T | `color` |
| 1361 | `.library-sidebar .library-search` | — | Reusable panel/workspace Library | T | `flex` |
| 1362 | `.library-store-link` | — | Reusable panel/workspace Library | T | `background`, `height`, `margin-top` |
| 1362 | `.library-store-link:hover` | — | Reusable panel/workspace Library | T | `background` |
| 1362 | `.library-navbtn:hover` | — | Reusable panel/workspace Library | T | `background`, `color` |
| 1362 | `.library-navbtn.on` | — | Reusable panel/workspace Library | T | `background`, `color` |
| 1363 | `.library-navbtn svg` | — | Reusable panel/workspace Library | T | `height`, `width` |
| 1364 | `.library-main` | — | Reusable panel/workspace Library | T | `background`, `display`, `flex`, `flex-direction`, `min-width`, `overflow` |
| 1364 | `.library-top` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `display`, `flex`, `gap`, `min-height`, `padding` |
| 1366 | `#library-screen .segmented` | — | Reusable panel/workspace Library | T | `--segment-inset`, `background`, `border-radius`, `box-shadow`, `gap`, `padding` |
| 1367 | `#library-screen .segmented button` | — | Reusable panel/workspace Library | T | `border-radius`, `color`, `height`, `padding`, `transition` |
| 1368 | `#library-screen .segmented button.on` | — | Reusable panel/workspace Library | T | `background`, `box-shadow`, `color` |
| 1369 | `.library-top .sp` | — | Reusable panel/workspace Library | T | `flex` |
| 1369 | `.library-top .library-search` | — | Reusable panel/workspace Library | T | `width` |
| 1370 | `.library-page-title` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `min-width` |
| 1370 | `.library-page-title b` | — | Reusable panel/workspace Library | T | `font-size`, `font-weight`, `letter-spacing` |
| 1370 | `.library-count` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-variant-numeric`, `white-space` |
| 1370 | `.library-top-action` | — | Reusable panel/workspace Library | T | `display` |
| 1370 | `.library-top-action .btn` | — | Reusable panel/workspace Library | T | `height` |
| 1371 | `.library-content` | — | Reusable panel/workspace Library | T | `background`, `flex`, `min-height`, `overflow`, `padding` |
| 1371 | `.library-grid` | — | Reusable panel/workspace Library | T | `align-content`, `display`, `gap`, `grid-template-columns` |
| 1372 | `.library-card` | — | Reusable panel/workspace Library | T | `background`, `border`, `border-radius`, `display`, `flex-direction`, `min-width`, `padding`, `position`, `transition` |
| 1372 | `.library-card:hover` | — | Reusable panel/workspace Library | T | `background` |
| 1372 | `.library-card:focus-within` | — | Reusable panel/workspace Library | T | `background` |
| 1372 | `.library-card.active` | — | Reusable panel/workspace Library | T | `background`, `box-shadow` |
| 1372 | `.library-card.deleted` | — | Reusable panel/workspace Library | T | `opacity` |
| 1373 | `.library-thumb` | — | Reusable panel/workspace Library | T | `aspect-ratio`, `background`, `border`, `border-radius`, `corner-shape`, `overflow`, `position`, `width` |
| 1373 | `.workspace-map` | — | Reusable panel/workspace Library | T | `aspect-ratio`, `background`, `border`, `border-radius`, `corner-shape`, `overflow`, `position`, `width` |
| 1373 | `.library-thumb img` | — | Reusable panel/workspace Library | T | `display`, `height`, `object-fit`, `width` |
| 1373 | `.library-section-map` | — | Reusable panel/workspace Library | T | `inset`, `position` |
| 1373 | `.library-section-map i` | — | Reusable panel/workspace Library | T | `background`, `border`, `border-radius`, `height`, `position` |
| 1374 | `.library-look-swatch` | — | Reusable panel/workspace Library | T | `background`, `inset`, `position` |
| 1375 | `.library-card-body` | — | Reusable panel/workspace Library | T | `align-items`, `display`, `gap`, `min-width`, `padding` |
| 1375 | `.library-card-copy` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 1375 | `.library-card-copy b` | — | Reusable panel/workspace Library | T | `overflow`, `text-overflow`, `white-space` |
| 1375 | `.library-card-copy span` | — | Reusable panel/workspace Library | T | `overflow`, `text-overflow`, `white-space` |
| 1375 | `.library-card-copy small` | — | Reusable panel/workspace Library | T | `overflow`, `text-overflow`, `white-space` |
| 1375 | `.library-card-copy b` | — | Reusable panel/workspace Library | T | `font-size`, `font-weight` |
| 1375 | `.library-card-copy span` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1375 | `.library-card-copy small` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1376 | `.library-more` | — | Reusable panel/workspace Library | T | `border-radius`, `color`, `display`, `flex`, `height`, `opacity`, `place-items`, `width` |
| 1376 | `.library-card:hover .library-more` | — | Reusable panel/workspace Library | T | `opacity` |
| 1376 | `.library-more:focus-visible` | — | Reusable panel/workspace Library | T | `opacity` |
| 1376 | `.library-more:hover` | — | Reusable panel/workspace Library | T | `background`, `color` |
| 1376 | `.library-more svg` | — | Reusable panel/workspace Library | T | `height`, `width` |
| 1376 | `.library-card-actions` | — | Reusable panel/workspace Library | T | `display`, `padding` |
| 1376 | `.library-card-actions .btn` | — | Reusable panel/workspace Library | T | `height`, `width` |
| 1376 | `.library-card-badge` | — | Reusable panel/workspace Library | T | `left`, `position`, `top`, `z-index` |
| 1377 | `.workspace-map` | — | Reusable panel/workspace Library | T | `display`, `gap`, `padding` |
| 1377 | `.workspace-map-dock` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap`, `width` |
| 1377 | `.workspace-map-dock.flex` | — | Reusable panel/workspace Library | T | `flex` |
| 1377 | `.workspace-map-dock i` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `flex`, `min-height` |
| 1378 | `.library-book .workspace-map` | — | Reusable panel/workspace Library | T | `aspect-ratio`, `background`, `border-radius`, `gap`, `padding` |
| 1379 | `.library-book .workspace-map-dock` | — | Reusable panel/workspace Library | T | `gap` |
| 1380 | `.workspace-card.active .library-book-cover` | — | Reusable panel/workspace Library | T | `box-shadow` |
| 1381 | `.workspace-card.active .library-book-label span` | — | Reusable panel/workspace Library | T | `color` |
| 1382 | `.workspace-map-panel` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `border`, `border-radius`, `color`, `corner-shape`, `display`, `flex`, `flex-direction`, `gap`, `justify-content`, `min-height`, `overflow` |
| 1383 | `.workspace-map-panel svg` | — | Reusable panel/workspace Library | T | `flex`, `height`, `width` |
| 1384 | `.workspace-map-panel b` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight`, `max-width`, `overflow`, `text-overflow`, `white-space` |
| 1385 | `.workspace-map-panel.empty` | — | Reusable panel/workspace Library | T | `background` |
| 1386 | `.workspace-preview` | — | Reusable panel/workspace Library | T | `align-items`, `display`, `flex-direction`, `gap`, `text-align` |
| 1386 | `.workspace-preview .workspace-map` | — | Reusable panel/workspace Library | T | `aspect-ratio`, `height`, `width` |
| 1386 | `.workspace-preview p` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `line-height` |
| 1386 | `.library-decision p` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `line-height` |
| 1386 | `.library-save-dialog p` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `line-height` |
| 1387 | `.library-save-dialog` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap` |
| 1387 | `.library-save-dialog p` | — | Reusable panel/workspace Library | T | `margin` |
| 1387 | `.library-field-label` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1388 | `.library-editing` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `border`, `border-radius`, `display`, `gap`, `margin-bottom`, `min-height`, `padding` |
| 1388 | `.library-editing>div` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 1388 | `.library-editing b` | — | Reusable panel/workspace Library | T | `font-size` |
| 1388 | `.library-editing span` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1388 | `.library-editing .library-editing-kicker` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight`, `letter-spacing` |
| 1389 | `.library-grid.is-empty` | — | Reusable panel/workspace Library | T | `align-items`, `columns`, `display`, `justify-content`, `min-height` |
| 1389 | `.library-empty` | — | Reusable panel/workspace Library | T | `align-items`, `color`, `display`, `flex-direction`, `gap`, `justify-content`, `padding`, `text-align`, `width` |
| 1389 | `.library-empty>svg` | — | Reusable panel/workspace Library | T | `color`, `flex`, `height`, `width` |
| 1389 | `.library-empty b` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight` |
| 1390 | `.library-card-hit` | — | Reusable panel/workspace Library | T | `background`, `border`, `border-radius`, `color`, `corner-shape`, `cursor`, `display`, `flex-direction`, `font`, `min-width`, `padding`, `text-align` |
| 1391 | `.library-card-hit:focus-visible` | — | Reusable panel/workspace Library | T | `box-shadow`, `outline` |
| 1392 | `.library-card:has(>.library-card-hit:focus-visible)::after` | — | Reusable panel/workspace Library | T | `border-radius`, `box-shadow`, `content`, `corner-shape`, `inset`, `pointer-events`, `position`, `z-index` |
| 1393 | `.library-thumb-icon` | — | Reusable panel/workspace Library | T | `align-items`, `color`, `display`, `flex-direction`, `gap`, `inset`, `justify-content`, `position` |
| 1393 | `.library-thumb-icon svg` | — | Reusable panel/workspace Library | T | `height`, `width` |
| 1394 | `.library-thumb-icon b` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight`, `max-width`, `overflow`, `text-overflow`, `white-space` |
| 1395 | `.library-live.has-preview>.library-thumb-icon` | — | Reusable panel/workspace Library | T | `display` |
| 1398 | `.library-live.is-sandboxed.has-art` | — | Reusable panel/workspace Library | T | `background` |
| 1402 | `.library-live.is-sandboxed.has-art>.library-thumb-icon` | — | Reusable panel/workspace Library | T | `color` |
| 1402 | `.library-live.is-sandboxed.has-art>.library-thumb-icon b` | — | Reusable panel/workspace Library | T | `color` |
| 1403 | `.library-card[draggable="true"]` | — | Reusable panel/workspace Library | T | `cursor` |
| 1403 | `.library-card[draggable="true"]:active` | — | Reusable panel/workspace Library | T | `cursor` |
| 1404 | `.library-card-live` | — | Reusable panel/workspace Library | T | `background`, `border`, `box-shadow`, `padding`, `transform` |
| 1404 | `.library-card-live:hover` | — | Reusable panel/workspace Library | T | `background`, `border`, `box-shadow`, `padding`, `transform` |
| 1404 | `.library-book .library-card-hit` | — | Reusable panel/workspace Library | T | `display`, `position`, `width` |
| 1405 | `.library-live` | — | Reusable panel/workspace Library | T | `height`, `overflow`, `position`, `width` |
| 1406 | `.library-live-frame` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `left`, `pointer-events`, `position`, `top`, `transform-origin` |
| 1407 | `.library-live-frame>.panel.library-clone` | — | Reusable panel/workspace Library | T | `border`, `box-shadow`, `flex`, `min-height` |
| 1408 | `.library-card-add` | — | Reusable panel/workspace Library | T | `background`, `color`, `opacity`, `position`, `top`, `transition`, `z-index` |
| 1408 | `.library-card-delete` | — | Reusable panel/workspace Library | T | `background`, `color`, `opacity`, `position`, `top`, `transition`, `z-index` |
| 1409 | `.library-card-add` | — | Reusable panel/workspace Library | T | `right` |
| 1409 | `.library-card-delete` | — | Reusable panel/workspace Library | T | `right` |
| 1410 | `.library-card:hover .library-card-add` | — | Reusable panel/workspace Library | T | `opacity` |
| 1410 | `.library-card:focus-within .library-card-add` | — | Reusable panel/workspace Library | T | `opacity` |
| 1410 | `.library-card:hover .library-card-delete` | — | Reusable panel/workspace Library | T | `opacity` |
| 1410 | `.library-card:focus-within .library-card-delete` | — | Reusable panel/workspace Library | T | `opacity` |
| 1410 | `.library-card-add:hover` | — | Reusable panel/workspace Library | T | `background` |
| 1410 | `.library-card-delete:hover` | — | Reusable panel/workspace Library | T | `background` |
| 1410 | `.library-card-add:hover` | — | Reusable panel/workspace Library | T | `color` |
| 1410 | `.library-card-delete:hover` | — | Reusable panel/workspace Library | T | `color` |
| 1414 | `.library-shelves` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap`, `min-width` |
| 1415 | `.library-shelf` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap`, `min-width` |
| 1416 | `.library-shelf-title` | — | Reusable panel/workspace Library | T | `align-items`, `color`, `display`, `font-size`, `font-weight`, `gap`, `margin`, `padding` |
| 1417 | `.library-shelf-books` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap` |
| 1418 | `.library-shelf-row` | — | Reusable panel/workspace Library | T | `--ledge-h`, `align-items`, `display`, `gap`, `padding`, `position` |
| 1419 | `.library-shelf-row::before` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `bottom`, `content`, `corner-shape`, `height`, `left`, `pointer-events`, `position`, `right` |
| 1420 | `.library-book` | — | Reusable panel/workspace Library | T | `background`, `border`, `display`, `flex`, `flex-direction`, `min-width`, `padding`, `position` |
| 1421 | `.library-book-cover` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `box-shadow`, `corner-shape`, `overflow`, `position`, `transition` |
| 1422 | `.library-book-cover:has(.library-card-hit:active)` | — | Reusable panel/workspace Library | T | `filter` |
| 1423 | `.library-book:hover .library-book-cover` | — | Reusable panel/workspace Library | T | `box-shadow` |
| 1423 | `.library-book:focus-within .library-book-cover` | — | Reusable panel/workspace Library | T | `box-shadow` |
| 1424 | `.library-book-label` | — | Reusable panel/workspace Library | T | `display`, `flex-direction`, `gap`, `height`, `justify-content`, `min-width`, `padding`, `position`, `width` |
| 1425 | `.library-book-label b` | — | Reusable panel/workspace Library | T | `overflow`, `text-overflow`, `white-space` |
| 1425 | `.library-book-label span` | — | Reusable panel/workspace Library | T | `overflow`, `text-overflow`, `white-space` |
| 1426 | `.library-book-label b` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight` |
| 1427 | `.library-book-label span` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1428 | `.library-book-idle .library-live` | — | Reusable panel/workspace Library | T | `opacity`, `transition` |
| 1428 | `.library-book-idle:hover .library-live` | — | Reusable panel/workspace Library | T | `opacity` |
| 1429 | `.library-book-store .library-book-cover` | — | Reusable panel/workspace Library | T | `background`, `box-shadow`, `transition` |
| 1429 | `.library-book-store:hover .library-book-cover` | — | Reusable panel/workspace Library | T | `background` |
| 1430 | `.library-book-store .library-thumb-icon` | — | Reusable panel/workspace Library | T | `color` |
| 1430 | `.library-book-store .library-thumb-icon b` | — | Reusable panel/workspace Library | T | `text-align`, `white-space` |
| 1431 | `.library-book:hover` | — | Reusable panel/workspace Library | T | `background` |
| 1431 | `.library-book:focus-within` | — | Reusable panel/workspace Library | T | `background` |
| 1431 | `.library-book.active` | — | Reusable panel/workspace Library | T | `background` |
| 1432 | `.library-book-cover:has(.library-card-hit:focus-visible)::after` | — | Reusable panel/workspace Library | T | `border-radius`, `box-shadow`, `content`, `corner-shape`, `inset`, `pointer-events`, `position`, `z-index` |
| 1433 | `.library-editor` | — | Reusable panel/workspace Library | T | `display`, `flex`, `gap`, `min-height`, `padding` |
| 1437 | `.library-stage` | — | Reusable panel/workspace Library | T | `--stage-ledge`, `container-type`, `display`, `flex`, `min-width`, `place-items` |
| 1438 | `.library-stage-book` | — | Reusable panel/workspace Library | T | `align-items`, `display`, `flex-direction` |
| 1439 | `.library-stage-frame` | — | Reusable panel/workspace Library | T | `--sh`, `--sw`, `aspect-ratio`, `background`, `border`, `border-radius`, `box-shadow`, `corner-shape`, `display`, `flex-direction`, `overflow`, `width` |
| 1440 | `.library-stage-live` | — | Reusable panel/workspace Library | T | `border-radius`, `corner-shape`, `display`, `flex`, `flex-direction`, `min-height` |
| 1442 | `.library-stage-live>.panel.popped` | — | Reusable panel/workspace Library | T | `border-radius`, `box-shadow`, `corner-shape`, `flex`, `min-height`, `width` |
| 1443 | `.library-stage-ghost` | — | Reusable panel/workspace Library | T | `flex`, `min-height`, `overflow`, `pointer-events`, `position` |
| 1444 | `.library-stage-empty` | — | Reusable panel/workspace Library | T | `align-items`, `color`, `display`, `flex`, `flex-direction`, `gap`, `justify-content`, `padding`, `text-align` |
| 1445 | `.library-stage-empty svg` | — | Reusable panel/workspace Library | T | `color`, `height`, `margin-bottom`, `width` |
| 1445 | `.library-stage-empty b` | — | Reusable panel/workspace Library | T | `color`, `font-weight` |
| 1445 | `.library-stage-empty span` | — | Reusable panel/workspace Library | T | `font-size`, `line-height`, `max-width`, `text-wrap` |
| 1447 | `.library-chat` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `corner-shape`, `display`, `flex`, `flex-direction`, `overflow`, `width` |
| 1448 | `.library-chat-log` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `overflow`, `padding` |
| 1449 | `.library-chat-foot` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `padding` |
| 1452 | `.library-chat-foot .agent-composer` | — | Reusable panel/workspace Library | T | `border-radius`, `box-shadow` |
| 1453 | `.library-welcome` | — | Reusable panel/workspace Library | T | `margin`, `max-width` |
| 1454 | `.library-welcome .agent-suggestions button` | — | Reusable panel/workspace Library | T | `min-height`, `padding` |
| 1456 | `.library-stage-label` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `gap`, `height`, `justify-content`, `padding`, `position` |
| 1457 | `.library-stage-label::before` | — | Reusable panel/workspace Library | T | `background`, `border-radius`, `bottom`, `content`, `corner-shape`, `height`, `left`, `pointer-events`, `position`, `right` |
| 1458 | `.library-stage-label b` | — | Reusable panel/workspace Library | T | `overflow`, `position`, `text-overflow`, `white-space` |
| 1458 | `.library-stage-label span` | — | Reusable panel/workspace Library | T | `overflow`, `position`, `text-overflow`, `white-space` |
| 1459 | `.library-stage-label b` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `font-weight` |
| 1460 | `.library-stage-label span` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1461 | `.library-chat-body` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `min-height` |
| 1462 | `.library-head-note` | — | Reusable panel/workspace Library | T | `color`, `font-size`, `margin-right`, `white-space` |
| 1463 | `.library-head-action` | — | Reusable panel/workspace Library | T | `height`, `margin-right` |
| 1471 | `.library-fly` | — | Reusable panel/workspace Library | T | `aspect-ratio`, `box-shadow`, `pointer-events`, `position`, `transform-origin`, `z-index` |
| 1477 | `#library-screen` | — | Reusable panel/workspace Library | T | `--library-travel` |
| 1478 | `#library-overlay.on` | — | Reusable panel/workspace Library | T | `animation` |
| 1479 | `#library-overlay.on #library-screen` | — | Reusable panel/workspace Library | T | `animation` |
| 1480 | `#library-overlay.closing` | — | Reusable panel/workspace Library | T | `animation`, `pointer-events` |
| 1481 | `#library-overlay.closing #library-screen` | — | Reusable panel/workspace Library | T | `animation` |
| 1484 | `#library-overlay.closing-editor #library-screen>*` | — | Reusable panel/workspace Library | T | `visibility` |
| 1485 | `.library-editor .library-chat` | — | Reusable panel/workspace Library | T | `animation` |
| 1487 | `.library-live.is-clipped>.library-live-frame` | — | Reusable panel/workspace Library | T | `-webkit-mask-image`, `mask-image` |
| 1488 | `.library-live-frame.is-fresh` | — | Reusable panel/workspace Library | T | `animation` |
| 1494 | `#library-overlay` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation` |
| 1494 | `#library-screen` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation` |
| 1494 | `.library-chat` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation` |
| 1494 | `.library-live-frame` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation` |
| 1495 | `#library-overlay.closing` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `display` |
| 1497 | `#workspace-editbar` | — | Reusable panel/workspace Library | T | `align-items`, `background`, `border`, `border-radius`, `bottom`, `box-shadow`, `display`, `gap`, `left`, `min-width`, `padding`, `position`, `transform`, `z-index` |
| 1498 | `#workspace-editbar>div` | — | Reusable panel/workspace Library | T | `display`, `flex`, `flex-direction`, `min-width` |
| 1498 | `#workspace-editbar b` | — | Reusable panel/workspace Library | T | `font-size` |
| 1498 | `#workspace-editbar span` | — | Reusable panel/workspace Library | T | `color`, `font-size` |
| 1499 | `#library-overlay` | @media(max-width:760px) | Reusable panel/workspace Library | T | `padding` |
| 1499 | `#library-screen` | @media(max-width:760px) | Reusable panel/workspace Library | T | `height`, `width` |
| 1499 | `.library-sidebar` | @media(max-width:760px) | Reusable panel/workspace Library | T | `width` |
| 1499 | `.library-scope` | @media(max-width:760px) | Reusable panel/workspace Library | T | `grid-template-columns` |
| 1499 | `.library-top` | @media(max-width:760px) | Reusable panel/workspace Library | T | `align-items`, `flex-wrap` |
| 1499 | `.library-page-title` | @media(max-width:760px) | Reusable panel/workspace Library | T | `flex-basis` |
| 1499 | `.library-top-action` | @media(max-width:760px) | Reusable panel/workspace Library | T | `width` |
| 1499 | `.library-top-action .btn` | @media(max-width:760px) | Reusable panel/workspace Library | T | `width` |
| 1499 | `.library-grid` | @media(max-width:760px) | Reusable panel/workspace Library | T | `grid-template-columns` |
| 1503 | `*` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation-duration`, `animation-iteration-count`, `scroll-behavior`, `transition-duration` |
| 1503 | `*::before` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation-duration`, `animation-iteration-count`, `scroll-behavior`, `transition-duration` |
| 1503 | `*::after` | @media (prefers-reduced-motion:reduce) | Reusable panel/workspace Library | T | `animation-duration`, `animation-iteration-count`, `scroll-behavior`, `transition-duration` |
| 1507 | `.segmented` | — | Effects browser | T | `background`, `border-radius`, `display`, `gap`, `padding` |
| 1508 | `.segmented button` | — | Effects browser | T | `border-radius`, `color`, `font-size`, `font-weight`, `height`, `padding`, `transition`, `white-space` |
| 1509 | `.segmented button:hover` | — | Effects browser | T | `color` |
| 1509 | `.segmented button.on` | — | Effects browser | T | `background`, `box-shadow`, `color` |
| 1509 | `.segmented button:focus-visible` | — | Effects browser | T | `box-shadow`, `outline` |
| 1510 | `.fxb` | — | Effects browser | T | `display`, `flex-direction`, `font-size`, `height`, `min-height` |
| 1512 | `.fxb-searchrow` | — | Effects browser | T | `flex`, `padding`, `position` |
| 1513 | `.fxb-searchicon` | — | Effects browser | T | `fill`, `height`, `left`, `pointer-events`, `position`, `stroke`, `stroke-linecap`, `stroke-width`, `top`, `translate`, `width` |
| 1514 | `.fxb-search` | — | Effects browser | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `height`, `padding`, `transition`, `width` |
| 1515 | `.fxb-search::placeholder` | — | Effects browser | T | `color` |
| 1516 | `.fxb-search:focus` | — | Effects browser | T | `box-shadow`, `outline` |
| 1517 | `.fxb-search::-webkit-search-cancel-button` | — | Effects browser | T | `-webkit-appearance` |
| 1521 | `.fxb-body` | — | Effects browser | T | `display`, `flex`, `min-height` |
| 1522 | `.fxb-rail` | — | Effects browser | T | `border-right`, `display`, `flex`, `flex-direction`, `gap`, `overflow`, `padding`, `scrollbar-width`, `width` |
| 1523 | `.fxb-rail::-webkit-scrollbar` | — | Effects browser | T | `display` |
| 1524 | `.fxb-cat` | — | Effects browser | T | `border-radius`, `color`, `display`, `font-size`, `height`, `overflow`, `padding`, `text-align`, `text-overflow`, `transition`, `white-space`, `width` |
| 1525 | `.fxb-cat:hover` | — | Effects browser | T | `color` |
| 1526 | `.fxb-cat.on` | — | Effects browser | T | `background`, `box-shadow`, `color` |
| 1527 | `.fxb-cat:focus-visible` | — | Effects browser | T | `box-shadow`, `outline` |
| 1529 | `.fxb-cat` | — | Effects browser | T | `flex-shrink` |
| 1529 | `.fxb-row` | — | Effects browser | T | `flex-shrink` |
| 1529 | `.fxb-sec` | — | Effects browser | T | `flex-shrink` |
| 1529 | `.fxb-empty` | — | Effects browser | T | `flex-shrink` |
| 1530 | `.fxb-list` | — | Effects browser | T | `display`, `flex`, `flex-direction`, `min-height`, `overflow`, `padding`, `scrollbar-width` |
| 1531 | `.fxb-sec` | — | Effects browser | T | `border-top`, `color`, `font-size`, `font-weight`, `height`, `letter-spacing`, `margin`, `padding` |
| 1532 | `.fxb-sec:first-child` | — | Effects browser | T | `padding-top` |
| 1533 | `.fxb-row` | — | Effects browser | T | `align-items`, `border-radius`, `color`, `cursor`, `display`, `font-size`, `gap`, `height`, `padding`, `text-align`, `transition` |
| 1534 | `.fxb-row:hover` | — | Effects browser | T | `background`, `color` |
| 1534 | `.fxb-row:focus-within` | — | Effects browser | T | `background`, `color` |
| 1534 | `.fxb-row.on` | — | Effects browser | T | `background`, `color` |
| 1535 | `.fxb-row:active` | — | Effects browser | T | `background`, `cursor` |
| 1536 | `.fxb-row:focus-visible` | — | Effects browser | T | `box-shadow`, `outline` |
| 1537 | `.fxb-label` | — | Effects browser | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 1538 | `.fxb-pick` | — | Effects browser | T | `align-self`, `color`, `cursor`, `font`, `text-align` |
| 1539 | `.fxb-pick:active` | — | Effects browser | T | `cursor` |
| 1540 | `.fxb-pick:focus-visible` | — | Effects browser | T | `border-radius`, `outline`, `outline-offset` |
| 1541 | `.fxb-add` | — | Effects browser | T | `border-radius`, `color`, `display`, `flex`, `height`, `opacity`, `place-items`, `transition`, `width` |
| 1542 | `.fxb-add svg` | — | Effects browser | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-width`, `width` |
| 1543 | `.fxb-row:hover .fxb-add` | — | Effects browser | T | `opacity` |
| 1543 | `.fxb-row:focus-within .fxb-add` | — | Effects browser | T | `opacity` |
| 1544 | `.fxb-add:focus-visible` | — | Effects browser | T | `outline`, `outline-offset` |
| 1545 | `.fxb-add:hover` | — | Effects browser | T | `background`, `color` |
| 1546 | `.fxb-empty` | — | Effects browser | T | `color`, `padding`, `text-align`, `text-wrap` |
| 1548 | `.fxb` | — | Effects browser | T | `container-type` |
| 1550 | `.fxb-body` | @container (max-width:230px) | Effects browser | T | `flex-direction` |
| 1551 | `.fxb-rail` | @container (max-width:230px) | Effects browser | T | `border-bottom`, `border-right`, `flex-direction`, `flex-wrap`, `gap`, `padding`, `width` |
| 1552 | `.fxb-cat` | @container (max-width:230px) | Effects browser | T | `font-size`, `height`, `padding`, `width` |
| 1557 | `#composition-zoom` | — | Effects browser | T | `appearance`, `background-image`, `background-position`, `background-repeat`, `background-size`, `padding-right` |
| 1557 | `#preview-controls select` | — | Effects browser | T | `appearance`, `background-image`, `background-position`, `background-repeat`, `background-size`, `padding-right` |
| 1568 | `:is(#library-screen,#projects-screen,.modal) .btn:not(.pri)` | — | Effects browser | T | `box-shadow` |
| 1569 | `.modal .field` | — | Effects browser | T | `box-shadow` |
| 1570 | `.modal .field:focus-within` | — | Effects browser | T | `box-shadow` |
| 1571 | `#palette .field:focus-within` | — | Effects browser | T | `box-shadow` |
| 1572 | `:is(.panel-library-search,.font-search,.fxb-search)` | — | Effects browser | T | `box-shadow` |
| 1573 | `.panel-library-search:focus-within` | — | Effects browser | T | `box-shadow` |
| 1573 | `.font-search:focus-visible` | — | Effects browser | T | `box-shadow` |
| 1573 | `.fxb-search:focus-visible` | — | Effects browser | T | `box-shadow` |
| 1574 | `:is(.ps-navbtn,.ps-view button,.library-navbtn,.library-scope button,.panel-library-open):focus-visible` | — | Effects browser | T | `box-shadow`, `outline` |
| 1575 | `#library-screen .segmented button:focus-visible` | — | Effects browser | T | `box-shadow`, `outline` |
| 1577 | `.agent-inline-prompt` | — | Effects browser | T | `-webkit-user-select`, `user-select` |
| 1577 | `.agent-inline-prompt *` | — | Effects browser | T | `-webkit-user-select`, `user-select` |
| 1578 | `.agent-inline-prompt` | — | Effects browser | T | `cursor`, `overflow-wrap`, `white-space` |
| 1579 | `.agent-inline-prompt:empty::before` | — | Effects browser | T | `color`, `content`, `pointer-events` |
| 1580 | `.agent-inline-attachment` | — | Effects browser | T | `--attachment-padding`, `--attachment-radius`, `-webkit-user-select`, `align-items`, `background`, `border-radius`, `box-sizing`, `color`, `cursor`, `display`, `font`, `gap`, `height`, `margin`, `max-width`, `padding`, `user-select`, `vertical-align` |
| 1581 | `.agent-inline-attachment>span:last-child` | — | Effects browser | T | `line-height`, `overflow`, `padding-right`, `text-overflow`, `white-space` |
| 1582 | `.agent-inline-attachment-type` | — | Effects browser | T | `align-items`, `background`, `border-radius`, `box-sizing`, `color`, `display`, `flex`, `font-size`, `height`, `justify-content`, `line-height`, `min-width`, `padding` |
| 1583 | `.agent-inline-attachment img` | — | Effects browser | T | `aspect-ratio`, `border-radius`, `height`, `object-fit`, `width` |
| 1584 | `.agent-inline-attachment.is-sent` | — | Effects browser | T | `-webkit-user-select`, `background`, `border`, `cursor`, `user-select` |
| 1585 | `.agent-inline-attachment.is-sent:hover>span:last-child` | — | Effects browser | T | `color`, `text-decoration`, `text-underline-offset` |
| 1586 | `.agent-inline-attachment.is-sent:focus-visible` | — | Effects browser | T | `outline`, `outline-offset` |
| 1587 | `.agent-inline-attachment[draggable="true"]` | — | Effects browser | T | `cursor` |
| 1588 | `.agent-inline-attachment.is-range-selected` | — | Effects browser | T | `background`, `outline` |
| 1588 | `.agent-inline-attachment.is-selected` | — | Effects browser | T | `background`, `outline` |
| 1590 | `.asset-card.is-cloud .asset-preview` | — | Effects browser | T | `background` |
| 1591 | `.asset-card.is-cloud .asset-offline` | — | Effects browser | T | `background`, `color` |
| 1592 | `.asset-card.is-cloud .asset-copy small.asset-missing` | — | Effects browser | T | `color` |
| 1593 | `.asset-card.is-cloud .asset-actions` | — | Effects browser | T | `opacity` |
| 1594 | `.asset-download:disabled` | — | Effects browser | T | `cursor`, `opacity` |
| 1596 | `.asset-selection-actions` | — | Effects browser | T | `align-items`, `color`, `display`, `font-size`, `justify-content`, `padding` |
| 1597 | `.asset-selection-actions button` | — | Effects browser | T | `align-items`, `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `height`, `justify-content`, `width` |
| 1598 | `.asset-selection-actions button:hover` | — | Effects browser | T | `background`, `color` |
| 1599 | `.asset-selection-actions button:focus-visible` | — | Effects browser | T | `outline`, `outline-offset` |
| 1600 | `.asset-selection-actions svg` | — | Effects browser | T | `height`, `width` |
| 1601 | `.asset-selection-box` | — | Effects browser | T | `background`, `border`, `pointer-events`, `position`, `z-index` |

### `apps/desktop/css/publish.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 12 | `.publish-modal .mb` | — | Extension publishing sheet | T | `gap`, `padding` |
| 13 | `.pub-sheet` | — | Extension publishing sheet | T | `display`, `flex-direction` |
| 14 | `.pub-sheet:focus` | — | Extension publishing sheet | T | `outline` |
| 15 | `.pub-head` | — | Extension publishing sheet | T | `background`, `padding`, `position`, `top`, `z-index` |
| 16 | `.pub-head h2` | — | Extension publishing sheet | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `text-wrap` |
| 17 | `.pub-head p` | — | Extension publishing sheet | T | `color`, `font-size`, `line-height`, `margin-top`, `max-width`, `overflow-wrap`, `text-wrap` |
| 18 | `.pub-head b` | — | Extension publishing sheet | T | `color`, `font-weight` |
| 19 | `.pub-form.sg-column` | — | Extension publishing sheet | T | `display`, `flex-direction`, `gap`, `margin`, `max-width`, `padding`, `width` |
| 20 | `.pub-section` | — | Extension publishing sheet | T | `display`, `flex-direction` |
| 21 | `.pub-sheet .pub-title` | — | Extension publishing sheet | T | `align-items`, `box-sizing`, `color`, `display`, `font-size`, `font-weight`, `height`, `letter-spacing`, `line-height`, `margin`, `padding` |
| 22 | `.pub-group.sg-group` | — | Extension publishing sheet | T | `margin` |
| 23 | `.pub-group>.settings-row.pub-row` | — | Extension publishing sheet | T | `min-height`, `padding` |
| 24 | `.pub-row .settings-copy` | — | Extension publishing sheet | T | `padding` |
| 25 | `.pub-row .settings-copy>span` | — | Extension publishing sheet | T | `max-width` |
| 26 | `.pub-row.is-stacked` | — | Extension publishing sheet | T | `align-items`, `flex-direction`, `gap` |
| 27 | `.pub-row.is-stacked .settings-copy>span` | — | Extension publishing sheet | T | `max-width` |
| 28 | `.pub-row .settings-copy>b` | — | Extension publishing sheet | T | `align-items`, `display`, `gap`, `min-width` |
| 29 | `.pub-path` | — | Extension publishing sheet | T | `font`, `overflow-wrap` |
| 30 | `.pub-path>i` | — | Extension publishing sheet | T | `color`, `font`, `font-style` |
| 31 | `.pub-row .settings-copy>.pub-problem` | — | Extension publishing sheet | T | `color` |
| 35 | `.pub-input` | — | Extension publishing sheet | T | `background`, `border`, `border-radius`, `color`, `flex`, `font-size`, `height`, `padding`, `width` |
| 35 | `.pub-select` | — | Extension publishing sheet | T | `background`, `border`, `border-radius`, `color`, `flex`, `font-size`, `height`, `padding`, `width` |
| 36 | `.pub-select` | — | Extension publishing sheet | T | `width` |
| 37 | `.pub-input.is-short` | — | Extension publishing sheet | T | `font-variant-numeric`, `width` |
| 38 | `.pub-select` | — | Extension publishing sheet | T | `-webkit-appearance`, `appearance`, `font-weight` |
| 39 | `.pub-notes` | — | Extension publishing sheet | T | `display`, `height`, `line-height`, `min-height`, `padding`, `resize`, `width` |
| 40 | `.pub-input::placeholder` | — | Extension publishing sheet | T | `color` |
| 41 | `.pub-input:focus-visible` | — | Extension publishing sheet | T | `box-shadow`, `outline` |
| 41 | `.pub-select:focus-visible` | — | Extension publishing sheet | T | `box-shadow`, `outline` |
| 42 | `.pub-input[aria-invalid="true"]` | — | Extension publishing sheet | T | `box-shadow` |
| 43 | `.pub-input:disabled` | — | Extension publishing sheet | T | `color` |
| 43 | `.pub-select:disabled` | — | Extension publishing sheet | T | `color` |
| 45 | `:root:not([data-theme="dark"]) .publish-modal :is(.pub-input,.pub-select)` | — | Extension publishing sheet | T | `background` |
| 49 | `.pub-icon` | — | Extension publishing sheet | T | `align-items`, `display`, `flex`, `gap` |
| 50 | `.pub-icon-drop` | — | Extension publishing sheet | T | `align-items`, `background`, `border-radius`, `box-shadow`, `display`, `gap`, `min-height`, `padding`, `position`, `transition` |
| 51 | `.pub-icon-drop:not(.has-icon)` | — | Extension publishing sheet | T | `padding` |
| 52 | `.pub-icon-drop::after` | — | Extension publishing sheet | T | `border`, `border-radius`, `content`, `inset`, `pointer-events`, `position`, `transition` |
| 53 | `.pub-icon-drop.has-icon::after` | — | Extension publishing sheet | T | `border-color`, `border-style` |
| 54 | `.pub-icon-drop.is-over` | — | Extension publishing sheet | T | `background` |
| 55 | `.pub-icon-drop.is-over::after` | — | Extension publishing sheet | T | `border-color`, `border-style` |
| 56 | `.pub-icon-empty` | — | Extension publishing sheet | T | `align-items`, `align-self`, `background`, `border`, `border-radius`, `color`, `display`, `flex`, `flex-direction`, `font`, `gap`, `justify-content`, `min-height`, `padding`, `text-align` |
| 57 | `.pub-icon-empty:hover:not(:disabled)` | — | Extension publishing sheet | T | `color` |
| 57 | `.pub-icon-drop.is-over .pub-icon-empty` | — | Extension publishing sheet | T | `color` |
| 58 | `.pub-icon-empty:focus-visible` | — | Extension publishing sheet | T | `box-shadow`, `outline` |
| 59 | `.pub-icon-empty svg` | — | Extension publishing sheet | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 60 | `.pub-icon-empty>span` | — | Extension publishing sheet | T | `display`, `flex-direction`, `font-size`, `gap`, `line-height` |
| 60 | `.pub-icon-copy` | — | Extension publishing sheet | T | `display`, `flex-direction`, `font-size`, `gap`, `line-height` |
| 61 | `.pub-icon-empty b` | — | Extension publishing sheet | T | `color`, `font-weight` |
| 61 | `.pub-icon-copy b` | — | Extension publishing sheet | T | `color`, `font-weight` |
| 62 | `.pub-icon-copy` | — | Extension publishing sheet | T | `color`, `flex`, `min-width` |
| 63 | `.pub-icon-drop .pub-icon-preview` | — | Extension publishing sheet | T | `border-radius`, `height`, `width` |
| 64 | `:root:not([data-theme="dark"]) .publish-modal .pub-icon-drop` | — | Extension publishing sheet | T | `background` |
| 67 | `.pub-crop` | — | Extension publishing sheet | T | `align-items`, `display`, `flex`, `gap`, `min-width` |
| 68 | `.pub-crop-view` | — | Extension publishing sheet | T | `-webkit-user-select`, `background`, `border-radius`, `box-shadow`, `cursor`, `flex`, `overflow`, `position`, `touch-action`, `user-select` |
| 69 | `.pub-crop-view.is-dragging` | — | Extension publishing sheet | T | `cursor` |
| 70 | `.pub-crop-view:focus-visible` | — | Extension publishing sheet | T | `box-shadow`, `outline` |
| 71 | `.pub-crop-view>img` | — | Extension publishing sheet | T | `left`, `max-width`, `pointer-events`, `position`, `top`, `transform-origin`, `transition` |
| 72 | `.pub-crop-grid` | — | Extension publishing sheet | T | `background`, `border-radius`, `inset`, `opacity`, `pointer-events`, `position`, `transition` |
| 73 | `.pub-crop-view.is-dragging .pub-crop-grid` | — | Extension publishing sheet | T | `opacity` |
| 74 | `.pub-crop-side` | — | Extension publishing sheet | T | `align-items`, `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 75 | `.pub-crop-zoom` | — | Extension publishing sheet | T | `align-items`, `color`, `display`, `gap`, `max-width`, `width` |
| 76 | `.pub-crop-zoom input` | — | Extension publishing sheet | T | `accent-color`, `flex`, `min-width` |
| 77 | `.pub-crop-zoom svg` | — | Extension publishing sheet | T | `fill`, `flex`, `height`, `stroke`, `stroke-width`, `width` |
| 78 | `.pub-crop-view>img` | @media (prefers-reduced-motion:reduce) | Extension publishing sheet | T | `transition` |
| 78 | `.pub-crop-grid` | @media (prefers-reduced-motion:reduce) | Extension publishing sheet | T | `transition` |
| 79 | `.pub-icon-preview` | — | Extension publishing sheet | T | `border-radius`, `height`, `object-fit`, `outline`, `outline-offset`, `width` |
| 80 | `.pub-file` | — | Extension publishing sheet | T | `height`, `opacity`, `pointer-events`, `position`, `width` |
| 81 | `.pub-sheet .sg-group .btn` | — | Extension publishing sheet | T | `border-radius`, `flex`, `height`, `padding` |
| 84 | `.pub-state` | — | Extension publishing sheet | T | `color`, `flex`, `height`, `margin-right`, `width` |
| 85 | `.pub-check-mark` | — | Extension publishing sheet | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 88 | `.pub-sandbox-actions` | — | Extension publishing sheet | T | `align-items`, `display`, `gap` |
| 89 | `.pub-sandbox-problems` | — | Extension publishing sheet | T | `display`, `flex-direction`, `gap`, `list-style`, `margin`, `padding` |
| 90 | `.pub-sandbox-line` | — | Extension publishing sheet | T | `-webkit-user-select`, `color`, `font-size`, `line-height`, `overflow-wrap`, `padding-left`, `position`, `text-wrap`, `user-select`, `white-space` |
| 91 | `.pub-sandbox-line::before` | — | Extension publishing sheet | T | `background`, `border-radius`, `content`, `height`, `left`, `position`, `top`, `width` |
| 93 | `.pub-terms` | — | Extension publishing sheet | T | `color`, `font-size`, `line-height`, `text-wrap` |
| 94 | `.pub-error` | — | Extension publishing sheet | T | `color`, `font-size`, `line-height`, `margin-top`, `overflow-wrap`, `text-wrap` |
| 95 | `.pub-foot` | — | Extension publishing sheet | T | `align-items`, `background`, `border-top`, `bottom`, `display`, `gap`, `justify-content`, `padding`, `position`, `z-index` |
| 96 | `.pub-foot>:first-child` | — | Extension publishing sheet | T | `flex`, `min-width` |
| 97 | `.pub-status` | — | Extension publishing sheet | T | `color`, `font-size`, `font-variant-numeric`, `line-height`, `padding-left` |
| 98 | `.pub-foot .btn` | — | Extension publishing sheet | T | `border-radius`, `height`, `padding` |
| 99 | `.publish-modal .btn.pri:disabled` | — | Extension publishing sheet | T | `background`, `color`, `opacity` |
| 100 | `.pub-sheet .btn` | — | Extension publishing sheet | T | `-webkit-user-select`, `cursor`, `user-select` |
| 100 | `.pub-sheet label` | — | Extension publishing sheet | T | `-webkit-user-select`, `cursor`, `user-select` |
| 103 | `.st-kv.is-own` | — | Extension publishing sheet | T | `align-items` |
| 104 | `.st-kv.is-own>b` | — | Extension publishing sheet | T | `flex` |
| 105 | `.st-kv.is-own .st-var-req` | — | Extension publishing sheet | T | `margin` |
| 107 | `.st-link.st-withdraw-open` | — | Extension publishing sheet | T | `color`, `flex`, `font-size`, `line-height`, `text-decoration` |
| 108 | `.st-link.st-withdraw-open:hover` | — | Extension publishing sheet | T | `color` |
| 109 | `.st-withdraw-ask` | — | Extension publishing sheet | T | `color`, `text-wrap` |
| 110 | `.st-withdraw-actions` | — | Extension publishing sheet | T | `display`, `flex`, `gap` |
| 111 | `.st-withdraw-actions .btn` | — | Extension publishing sheet | T | `border-radius`, `height`, `padding` |
| 112 | `.st-kv>b>.st-change-why` | — | Extension publishing sheet | T | `color`, `display`, `font`, `font-style`, `margin-top` |

### `apps/desktop/css/settings.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 12 | `#settings-screen` | — | Settings sheet, sidebar, extension settings | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding` |
| 12 | `.export-modal` | — | Settings sheet, sidebar, extension settings | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding` |
| 12 | `.account-modal` | — | Settings sheet, sidebar, extension settings | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding` |
| 12 | `.publish-modal` | — | Settings sheet, sidebar, extension settings | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding` |
| 12 | `.transcription-modal` | — | Settings sheet, sidebar, extension settings | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding` |
| 25 | `#settings-screen` | — | Settings sheet, sidebar, extension settings | T | `background`, `display`, `inset`, `min-height`, `padding`, `position`, `z-index` |
| 28 | `#settings-screen.on` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 29 | `#settings-screen:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `outline` |
| 31 | `body:has(#settings-screen.on) #body` | — | Settings sheet, sidebar, extension settings | T | `visibility` |
| 31 | `body:has(#settings-screen.on) #status` | — | Settings sheet, sidebar, extension settings | T | `visibility` |
| 31 | `body:has(#settings-screen.on) #projects-screen` | — | Settings sheet, sidebar, extension settings | T | `visibility` |
| 34 | `.sg-sidebar` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex`, `flex-direction`, `gap`, `padding`, `width` |
| 35 | `.sg-search` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `flex`, `gap`, `height`, `padding` |
| 36 | `.sg-search svg` | — | Settings sheet, sidebar, extension settings | T | `flex`, `height`, `width` |
| 37 | `.sg-search input` | — | Settings sheet, sidebar, extension settings | T | `color`, `flex`, `font-size`, `min-width` |
| 38 | `.sg-search input::placeholder` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 39 | `.sg-nav` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex`, `flex-direction`, `gap`, `min-height`, `overflow-y`, `position` |
| 40 | `.sg-navbtn` | — | Settings sheet, sidebar, extension settings | T | `position`, `z-index` |
| 41 | `.sg-nav-group` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 42 | `.sg-nav-title` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `padding` |
| 43 | `.sg-nav-empty` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `padding` |
| 45 | `.sg-navbtn` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `border`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `padding`, `text-align`, `transition`, `width` |
| 46 | `.sg-navbtn svg` | — | Settings sheet, sidebar, extension settings | T | `color`, `flex`, `height`, `transition`, `width` |
| 47 | `.sg-navbtn span` | — | Settings sheet, sidebar, extension settings | T | `min-width`, `overflow`, `text-overflow`, `white-space` |
| 48 | `.sg-navbtn:hover` | — | Settings sheet, sidebar, extension settings | T | `background`, `color` |
| 49 | `.sg-navbtn.on` | — | Settings sheet, sidebar, extension settings | T | `background`, `color` |
| 50 | `.sg-navbtn.on::after` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 51 | `.sg-navbtn.on svg` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 52 | `.sg-navbtn:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `outline` |
| 56 | `.sg-actions` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `flex`, `height`, `justify-content`, `padding` |
| 57 | `.sg-actions .btn` | — | Settings sheet, sidebar, extension settings | T | `height`, `padding` |
| 58 | `.sg-done` | — | Settings sheet, sidebar, extension settings | T | `color`, `flex`, `margin-top`, `width` |
| 59 | `.sg-done svg` | — | Settings sheet, sidebar, extension settings | T | `transform` |
| 60 | `.sg-done:hover` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 63 | `.sg-main` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-radius`, `box-shadow`, `display`, `flex`, `flex-direction`, `min-width`, `overflow`, `position` |
| 64 | `.sg-scroll` | — | Settings sheet, sidebar, extension settings | T | `container-type`, `flex`, `min-height`, `overflow`, `padding`, `scrollbar-gutter` |
| 65 | `.sg-column` | — | Settings sheet, sidebar, extension settings | T | `margin`, `max-width`, `width` |
| 68 | `.sg-page` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 69 | `.sg-page[hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 70 | `.sg-column [data-settings-search-hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 70 | `.sg-column [hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 73 | `.sg-column>.sg-page:not([data-settings-search-hidden])~.sg-page:not([data-settings-search-hidden])` | — | Settings sheet, sidebar, extension settings | T | `margin-top`, `padding-top`, `position` |
| 74 | `.sg-column>.sg-page:not([data-settings-search-hidden])~.sg-page:not([data-settings-search-hidden])::before` | — | Settings sheet, sidebar, extension settings | T | `background`, `content`, `height`, `left`, `position`, `right`, `top` |
| 75 | `.sg-search-results` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `margin`, `max-width`, `overflow-wrap`, `width` |
| 76 | `::highlight(settings-search)` | — | Settings sheet, sidebar, extension settings | T | `background`, `color` |
| 77 | `.sg-page:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `outline` |
| 78 | `.sg-heading` | — | Settings sheet, sidebar, extension settings | T | `margin` |
| 79 | `.sg-heading h2` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height` |
| 80 | `.sg-heading p` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 81 | `.sg-sections` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 81 | `.sg-column .settings-sections` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 82 | `.sg-column .settings-sections.is-scroll` | — | Settings sheet, sidebar, extension settings | T | `flex`, `overflow` |
| 83 | `.sg-section` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction` |
| 83 | `.sg-column .settings-section` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction` |
| 84 | `.sg-section-title` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `box-sizing`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `line-height`, `padding` |
| 84 | `.sg-column .settings-section-heading` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `box-sizing`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `line-height`, `padding` |
| 85 | `.sg-column .settings-section-heading>b` | — | Settings sheet, sidebar, extension settings | T | `font-weight` |
| 86 | `.sg-column .settings-section-heading>span` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-weight`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 87 | `.sg-column .settings-section-heading>span:empty` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 88 | `.settings-note` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `margin-top`, `padding`, `text-wrap` |
| 91 | `.sg-group` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `display`, `flex-direction`, `margin`, `overflow` |
| 92 | `.sg-group>*+*` | — | Settings sheet, sidebar, extension settings | T | `border-top` |
| 93 | `.sg-group>.settings-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 93 | `.settings-provider-fields>.settings-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 93 | `.sg-group>.settings-extension-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 94 | `.sg-group>.settings-row>:last-child` | — | Settings sheet, sidebar, extension settings | T | `flex` |
| 94 | `.settings-provider-fields>.settings-row>:last-child` | — | Settings sheet, sidebar, extension settings | T | `flex` |
| 95 | `.settings-copy` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex`, `flex-direction`, `gap`, `min-width`, `padding` |
| 95 | `.settings-extension-copy` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex`, `flex-direction`, `gap`, `min-width`, `padding` |
| 96 | `.settings-copy>b` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `font-weight`, `line-height` |
| 96 | `.settings-extension-name>b` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `font-weight`, `line-height` |
| 97 | `.settings-copy>span` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `text-wrap` |
| 97 | `.settings-extension-copy>span` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `text-wrap` |
| 98 | `.settings-provider-title` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap` |
| 99 | `.settings-provider>.btn` | — | Settings sheet, sidebar, extension settings | T | `min-width` |
| 100 | `.settings-dot` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-radius`, `height`, `transition`, `width` |
| 101 | `.settings-dot[data-state="connected"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 102 | `.settings-dot[data-state="connecting"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 103 | `.settings-dot[data-state="unavailable"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 106 | `.sg-column .settings-select` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `font-weight`, `height`, `padding` |
| 106 | `.sg-column .settings-input` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `font-weight`, `height`, `padding` |
| 106 | `.sg-column .pm-select.settings-select` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `font-weight`, `height`, `padding` |
| 106 | `.sg-column .pm-select.settings-appearance` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `font-size`, `font-weight`, `height`, `padding` |
| 107 | `.sg-column .pm-select.settings-select` | — | Settings sheet, sidebar, extension settings | T | `max-width`, `padding-right` |
| 107 | `.sg-column .pm-select.settings-appearance` | — | Settings sheet, sidebar, extension settings | T | `max-width`, `padding-right` |
| 109 | `:root:not([data-theme="dark"]) .sg-column .pm-select` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 110 | `:root:not([data-theme="dark"]) .sg-column .pm-select:hover` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 110 | `:root:not([data-theme="dark"]) .sg-column .pm-select[aria-expanded="true"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 112 | `.sg-group .btn` | — | Settings sheet, sidebar, extension settings | T | `border-radius` |
| 113 | `.sg-column .settings-input` | — | Settings sheet, sidebar, extension settings | T | `font-weight` |
| 114 | `.sg-column .settings-input.is-wide` | — | Settings sheet, sidebar, extension settings | T | `width` |
| 115 | `.sg-column .settings-input:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `box-shadow` |
| 115 | `.sg-column .pm-select:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `box-shadow` |
| 116 | `.sg-column .settings-field-pair` | — | Settings sheet, sidebar, extension settings | T | `gap` |
| 118 | `.sg-column input[type="checkbox"]` | — | Settings sheet, sidebar, extension settings | T | `-webkit-appearance`, `appearance`, `background`, `border`, `border-radius`, `box-shadow`, `cursor`, `display`, `flex`, `height`, `margin`, `place-items`, `transition`, `width` |
| 119 | `.sg-column input[type="checkbox"]::before` | — | Settings sheet, sidebar, extension settings | T | `-webkit-mask`, `background`, `content`, `height`, `mask`, `opacity`, `transform`, `transition`, `width` |
| 120 | `.sg-column input[type="checkbox"]:checked` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-color` |
| 121 | `.sg-column input[type="checkbox"]:checked::before` | — | Settings sheet, sidebar, extension settings | T | `opacity`, `transform` |
| 122 | `.sg-column input[type="checkbox"]:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `box-shadow`, `outline` |
| 123 | `.sg-column input[type="checkbox"]:disabled` | — | Settings sheet, sidebar, extension settings | T | `cursor`, `opacity` |
| 124 | `.sg-column .toggle` | — | Settings sheet, sidebar, extension settings | T | `flex` |
| 126 | `.sg-column .settings-row>.color-field` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `height`, `padding`, `width` |
| 129 | `.sg-column .settings-row` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `cursor`, `user-select` |
| 129 | `.sg-column label` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `cursor`, `user-select` |
| 129 | `.sg-column .toggle` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `cursor`, `user-select` |
| 129 | `.sg-column input[type="checkbox"]` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `cursor`, `user-select` |
| 130 | `.sg-column input` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `user-select` |
| 130 | `.sg-column textarea` | — | Settings sheet, sidebar, extension settings | T | `-webkit-user-select`, `user-select` |
| 133 | `.settings-provider-fields` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction` |
| 134 | `.settings-provider-fields>*+*` | — | Settings sheet, sidebar, extension settings | T | `border-top` |
| 135 | `.settings-provider-fields .settings-row>.pm-select` | — | Settings sheet, sidebar, extension settings | T | `max-width` |
| 136 | `.settings-provider-fields [role="status"]` | — | Settings sheet, sidebar, extension settings | T | `overflow-wrap` |
| 141 | `.settings-extension-root` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 142 | `.settings-extension-root>[hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 143 | `.sg-group>.settings-extension-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `gap`, `min-height`, `padding`, `transition` |
| 144 | `.sg-group>.settings-extension-row:hover` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 145 | `.settings-extension-open` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `color`, `display`, `flex`, `font`, `min-height`, `min-width`, `padding`, `text-align` |
| 146 | `.settings-extension-open:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `outline` |
| 147 | `.settings-extension-copy` | — | Settings sheet, sidebar, extension settings | T | `flex`, `min-width`, `padding` |
| 148 | `.settings-extension-name` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap`, `min-width` |
| 149 | `.settings-extension-name>b` | — | Settings sheet, sidebar, extension settings | T | `overflow`, `text-overflow`, `white-space` |
| 150 | `.settings-extension-dot` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-radius`, `flex`, `height`, `width` |
| 151 | `.settings-extension-dot[data-tone="ok"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 152 | `.settings-extension-dot[data-tone="warning"]` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 153 | `.settings-extension-tag` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-radius`, `color`, `flex`, `font-size`, `line-height`, `padding` |
| 154 | `.settings-extension-copy>span` | — | Settings sheet, sidebar, extension settings | T | `display`, `overflow`, `text-overflow`, `white-space` |
| 155 | `.settings-extension-copy>.settings-extension-error` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 155 | `.settings-copy>.settings-extension-error` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 156 | `.settings-copy>.is-replacement` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 157 | `.settings-extension-controls` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `flex`, `gap` |
| 158 | `.settings-extension-status` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `white-space` |
| 159 | `.settings-extension-chevron` | — | Settings sheet, sidebar, extension settings | T | `color`, `display`, `height`, `place-items`, `transition`, `width` |
| 160 | `.settings-extension-chevron svg` | — | Settings sheet, sidebar, extension settings | T | `height`, `width` |
| 161 | `.settings-extension-row:hover .settings-extension-chevron` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 162 | `.settings-extension-empty` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `padding` |
| 168 | `.settings-extension-detail` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap` |
| 169 | `.settings-extension-back` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `align-self`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `margin`, `padding`, `transition` |
| 170 | `.settings-extension-back svg` | — | Settings sheet, sidebar, extension settings | T | `height`, `transform`, `width` |
| 171 | `.settings-extension-back:hover` | — | Settings sheet, sidebar, extension settings | T | `background`, `color` |
| 172 | `.settings-extension-head` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `margin-bottom` |
| 173 | `.settings-extension-title` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap` |
| 174 | `.settings-extension-title .settings-extension-dot` | — | Settings sheet, sidebar, extension settings | T | `height`, `width` |
| 175 | `.settings-extension-title h3` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height` |
| 176 | `.settings-extension-title .settings-extension-tag` | — | Settings sheet, sidebar, extension settings | T | `margin-top` |
| 177 | `.settings-extension-head p` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 178 | `.settings-extension-detail .settings-copy>span` | — | Settings sheet, sidebar, extension settings | T | `overflow-wrap`, `white-space` |
| 179 | `.settings-extension-detail .settings-row>.btn.settings-extension-delete` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 180 | `.settings-extension-confirm` | — | Settings sheet, sidebar, extension settings | T | `font-size`, `line-height`, `padding` |
| 181 | `.settings-extension-confirm p` | — | Settings sheet, sidebar, extension settings | T | `color`, `margin`, `text-wrap` |
| 182 | `.settings-extension-confirm-actions` | — | Settings sheet, sidebar, extension settings | T | `display`, `gap` |
| 183 | `.settings-extension-confirm .btn.settings-extension-delete` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 186 | `.sg-column .toggle` | — | Settings sheet, sidebar, extension settings | T | `background`, `border-radius`, `box-shadow`, `flex`, `height`, `transition`, `width` |
| 187 | `.sg-column .toggle i` | — | Settings sheet, sidebar, extension settings | T | `background`, `box-shadow`, `height`, `left`, `top`, `transition`, `width` |
| 188 | `.sg-column .toggle.on` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 189 | `.sg-column .toggle.on i` | — | Settings sheet, sidebar, extension settings | T | `background`, `transform` |
| 190 | `.sg-column .toggle:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `box-shadow`, `outline` |
| 191 | `.sg-column .toggle:disabled` | — | Settings sheet, sidebar, extension settings | T | `opacity` |
| 195 | `.sg-group>.settings-row` | @container (max-width:28rem) | Settings sheet, sidebar, extension settings | T | `flex-wrap` |
| 196 | `.sg-group>.settings-row>:last-child` | @container (max-width:28rem) | Settings sheet, sidebar, extension settings | T | `margin-left` |
| 199 | `.sg-sidebar` | @media (max-width:760px) | Settings sheet, sidebar, extension settings | T | `width` |
| 200 | `.sg-scroll` | @media (max-width:760px) | Settings sheet, sidebar, extension settings | T | `padding` |
| 205 | `.sg-column .settings-background-control>.color-field` | — | Settings sheet, sidebar, extension settings | T | `background`, `border`, `border-radius`, `box-shadow`, `height`, `padding`, `width` |
| 206 | `.sg-section.is-dim` | — | Settings sheet, sidebar, extension settings | T | `opacity`, `transition` |
| 209 | `.settings-extension-root` | — | Settings sheet, sidebar, extension settings | T | `gap`, `min-width` |
| 210 | `.settings-extension-toolbar` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-direction`, `gap`, `margin` |
| 211 | `.settings-extension-search-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap` |
| 212 | `.settings-extension-search-row .settings-extension-search` | — | Settings sheet, sidebar, extension settings | T | `flex`, `height`, `min-width`, `text-align` |
| 213 | `.settings-extension-search-row .settings-select` | — | Settings sheet, sidebar, extension settings | T | `max-width`, `min-width` |
| 214 | `.settings-extension-filters` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-wrap`, `gap` |
| 215 | `.settings-extension-filters .btn` | — | Settings sheet, sidebar, extension settings | T | `background`, `color`, `height`, `padding` |
| 216 | `.settings-extension-filters .btn.on` | — | Settings sheet, sidebar, extension settings | T | `background`, `color` |
| 217 | `.settings-extension-list.sg-group` | — | Settings sheet, sidebar, extension settings | T | `background`, `display`, `height`, `margin`, `max-height`, `overflow`, `padding` |
| 218 | `.settings-extension-category` | — | Settings sheet, sidebar, extension settings | T | `min-width` |
| 219 | `.settings-extension-group` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `padding`, `text-align`, `width` |
| 220 | `.settings-extension-group:hover` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 221 | `.settings-extension-group::before` | — | Settings sheet, sidebar, extension settings | T | `content`, `flex`, `font-size`, `text-align`, `transform`, `width` |
| 222 | `.settings-extension-group[aria-expanded="false"]::before` | — | Settings sheet, sidebar, extension settings | T | `transform` |
| 223 | `.settings-extension-group-icon` | — | Settings sheet, sidebar, extension settings | T | `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 224 | `.settings-extension-group-icon svg` | — | Settings sheet, sidebar, extension settings | T | `height`, `width` |
| 225 | `.settings-extension-group-label` | — | Settings sheet, sidebar, extension settings | T | `min-width`, `overflow`, `text-overflow`, `white-space` |
| 226 | `.settings-extension-count` | — | Settings sheet, sidebar, extension settings | T | `border`, `border-radius`, `color`, `flex`, `font-size`, `font-variant-numeric`, `font-weight`, `line-height`, `min-width`, `padding`, `text-align` |
| 227 | `.settings-extension-group-body` | — | Settings sheet, sidebar, extension settings | T | `margin` |
| 228 | `.settings-extension-group-body[hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 229 | `.settings-extension-group-body .settings-extension-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `contain-intrinsic-size`, `content-visibility`, `display`, `gap`, `height`, `padding-right` |
| 230 | `.settings-extension-group-body .settings-extension-row:hover` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 231 | `.settings-extension-row .settings-extension-open` | — | Settings sheet, sidebar, extension settings | T | `height`, `min-height`, `padding-left` |
| 232 | `.settings-extension-open:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `border-radius`, `outline`, `outline-offset` |
| 232 | `.settings-extension-group:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `border-radius`, `outline`, `outline-offset` |
| 232 | `.settings-extension-list:focus-visible` | — | Settings sheet, sidebar, extension settings | T | `border-radius`, `outline`, `outline-offset` |
| 232 | `.settings-extension-toolbar :focus-visible` | — | Settings sheet, sidebar, extension settings | T | `border-radius`, `outline`, `outline-offset` |
| 233 | `.settings-extension-status.is-ok` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 233 | `.settings-extension-status.is-quiet` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 233 | `.settings-extension-status.is-setup` | — | Settings sheet, sidebar, extension settings | T | `color` |
| 234 | `.settings-extension-footer` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `padding-top` |
| 234 | `.settings-extension-message` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `padding-top` |
| 235 | `.settings-extension-message:empty` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 236 | `.settings-extension-empty` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `flex-direction`, `gap` |
| 238 | `dialog.settings-extension-detail` | — | Settings sheet, sidebar, extension settings | T | `--sg-group-radius`, `--sg-row`, `--sg-row-padding`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `gap`, `inset`, `margin`, `max-height`, `overflow-y`, `overscroll-behavior`, `padding`, `position`, `width` |
| 239 | `dialog.settings-extension-detail[open]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 240 | `dialog.settings-extension-detail:not([open])` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 240 | `dialog.settings-extension-detail[hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 241 | `dialog.settings-extension-detail::backdrop` | — | Settings sheet, sidebar, extension settings | T | `background` |
| 242 | `dialog.settings-extension-detail .settings-extension-back` | — | Settings sheet, sidebar, extension settings | T | `align-self`, `background`, `color`, `height`, `margin`, `padding` |
| 243 | `dialog.settings-extension-detail .settings-section-heading` | — | Settings sheet, sidebar, extension settings | T | `color`, `display`, `font-size`, `gap`, `padding-bottom` |
| 244 | `dialog.settings-extension-detail .settings-row` | — | Settings sheet, sidebar, extension settings | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 245 | `dialog.settings-extension-detail :focus-visible` | — | Settings sheet, sidebar, extension settings | T | `outline`, `outline-offset` |
| 247 | `.settings-extension-search-row` | @container (max-width:460px) | Settings sheet, sidebar, extension settings | T | `flex-wrap` |
| 248 | `.settings-extension-search-row .settings-extension-search` | @container (max-width:460px) | Settings sheet, sidebar, extension settings | T | `flex-basis` |
| 249 | `.settings-extension-controls` | @container (max-width:460px) | Settings sheet, sidebar, extension settings | T | `gap` |
| 250 | `.settings-extension-row .settings-extension-status` | @container (max-width:460px) | Settings sheet, sidebar, extension settings | T | `display` |
| 251 | `.settings-extension-row .settings-extension-tag` | @container (max-width:460px) | Settings sheet, sidebar, extension settings | T | `display` |
| 254 | `#settings-screen .sg-sidebar` | @media(max-width:620px) | Settings sheet, sidebar, extension settings | T | `padding-inline`, `width` |
| 255 | `#settings-screen .sg-scroll` | @media(max-width:620px) | Settings sheet, sidebar, extension settings | T | `padding-inline` |
| 256 | `dialog.settings-extension-detail` | @media(max-width:620px) | Settings sheet, sidebar, extension settings | T | `padding` |
| 258 | `.settings-extension-root>.settings-section>.settings-section-heading` | — | Settings sheet, sidebar, extension settings | T | `justify-content` |
| 260 | `.settings-extension-footer` | — | Settings sheet, sidebar, extension settings | T | `display`, `flex-wrap`, `gap`, `justify-content` |
| 262 | `.settings-extension-kinds` | — | Settings sheet, sidebar, extension settings | T | `color`, `font-size`, `font-weight`, `max-width`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 264 | `.settings-extension-group.is-subgroup` | — | Settings sheet, sidebar, extension settings | T | `font-weight`, `height`, `padding-left` |
| 265 | `.settings-extension-subgroup-body` | — | Settings sheet, sidebar, extension settings | T | `margin` |
| 266 | `.settings-extension-subgroup-body[hidden]` | — | Settings sheet, sidebar, extension settings | T | `display` |
| 267 | `.settings-extension-subgroup-body .settings-extension-open` | — | Settings sheet, sidebar, extension settings | T | `padding-left` |

### `apps/desktop/css/store.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 16 | `#projects-screen .ps-main>[hidden]` | — | Store browsing, install/update detail | T | `display` |
| 17 | `.ps-store` | — | Store browsing, install/update detail | T | `display`, `flex`, `min-height` |
| 18 | `#store-screen` | — | Store browsing, install/update detail | T | `--ctl-h`, `--sg-control-radius`, `--sg-edge`, `--sg-edge-focus`, `--sg-edge-hot`, `--sg-group-edge`, `--sg-group-radius`, `--sg-row`, `--sg-row-padding`, `--st-inset`, `display`, `flex`, `flex-direction`, `min-height`, `min-width` |
| 32 | `#store-screen:focus` | — | Store browsing, install/update detail | T | `outline` |
| 35 | `.st-main` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `min-height`, `min-width` |
| 40 | `.st-head` | — | Store browsing, install/update detail | T | `display`, `flex-direction`, `gap`, `margin`, `max-width`, `width` |
| 41 | `.st-title` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height` |
| 43 | `.st-toolbar` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap` |
| 44 | `#store-screen .segmented` | — | Store browsing, install/update detail | T | `--segment-inset`, `background`, `border-radius`, `box-shadow`, `flex`, `gap`, `margin-right`, `padding` |
| 45 | `#store-screen .segmented button` | — | Store browsing, install/update detail | T | `border-radius`, `color`, `font-size`, `font-weight`, `height`, `padding`, `transition` |
| 46 | `#store-screen .segmented button:hover` | — | Store browsing, install/update detail | T | `color` |
| 47 | `#store-screen .segmented button.on` | — | Store browsing, install/update detail | T | `background`, `box-shadow`, `color` |
| 48 | `#store-screen .segmented button:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 50 | `.st-search` | — | Store browsing, install/update detail | T | `align-items`, `background`, `border-radius`, `color`, `display`, `gap`, `height`, `padding`, `transition`, `width` |
| 51 | `.st-search svg` | — | Store browsing, install/update detail | T | `flex`, `height`, `width` |
| 52 | `.st-search input` | — | Store browsing, install/update detail | T | `color`, `flex`, `font-size`, `min-width` |
| 53 | `.st-search input::placeholder` | — | Store browsing, install/update detail | T | `color` |
| 54 | `.st-search:focus-within` | — | Store browsing, install/update detail | T | `box-shadow` |
| 56 | `.st-add` | — | Store browsing, install/update detail | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `flex`, `font-size`, `font-weight`, `gap`, `height`, `padding` |
| 57 | `.st-add svg` | — | Store browsing, install/update detail | T | `color`, `height`, `width` |
| 58 | `.st-add:hover` | — | Store browsing, install/update detail | T | `background` |
| 59 | `.st-add:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 60 | `.st-search` | @container (max-width:40rem) | Store browsing, install/update detail | T | `flex`, `width` |
| 62 | `.st-scroll` | — | Store browsing, install/update detail | T | `container-type`, `flex`, `min-height`, `overflow`, `padding`, `position`, `scrollbar-gutter` |
| 63 | `.st-column` | — | Store browsing, install/update detail | T | `display`, `flex-direction`, `gap`, `margin`, `max-width`, `width` |
| 67 | `.st-thumb` | — | Store browsing, install/update detail | T | `background` |
| 72 | `.st-thumb` | — | Store browsing, install/update detail | T | `border-radius`, `display`, `flex`, `height`, `overflow`, `position`, `width` |
| 73 | `.st-thumb.is-hero` | — | Store browsing, install/update detail | T | `border-radius`, `height`, `width` |
| 82 | `.st-carousel` | — | Store browsing, install/update detail | T | `display`, `flex-direction`, `gap` |
| 83 | `.st-track` | — | Store browsing, install/update detail | T | `display`, `gap`, `overflow-x`, `overflow-y`, `overscroll-behavior-x`, `position`, `scroll-snap-type`, `scrollbar-width` |
| 86 | `.st-track::-webkit-scrollbar` | — | Store browsing, install/update detail | T | `display` |
| 91 | `.st-track` | — | Store browsing, install/update detail | T | `--st-fade`, `mask-image`, `transition` |
| 100 | `.st-track[data-overflow-start="1"]` | — | Store browsing, install/update detail | T | `--st-fade-start` |
| 101 | `.st-track[data-overflow-end="1"]` | — | Store browsing, install/update detail | T | `--st-fade-end` |
| 102 | `.st-track` | @media (prefers-reduced-motion:reduce) | Store browsing, install/update detail | T | `transition` |
| 107 | `.st-slide` | — | Store browsing, install/update detail | T | `--slide-chip`, `--slide-chip-hover`, `--slide-chip-shadow`, `--slide-icon-shadow`, `--slide-pad`, `background`, `border-radius`, `display`, `flex`, `isolation`, `overflow`, `padding`, `position`, `scroll-snap-align` |
| 114 | `:root[data-theme="dark"] .st-slide` | — | Store browsing, install/update detail | T | `--slide-chip`, `--slide-chip-hover`, `--slide-chip-shadow`, `--slide-icon-shadow` |
| 119 | `.st-slide-bg` | — | Store browsing, install/update detail | T | `background`, `inset`, `position`, `z-index` |
| 124 | `:root[data-theme="dark"] .st-slide-bg` | — | Store browsing, install/update detail | T | `background` |
| 125 | `.st-slide-body` | — | Store browsing, install/update detail | T | `align-items`, `display`, `flex`, `flex-direction`, `gap`, `min-width`, `text-align` |
| 126 | `.st-eyebrow` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `text-transform` |
| 127 | `.st-slide-title` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin-top`, `text-wrap` |
| 128 | `.st-slide-icons` | — | Store browsing, install/update detail | T | `align-items`, `display`, `flex`, `gap`, `margin`, `width` |
| 130 | `.st-slide-icon` | — | Store browsing, install/update detail | T | `align-items`, `background`, `border`, `display`, `flex`, `flex-direction`, `gap`, `min-width`, `padding`, `text-align`, `transform`, `transition` |
| 131 | `.st-slide-icon .st-thumb` | — | Store browsing, install/update detail | T | `border-radius`, `filter`, `height`, `width` |
| 133 | `.st-slide-name` | — | Store browsing, install/update detail | T | `-webkit-box-orient`, `-webkit-line-clamp`, `color`, `display`, `font-size`, `font-weight`, `line-height`, `max-width`, `overflow`, `text-wrap`, `transition` |
| 134 | `.st-slide-icon:hover .st-slide-name` | — | Store browsing, install/update detail | T | `opacity` |
| 135 | `.st-slide-icon:focus-visible` | — | Store browsing, install/update detail | T | `outline` |
| 136 | `.st-slide-icon:focus-visible .st-thumb` | — | Store browsing, install/update detail | T | `box-shadow` |
| 137 | `.st-explore` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `font-size`, `font-weight`, `height`, `padding`, `transition` |
| 138 | `.st-explore:hover` | — | Store browsing, install/update detail | T | `background` |
| 139 | `.st-explore:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 141 | `.st-carousel-bar` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `padding-left` |
| 142 | `.st-arrow` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `color`, `display`, `height`, `padding`, `place-items`, `transition`, `width` |
| 143 | `.st-arrow svg` | — | Store browsing, install/update detail | T | `height`, `transform`, `width` |
| 144 | `.st-arrow.is-next svg` | — | Store browsing, install/update detail | T | `transform` |
| 145 | `.st-arrow:hover:not(:disabled)` | — | Store browsing, install/update detail | T | `background`, `color` |
| 146 | `.st-arrow:disabled` | — | Store browsing, install/update detail | T | `cursor`, `opacity` |
| 147 | `.st-arrow:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 148 | `.st-dots` | — | Store browsing, install/update detail | T | `align-items`, `display`, `flex`, `gap` |
| 149 | `.st-dots button` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `height`, `padding`, `transition`, `width` |
| 150 | `.st-dots button[aria-selected="true"]` | — | Store browsing, install/update detail | T | `background`, `width` |
| 151 | `.st-dots button:hover:not([aria-selected="true"])` | — | Store browsing, install/update detail | T | `background` |
| 152 | `.st-dots button:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 153 | `.st-slide.is-skeleton` | — | Store browsing, install/update detail | T | `height`, `pointer-events`, `width` |
| 154 | `.st-slide.is-skeleton .st-skel-lines` | — | Store browsing, install/update detail | T | `align-self`, `flex`, `width` |
| 156 | `.st-slide` | @container (max-width:40rem) | Store browsing, install/update detail | T | `--slide-pad`, `flex-basis` |
| 157 | `.st-slide.is-skeleton` | @container (max-width:40rem) | Store browsing, install/update detail | T | `width` |
| 158 | `.st-arrow` | @container (max-width:40rem) | Store browsing, install/update detail | T | `display` |
| 159 | `.st-slide-icons` | @container (max-width:40rem) | Store browsing, install/update detail | T | `gap` |
| 160 | `.st-slide-icon:nth-child(n+3)` | @container (max-width:40rem) | Store browsing, install/update detail | T | `display` |
| 166 | `.st-sec` | — | Store browsing, install/update detail | T | `display`, `flex-direction` |
| 167 | `.st-sec-head` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `height`, `margin-bottom`, `padding` |
| 170 | `.st-column>.st-sec-head` | — | Store browsing, install/update detail | T | `margin-bottom` |
| 171 | `.st-sec-head h3` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height` |
| 172 | `.st-sec-head .btn` | — | Store browsing, install/update detail | T | `border-radius`, `color`, `font-size`, `height`, `margin-left`, `margin-right`, `padding` |
| 173 | `.st-count` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-variant-numeric`, `line-height` |
| 174 | `.st-show-all` | — | Store browsing, install/update detail | T | `align-items`, `border`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `margin-left`, `margin-right`, `padding`, `transition` |
| 175 | `.st-show-all:hover` | — | Store browsing, install/update detail | T | `background`, `color` |
| 176 | `.st-show-all:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 178 | `.st-sec-title` | — | Store browsing, install/update detail | T | `align-items`, `box-sizing`, `color`, `display`, `font-size`, `font-weight`, `height`, `line-height`, `padding` |
| 181 | `.st-column.is-detail .st-sec-head` | — | Store browsing, install/update detail | T | `height`, `margin-bottom` |
| 182 | `.st-column.is-detail .st-sec-head h3` | — | Store browsing, install/update detail | T | `align-items`, `color`, `display`, `font-size`, `font-weight`, `letter-spacing`, `line-height` |
| 188 | `.st-cards` | — | Store browsing, install/update detail | T | `display`, `gap`, `grid-template-columns` |
| 189 | `.st-cards` | @container (max-width:40rem) | Store browsing, install/update detail | T | `grid-template-columns` |
| 190 | `.st-item` | — | Store browsing, install/update detail | T | `align-items`, `background`, `border-radius`, `display`, `gap`, `min-height`, `padding`, `position`, `transition` |
| 191 | `.st-item:hover` | — | Store browsing, install/update detail | T | `background` |
| 192 | `.st-item-open` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `inset`, `padding`, `position`, `z-index` |
| 193 | `.st-item-open:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 194 | `.st-item-icon` | — | Store browsing, install/update detail | T | `background`, `border-radius`, `display`, `flex`, `height`, `place-items`, `width` |
| 195 | `.st-item-icon .st-thumb` | — | Store browsing, install/update detail | T | `border-radius`, `height`, `width` |
| 196 | `.st-item-copy` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 197 | `.st-item-copy b` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `line-height`, `overflow`, `text-overflow`, `white-space` |
| 198 | `.st-item-line` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `overflow`, `text-overflow`, `white-space` |
| 200 | `.st-item-meta` | — | Store browsing, install/update detail | T | `color`, `display`, `font-size`, `line-height`, `margin-top`, `min-width`, `white-space` |
| 202 | `.st-item-meta` | — | Store browsing, install/update detail | T | `overflow` |
| 203 | `.st-item-meta>*` | — | Store browsing, install/update detail | T | `flex`, `min-width`, `overflow`, `text-overflow` |
| 204 | `.st-item-meta>.st-item-lineage` | — | Store browsing, install/update detail | T | `flex-shrink` |
| 204 | `.st-item-meta:not(:has(>.st-item-lineage))>:last-child` | — | Store browsing, install/update detail | T | `flex-shrink` |
| 205 | `.st-item-meta>*+*::before` | — | Store browsing, install/update detail | T | `color`, `content`, `font-weight`, `margin` |
| 208 | `.st-item-dev` | — | Store browsing, install/update detail | T | `background`, `border`, `color`, `font`, `padding`, `position`, `text-align`, `transition`, `white-space`, `z-index` |
| 209 | `.st-item-dev:hover` | — | Store browsing, install/update detail | T | `color` |
| 210 | `.st-item-dev:focus-visible` | — | Store browsing, install/update detail | T | `border-radius`, `box-shadow`, `outline` |
| 211 | `.st-item-action` | — | Store browsing, install/update detail | T | `align-items`, `display`, `flex`, `gap`, `position`, `z-index` |
| 212 | `.st-plus` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `color`, `display`, `height`, `padding`, `place-items`, `transition`, `width` |
| 213 | `.st-plus svg` | — | Store browsing, install/update detail | T | `height`, `width` |
| 214 | `button.st-plus:hover` | — | Store browsing, install/update detail | T | `background`, `color` |
| 215 | `.st-plus:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 216 | `.st-plus.is-done` | — | Store browsing, install/update detail | T | `background`, `color` |
| 217 | `.st-plus.is-done svg` | — | Store browsing, install/update detail | T | `height`, `width` |
| 218 | `.st-item.is-skeleton` | — | Store browsing, install/update detail | T | `pointer-events` |
| 219 | `.st-item.is-skeleton .st-item-icon` | — | Store browsing, install/update detail | T | `background` |
| 222 | `.st-install` | — | Store browsing, install/update detail | T | `border-radius`, `flex`, `height`, `padding` |
| 223 | `.st-installed` | — | Store browsing, install/update detail | T | `color`, `flex`, `font-size`, `font-weight`, `white-space` |
| 224 | `.st-item-action .st-installed` | — | Store browsing, install/update detail | T | `padding` |
| 227 | `.st-installed-host` | — | Store browsing, install/update detail | T | `margin`, `width` |
| 228 | `.st-installed-host .settings-extension-root` | — | Store browsing, install/update detail | T | `gap` |
| 229 | `.st-installed-host .settings-section-heading>b` | — | Store browsing, install/update detail | T | `display` |
| 231 | `.st-installed-host .sg-group` | — | Store browsing, install/update detail | T | `margin` |
| 234 | `.st-back` | — | Store browsing, install/update detail | T | `align-items`, `align-self`, `border`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `height`, `margin`, `padding`, `transition` |
| 235 | `.st-back svg` | — | Store browsing, install/update detail | T | `height`, `transform`, `width` |
| 236 | `.st-back:hover` | — | Store browsing, install/update detail | T | `background`, `color` |
| 237 | `.st-back:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 238 | `.st-detail-head` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `padding` |
| 239 | `.st-detail-copy` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `min-width` |
| 240 | `.st-detail-title` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap` |
| 241 | `.st-detail-title h2` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 243 | `.st-lede` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `margin-top`, `max-width`, `text-wrap` |
| 245 | `.st-access` | — | Store browsing, install/update detail | T | `align-items`, `color`, `display`, `flex-wrap`, `font-size`, `gap`, `line-height`, `margin-top` |
| 246 | `.st-access>span` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap` |
| 247 | `.st-access .is-warn` | — | Store browsing, install/update detail | T | `color` |
| 248 | `.st-access svg` | — | Store browsing, install/update detail | T | `height`, `width` |
| 249 | `.st-access-sep` | — | Store browsing, install/update detail | T | `color` |
| 252 | `.st-detail-actions` | — | Store browsing, install/update detail | T | `align-items`, `align-self`, `display`, `flex`, `gap`, `padding-top` |
| 253 | `.st-detail-actions .btn` | — | Store browsing, install/update detail | T | `border-radius`, `height`, `padding` |
| 254 | `.st-update-note` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `margin-top`, `max-width`, `padding`, `text-wrap` |
| 255 | `.st-update-note.is-warn` | — | Store browsing, install/update detail | T | `color` |
| 256 | `.st-update-note .st-link` | — | Store browsing, install/update detail | T | `margin-left` |
| 257 | `.st-about` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `max-width`, `padding`, `text-wrap` |
| 258 | `.st-card` | — | Store browsing, install/update detail | T | `background`, `border-radius`, `display`, `flex-direction`, `overflow` |
| 261 | `.st-kv` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 262 | `.st-kv>span` | — | Store browsing, install/update detail | T | `color`, `flex`, `font-size`, `line-height`, `width` |
| 263 | `.st-kv>span>i` | — | Store browsing, install/update detail | T | `color`, `font-style`, `margin-left` |
| 264 | `.st-kv>b` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `line-height`, `min-width`, `overflow-wrap` |
| 265 | `.st-kv.is-version>b` | — | Store browsing, install/update detail | T | `color` |
| 266 | `.st-kv-aside` | — | Store browsing, install/update detail | T | `color`, `margin-left` |
| 267 | `.st-file` | — | Store browsing, install/update detail | T | `align-items`, `border`, `color`, `display`, `font`, `gap`, `height`, `justify-content`, `padding`, `text-align`, `transition` |
| 268 | `.st-file:hover` | — | Store browsing, install/update detail | T | `background` |
| 269 | `.st-file svg` | — | Store browsing, install/update detail | T | `color`, `height`, `width` |
| 270 | `.st-note` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `margin-top`, `padding`, `text-wrap` |
| 273 | `.st-lineage` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `margin-top` |
| 274 | `.st-lineage-sep` | — | Store browsing, install/update detail | T | `color`, `margin` |
| 275 | `.st-link` | — | Store browsing, install/update detail | T | `border`, `color`, `font`, `font-weight`, `padding`, `text-decoration`, `text-decoration-color`, `text-underline-offset`, `transition` |
| 276 | `.st-link:hover` | — | Store browsing, install/update detail | T | `color` |
| 277 | `.st-link:focus-visible` | — | Store browsing, install/update detail | T | `border-radius`, `box-shadow`, `outline` |
| 280 | `.st-link.st-dev-link` | — | Store browsing, install/update detail | T | `color`, `font-weight`, `text-decoration-color` |
| 281 | `.st-link.st-dev-link:hover` | — | Store browsing, install/update detail | T | `text-decoration-color` |
| 285 | `#store-screen` | — | Store browsing, install/update detail | T | `--verified` |
| 286 | `:root[data-theme="dark"] #store-screen` | — | Store browsing, install/update detail | T | `--verified` |
| 287 | `.st-verified` | — | Store browsing, install/update detail | T | `display`, `flex`, `margin-left`, `vertical-align` |
| 288 | `.st-verified-seal` | — | Store browsing, install/update detail | T | `fill` |
| 289 | `.st-verified-tick` | — | Store browsing, install/update detail | T | `fill`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 290 | `.st-kv-dev` | — | Store browsing, install/update detail | T | `align-items`, `display` |
| 292 | `.st-dev-head .st-verified` | — | Store browsing, install/update detail | T | `margin-left`, `transform` |
| 297 | `.st-dev-head` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `padding` |
| 298 | `.acct-avatar.st-dev-avatar` | — | Store browsing, install/update detail | T | `--size`, `font-size`, `font-weight` |
| 299 | `.st-dev-head h2` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 302 | `.st-var` | — | Store browsing, install/update detail | T | `align-items`, `display`, `gap`, `min-height`, `padding` |
| 303 | `.st-var-copy` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 304 | `.st-var-copy>b` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `line-height` |
| 305 | `.st-var-copy>span` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `text-wrap` |
| 306 | `.st-var-copy code` | — | Store browsing, install/update detail | T | `color`, `font`, `margin-left` |
| 307 | `.st-var-req` | — | Store browsing, install/update detail | T | `background`, `border-radius`, `color`, `font-size`, `font-weight`, `line-height`, `margin-left`, `padding`, `vertical-align` |
| 308 | `#store-screen .st-var-input` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `color`, `flex`, `font-size`, `height`, `padding`, `width` |
| 309 | `#store-screen .st-var-input::placeholder` | — | Store browsing, install/update detail | T | `color` |
| 310 | `#store-screen .st-var-input:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 312 | `.st-sec-sub` | — | Store browsing, install/update detail | T | `color`, `font-weight`, `margin-left` |
| 315 | `.st-empty` | — | Store browsing, install/update detail | T | `align-items`, `display`, `flex-direction`, `gap`, `padding` |
| 316 | `.st-empty b` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `line-height` |
| 317 | `.st-empty span` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `max-width`, `text-wrap` |
| 318 | `.st-empty .btn` | — | Store browsing, install/update detail | T | `margin-top` |
| 319 | `.st-empty .st-coord` | — | Store browsing, install/update detail | T | `color`, `font` |
| 321 | `#store-screen .st-item` | — | Store browsing, install/update detail | T | `-webkit-user-select`, `user-select` |
| 321 | `#store-screen .st-carousel` | — | Store browsing, install/update detail | T | `-webkit-user-select`, `user-select` |
| 321 | `#store-screen .st-head` | — | Store browsing, install/update detail | T | `-webkit-user-select`, `user-select` |
| 321 | `#store-screen .st-empty-card` | — | Store browsing, install/update detail | T | `-webkit-user-select`, `user-select` |
| 326 | `.st-thumb.st-skel` | — | Store browsing, install/update detail | T | `background` |
| 326 | `.st-skel-lines i` | — | Store browsing, install/update detail | T | `background` |
| 327 | `.st-skel-lines` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `gap` |
| 328 | `.st-skel-lines i` | — | Store browsing, install/update detail | T | `border-radius`, `display`, `height`, `width` |
| 329 | `.st-skel-lines i+i` | — | Store browsing, install/update detail | T | `width` |
| 330 | `.is-skeleton .st-skel` | — | Store browsing, install/update detail | T | `animation` |
| 330 | `.is-skeleton .st-skel-lines` | — | Store browsing, install/update detail | T | `animation` |
| 332 | `.is-skeleton .st-skel` | @media (prefers-reduced-motion:reduce) | Store browsing, install/update detail | T | `animation` |
| 332 | `.is-skeleton .st-skel-lines` | @media (prefers-reduced-motion:reduce) | Store browsing, install/update detail | T | `animation` |
| 333 | `.st-failure` | — | Store browsing, install/update detail | T | `align-items`, `color`, `display`, `font-size`, `gap`, `line-height`, `padding` |
| 334 | `.st-source-error` | — | Store browsing, install/update detail | T | `padding` |
| 335 | `.st-more` | — | Store browsing, install/update detail | T | `align-self`, `margin` |
| 337 | `.st-toggle` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `color`, `flex`, `font-size`, `font-weight`, `height`, `min-width`, `padding`, `transition` |
| 338 | `.st-toggle[aria-pressed="true"]` | — | Store browsing, install/update detail | T | `color` |
| 339 | `.st-toggle:hover` | — | Store browsing, install/update detail | T | `background`, `color` |
| 340 | `.st-toggle:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 342 | `.st-detail-actions .st-installed` | — | Store browsing, install/update detail | T | `align-items`, `color`, `display`, `gap`, `height`, `margin`, `padding` |
| 343 | `.st-detail-actions .st-installed::before` | — | Store browsing, install/update detail | T | `-webkit-mask`, `background`, `content`, `height`, `mask`, `width` |
| 344 | `#store-screen .st-detail-secondary` | — | Store browsing, install/update detail | T | `color` |
| 345 | `#store-screen .st-detail-secondary:hover` | — | Store browsing, install/update detail | T | `color` |
| 346 | `.st-kv.is-file` | — | Store browsing, install/update detail | T | `align-items`, `justify-content`, `min-height`, `padding` |
| 347 | `.st-kv.is-file>b` | — | Store browsing, install/update detail | T | `font` |
| 348 | `.st-change` | — | Store browsing, install/update detail | T | `color`, `flex`, `font-size`, `font-weight` |
| 349 | `.st-change.is-added` | — | Store browsing, install/update detail | T | `color` |
| 350 | `.st-change.is-removed` | — | Store browsing, install/update detail | T | `color` |
| 351 | `.st-kv.is-withdrawn>span` | — | Store browsing, install/update detail | T | `color` |
| 351 | `.st-kv.is-withdrawn>b` | — | Store browsing, install/update detail | T | `color` |
| 352 | `.st-file.is-open svg` | — | Store browsing, install/update detail | T | `transform` |
| 353 | `.st-file svg` | — | Store browsing, install/update detail | T | `transition` |
| 354 | `.st-source` | — | Store browsing, install/update detail | T | `-webkit-user-select`, `background`, `color`, `font`, `margin`, `max-height`, `overflow`, `padding`, `tab-size`, `user-select`, `white-space` |
| 355 | `.st-source[aria-busy="true"]` | — | Store browsing, install/update detail | T | `min-height` |
| 359 | `#store-screen .btn:not(.pri)` | — | Store browsing, install/update detail | T | `box-shadow` |
| 360 | `#store-screen .btn.pri` | — | Store browsing, install/update detail | T | `background`, `box-shadow`, `color` |
| 361 | `#store-screen .btn.pri:hover` | — | Store browsing, install/update detail | T | `background` |
| 362 | `#store-screen .btn.pri:disabled` | — | Store browsing, install/update detail | T | `opacity` |
| 363 | `#store-screen .btn:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 364 | `.st-scroll` | @media (max-width:760px) | Store browsing, install/update detail | T | `padding` |
| 367 | `.st-stack` | — | Store browsing, install/update detail | T | `display`, `flex-direction`, `gap` |
| 368 | `.st-item-status.is-hot` | — | Store browsing, install/update detail | T | `color`, `font-weight` |
| 369 | `.st-item.is-off .st-item-icon .st-thumb` | — | Store browsing, install/update detail | T | `filter`, `opacity` |
| 370 | `.st-item.is-off .st-item-copy b` | — | Store browsing, install/update detail | T | `color` |
| 371 | `.st-item-more` | — | Store browsing, install/update detail | T | `background`, `border`, `border-radius`, `color`, `display`, `height`, `padding`, `place-items`, `transition`, `width` |
| 372 | `.st-item-more svg` | — | Store browsing, install/update detail | T | `height`, `width` |
| 373 | `.st-item-more:hover` | — | Store browsing, install/update detail | T | `background`, `color` |
| 374 | `.st-item-more:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 376 | `#store-screen .st-item-action .toggle` | — | Store browsing, install/update detail | T | `margin` |
| 377 | `#store-screen .toggle.on` | — | Store browsing, install/update detail | T | `background` |
| 378 | `#store-screen .toggle.on i` | — | Store browsing, install/update detail | T | `background` |
| 379 | `#store-screen .toggle:focus-visible` | — | Store browsing, install/update detail | T | `box-shadow`, `outline` |
| 382 | `.st-empty-card` | — | Store browsing, install/update detail | T | `align-items`, `background`, `border-radius`, `display`, `gap`, `min-height`, `padding` |
| 383 | `.st-empty-copy` | — | Store browsing, install/update detail | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 384 | `.st-empty-copy b` | — | Store browsing, install/update detail | T | `color`, `font-size`, `font-weight`, `line-height` |
| 385 | `.st-empty-copy span` | — | Store browsing, install/update detail | T | `color`, `font-size`, `line-height`, `max-width`, `text-wrap` |
| 386 | `#store-screen .st-empty-go` | — | Store browsing, install/update detail | T | `border-radius`, `flex`, `font-size`, `height`, `padding` |
| 393 | `#store-screen :is(.st-add,.st-explore,.st-show-all,.st-back,.st-plus,.st-arrow,.st-item-more,.st-slide-icon,.st-link,.st-file,.st-toggle,.segmented button,.st-item,.st-search)` | — | Store browsing, install/update detail | T | `transition-duration`, `transition-property`, `transition-timing-function` |
| 395 | `.st-dots button` | — | Store browsing, install/update detail | T | `transition` |
| 397 | `#store-screen :is(.st-add,.st-explore,.st-show-all,.st-back,.segmented button,.st-empty-go,.st-install,.st-detail-actions .btn):active` | @media (prefers-reduced-motion:no-preference) | Store browsing, install/update detail | T | `transform` |
| 398 | `#store-screen :is(.st-plus,.st-arrow,.st-item-more):active` | @media (prefers-reduced-motion:no-preference) | Store browsing, install/update detail | T | `transform` |
| 399 | `#store-screen .st-slide-icon:active` | @media (prefers-reduced-motion:no-preference) | Store browsing, install/update detail | T | `transform` |
| 400 | `#store-screen .st-item:has(>.st-item-open:active)` | @media (prefers-reduced-motion:no-preference) | Store browsing, install/update detail | T | `transform` |
| 403 | `#store-screen *` | @media (prefers-reduced-motion:reduce) | Store browsing, install/update detail | T | `transition-duration` |
| 408 | `#store-screen :is(.st-item-icon,.st-plus:not(.is-done))` | — | Store browsing, install/update detail | T | `background` |
| 409 | `#store-screen button.st-plus:hover` | — | Store browsing, install/update detail | T | `background` |
| 410 | `:root[data-theme="dark"] #store-screen :is(.st-item-icon,.st-plus:not(.is-done))` | — | Store browsing, install/update detail | T | `background` |
| 411 | `:root[data-theme="dark"] #store-screen button.st-plus:hover` | — | Store browsing, install/update detail | T | `background` |

### `apps/desktop/css/transcription.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 7 | `.tr-sheet:focus` | — | Transcription model/settings UI | T | `outline` |
| 8 | `.tr-form.sg-column` | — | Transcription model/settings UI | T | `display`, `flex-direction`, `margin`, `max-width`, `width` |
| 9 | `.tr-form .sg-group` | — | Transcription model/settings UI | T | `border-radius`, `margin` |
| 12 | `.tr-model` | — | Transcription model/settings UI | T | `min-height`, `position`, `transition` |
| 13 | `.tr-model:hover` | — | Transcription model/settings UI | T | `background` |
| 14 | `.tr-model input` | — | Transcription model/settings UI | T | `opacity`, `pointer-events`, `position` |
| 16 | `.tr-radio` | — | Transcription model/settings UI | T | `background`, `border-radius`, `box-shadow`, `display`, `flex`, `height`, `margin-left`, `place-items`, `transition`, `width` |
| 17 | `.tr-radio::after` | — | Transcription model/settings UI | T | `background`, `border-radius`, `content`, `height`, `transform`, `transition`, `width` |
| 18 | `.tr-model.is-checked .tr-radio` | — | Transcription model/settings UI | T | `box-shadow` |
| 19 | `.tr-model.is-checked .tr-radio::after` | — | Transcription model/settings UI | T | `transform` |
| 20 | `.tr-model:has(input:focus-visible) .tr-radio` | — | Transcription model/settings UI | T | `box-shadow` |
| 21 | `.tr-model:has(input:disabled)` | — | Transcription model/settings UI | T | `opacity` |
| 22 | `.tr-model:has(input:disabled):hover` | — | Transcription model/settings UI | T | `background` |
| 23 | `.tr-model .settings-copy` | — | Transcription model/settings UI | T | `padding` |
| 24 | `.tr-sheet .settings-copy>b` | — | Transcription model/settings UI | T | `align-items`, `display`, `gap` |
| 24 | `.tr-settings-row .settings-copy>b` | — | Transcription model/settings UI | T | `align-items`, `display`, `gap` |
| 25 | `.tr-sheet .settings-copy>.tr-size` | — | Transcription model/settings UI | T | `color` |
| 25 | `.tr-settings-row .settings-copy>.tr-size` | — | Transcription model/settings UI | T | `color` |
| 26 | `.tr-tag` | — | Transcription model/settings UI | T | `background`, `border-radius`, `color`, `flex`, `font-size`, `font-weight`, `line-height`, `padding` |
| 29 | `.tr-progress` | — | Transcription model/settings UI | T | `display`, `flex-direction`, `gap`, `margin-top` |
| 30 | `.tr-progress-line` | — | Transcription model/settings UI | T | `color`, `display`, `font-size`, `gap`, `justify-content`, `line-height` |
| 31 | `.tr-progress-count` | — | Transcription model/settings UI | T | `color`, `font-variant-numeric`, `white-space` |
| 32 | `.tr-bar` | — | Transcription model/settings UI | T | `background`, `border-radius`, `display`, `height`, `overflow`, `position` |
| 33 | `.tr-bar>i` | — | Transcription model/settings UI | T | `background`, `border-radius`, `inset`, `position`, `transform-origin`, `transition` |
| 34 | `.tr-note` | — | Transcription model/settings UI | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 35 | `.tr-error` | — | Transcription model/settings UI | T | `color`, `font-size`, `line-height`, `margin-top`, `overflow-wrap`, `text-wrap` |
| 36 | `.tr-foot` | — | Transcription model/settings UI | T | `display`, `gap`, `justify-content`, `margin-top` |
| 37 | `.tr-foot .btn` | — | Transcription model/settings UI | T | `border-radius`, `height`, `padding` |
| 38 | `.transcription-modal .btn.pri:disabled` | — | Transcription model/settings UI | T | `background`, `color`, `opacity` |
| 39 | `.tr-sheet :is(.btn,label)` | — | Transcription model/settings UI | T | `-webkit-user-select`, `cursor`, `user-select` |
| 39 | `.tr-settings-row .btn` | — | Transcription model/settings UI | T | `-webkit-user-select`, `cursor`, `user-select` |
| 42 | `.sg-group>.settings-row.tr-settings-row` | — | Transcription model/settings UI | T | `align-items`, `min-height` |
| 44 | `.tr-settings-actions` | — | Transcription model/settings UI | T | `align-items`, `display`, `flex`, `gap`, `justify-content`, `min-width` |
| 45 | `.tr-settings-actions .btn` | — | Transcription model/settings UI | T | `height`, `padding` |
| 46 | `.tr-settings-progress` | — | Transcription model/settings UI | T | `display`, `flex-direction`, `gap`, `margin-top`, `max-width` |
| 47 | `.tr-settings-row .settings-copy>.tr-row-error` | — | Transcription model/settings UI | T | `color` |
| 47 | `.settings-note.tr-row-error` | — | Transcription model/settings UI | T | `color` |
| 50 | `.tr-section-head` | — | Transcription model/settings UI | T | `align-items`, `display`, `gap`, `justify-content`, `min-height`, `padding-bottom` |
| 51 | `.tr-section-head>.sg-section-title` | — | Transcription model/settings UI | T | `padding-bottom` |
| 52 | `.sg-column .tr-section-head .pm-select.settings-select` | — | Transcription model/settings UI | T | `font-size`, `height` |
| 52 | `.sg-column .tr-section-head .settings-select` | — | Transcription model/settings UI | T | `font-size`, `height` |
| 53 | `.tr-empty` | — | Transcription model/settings UI | T | `margin-top` |
| 56 | `.tr-foot .tr-browse` | — | Transcription model/settings UI | T | `margin-right` |
| 58 | `.tr-bar>i` | @media (prefers-reduced-motion:reduce) | Transcription model/settings UI | T | `transition` |
| 58 | `.tr-radio` | @media (prefers-reduced-motion:reduce) | Transcription model/settings UI | T | `transition` |
| 58 | `.tr-radio::after` | @media (prefers-reduced-motion:reduce) | Transcription model/settings UI | T | `transition` |
| 58 | `.tr-model` | @media (prefers-reduced-motion:reduce) | Transcription model/settings UI | T | `transition` |

### `apps/desktop/css/vars.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 6 | `.vars-sheet:focus` | — | Extension environment Set up sheet | T | `outline` |
| 7 | `.vars-form.sg-column` | — | Extension environment Set up sheet | T | `display`, `flex-direction`, `margin`, `max-width`, `width` |
| 8 | `.vars-form .sg-group` | — | Extension environment Set up sheet | T | `border-radius`, `margin` |
| 9 | `.vars-form .sg-group>.settings-row` | — | Extension environment Set up sheet | T | `min-height` |
| 10 | `.vars-row .settings-copy>b` | — | Extension environment Set up sheet | T | `align-items`, `display`, `gap`, `min-width` |
| 11 | `.vars-req` | — | Extension environment Set up sheet | T | `background`, `border-radius`, `color`, `flex`, `font-size`, `font-weight`, `line-height`, `padding` |
| 12 | `.vars-row .settings-copy>.vars-attention` | — | Extension environment Set up sheet | T | `color` |
| 15 | `.vars-field` | — | Extension environment Set up sheet | T | `align-items`, `background`, `border-radius`, `display`, `flex`, `height`, `position`, `width` |
| 16 | `.vars-field input` | — | Extension environment Set up sheet | T | `background`, `border`, `color`, `flex`, `font-size`, `height`, `min-width`, `padding` |
| 17 | `.vars-field input:focus-visible` | — | Extension environment Set up sheet | T | `outline` |
| 18 | `.vars-field input:disabled` | — | Extension environment Set up sheet | T | `color` |
| 19 | `.vars-eye` | — | Extension environment Set up sheet | T | `border-radius`, `color`, `display`, `flex`, `height`, `margin-right`, `place-items`, `transition`, `width` |
| 20 | `.vars-eye svg` | — | Extension environment Set up sheet | T | `height`, `width` |
| 21 | `.vars-eye:hover` | — | Extension environment Set up sheet | T | `color` |
| 21 | `.vars-eye[aria-pressed="true"]` | — | Extension environment Set up sheet | T | `color` |
| 22 | `.vars-eye:focus-visible` | — | Extension environment Set up sheet | T | `box-shadow`, `outline` |
| 25 | `.vars-stored` | — | Extension environment Set up sheet | T | `align-items`, `display`, `flex`, `font-size`, `gap`, `line-height` |
| 26 | `.vars-dots` | — | Extension environment Set up sheet | T | `-webkit-user-select`, `color`, `letter-spacing`, `user-select` |
| 27 | `.vars-stored .acct-link:disabled` | — | Extension environment Set up sheet | T | `color` |
| 29 | `.vars-note` | — | Extension environment Set up sheet | T | `color`, `font-size`, `line-height`, `margin-top`, `text-wrap` |
| 30 | `.vars-error` | — | Extension environment Set up sheet | T | `color`, `font-size`, `line-height`, `margin-top`, `overflow-wrap`, `text-wrap` |
| 31 | `.vars-foot` | — | Extension environment Set up sheet | T | `display`, `gap`, `justify-content`, `margin-top` |
| 32 | `.vars-foot .btn` | — | Extension environment Set up sheet | T | `border-radius`, `height`, `padding` |
| 33 | `.vars-modal .btn.pri:disabled` | — | Extension environment Set up sheet | T | `background`, `color`, `opacity` |
| 34 | `.vars-sheet :is(.btn,.acct-link,.vars-eye)` | — | Extension environment Set up sheet | T | `-webkit-user-select`, `cursor`, `user-select` |
| 34 | `.vars-sheet label` | — | Extension environment Set up sheet | T | `-webkit-user-select`, `cursor`, `user-select` |
| 37 | `.settings-extension-controls>.btn.settings-extension-setup` | — | Extension environment Set up sheet | T | `height`, `padding` |
| 38 | `.settings-extension-copy>.settings-extension-setup-note` | — | Extension environment Set up sheet | T | `color` |
| 38 | `.settings-copy>.settings-extension-setup-note` | — | Extension environment Set up sheet | T | `color` |

### `apps/desktop/src/renderer/src/controls/controls.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 1 | `.pm-control-input:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 1 | `.num:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 1 | `.sel:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 8 | `input.num:not([type=range]):not([type=color]):not([type=file])` | — | Shared controls, fields, pickers and menus | T | `cursor`, `field-sizing`, `flex`, `min-width`, `width` |
| 16 | `input.num.editing:not([type=range]):not([type=color]):not([type=file])` | — | Shared controls, fields, pickers and menus | T | `cursor` |
| 20 | `select.sel` | — | Shared controls, fields, pickers and menus | T | `appearance`, `border`, `color`, `font`, `font-family` |
| 30 | `.panel-sr-only` | — | Shared controls, fields, pickers and menus | T | `border`, `clip`, `clip-path`, `height`, `margin`, `overflow`, `padding`, `position`, `white-space`, `width` |
| 47 | `.color-picker-layer` | — | Shared controls, fields, pickers and menus | T | `background`, `inset`, `position`, `z-index` |
| 53 | `.color-picker-layer[data-state="closed"]` | — | Shared controls, fields, pickers and menus | T | `pointer-events` |
| 55 | `.color-picker` | — | Shared controls, fields, pickers and menus | T | `--cp-button`, `--cp-strip`, `--cp-well`, `-webkit-user-select`, `background`, `border-radius`, `box-shadow`, `color`, `font-family`, `font-size`, `outline`, `position`, `user-select`, `width`, `z-index` |
| 72 | `:root[data-theme="dark"] .color-picker` | — | Shared controls, fields, pickers and menus | T | `--cp-strip`, `box-shadow` |
| 76 | `.color-picker[data-side="bottom"]` | — | Shared controls, fields, pickers and menus | T | `transform-origin` |
| 77 | `.color-picker[data-side="top"]` | — | Shared controls, fields, pickers and menus | T | `--pm-menu-slide`, `transform-origin` |
| 78 | `.color-picker[data-state="open"]` | — | Shared controls, fields, pickers and menus | T | `animation` |
| 79 | `.color-picker[data-state="closed"]` | — | Shared controls, fields, pickers and menus | T | `animation`, `pointer-events` |
| 81 | `.color-picker button` | — | Shared controls, fields, pickers and menus | T | `border`, `cursor`, `font`, `padding` |
| 87 | `.color-picker :is(button, [role="slider"]):focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 96 | `.cp-header` | — | Shared controls, fields, pickers and menus | T | `--cp-rule`, `align-items`, `background`, `border-bottom`, `border-radius`, `display`, `height`, `justify-content` |
| 106 | `.cp-spaces` | — | Shared controls, fields, pickers and menus | T | `display` |
| 108 | `.cp-spaces button` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-bottom`, `border-radius`, `border-top`, `color`, `margin-bottom`, `padding`, `position`, `transition` |
| 120 | `.cp-spaces button:first-child` | — | Shared controls, fields, pickers and menus | T | `border-left`, `border-top-left-radius` |
| 124 | `.cp-spaces button[aria-selected="true"]` | — | Shared controls, fields, pickers and menus | T | `background`, `border-color`, `color`, `z-index` |
| 130 | `.cp-spaces button[aria-selected="false"]:hover` | — | Shared controls, fields, pickers and menus | T | `color` |
| 131 | `.cp-actions` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `padding-right` |
| 137 | `.cp-icon` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `color`, `display`, `height`, `place-items`, `transition`, `width` |
| 147 | `.cp-icon:hover` | — | Shared controls, fields, pickers and menus | T | `background`, `color` |
| 147 | `.cp-icon[aria-expanded="true"]` | — | Shared controls, fields, pickers and menus | T | `background`, `color` |
| 149 | `.cp-icon:disabled` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 150 | `.cp-icon svg` | — | Shared controls, fields, pickers and menus | T | `height`, `width` |
| 152 | `.cp-body` | — | Shared controls, fields, pickers and menus | T | `display`, `gap`, `padding` |
| 159 | `.cp-plane` | — | Shared controls, fields, pickers and menus | T | `background`, `cursor`, `flex`, `height`, `position`, `touch-action`, `width` |
| 171 | `.cp-plane i` | — | Shared controls, fields, pickers and menus | T | `border`, `border-radius`, `box-shadow`, `height`, `pointer-events`, `position`, `transform`, `width` |
| 183 | `.cp-checker` | — | Shared controls, fields, pickers and menus | T | `background-color`, `background-image`, `background-position`, `background-size` |
| 191 | `.color-field .sw.cp-checker` | — | Shared controls, fields, pickers and menus | T | `background-color`, `background-image`, `background-position`, `background-size` |
| 200 | `.color-field-alpha` | — | Shared controls, fields, pickers and menus | T | `color`, `margin-left` |
| 206 | `.cp-track` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `cursor`, `flex`, `height`, `position`, `touch-action`, `width` |
| 215 | `.cp-alpha` | — | Shared controls, fields, pickers and menus | T | `overflow` |
| 216 | `.cp-alpha::before` | — | Shared controls, fields, pickers and menus | T | `background`, `content`, `inset`, `position` |
| 222 | `.cp-hue` | — | Shared controls, fields, pickers and menus | T | `background` |
| 225 | `.cp-track i` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `box-shadow`, `height`, `left`, `pointer-events`, `position`, `transform`, `width` |
| 237 | `.cp-details` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-width` |
| 243 | `.cp-preview` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `box-shadow`, `display`, `height`, `overflow`, `position` |
| 251 | `.cp-previous` | — | Shared controls, fields, pickers and menus | T | `height`, `width` |
| 251 | `.cp-new` | — | Shared controls, fields, pickers and menus | T | `height`, `width` |
| 256 | `.cp-clipped` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `color`, `font-size`, `line-height`, `padding`, `position`, `right`, `top` |
| 267 | `.cp-preview-labels` | — | Shared controls, fields, pickers and menus | T | `align-items`, `color`, `display`, `font-size`, `height`, `justify-content` |
| 277 | `.cp-row` | — | Shared controls, fields, pickers and menus | T | `display`, `gap`, `height` |
| 282 | `.cp-channels` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `display`, `flex`, `height`, `min-width` |
| 290 | `.cp-channel` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-width`, `position` |
| 295 | `.cp-channel + .cp-channel` | — | Shared controls, fields, pickers and menus | T | `box-shadow` |
| 296 | `.cp-channel input` | — | Shared controls, fields, pickers and menus | T | `-webkit-user-select`, `appearance`, `background`, `border`, `color`, `font-family`, `font-size`, `font-variant-numeric`, `height`, `outline`, `padding`, `text-align`, `user-select`, `width` |
| 312 | `.cp-channel input::-webkit-inner-spin-button` | — | Shared controls, fields, pickers and menus | T | `appearance`, `margin` |
| 312 | `.cp-channel input::-webkit-outer-spin-button` | — | Shared controls, fields, pickers and menus | T | `appearance`, `margin` |
| 314 | `.cp-channel:focus-within` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `box-shadow` |
| 315 | `.cp-channel span` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `left`, `pointer-events`, `position`, `right`, `text-align`, `top` |
| 325 | `.cp-square` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 336 | `.cp-row .cp-square` | — | Shared controls, fields, pickers and menus | T | `height` |
| 337 | `.cp-square:hover` | — | Shared controls, fields, pickers and menus | T | `background`, `color` |
| 337 | `.cp-square[aria-expanded="true"]` | — | Shared controls, fields, pickers and menus | T | `background`, `color` |
| 339 | `.cp-square svg` | — | Shared controls, fields, pickers and menus | T | `height`, `width` |
| 342 | `.cp-value-row` | — | Shared controls, fields, pickers and menus | T | `display`, `gap`, `height` |
| 347 | `.color-picker .cp-value` | — | Shared controls, fields, pickers and menus | T | `-webkit-user-select`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `font-family`, `font-size`, `font-variant-numeric`, `min-width`, `outline`, `padding`, `text-align`, `transition`, `user-select`, `width` |
| 366 | `.color-picker .cp-value:focus` | — | Shared controls, fields, pickers and menus | T | `box-shadow` |
| 367 | `.color-picker .cp-value[aria-invalid="true"]` | — | Shared controls, fields, pickers and menus | T | `color` |
| 369 | `.cp-menu-heading` | — | Shared controls, fields, pickers and menus | T | `color`, `font-weight`, `white-space` |
| 372 | `.toggle` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `transition` |
| 372 | `.toggle i` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `transition` |
| 372 | `.fill-spectrum-color` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `transition` |
| 377 | `.color-picker[data-state]` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `animation` |
| 384 | `.onoff` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `display`, `flex`, `gap`, `height`, `min-width`, `padding`, `position`, `width` |
| 396 | `.onoff-pill` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `bottom`, `left`, `pointer-events`, `position`, `top`, `transform`, `transition`, `width` |
| 408 | `.onoff.on .onoff-pill` | — | Shared controls, fields, pickers and menus | T | `transform` |
| 409 | `.onoff > button` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `color`, `cursor`, `flex`, `font`, `font-size`, `font-weight`, `min-width`, `padding`, `position`, `transition`, `z-index` |
| 425 | `.onoff > button:hover` | — | Shared controls, fields, pickers and menus | T | `color` |
| 426 | `.onoff > button[aria-checked="true"]` | — | Shared controls, fields, pickers and menus | T | `color` |
| 427 | `.onoff > button:active` | — | Shared controls, fields, pickers and menus | T | `transform`, `transition` |
| 428 | `.onoff > button:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 429 | `.onoff.mixed > button` | — | Shared controls, fields, pickers and menus | T | `color` |
| 430 | `.onoff-pill` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `transition` |
| 436 | `.fill-picker` | — | Shared controls, fields, pickers and menus | T | `animation`, `background`, `border-radius`, `box-shadow`, `outline`, `overflow`, `position`, `width`, `z-index` |
| 447 | `:root[data-theme="dark"] .fill-picker` | — | Shared controls, fields, pickers and menus | T | `box-shadow` |
| 450 | `.fill-picker[data-side="bottom"]` | — | Shared controls, fields, pickers and menus | T | `transform-origin` |
| 451 | `.fill-picker[data-side="top"]` | — | Shared controls, fields, pickers and menus | T | `--pm-menu-slide`, `transform-origin` |
| 452 | `:root .color-picker.cp-embedded` | — | Shared controls, fields, pickers and menus | T | `animation`, `background`, `border-radius`, `box-shadow`, `position`, `width`, `z-index` |
| 461 | `.cp-embedded [hidden]` | — | Shared controls, fields, pickers and menus | T | `display` |
| 462 | `.cp-space` | — | Shared controls, fields, pickers and menus | T | `padding`, `width` |
| 467 | `.fill-gradient` | — | Shared controls, fields, pickers and menus | T | `display`, `flex-direction`, `gap`, `padding` |
| 473 | `.fill-preview` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `box-shadow`, `height`, `overflow`, `position` |
| 480 | `.fill-preview span` | — | Shared controls, fields, pickers and menus | T | `inset`, `position` |
| 481 | `.fill-stops` | — | Shared controls, fields, pickers and menus | T | `display`, `flex-direction`, `gap` |
| 486 | `.fill-stop` | — | Shared controls, fields, pickers and menus | T | `align-items`, `border-radius`, `display`, `gap`, `grid-template-columns`, `height`, `padding` |
| 495 | `.fill-stop:hover` | — | Shared controls, fields, pickers and menus | T | `background` |
| 496 | `.fill-stop.on` | — | Shared controls, fields, pickers and menus | T | `background` |
| 497 | `.fill-stop-swatch.cp-checker` | — | Shared controls, fields, pickers and menus | T | `background-image`, `background-position`, `background-size`, `border-radius`, `box-shadow`, `height`, `width` |
| 509 | `.fill-stop-color` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `color`, `font`, `height`, `min-width`, `outline`, `padding` |
| 520 | `.fill-stop-color:focus` | — | Shared controls, fields, pickers and menus | T | `box-shadow` |
| 521 | `.fill-picker input[type=range]` | — | Shared controls, fields, pickers and menus | T | `min-width`, `width` |
| 525 | `.fill-stop .mono` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `text-align` |
| 530 | `.fill-stop-remove` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 531 | `.fill-stop:hover .fill-stop-remove` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 531 | `.fill-stop-remove:focus-visible` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 533 | `.fill-stop-remove:disabled` | — | Shared controls, fields, pickers and menus | T | `visibility` |
| 534 | `.fill-picker-tools` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `padding` |
| 540 | `.fill-angle` | — | Shared controls, fields, pickers and menus | T | `align-items`, `color`, `display`, `flex`, `font-size`, `gap` |
| 548 | `.fill-angle input` | — | Shared controls, fields, pickers and menus | T | `flex` |
| 549 | `.fill-angle .mono` | — | Shared controls, fields, pickers and menus | T | `font-size`, `min-width`, `text-align` |
| 554 | `.fill-add-stop` | — | Shared controls, fields, pickers and menus | T | `font-size`, `height`, `margin-left`, `padding` |
| 560 | `.fill-none` | — | Shared controls, fields, pickers and menus | T | `color`, `margin`, `padding` |
| 567 | `.row.split:has(.point-field)` | — | Shared controls, fields, pickers and menus | T | `align-items`, `height`, `min-height`, `padding-block` |

### `apps/desktop/src/renderer/src/controls/search-picker.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 3 | `.thread-popup` | — | Shared controls, fields, pickers and menus | T | `box-sizing`, `display`, `flex-direction`, `font-weight`, `overflow` |
| 4 | `.thread-search` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `color`, `flex-shrink`, `font`, `height`, `margin`, `padding` |
| 5 | `.thread-search::placeholder` | — | Shared controls, fields, pickers and menus | T | `color` |
| 6 | `.thread-search:focus` | — | Shared controls, fields, pickers and menus | T | `outline` |
| 7 | `.thread-list` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-height`, `overflow-y`, `position`, `scrollbar-width` |
| 8 | `.thread-glider` | — | Shared controls, fields, pickers and menus | T | `left`, `right` |
| 9 | `.thread-row` | — | Shared controls, fields, pickers and menus | T | `align-items`, `border-radius`, `display`, `gap`, `padding`, `position`, `z-index` |
| 10 | `.thread-row:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline` |
| 11 | `.thread-row.current` | — | Shared controls, fields, pickers and menus | T | `background` |
| 12 | `.thread-row-text` | — | Shared controls, fields, pickers and menus | T | `display`, `flex`, `flex-direction`, `gap`, `min-width` |
| 13 | `.thread-row-title` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `overflow`, `text-overflow`, `white-space` |
| 14 | `.thread-running` | — | Shared controls, fields, pickers and menus | T | `align-items`, `background`, `border-radius`, `color`, `display`, `flex-shrink`, `font-size`, `gap`, `padding` |
| 15 | `.thread-dot` | — | Shared controls, fields, pickers and menus | T | `animation`, `background`, `border-radius`, `flex-shrink`, `height`, `width` |
| 17 | `.thread-dot` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `animation` |
| 18 | `.thread-row-meta` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `overflow`, `text-overflow`, `white-space` |
| 19 | `.thread-row-meta.current` | — | Shared controls, fields, pickers and menus | T | `color` |
| 20 | `.thread-delete` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `color`, `display`, `flex-shrink`, `height`, `opacity`, `padding`, `place-items`, `width` |
| 21 | `.thread-row:hover .thread-delete` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 21 | `.thread-row.current .thread-delete` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 21 | `.thread-delete:focus-visible` | — | Shared controls, fields, pickers and menus | T | `opacity` |
| 22 | `.thread-delete:hover` | — | Shared controls, fields, pickers and menus | T | `background`, `color` |
| 23 | `.thread-empty` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `margin` |

### `apps/desktop/src/renderer/src/legacy/core/image-sequence.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 1 | `.sequence-import` | — | Component image-sequence | T | `display`, `gap`, `line-height` |
| 7 | `.sequence-import p` | — | Component image-sequence | T | `margin` |
| 8 | `.sequence-import-intro` | — | Component image-sequence | T | `color` |
| 9 | `.sequence-import .sequence-import-note` | — | Component image-sequence | T | `color`, `font-size`, `margin-top` |
| 11 | `.sequence-import-source` | — | Component image-sequence | T | `align-items`, `background`, `border-radius`, `display`, `gap`, `padding` |
| 20 | `.sequence-import-icon` | — | Component image-sequence | T | `background`, `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 31 | `.sequence-import-icon svg` | — | Component image-sequence | T | `height`, `width` |
| 32 | `.sequence-import-source-text` | — | Component image-sequence | T | `min-width` |
| 33 | `.sequence-import strong` | — | Component image-sequence | T | `font-weight` |
| 35 | `.sequence-import-files` | — | Component image-sequence | T | `color`, `font-size`, `overflow`, `text-overflow`, `white-space` |
| 43 | `.sequence-import-settings` | — | Component image-sequence | T | `display`, `gap`, `grid-template-columns` |
| 49 | `.sequence-import-rate` | — | Component image-sequence | T | `align-content`, `color`, `display`, `font-size`, `gap` |
| 49 | `.sequence-import-timing` | — | Component image-sequence | T | `align-content`, `color`, `display`, `font-size`, `gap` |
| 58 | `.sequence-import-input` | — | Component image-sequence | T | `align-items`, `background`, `border`, `border-radius`, `display`, `gap`, `height`, `padding` |
| 69 | `.sequence-import-input:focus-within` | — | Component image-sequence | T | `outline`, `outline-offset` |
| 74 | `.sequence-import input.sequence-import-fps` | — | Component image-sequence | T | `background`, `border`, `box-shadow`, `color`, `font`, `height`, `min-width`, `outline`, `padding`, `width` |
| 87 | `.sequence-import-rate small` | — | Component image-sequence | T | `font-size` |
| 88 | `.sequence-import-duration` | — | Component image-sequence | T | `align-items`, `color`, `display`, `font`, `font-variant-numeric`, `height` |
| 97 | `.sequence-import-warning` | — | Component image-sequence | T | `background`, `border-radius`, `display`, `gap`, `overflow-wrap`, `padding` |
| 106 | `.sequence-import-warning[hidden]` | — | Component image-sequence | T | `display` |
| 107 | `.sequence-import-warning strong` | — | Component image-sequence | T | `color` |
| 108 | `.sequence-import-warning p` | — | Component image-sequence | T | `color`, `font-size` |
| 109 | `.sequence-import-reimport` | — | Component image-sequence | T | `justify-self`, `margin-top` |

### `apps/desktop/src/renderer/src/onboarding/welcome.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 8 | `*` | — | Onboarding | T | `box-sizing` |
| 10 | `html` | — | Onboarding | T | `height`, `margin`, `width` |
| 10 | `body` | — | Onboarding | T | `height`, `margin`, `width` |
| 13 | `body` | — | Onboarding | T | `-webkit-app-region`, `-webkit-font-smoothing`, `background`, `color`, `display`, `font-family`, `overflow`, `place-items` |
| 24 | `.welcome-card` | — | Onboarding | T | `padding`, `text-align`, `width` |
| 30 | `.welcome-card > *` | — | Onboarding | T | `animation` |
| 33 | `.welcome-entering .welcome-card > *` | — | Onboarding | T | `animation-play-state` |
| 34 | `.welcome-card > :nth-child(2)` | — | Onboarding | T | `animation-delay` |
| 35 | `.welcome-card > :nth-child(3)` | — | Onboarding | T | `animation-delay` |
| 36 | `.welcome-card > :nth-child(4)` | — | Onboarding | T | `animation-delay` |
| 43 | `.welcome-mark` | — | Onboarding | T | `fill`, `height`, `margin-bottom`, `overflow`, `width` |
| 52 | `h1` | — | Onboarding | T | `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin` |
| 60 | `.welcome-actions` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `display`, `gap`, `justify-content` |
| 68 | `#begin` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `cursor`, `display`, `font`, `height`, `justify-content`, `padding`, `transition`, `width` |
| 86 | `#begin:hover` | — | Onboarding | T | `background` |
| 87 | `#begin:active` | — | Onboarding | T | `transform` |
| 88 | `#begin:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 89 | `#begin:focus-visible` | — | Onboarding | T | `background` |
| 90 | `#begin:disabled` | — | Onboarding | T | `opacity` |
| 92 | `#begin-status` | — | Onboarding | T | `color`, `font-size`, `margin`, `min-height` |
| 100 | `#begin` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |
| 101 | `.welcome-card > *` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 104 | `[hidden]` | — | Onboarding | T | `display` |

### `apps/desktop/src/renderer/src/panels/agent-shell.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 2 | `#agent-popover-root` | — | Panel agent-shell | T | `inset`, `pointer-events`, `position`, `z-index` |
| 9 | `.agent-popover` | — | Panel agent-shell | T | `-webkit-app-region`, `background`, `border-radius`, `box-shadow`, `box-sizing`, `color`, `display`, `flex-direction`, `min-height`, `min-width`, `overflow`, `pointer-events`, `position`, `transition` |
| 30 | `#agent-popover-root:not([data-open="true"]) .agent-popover` | — | Panel agent-shell | T | `opacity`, `pointer-events`, `scale`, `transition`, `translate`, `visibility` |
| 44 | `#agent-popover-root[data-collapsing="true"] .agent-popover` | — | Panel agent-shell | T | `opacity`, `scale`, `transform-origin`, `transition`, `translate` |
| 56 | `.agent-popover-panel-host` | — | Panel agent-shell | T | `display`, `flex`, `min-height`, `min-width`, `overflow` |
| 57 | `#agent-popover .agent-popover-panel-host > #panel-agent` | — | Panel agent-shell | T | `background`, `border`, `border-radius`, `display`, `flex`, `flex-direction`, `height`, `margin`, `max-width`, `min-height`, `min-width`, `width` |
| 71 | `#agent-popover .agent-popover-panel-host > #panel-agent > header` | — | Panel agent-shell | T | `display` |
| 72 | `#agent-popover .agent-popover-panel-host > #panel-agent > .body` | — | Panel agent-shell | T | `flex`, `min-height` |
| 73 | `#agent-popover .agent-panel-toolbar` | — | Panel agent-shell | T | `align-items`, `display`, `gap`, `padding` |
| 74 | `#agent-popover .thread-bar` | — | Panel agent-shell | T | `flex`, `margin`, `padding` |
| 75 | `#agent-popover .agent-popover-control` | — | Panel agent-shell | T | `background`, `border`, `border-radius`, `color`, `display`, `flex`, `height`, `padding`, `place-items`, `width` |
| 79 | `#agent-popover .agent-popover-control svg` | — | Panel agent-shell | T | `height`, `width` |
| 80 | `#agent-popover .agent-popover-control:hover:not(:disabled)` | — | Panel agent-shell | T | `background`, `color` |
| 81 | `#agent-popover .agent-popover-control:disabled` | — | Panel agent-shell | T | `opacity` |
| 82 | `#agent-popover .agent-popover-control:focus-visible` | — | Panel agent-shell | T | `outline`, `outline-offset` |
| 83 | `#agent-popover .agent-popover-grip` | — | Panel agent-shell | T | `cursor`, `touch-action` |
| 84 | `#agent-popover .agent-popover-grip:active` | — | Panel agent-shell | T | `cursor` |
| 85 | `#agent-popover .thread-bar` | — | Panel agent-shell | T | `cursor`, `touch-action` |
| 85 | `#agent-popover .thread-trigger` | — | Panel agent-shell | T | `cursor`, `touch-action` |
| 87 | `#agent-popover .thread-bar:active` | — | Panel agent-shell | T | `cursor` |
| 87 | `#agent-popover .thread-trigger:active` | — | Panel agent-shell | T | `cursor` |
| 91 | `.agent-floating-window` | — | Panel agent-shell | T | `-webkit-app-region`, `background`, `border-radius`, `box-shadow`, `box-sizing`, `color`, `display`, `flex-direction`, `height`, `min-height`, `min-width`, `overflow`, `pointer-events`, `position`, `width` |
| 109 | `.agent-floating-header` | — | Panel agent-shell | T | `align-items`, `background`, `box-shadow`, `display`, `flex`, `gap`, `height`, `padding`, `position` |
| 121 | `.agent-floating-header button` | — | Panel agent-shell | T | `border`, `border-radius`, `color`, `display`, `flex`, `height`, `padding`, `place-items`, `transition`, `width` |
| 134 | `.agent-floating-header button svg` | — | Panel agent-shell | T | `height`, `width` |
| 135 | `.agent-floating-header button:focus-visible` | — | Panel agent-shell | T | `outline`, `outline-offset` |
| 136 | `.agent-floating-header .agent-floating-drag` | — | Panel agent-shell | T | `cursor`, `touch-action` |
| 137 | `.agent-floating-header .agent-floating-drag:active` | — | Panel agent-shell | T | `cursor` |
| 138 | `.agent-floating-title` | — | Panel agent-shell | T | `color`, `font-size`, `font-weight`, `padding-inline-start` |
| 139 | `.agent-floating-spacer` | — | Panel agent-shell | T | `flex` |
| 142 | `.agent-floating-header button:hover` | @media (hover: hover) and (pointer: fine) | Panel agent-shell | T | `background`, `color` |
| 146 | `.agent-popover` | @media (prefers-reduced-motion: reduce) | Panel agent-shell | T | `scale`, `transition`, `translate` |
| 146 | `#agent-popover-root:not([data-open="true"]) .agent-popover` | @media (prefers-reduced-motion: reduce) | Panel agent-shell | T | `scale`, `transition`, `translate` |
| 146 | `#agent-popover-root[data-collapsing="true"] .agent-popover` | @media (prefers-reduced-motion: reduce) | Panel agent-shell | T | `scale`, `transition`, `translate` |
| 149 | `.agent-floating-header button` | @media (prefers-reduced-motion: reduce) | Panel agent-shell | T | `transition` |

### `apps/desktop/src/renderer/src/panels/panels.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 4 | `[data-svelte-panel].simple-panel-list` | — | Panel panels | T | `display`, `flex-direction`, `padding` |
| 10 | `.simple-panel-row` | — | Panel panels | T | `gap`, `height`, `padding` |
| 16 | `.simple-panel-row .simple-row-action` | — | Panel panels | T | `align-items`, `color`, `display`, `flex`, `gap`, `height`, `min-width`, `text-align` |
| 27 | `.simple-panel-row .simple-row-action .nm` | — | Panel panels | T | `color`, `transition` |
| 28 | `.simple-panel-row:hover .simple-row-action .nm` | — | Panel panels | T | `color` |
| 28 | `.simple-panel-row.sel .simple-row-action .nm` | — | Panel panels | T | `color` |
| 28 | `.simple-panel-row:focus-within .simple-row-action .nm` | — | Panel panels | T | `color` |
| 33 | `.simple-panel-row > .stopwatch` | — | Panel panels | T | `height`, `opacity`, `transition`, `width` |
| 39 | `.simple-panel-row:hover > .stopwatch` | — | Panel panels | T | `opacity` |
| 39 | `.simple-panel-row:focus-within > .stopwatch` | — | Panel panels | T | `opacity` |
| 39 | `.simple-panel-row > .stopwatch:focus-visible` | — | Panel panels | T | `opacity` |
| 42 | `.simple-panel-row > .stopwatch:hover` | — | Panel panels | T | `color` |
| 44 | `.simple-effect-row` | — | Panel panels | T | `text-align`, `width` |
| 49 | `.simple-panel-section` | — | Panel panels | T | `margin` |
| 54 | `.simple-panel-actions` | — | Panel panels | T | `border-top`, `display`, `gap`, `margin-top`, `padding` |
| 61 | `.simple-panel-actions .chip` | — | Panel panels | T | `flex`, `height`, `min-width` |
| 62 | `.simple-panel-actions .chip.quiet` | — | Panel panels | T | `flex` |
| 64 | `.simple-panel-hint` | — | Panel panels | T | `color`, `font-size`, `padding`, `text-align` |
| 71 | `.simple-panel-list > .empty` | — | Panel panels | T | `padding` |
| 73 | `.notes-panel` | — | Panel panels | T | `height`, `width` |
| 78 | `.notes-panel textarea` | — | Panel panels | T | `background`, `color`, `font-size`, `height`, `line-height`, `min-height`, `padding`, `resize`, `width` |
| 90 | `.notes-panel textarea:focus-visible` | — | Panel panels | T | `outline`, `outline-offset` |

### `apps/desktop/src/renderer/public/onboarding/onboarding.css`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 1 | `:root` | — | Onboarding | T | `color-scheme`, `font-family`, `font-synthesis` |
| 7 | `*` | — | Onboarding | T | `box-sizing` |
| 9 | `html` | — | Onboarding | T | `height`, `margin`, `width` |
| 9 | `body` | — | Onboarding | T | `height`, `margin`, `width` |
| 12 | `.animation-stage` | — | Onboarding | T | `background`, `display`, `overflow`, `place-items`, `pointer-events` |
| 20 | `.animation-stage.onboarding-playing` | — | Onboarding | T | `animation` |
| 24 | `.animation-stage.onboarding-ending` | — | Onboarding | T | `animation` |
| 38 | `#onboarding-svg` | — | Onboarding | T | `background`, `display`, `height`, `width` |
| 45 | `#onboarding-hdr` | — | Onboarding | T | `background`, `height`, `inset`, `position`, `width` |
| 53 | `.onboarding-hdr-active #onboarding-svg` | — | Onboarding | T | `opacity` |
| 55 | `#skip-intro` | — | Onboarding | T | `background`, `border`, `border-radius`, `bottom`, `color`, `cursor`, `font`, `padding`, `pointer-events`, `position`, `right`, `z-index` |
| 61 | `#skip-intro:hover` | — | Onboarding | T | `background` |
| 62 | `#skip-intro:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 64 | `.animation-stage` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 65 | `#onboarding-svg` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `display` |
| 65 | `#onboarding-hdr` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `display` |

### `apps/desktop/src/renderer/src/captions/CaptionsPanel.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 200 | `.captions-panel` | — | Component CaptionsPanel | T | `display`, `flex-direction`, `font-size`, `height`, `min-height` |
| 201 | `.captions-head` | — | Component CaptionsPanel | T | `align-items`, `display`, `flex`, `gap`, `padding` |
| 202 | `.captions-layer` | — | Component CaptionsPanel | T | `align-items`, `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `flex`, `font`, `gap`, `height`, `max-width`, `padding` |
| 206 | `.captions-layer span` | — | Component CaptionsPanel | T | `overflow`, `text-overflow`, `white-space` |
| 207 | `.captions-layer svg` | — | Component CaptionsPanel | T | `color`, `flex`, `height`, `width` |
| 208 | `.captions-layer:hover` | — | Component CaptionsPanel | T | `background`, `color` |
| 209 | `.captions-layer:focus-visible` | — | Component CaptionsPanel | T | `box-shadow`, `outline` |
| 210 | `.captions-search` | — | Component CaptionsPanel | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `font-size`, `height`, `min-width`, `padding` |
| 214 | `.captions-search::placeholder` | — | Component CaptionsPanel | T | `color` |
| 215 | `.captions-search:focus` | — | Component CaptionsPanel | T | `outline` |
| 216 | `.captions-search:focus-visible` | — | Component CaptionsPanel | T | `box-shadow` |
| 217 | `.captions-search::-webkit-search-cancel-button` | — | Component CaptionsPanel | T | `-webkit-appearance` |
| 218 | `.captions-scroll` | — | Component CaptionsPanel | T | `flex`, `min-height`, `overflow`, `padding` |
| 219 | `.captions-list` | — | Component CaptionsPanel | T | `outline`, `position` |
| 220 | `.captions-row` | — | Component CaptionsPanel | T | `-webkit-user-select`, `align-items`, `border-radius`, `color`, `cursor`, `display`, `gap`, `height`, `left`, `padding`, `position`, `right`, `user-select` |
| 224 | `.captions-row:hover` | — | Component CaptionsPanel | T | `background` |
| 225 | `.captions-row.picked` | — | Component CaptionsPanel | T | `background`, `color` |
| 226 | `.captions-row.now .tx` | — | Component CaptionsPanel | T | `color` |
| 227 | `.captions-list:focus-visible .captions-row.focus` | — | Component CaptionsPanel | T | `box-shadow` |
| 228 | `.tc` | — | Component CaptionsPanel | T | `color`, `flex`, `font-family`, `font-size`, `font-variant-numeric`, `width` |
| 229 | `.captions-row.now .tc` | — | Component CaptionsPanel | T | `color` |
| 230 | `.tx` | — | Component CaptionsPanel | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 231 | `.captions-editor` | — | Component CaptionsPanel | T | `-webkit-user-select`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `flex`, `font`, `height`, `line-height`, `min-width`, `overflow`, `padding`, `resize`, `user-select` |
| 236 | `.captions-editor:focus` | — | Component CaptionsPanel | T | `outline` |
| 237 | `.empty` | — | Component CaptionsPanel | T | `color`, `font-size`, `line-height`, `padding`, `text-align`, `text-wrap` |
| 238 | `.captions-empty p` | — | Component CaptionsPanel | T | `margin` |
| 239 | `.captions-empty-actions` | — | Component CaptionsPanel | T | `display`, `gap`, `justify-content` |
| 240 | `.captions-empty-actions .chip` | — | Component CaptionsPanel | T | `height`, `padding` |

### `apps/desktop/src/renderer/src/controls/BackTitle.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 51 | `.pm-back-title` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin` |
| 52 | `.pm-back-title-inner` | — | Shared controls, fields, pickers and menus | T | `align-items`, `border-radius`, `display`, `transition`, `vertical-align` |
| 53 | `.pm-back-title-chev` | — | Shared controls, fields, pickers and menus | T | `align-items`, `color`, `display`, `height`, `opacity`, `overflow`, `transition`, `width` |
| 54 | `.pm-back-title-chev :global(svg)` | — | Shared controls, fields, pickers and menus | T | `flex`, `height`, `transform`, `width` |
| 55 | `.is-back .pm-back-title-chev` | — | Shared controls, fields, pickers and menus | T | `opacity`, `width` |
| 56 | `.is-back .pm-back-title-inner` | — | Shared controls, fields, pickers and menus | T | `-webkit-user-select`, `cursor`, `user-select` |
| 57 | `.is-back .pm-back-title-inner:hover` | — | Shared controls, fields, pickers and menus | T | `color` |
| 57 | `.is-back .pm-back-title-inner:hover .pm-back-title-chev` | — | Shared controls, fields, pickers and menus | T | `color` |
| 58 | `.is-back .pm-back-title-inner:focus-visible` | — | Shared controls, fields, pickers and menus | T | `box-shadow`, `outline` |
| 59 | `.pm-back-title-chev` | @media (prefers-reduced-motion: reduce) | Shared controls, fields, pickers and menus | T | `transition` |

### `apps/desktop/src/renderer/src/controls/Disclosure.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 14 | `.control-disclosure` | — | Shared controls, fields, pickers and menus | T | `margin-top`, `min-width` |
| 15 | `summary` | — | Shared controls, fields, pickers and menus | T | `color`, `cursor`, `font-size`, `padding`, `user-select` |
| 16 | `summary:hover` | — | Shared controls, fields, pickers and menus | T | `color` |
| 17 | `summary:focus-visible` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `outline`, `outline-offset` |
| 18 | `.disclosure-count` | — | Shared controls, fields, pickers and menus | T | `color`, `font-variant-numeric`, `margin-left` |
| 19 | `.disclosure-changed` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `display`, `height`, `margin-left`, `vertical-align`, `width` |

### `apps/desktop/src/renderer/src/controls/FontField.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 297 | `.font-select` | — | Shared controls, fields, pickers and menus | T | `min-width`, `overflow`, `text-overflow`, `white-space` |
| 298 | `.font-menu` | — | Shared controls, fields, pickers and menus | T | `box-sizing`, `display`, `flex-direction`, `font-weight`, `overflow` |
| 299 | `.font-menu-search` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `color`, `flex-shrink`, `font`, `height`, `margin`, `padding` |
| 300 | `.font-menu-search::placeholder` | — | Shared controls, fields, pickers and menus | T | `color` |
| 301 | `.font-menu-search:focus` | — | Shared controls, fields, pickers and menus | T | `outline` |
| 302 | `.font-menu-list` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-height`, `overflow-y`, `position`, `scrollbar-width` |
| 303 | `.font-menu-glider` | — | Shared controls, fields, pickers and menus | T | `left`, `right` |
| 304 | `.font-menu-row:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline` |
| 307 | `.font-menu-name` | — | Shared controls, fields, pickers and menus | T | `font-size`, `line-height` |
| 308 | `.font-menu-empty` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size`, `margin` |

### `apps/desktop/src/renderer/src/controls/PointField.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 34 | `.point-field` | — | Shared controls, fields, pickers and menus | T | `container-type`, `min-width`, `width` |
| 35 | `.point-layout` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `grid-template-columns`, `min-width` |
| 36 | `.point-pad` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `border-radius`, `cursor`, `flex`, `height`, `min-width`, `outline`, `padding`, `position`, `touch-action` |
| 37 | `.point-pad::before` | — | Shared controls, fields, pickers and menus | T | `background`, `content`, `position` |
| 37 | `.point-pad::after` | — | Shared controls, fields, pickers and menus | T | `background`, `content`, `position` |
| 38 | `.point-pad::before` | — | Shared controls, fields, pickers and menus | T | `height`, `left`, `right`, `top` |
| 39 | `.point-pad::after` | — | Shared controls, fields, pickers and menus | T | `bottom`, `left`, `top`, `width` |
| 40 | `.point-dot` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `height`, `pointer-events`, `position`, `transform`, `width` |
| 41 | `.point-values` | — | Shared controls, fields, pickers and menus | T | `display`, `flex-direction`, `gap`, `min-width` |
| 42 | `.point-axis` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `min-width` |
| 43 | `.point-axis > span` | — | Shared controls, fields, pickers and menus | T | `color`, `font-size` |
| 44 | `.point-axis :global(.well)` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-width` |
| 46 | `.point-layout` | @container (max-width:140px) | Shared controls, fields, pickers and menus | T | `gap`, `grid-template-columns` |
| 47 | `.point-pad` | @container (max-width:140px) | Shared controls, fields, pickers and menus | T | `height` |
| 49 | `.point-pad:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |

### `apps/desktop/src/renderer/src/controls/RampField.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 50 | `.ramp-field` | — | Shared controls, fields, pickers and menus | T | `min-width`, `padding`, `width` |
| 51 | `.ramp-bar` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `height`, `position`, `width` |
| 52 | `.ramp-paint` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `inset`, `pointer-events`, `position` |
| 53 | `.ramp-stop` | — | Shared controls, fields, pickers and menus | T | `border`, `border-radius`, `bottom`, `cursor`, `height`, `position`, `touch-action`, `transform`, `width` |
| 54 | `.ramp-stop.active` | — | Shared controls, fields, pickers and menus | T | `border-color`, `box-shadow` |
| 55 | `.ramp-stop:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 55 | `.ramp-midpoint-track:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 56 | `.ramp-midpoint-track` | — | Shared controls, fields, pickers and menus | T | `cursor`, `height`, `left`, `outline`, `position`, `right`, `top`, `touch-action` |
| 57 | `.ramp-diamond` | — | Shared controls, fields, pickers and menus | T | `background`, `border`, `height`, `pointer-events`, `position`, `top`, `transform`, `width` |

### `apps/desktop/src/renderer/src/controls/SliderField.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 41 | `.visual-number` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `gap`, `min-width`, `width` |
| 42 | `.visual-number :global(.num)` | — | Shared controls, fields, pickers and menus | T | `flex`, `min-width`, `width` |
| 43 | `.visual-track` | — | Shared controls, fields, pickers and menus | T | `cursor`, `flex`, `height`, `outline`, `position`, `touch-action` |
| 44 | `.visual-track:focus-visible` | — | Shared controls, fields, pickers and menus | T | `border-radius`, `outline`, `outline-offset` |
| 45 | `.filled-slider` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `gap`, `height`, `overflow`, `position` |
| 46 | `.filled-slider:has(:focus-visible)` | — | Shared controls, fields, pickers and menus | T | `outline`, `outline-offset` |
| 47 | `.filled-slider .visual-track:focus-visible` | — | Shared controls, fields, pickers and menus | T | `outline` |
| 48 | `.visual-track-fill` | — | Shared controls, fields, pickers and menus | T | `background`, `inset`, `max-width`, `opacity`, `pointer-events`, `position`, `width` |
| 49 | `.filled-slider .visual-track` | — | Shared controls, fields, pickers and menus | T | `align-items`, `display`, `min-width` |
| 50 | `.filled-slider :global(input.num:not([type=range]):not([type=color]):not([type=file]))` | — | Shared controls, fields, pickers and menus | T | `background`, `flex`, `order`, `padding-inline`, `position`, `text-align`, `width` |
| 51 | `.filled-slider :global(input.num.editing)` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius` |
| 52 | `.dial` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `flex`, `height`, `width` |
| 53 | `.dial-hand` | — | Shared controls, fields, pickers and menus | T | `background`, `border-radius`, `height`, `left`, `position`, `top`, `transform`, `transform-origin`, `width` |
| 54 | `.angle :global(.num)` | — | Shared controls, fields, pickers and menus | T | `flex`, `width` |

### `apps/desktop/src/renderer/src/errors/ErrorFix.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 63 | `.error-fix` | — | Component ErrorFix | T | `display`, `flex-direction`, `gap`, `margin-top` |
| 64 | `dl` | — | Component ErrorFix | T | `background`, `border-radius`, `display`, `gap`, `margin`, `overflow` |
| 65 | `dl div` | — | Component ErrorFix | T | `background`, `display`, `gap`, `justify-content`, `padding` |
| 66 | `dt` | — | Component ErrorFix | T | `color`, `font-size` |
| 67 | `dd` | — | Component ErrorFix | T | `color`, `font`, `font-variant-numeric`, `margin`, `text-align` |
| 68 | `dl div:first-child dd` | — | Component ErrorFix | T | `color` |
| 69 | `dl div.is-fixed dd` | — | Component ErrorFix | T | `color` |
| 70 | `.update` | — | Component ErrorFix | T | `display`, `flex-direction`, `gap` |
| 71 | `.update-button` | — | Component ErrorFix | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font`, `font-size`, `font-weight`, `gap`, `justify-content`, `line-height`, `min-height`, `padding`, `width` |
| 73 | `.update-button:hover` | — | Component ErrorFix | T | `background` |
| 74 | `.update-button:active` | — | Component ErrorFix | T | `transform` |
| 75 | `.update-button:disabled` | — | Component ErrorFix | T | `background`, `color`, `transform` |
| 76 | `.update-button:focus-visible` | — | Component ErrorFix | T | `outline`, `outline-offset` |
| 77 | `svg` | — | Component ErrorFix | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 78 | `.spin` | — | Component ErrorFix | T | `animation`, `border`, `border-radius`, `border-top-color`, `flex`, `height`, `width` |
| 79 | `p` | — | Component ErrorFix | T | `font-size`, `line-height`, `margin` |
| 80 | `.done` | — | Component ErrorFix | T | `align-items`, `color`, `display`, `gap` |
| 81 | `.done svg` | — | Component ErrorFix | T | `color` |
| 82 | `.failed` | — | Component ErrorFix | T | `color`, `overflow-wrap` |
| 84 | `.spin` | @media (prefers-reduced-motion: reduce) | Component ErrorFix | T | `animation` |

### `apps/desktop/src/renderer/src/errors/ErrorLog.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 43 | `.error-actions` | — | Component ErrorLog | T | `align-items`, `display`, `flex-wrap`, `gap`, `margin-top` |
| 44 | `.log-toggle` | — | Component ErrorLog | T | `align-items`, `border-radius`, `color`, `display`, `font`, `font-size`, `font-weight`, `gap`, `height`, `margin-left`, `padding` |
| 46 | `.log-toggle:hover` | — | Component ErrorLog | T | `background`, `color` |
| 46 | `.log-toggle[aria-expanded="true"]` | — | Component ErrorLog | T | `background`, `color` |
| 47 | `svg` | — | Component ErrorLog | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 48 | `.log-toggle svg` | — | Component ErrorLog | T | `transition` |
| 49 | `.log-toggle svg.open` | — | Component ErrorLog | T | `transform` |
| 50 | `.log` | — | Component ErrorLog | T | `background`, `border-radius`, `margin-top`, `position` |
| 51 | `pre` | — | Component ErrorLog | T | `color`, `font`, `margin`, `max-height`, `overflow`, `overflow-wrap`, `padding`, `user-select`, `white-space` |
| 53 | `.log-copy` | — | Component ErrorLog | T | `border-radius`, `color`, `display`, `height`, `place-items`, `position`, `right`, `top`, `width` |
| 54 | `.log-copy:hover` | — | Component ErrorLog | T | `background`, `color` |
| 55 | `button:focus-visible` | — | Component ErrorLog | T | `outline`, `outline-offset` |
| 56 | `.log-toggle svg` | @media (prefers-reduced-motion: reduce) | Component ErrorLog | T | `transition` |

### `apps/desktop/src/renderer/src/errors/ErrorNotice.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 37 | `.error-notice` | — | Component ErrorNotice | T | `background`, `border`, `border-radius`, `box-sizing`, `color`, `font`, `min-width`, `overflow-wrap`, `padding`, `width` |
| 38 | `.error-heading` | — | Component ErrorNotice | T | `align-items`, `display`, `gap`, `margin-bottom` |
| 40 | `.error-body` | — | Component ErrorNotice | T | `padding-left` |
| 41 | `strong` | — | Component ErrorNotice | T | `font-size`, `font-weight` |
| 42 | `.error-marker` | — | Component ErrorNotice | T | `background`, `border-radius`, `color`, `display`, `flex`, `font-size`, `font-weight`, `height`, `place-items`, `width` |
| 44 | `.error-notice.is-alert` | — | Component ErrorNotice | T | `background` |
| 45 | `.error-notice.is-alert .error-marker` | — | Component ErrorNotice | T | `background`, `color`, `margin-top` |
| 46 | `.alert-text` | — | Component ErrorNotice | T | `align-items`, `color`, `display`, `font`, `gap`, `margin` |
| 47 | `.error-notice p` | — | Component ErrorNotice | T | `color`, `font`, `margin` |

### `apps/desktop/src/renderer/src/errors/ToastError.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 28 | `.body` | — | Component ToastError | T | `display`, `flex-direction`, `gap`, `min-width`, `padding` |
| 29 | `strong` | — | Component ToastError | T | `color`, `font-size`, `font-weight`, `line-height` |
| 30 | `.message` | — | Component ToastError | T | `color`, `font-weight`, `line-height`, `overflow-wrap` |
| 31 | `.foot:empty` | — | Component ToastError | T | `display` |
| 32 | `.foot :global(.error-actions)` | — | Component ToastError | T | `margin-top` |

### `apps/desktop/src/renderer/src/mixer/Fader.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 261 | `.mx-fader` | — | Component Fader | T | `height`, `min-height`, `outline`, `position`, `touch-action`, `width` |
| 270 | `.mx-fader:focus-visible::after` | — | Component Fader | T | `border-radius`, `box-shadow`, `content`, `inset`, `pointer-events`, `position` |
| 279 | `.mx-fader.disabled` | — | Component Fader | T | `opacity` |
| 281 | `.mx-tick` | — | Component Fader | T | `background`, `height`, `pointer-events`, `position`, `right`, `width` |
| 290 | `.mx-tick.major` | — | Component Fader | T | `background`, `right`, `width` |
| 292 | `.mx-tick.unity` | — | Component Fader | T | `background`, `right`, `width` |
| 294 | `.mx-knob` | — | Component Fader | T | `background`, `border-radius`, `bottom`, `box-shadow`, `display`, `height`, `left`, `place-items`, `position`, `width`, `will-change` |
| 308 | `.mx-knob span` | — | Component Fader | T | `background`, `height`, `opacity`, `width` |
| 315 | `.mx-tick.fine` | — | Component Fader | T | `display` |
| 318 | `.mx-tick.fine` | @container mixer (min-height: 440px) | Component Fader | T | `display` |
| 322 | `.mx-tick.minor` | @container mixer (max-height: 210px) | Component Fader | T | `display` |

### `apps/desktop/src/renderer/src/mixer/LevelReadout.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 105 | `.mx-readout` | — | Component LevelReadout | T | `background`, `border`, `border-radius`, `color`, `cursor`, `font`, `font-variant-numeric`, `height`, `outline`, `padding`, `text-align`, `transition`, `width` |
| 121 | `.mx-readout:not(:disabled):hover` | — | Component LevelReadout | T | `background`, `color` |
| 121 | `.mx-readout.editing` | — | Component LevelReadout | T | `background`, `color` |
| 124 | `.mx-readout.editing` | — | Component LevelReadout | T | `background`, `box-shadow`, `cursor` |
| 126 | `.mx-readout:focus-visible:not(.editing)` | — | Component LevelReadout | T | `box-shadow` |
| 130 | `.mx-readout:disabled` | — | Component LevelReadout | T | `opacity` |

### `apps/desktop/src/renderer/src/mixer/MixerStrip.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 156 | `.mx-strip` | — | Component MixerStrip | T | `display`, `flex`, `grid-template-rows`, `height`, `justify-items`, `min-height`, `padding`, `row-gap`, `width` |
| 168 | `.mx-top` | — | Component MixerStrip | T | `display`, `justify-content`, `position`, `width` |
| 175 | `.mx-key` | — | Component MixerStrip | T | `border-radius`, `color`, `display`, `height`, `left`, `opacity`, `padding`, `place-items`, `position`, `top`, `transition`, `width` |
| 190 | `.mx-strip:hover .mx-key` | — | Component MixerStrip | T | `opacity` |
| 190 | `.mx-key:focus-visible` | — | Component MixerStrip | T | `opacity` |
| 190 | `.mx-key.on` | — | Component MixerStrip | T | `opacity` |
| 193 | `.mx-key:hover` | — | Component MixerStrip | T | `color` |
| 194 | `.mx-key.on` | — | Component MixerStrip | T | `color` |
| 195 | `.mx-key:disabled` | — | Component MixerStrip | T | `visibility` |
| 197 | `.mx-key svg` | — | Component MixerStrip | T | `fill`, `height`, `stroke`, `stroke-width`, `width` |
| 198 | `.mx-key.at-key svg` | — | Component MixerStrip | T | `fill` |
| 200 | `.mx-body` | — | Component MixerStrip | T | `column-gap`, `display`, `grid-template-columns`, `height`, `min-height` |
| 212 | `.mx-meter` | — | Component MixerStrip | T | `display`, `grid-template-rows`, `min-height`, `padding-bottom`, `row-gap` |
| 220 | `.mx-body > :global(.mx-fader)` | — | Component MixerStrip | T | `height`, `margin-top` |
| 222 | `.mx-clip` | — | Component MixerStrip | T | `background`, `border-radius`, `height`, `padding`, `position`, `transition`, `width` |
| 232 | `.mx-clip::before` | — | Component MixerStrip | T | `content`, `inset`, `position` |
| 233 | `.mx-clip.on` | — | Component MixerStrip | T | `background` |
| 234 | `.mx-clip:focus-visible` | — | Component MixerStrip | T | `box-shadow`, `outline` |
| 236 | `.mx-canvas` | — | Component MixerStrip | T | `border-radius`, `display`, `height`, `min-height`, `width` |
| 244 | `.mx-scale` | — | Component MixerStrip | T | `margin`, `min-height`, `position` |
| 250 | `.mx-scale span` | — | Component MixerStrip | T | `color`, `font`, `font-variant-numeric`, `left`, `position`, `transform`, `white-space` |
| 260 | `.mx-scale span.unity` | — | Component MixerStrip | T | `color` |
| 262 | `.mx-switches` | — | Component MixerStrip | T | `display`, `gap` |
| 267 | `.mx-switch` | — | Component MixerStrip | T | `background`, `border-radius`, `color`, `font`, `height`, `padding`, `text-align`, `transition`, `width` |
| 279 | `.mx-switch:hover:not(:disabled)` | — | Component MixerStrip | T | `background`, `color` |
| 280 | `.mx-switch:disabled` | — | Component MixerStrip | T | `opacity` |
| 281 | `.mx-switch.mute.on` | — | Component MixerStrip | T | `background`, `color` |
| 282 | `.mx-switch.solo.on` | — | Component MixerStrip | T | `background`, `color` |
| 284 | `.mx-switch:focus-visible` | — | Component MixerStrip | T | `box-shadow`, `outline` |
| 284 | `.mx-name:focus-visible` | — | Component MixerStrip | T | `box-shadow`, `outline` |
| 284 | `.mx-key:focus-visible` | — | Component MixerStrip | T | `box-shadow`, `outline` |
| 291 | `.mx-name` | — | Component MixerStrip | T | `border-radius`, `color`, `font`, `height`, `max-width`, `overflow`, `padding`, `text-align`, `text-overflow`, `transition`, `white-space` |
| 305 | `button.mx-name:hover` | — | Component MixerStrip | T | `color` |
| 306 | `.mx-strip.selected .mx-name` | — | Component MixerStrip | T | `color`, `font-weight` |
| 307 | `.mx-strip.master .mx-name` | — | Component MixerStrip | T | `color`, `font-weight` |
| 310 | `.mx-strip.quiet .mx-meter` | — | Component MixerStrip | T | `opacity` |
| 310 | `.mx-strip.quiet .mx-scale` | — | Component MixerStrip | T | `opacity` |
| 312 | `.mx-strip.quiet .mx-name` | — | Component MixerStrip | T | `color` |
| 315 | `.mx-scale span.minor` | @container mixer (max-height: 230px) | Component MixerStrip | T | `display` |
| 319 | `.mx-strip` | @container mixer (max-height: 170px) | Component MixerStrip | T | `padding-top`, `row-gap` |

### `apps/desktop/src/renderer/src/onboarding/AgentStep.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 56 | `.agent-step` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `display`, `flex-direction`, `padding-top`, `position`, `text-align`, `width`, `z-index` |
| 58 | `.agent-step > *` | — | Onboarding | T | `animation` |
| 60 | `.back` | — | Onboarding | T | `background`, `color`, `display`, `height`, `left`, `place-items`, `position`, `top`, `width` |
| 61 | `.back:hover` | — | Onboarding | T | `background`, `color` |
| 62 | `h1` | — | Onboarding | T | `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin`, `outline`, `white-space` |
| 63 | `.intro` | — | Onboarding | T | `color`, `font-size`, `line-height`, `margin` |
| 64 | `.choices` | — | Onboarding | T | `display`, `flex-direction`, `gap`, `margin-top`, `width` |
| 65 | `.choice` | — | Onboarding | T | `align-items`, `background`, `border-radius`, `box-sizing`, `cursor`, `display`, `gap`, `min-height`, `padding`, `position`, `text-align`, `transition` |
| 66 | `.choice:hover` | — | Onboarding | T | `background` |
| 67 | `.choice.selected` | — | Onboarding | T | `background`, `box-shadow` |
| 68 | `.choice:has(input:focus-visible)` | — | Onboarding | T | `outline`, `outline-offset` |
| 69 | `input` | — | Onboarding | T | `opacity`, `pointer-events`, `position` |
| 71 | `.check` | — | Onboarding | T | `background`, `border-radius`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 72 | `.check svg` | — | Onboarding | T | `height`, `opacity`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 73 | `.selected .check` | — | Onboarding | T | `background` |
| 74 | `.selected .check svg` | — | Onboarding | T | `opacity` |
| 75 | `.text` | — | Onboarding | T | `display`, `flex`, `gap`, `min-width` |
| 76 | `.name` | — | Onboarding | T | `color`, `font-size`, `font-weight` |
| 77 | `.detail` | — | Onboarding | T | `color`, `font-size`, `line-height` |
| 78 | `.model` | — | Onboarding | T | `flex`, `position`, `width` |
| 79 | `select` | — | Onboarding | T | `appearance`, `background`, `border`, `border-radius`, `color`, `font`, `height`, `padding`, `transition`, `width` |
| 80 | `select:hover` | — | Onboarding | T | `background` |
| 81 | `.choice:not(.selected) select` | — | Onboarding | T | `opacity` |
| 82 | `select:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 83 | `.chevron` | — | Onboarding | T | `height`, `pointer-events`, `position`, `right`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `top`, `translate`, `width` |
| 84 | `.actions` | — | Onboarding | T | `align-items`, `display`, `gap`, `margin-top` |
| 85 | `svg` | — | Onboarding | T | `fill`, `stroke` |
| 86 | `.back svg` | — | Onboarding | T | `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 87 | `button` | — | Onboarding | T | `border`, `border-radius`, `cursor`, `font`, `transition` |
| 88 | `.primary` | — | Onboarding | T | `background`, `color`, `min-height`, `padding` |
| 89 | `.primary:hover` | — | Onboarding | T | `background` |
| 90 | `button:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 92 | `.agent-step` | @media (max-height: 600px) | Onboarding | T | `padding-top`, `width` |
| 92 | `.choices` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 92 | `.actions` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 92 | `.choice` | @media (max-height: 600px) | Onboarding | T | `min-height`, `padding-block` |
| 93 | `.agent-step > *` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 93 | `button` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |
| 93 | `.choice` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |
| 93 | `select` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |

### `apps/desktop/src/renderer/src/onboarding/SuitcaseTowers.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 76 | `.scene` | — | Onboarding | T | `--c`, `inset`, `overflow`, `perspective`, `perspective-origin`, `pointer-events`, `position`, `z-index` |
| 78 | `.case` | — | Onboarding | T | `--shift`, `animation`, `bottom`, `height`, `position`, `transform`, `transform-style`, `width` |
| 79 | `.case.left` | — | Onboarding | T | `left` |
| 80 | `.case.right` | — | Onboarding | T | `right` |
| 82 | `.face` | — | Onboarding | T | `--falloff`, `background`, `display`, `height`, `left`, `position`, `top`, `transform-origin` |
| 83 | `.front` | — | Onboarding | T | `--ribs`, `background`, `background-position` |
| 89 | `.left.far .front` | — | Onboarding | T | `background`, `background-position` |
| 90 | `.right.far .front` | — | Onboarding | T | `background`, `background-position` |
| 92 | `.side` | — | Onboarding | T | `--seam` |
| 93 | `.side` | — | Onboarding | T | `background` |
| 94 | `.grip` | — | Onboarding | T | `background`, `border-radius`, `box-shadow`, `height`, `left`, `position`, `top`, `transform`, `width` |
| 95 | `.trolley` | — | Onboarding | T | `border`, `border-bottom`, `border-radius`, `bottom`, `height`, `left`, `position`, `right`, `transform` |
| 96 | `.tag` | — | Onboarding | T | `background`, `border-radius`, `box-shadow`, `height`, `position`, `right`, `top`, `transform`, `transform-origin`, `width` |
| 97 | `.tag::before` | — | Onboarding | T | `background`, `border-radius`, `box-shadow`, `content`, `height`, `left`, `margin-left`, `position`, `top`, `width` |
| 99 | `.case` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |

### `apps/desktop/src/renderer/src/onboarding/TimelineStep.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 84 | `.timeline-step` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `display`, `flex-direction`, `padding-top`, `position`, `text-align`, `width`, `z-index` |
| 86 | `.timeline-step > *` | — | Onboarding | T | `animation` |
| 88 | `.back` | — | Onboarding | T | `background`, `color`, `display`, `height`, `left`, `place-items`, `position`, `top`, `width` |
| 89 | `.back:hover` | — | Onboarding | T | `background`, `color` |
| 90 | `h1` | — | Onboarding | T | `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin`, `outline`, `white-space` |
| 91 | `.intro` | — | Onboarding | T | `color`, `font-size`, `line-height`, `margin` |
| 92 | `.choices` | — | Onboarding | T | `display`, `flex-direction`, `gap`, `margin-top`, `width` |
| 93 | `.choice` | — | Onboarding | T | `align-items`, `background`, `border-radius`, `cursor`, `display`, `gap`, `grid-template-columns`, `padding`, `position`, `text-align`, `transition` |
| 94 | `.choice:hover` | — | Onboarding | T | `background` |
| 95 | `.choice.selected` | — | Onboarding | T | `background`, `box-shadow` |
| 96 | `.choice:has(input:focus-visible)` | — | Onboarding | T | `outline`, `outline-offset` |
| 97 | `input` | — | Onboarding | T | `opacity`, `pointer-events`, `position` |
| 98 | `.label` | — | Onboarding | T | `align-items`, `display`, `gap` |
| 99 | `.name` | — | Onboarding | T | `color`, `font-size`, `font-weight` |
| 101 | `.check` | — | Onboarding | T | `background`, `border-radius`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 102 | `.check svg` | — | Onboarding | T | `height`, `opacity`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 103 | `.selected .check` | — | Onboarding | T | `background` |
| 104 | `.selected .check svg` | — | Onboarding | T | `opacity` |
| 105 | `.preview` | — | Onboarding | T | `background`, `border-radius`, `display` |
| 106 | `.preview svg` | — | Onboarding | T | `display`, `height`, `width` |
| 107 | `.choice:not(.selected) .preview svg` | — | Onboarding | T | `opacity`, `transition` |
| 108 | `.choice:not(.selected):hover .preview svg` | — | Onboarding | T | `opacity` |
| 109 | `.track-label` | — | Onboarding | T | `fill`, `font` |
| 110 | `.lane` | — | Onboarding | T | `fill` |
| 113 | `.text` | — | Onboarding | T | `--body`, `--fg` |
| 114 | `.video` | — | Onboarding | T | `--body`, `--fg` |
| 115 | `.audio` | — | Onboarding | T | `--body`, `--fg` |
| 116 | `.clip` | — | Onboarding | T | `fill` |
| 117 | `.clip-label` | — | Onboarding | T | `fill`, `font` |
| 118 | `:global([data-theme='dark']) .clip` | — | Onboarding | T | `fill` |
| 119 | `:global([data-theme='dark']) .clip-label` | — | Onboarding | T | `fill` |
| 120 | `.wave` | — | Onboarding | T | `stroke`, `stroke-linecap`, `stroke-width` |
| 121 | `:global([data-theme='dark']) .wave` | — | Onboarding | T | `stroke` |
| 122 | `.playhead` | — | Onboarding | T | `fill` |
| 123 | `.actions` | — | Onboarding | T | `align-items`, `display`, `gap`, `margin-top` |
| 124 | `svg` | — | Onboarding | T | `fill`, `stroke` |
| 125 | `.back svg` | — | Onboarding | T | `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 126 | `button` | — | Onboarding | T | `border`, `border-radius`, `cursor`, `font`, `transition` |
| 127 | `.primary` | — | Onboarding | T | `background`, `color`, `min-height`, `padding` |
| 128 | `.primary:hover` | — | Onboarding | T | `background` |
| 129 | `button:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 131 | `.timeline-step` | @media (max-height: 600px) | Onboarding | T | `padding-top`, `width` |
| 131 | `.choices` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 131 | `.actions` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 132 | `.timeline-step > *` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 132 | `button` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |
| 132 | `.choice` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |

### `apps/desktop/src/renderer/src/onboarding/TranscriptionStep.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 102 | `.transcription-step` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `display`, `flex-direction`, `padding-top`, `position`, `text-align`, `width`, `z-index` |
| 104 | `.transcription-step > *` | — | Onboarding | T | `animation` |
| 106 | `.back` | — | Onboarding | T | `background`, `color`, `display`, `height`, `left`, `place-items`, `position`, `top`, `width` |
| 107 | `.back:hover` | — | Onboarding | T | `background`, `color` |
| 108 | `h1` | — | Onboarding | T | `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin`, `outline`, `white-space` |
| 109 | `.intro` | — | Onboarding | T | `color`, `font-size`, `line-height`, `margin` |
| 110 | `.choices` | — | Onboarding | T | `display`, `flex-direction`, `gap`, `margin-top`, `width` |
| 111 | `.choice` | — | Onboarding | T | `align-items`, `background`, `border-radius`, `box-sizing`, `cursor`, `display`, `gap`, `min-height`, `padding`, `position`, `text-align`, `transition` |
| 112 | `.choice:hover` | — | Onboarding | T | `background` |
| 113 | `.choice.selected` | — | Onboarding | T | `background`, `box-shadow` |
| 115 | `.choice:has(input:focus-visible)` | — | Onboarding | T | `box-shadow` |
| 116 | `input` | — | Onboarding | T | `opacity`, `pointer-events`, `position` |
| 118 | `.check` | — | Onboarding | T | `background`, `border-radius`, `display`, `flex`, `height`, `place-items`, `transition`, `width` |
| 119 | `.check svg` | — | Onboarding | T | `height`, `opacity`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 120 | `.selected .check` | — | Onboarding | T | `background` |
| 121 | `.selected .check svg` | — | Onboarding | T | `opacity` |
| 122 | `.text` | — | Onboarding | T | `display`, `flex`, `gap`, `min-width` |
| 123 | `.name` | — | Onboarding | T | `align-items`, `color`, `display`, `font-size`, `font-weight`, `gap` |
| 124 | `.tag` | — | Onboarding | T | `background`, `border-radius`, `color`, `font-size`, `font-weight`, `line-height`, `padding` |
| 125 | `.detail` | — | Onboarding | T | `color`, `font-size`, `line-height` |
| 126 | `.facts` | — | Onboarding | T | `align-items`, `display`, `flex`, `gap` |
| 127 | `.size` | — | Onboarding | T | `color`, `font-size`, `font-variant-numeric`, `min-width`, `text-align` |
| 128 | `.unavailable` | — | Onboarding | T | `color`, `font-size`, `margin` |
| 129 | `.actions` | — | Onboarding | T | `align-items`, `display`, `gap`, `margin-top` |
| 130 | `.note` | — | Onboarding | T | `color`, `font-size`, `margin` |
| 131 | `svg` | — | Onboarding | T | `fill`, `stroke` |
| 132 | `.back svg` | — | Onboarding | T | `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 133 | `button` | — | Onboarding | T | `border`, `border-radius`, `cursor`, `font`, `transition` |
| 134 | `.primary` | — | Onboarding | T | `min-height`, `padding` |
| 134 | `.secondary` | — | Onboarding | T | `min-height`, `padding` |
| 135 | `.primary` | — | Onboarding | T | `background`, `color` |
| 136 | `.primary:hover` | — | Onboarding | T | `background` |
| 137 | `.secondary` | — | Onboarding | T | `background`, `color` |
| 138 | `.secondary:hover` | — | Onboarding | T | `background`, `color` |
| 139 | `button:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 141 | `.transcription-step` | @media (max-height: 600px) | Onboarding | T | `padding-top`, `width` |
| 141 | `.choices` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 141 | `.actions` | @media (max-height: 600px) | Onboarding | T | `margin-top` |
| 141 | `.choice` | @media (max-height: 600px) | Onboarding | T | `min-height`, `padding-block` |
| 142 | `.transcription-step > *` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 142 | `button` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |
| 142 | `.choice` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |

### `apps/desktop/src/renderer/src/onboarding/WorkspaceStep.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 35 | `.workspace-step` | — | Onboarding | T | `-webkit-app-region`, `align-items`, `display`, `flex-direction`, `padding-top`, `position`, `text-align`, `width`, `z-index` |
| 37 | `.workspace-step > *` | — | Onboarding | T | `animation` |
| 38 | `.back` | — | Onboarding | T | `background`, `color`, `display`, `height`, `left`, `place-items`, `position`, `top`, `width` |
| 39 | `.back:hover` | — | Onboarding | T | `background`, `color` |
| 40 | `.source` | — | Onboarding | T | `align-items`, `color`, `display`, `font-size`, `font-weight`, `gap`, `margin` |
| 41 | `.app-mark` | — | Onboarding | T | `background`, `border-radius`, `color`, `display`, `font-size`, `height`, `letter-spacing`, `place-items`, `width` |
| 42 | `h1` | — | Onboarding | T | `font-size`, `font-weight`, `letter-spacing`, `line-height`, `margin`, `outline`, `white-space` |
| 43 | `.intro` | — | Onboarding | T | `color`, `font-size`, `line-height`, `margin` |
| 44 | `.actions` | — | Onboarding | T | `align-items`, `display`, `gap`, `margin-top` |
| 45 | `svg` | — | Onboarding | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 46 | `button` | — | Onboarding | T | `border`, `border-radius`, `cursor`, `font`, `transition` |
| 47 | `.primary` | — | Onboarding | T | `min-height`, `padding` |
| 47 | `.secondary` | — | Onboarding | T | `min-height`, `padding` |
| 48 | `.primary` | — | Onboarding | T | `background`, `color` |
| 49 | `.primary:hover` | — | Onboarding | T | `background` |
| 50 | `.secondary` | — | Onboarding | T | `background`, `color` |
| 51 | `.secondary:hover` | — | Onboarding | T | `background`, `color` |
| 52 | `button:focus-visible` | — | Onboarding | T | `outline`, `outline-offset` |
| 53 | `button:disabled` | — | Onboarding | T | `opacity` |
| 54 | `.status` | — | Onboarding | T | `color`, `font-size`, `margin`, `min-height` |
| 55 | `.workspace-step > *` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `animation` |
| 55 | `button` | @media (prefers-reduced-motion: reduce) | Onboarding | T | `transition` |

### `apps/desktop/src/renderer/src/overlays/Menu.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 155 | `.menu-icon` | — | Component Menu | T | `align-items`, `display`, `flex`, `height`, `justify-content`, `width` |
| 156 | `.menu-icon :global(svg)` | — | Component Menu | T | `height`, `width` |
| 157 | `.curve-grid` | — | Component Menu | T | `display`, `gap`, `grid-template-columns`, `width` |
| 158 | `.curve-grid > :global(:not(.curve-option))` | — | Component Menu | T | `grid-column` |
| 159 | `.curve-grid .curve-option` | — | Component Menu | T | `display`, `flex-direction`, `font-size`, `gap`, `justify-content`, `min-height`, `padding`, `text-align` |
| 160 | `.curve-option.on` | — | Component Menu | T | `background`, `box-shadow` |
| 161 | `.curve-preview` | — | Component Menu | T | `flex`, `height`, `overflow`, `width` |
| 162 | `.curve-guide` | — | Component Menu | T | `fill`, `opacity`, `stroke`, `stroke-width` |
| 163 | `.curve-handles` | — | Component Menu | T | `fill`, `opacity`, `stroke`, `stroke-width` |
| 164 | `.curve-line` | — | Component Menu | T | `fill`, `stroke`, `stroke-width` |
| 165 | `.curve-control` | — | Component Menu | T | `fill`, `stroke`, `stroke-width` |
| 166 | `.curve-key` | — | Component Menu | T | `fill` |

### `apps/desktop/src/renderer/src/overlays/Modal.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 183 | `.modal-action-error` | — | Component Modal | T | `padding` |

### `apps/desktop/src/renderer/src/overlays/Toasts.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 204 | `.save-icon` | — | Component Toasts | T | `display` |
| 205 | `.save-icon>.normal-icon` | — | Component Toasts | T | `grid-area`, `transition` |
| 205 | `.saved-check` | — | Component Toasts | T | `grid-area`, `transition` |
| 206 | `.normal-icon` | — | Component Toasts | T | `display` |
| 207 | `.saved-check` | — | Component Toasts | T | `color`, `height`, `opacity`, `transform`, `width` |
| 208 | `.save-icon.complete .normal-icon` | — | Component Toasts | T | `opacity`, `transform` |
| 209 | `.save-icon.complete .saved-check` | — | Component Toasts | T | `opacity`, `transform` |
| 210 | `.save-body` | — | Component Toasts | T | `min-width`, `overflow-wrap` |
| 211 | `.save-labels` | — | Component Toasts | T | `display` |
| 212 | `.save-labels>span` | — | Component Toasts | T | `grid-area`, `min-width`, `transition` |
| 213 | `.saved-label` | — | Component Toasts | T | `opacity`, `transform` |
| 214 | `.complete .saving-label` | — | Component Toasts | T | `opacity`, `transform` |
| 215 | `.complete .saved-label` | — | Component Toasts | T | `opacity`, `transform` |
| 216 | `.progress-space` | — | Component Toasts | T | `display`, `grid-template-rows`, `opacity`, `transition` |
| 217 | `.complete .progress-space` | — | Component Toasts | T | `grid-template-rows`, `opacity` |
| 218 | `.save-progress` | — | Component Toasts | T | `min-height`, `overflow`, `position` |
| 219 | `.save-progress::before` | — | Component Toasts | T | `background`, `border-radius`, `content`, `display`, `height`, `margin-top`, `opacity` |
| 220 | `.progress-fill` | — | Component Toasts | T | `background`, `border-radius`, `bottom`, `height`, `left`, `position`, `transform-origin`, `transition`, `width` |
| 221 | `.progress-fill.indeterminate` | — | Component Toasts | T | `animation`, `width` |
| 223 | `.save-labels>span` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `transition` |
| 223 | `.progress-space` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `transition` |
| 223 | `.progress-fill` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `transition` |
| 223 | `.save-icon>.normal-icon` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `transition` |
| 223 | `.saved-check` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `transition` |
| 223 | `.progress-fill.indeterminate` | @media(prefers-reduced-motion:reduce) | Component Toasts | T | `animation`, `transform` |
| 228 | `.toast[data-toast-error]` | — | Component Toasts | T | `align-items`, `min-width` |
| 228 | `.toast[data-toast-kind="alert"]` | — | Component Toasts | T | `align-items`, `min-width` |
| 230 | `.toast[data-toast-error] :global(.toast-icon)` | — | Component Toasts | T | `margin-top` |
| 230 | `.toast[data-toast-kind="alert"] :global(.toast-icon)` | — | Component Toasts | T | `margin-top` |
| 230 | `.toast[data-toast-error]>button` | — | Component Toasts | T | `margin-top` |
| 230 | `.toast[data-toast-kind="alert"]>button` | — | Component Toasts | T | `margin-top` |
| 236 | `.toast>.toast-dismiss` | — | Component Toasts | T | `color`, `margin-left` |
| 237 | `.toast[data-toast-error]>.toast-dismiss` | — | Component Toasts | T | `margin-top` |
| 237 | `.toast[data-toast-kind="alert"]>.toast-dismiss` | — | Component Toasts | T | `margin-top` |
| 239 | `.toast-dismiss :global(.pm-icon)` | — | Component Toasts | T | `height`, `width` |
| 240 | `.toast>.toast-dismiss:hover` | — | Component Toasts | T | `color` |
| 241 | `.toast>.toast-dismiss:focus-visible` | — | Component Toasts | T | `outline`, `outline-offset` |
| 242 | `.alert-body` | — | Component Toasts | T | `display`, `flex-direction`, `gap`, `min-width`, `padding` |
| 243 | `.alert-body>strong` | — | Component Toasts | T | `color`, `font-size`, `font-weight`, `line-height` |
| 244 | `.alert-body>span` | — | Component Toasts | T | `line-height`, `min-width`, `overflow-wrap`, `white-space` |
| 249 | `:global(.toastwrap)` | — | Component Toasts | T | `box-sizing`, `max-height`, `max-width`, `overflow-y`, `overscroll-behavior`, `padding`, `pointer-events`, `top` |
| 250 | `.toast` | — | Component Toasts | T | `flex-shrink`, `pointer-events` |

### `apps/desktop/src/renderer/src/panels/AgentPanel.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 242 | `.agent-panel-toolbar` | — | Panel AgentPanel | T | `flex`, `min-width` |
| 243 | `.agent-popover-control` | — | Panel AgentPanel | T | `display` |
| 244 | `.agent-setup-scroll` | — | Panel AgentPanel | T | `mask-image`, `padding` |

### `apps/desktop/src/renderer/src/panels/EasingPanel.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 346 | `.easing-panel` | — | Panel EasingPanel | T | `color`, `display`, `flex-direction`, `min-width`, `padding`, `width` |
| 347 | `.easing-toolbar` | — | Panel EasingPanel | T | `align-items`, `display`, `flex`, `gap`, `height` |
| 348 | `.easing-toolbar .iconbtn` | — | Panel EasingPanel | T | `flex`, `height`, `min-width` |
| 349 | `.easing-toolbar .iconbtn.on` | — | Panel EasingPanel | T | `background`, `color` |
| 350 | `.easing-toolbar .copy` | — | Panel EasingPanel | T | `position` |
| 351 | `.copy span` | — | Panel EasingPanel | T | `color`, `font-size`, `position`, `right`, `top` |
| 352 | `.graph-wrap` | — | Panel EasingPanel | T | `--accent` |
| 353 | `.graph-tools` | — | Panel EasingPanel | T | `gap`, `height` |
| 354 | `.easing-fields` | — | Panel EasingPanel | T | `padding` |
| 355 | `.preset-groups` | — | Panel EasingPanel | T | `align-items`, `display`, `gap`, `min-height`, `overflow-x`, `padding` |
| 356 | `.preset-groups .iconbtn` | — | Panel EasingPanel | T | `flex`, `height`, `width` |
| 357 | `.group-tab` | — | Panel EasingPanel | T | `background`, `border`, `border-radius`, `color`, `flex`, `font-size`, `height`, `max-width`, `overflow`, `padding`, `text-overflow`, `white-space` |
| 358 | `.group-tab.on` | — | Panel EasingPanel | T | `background`, `color` |
| 359 | `.group-tab:focus-visible` | — | Panel EasingPanel | T | `outline`, `outline-offset` |
| 359 | `.easing-preset:focus-visible` | — | Panel EasingPanel | T | `outline`, `outline-offset` |
| 360 | `.preset-library` | — | Panel EasingPanel | T | `display`, `flex-direction`, `gap`, `padding` |
| 361 | `.preset-grid` | — | Panel EasingPanel | T | `display`, `gap`, `min-width` |
| 362 | `.preset-grid:empty` | — | Panel EasingPanel | T | `display` |
| 363 | `.easing-preset` | — | Panel EasingPanel | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `flex-direction`, `gap`, `justify-content`, `min-width`, `padding` |
| 364 | `.easing-preset svg` | — | Panel EasingPanel | T | `aspect-ratio`, `height`, `max-height`, `width` |
| 365 | `.easing-preset:hover` | — | Panel EasingPanel | T | `background` |
| 365 | `.easing-preset:focus-visible` | — | Panel EasingPanel | T | `background` |
| 366 | `.easing-preset span` | — | Panel EasingPanel | T | `color`, `font-size`, `line-height`, `overflow`, `text-overflow`, `white-space`, `width` |
| 367 | `.easing-preview` | — | Panel EasingPanel | T | `flex`, `height`, `overflow`, `position` |
| 368 | `.easing-preview i` | — | Panel EasingPanel | T | `background`, `border-radius`, `bottom`, `height`, `position`, `transform`, `width` |
| 369 | `.easing-info` | — | Panel EasingPanel | T | `align-items`, `color`, `display`, `font-size`, `line-height`, `min-height`, `overflow-wrap` |

### `apps/desktop/src/renderer/src/panels/MixerPanel.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 242 | `.mixer` | — | Panel MixerPanel | T | `--mx-track`, `align-items`, `container`, `display`, `height`, `min-height`, `width` |
| 253 | `:global(:root[data-theme="dark"]) .mixer` | — | Panel MixerPanel | T | `--mx-track` |
| 255 | `.mixer-strips` | — | Panel MixerPanel | T | `--fade-end`, `--fade-start`, `display`, `flex`, `gap`, `height`, `mask-image`, `min-width`, `overflow-x`, `overflow-y`, `padding`, `scrollbar-width` |
| 270 | `.mixer-strips.fade-start` | — | Panel MixerPanel | T | `--fade-start` |
| 271 | `.mixer-strips.fade-end` | — | Panel MixerPanel | T | `--fade-end` |
| 273 | `.mixer-master` | — | Panel MixerPanel | T | `flex`, `height`, `padding` |
| 279 | `.mixer-empty` | — | Panel MixerPanel | T | `align-self`, `color`, `flex`, `font-size`, `padding`, `text-align` |

### `apps/desktop/src/renderer/src/panels/RepairAgent.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 279 | `:global(.extension-repair-popup[hidden])` | — | Panel RepairAgent | T | `display` |
| 282 | `.repair-window` | — | Panel RepairAgent | T | `-webkit-app-region`, `bottom`, `right`, `z-index` |
| 283 | `.repair-window.is-placed` | — | Panel RepairAgent | T | `bottom`, `right` |
| 284 | `.repair-window .agent-panel-body` | — | Panel RepairAgent | T | `display`, `flex`, `min-height` |
| 286 | `.repair-thread-bar` | — | Panel RepairAgent | T | `align-items`, `color`, `display`, `flex-shrink`, `font-size`, `gap`, `height`, `margin`, `min-width`, `padding` |
| 287 | `.repair-thread-title` | — | Panel RepairAgent | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 288 | `.repair-thread-dot` | — | Panel RepairAgent | T | `background`, `border-radius`, `flex`, `height`, `width` |
| 289 | `.repair-message` | — | Panel RepairAgent | T | `display` |
| 290 | `.repair-attachments` | — | Panel RepairAgent | T | `padding` |
| 291 | `.repair-connection` | — | Panel RepairAgent | T | `align-items`, `display`, `flex-direction`, `gap` |
| 292 | `.agent-composer textarea::placeholder` | — | Panel RepairAgent | T | `color` |

### `apps/desktop/src/renderer/src/panels/ShaderPanel.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 92 | `.shader-panel` | — | Panel ShaderPanel | T | `display`, `flex-direction`, `height`, `min-height`, `width` |
| 100 | `.shader-empty` | — | Panel ShaderPanel | T | `align-items`, `color`, `display`, `flex-direction`, `gap`, `height`, `justify-content`, `padding`, `text-align` |
| 112 | `.shader-empty b` | — | Panel ShaderPanel | T | `color`, `font-size`, `font-weight` |
| 118 | `.shader-empty span` | — | Panel ShaderPanel | T | `font-size`, `line-height`, `max-width` |
| 124 | `.codebar-spacer` | — | Panel ShaderPanel | T | `flex` |
| 128 | `.preset-select` | — | Panel ShaderPanel | T | `max-width` |
| 132 | `.compile-button` | — | Panel ShaderPanel | T | `height` |

### `apps/desktop/src/renderer/src/panels/agent/AgentAtmosphere.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 57 | `.agent-atmosphere` | — | Panel AgentAtmosphere | T | `color`, `contain`, `display`, `flex`, `height`, `isolation`, `overflow`, `pointer-events`, `position`, `user-select`, `width` |
| 70 | `.compact` | — | Panel AgentAtmosphere | T | `height`, `width` |
| 71 | `svg` | — | Panel AgentAtmosphere | T | `height`, `inset`, `overflow`, `position`, `width` |
| 72 | `svg path` | — | Panel AgentAtmosphere | T | `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width` |
| 73 | `.plane-back` | — | Panel AgentAtmosphere | T | `opacity` |
| 74 | `.plane-middle` | — | Panel AgentAtmosphere | T | `opacity` |
| 75 | `.plane-front` | — | Panel AgentAtmosphere | T | `opacity` |
| 76 | `.plane-crest` | — | Panel AgentAtmosphere | T | `opacity`, `transition` |
| 77 | `.active .plane-crest` | — | Panel AgentAtmosphere | T | `opacity`, `stroke` |
| 78 | `.atmosphere-light` | — | Panel AgentAtmosphere | T | `background`, `inset`, `mask-image`, `opacity`, `overflow`, `position`, `transition` |
| 87 | `.active .atmosphere-light` | — | Panel AgentAtmosphere | T | `opacity` |
| 88 | `.atmosphere-field` | — | Panel AgentAtmosphere | T | `inset`, `opacity`, `position` |
| 93 | `.atmosphere-field :global(canvas)` | — | Panel AgentAtmosphere | T | `display`, `height`, `pointer-events`, `width` |
| 94 | `.compact .atmosphere-light` | — | Panel AgentAtmosphere | T | `inset` |
| 95 | `.compact .plane-back` | — | Panel AgentAtmosphere | T | `opacity` |
| 95 | `.compact .plane-front` | — | Panel AgentAtmosphere | T | `opacity` |
| 96 | `.compact svg path` | — | Panel AgentAtmosphere | T | `stroke-width` |
| 98 | `.plane-crest` | @media (prefers-reduced-motion: reduce) | Panel AgentAtmosphere | T | `transition` |
| 98 | `.atmosphere-light` | @media (prefers-reduced-motion: reduce) | Panel AgentAtmosphere | T | `transition` |
| 99 | `.atmosphere-field` | @media (prefers-reduced-motion: reduce) | Panel AgentAtmosphere | T | `display` |

### `apps/desktop/src/renderer/src/panels/agent/ConnectionGate.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 64 | `.agent-connect-gate` | — | Panel ConnectionGate | T | `display`, `flex`, `flex-direction`, `gap`, `margin`, `max-width`, `text-align`, `width` |
| 65 | `.connection-providers` | — | Panel ConnectionGate | T | `background`, `border-radius`, `display`, `gap`, `padding` |
| 66 | `.connection-providers button` | — | Panel ConnectionGate | T | `border-radius`, `color`, `flex`, `font-size`, `font-weight`, `line-height`, `min-height`, `min-width`, `padding` |
| 67 | `.connection-providers button:hover` | — | Panel ConnectionGate | T | `background`, `color` |
| 68 | `.connection-providers button[aria-pressed="true"]` | — | Panel ConnectionGate | T | `background`, `box-shadow`, `color` |
| 69 | `.connection-card` | — | Panel ConnectionGate | T | `background`, `border`, `border-radius`, `display`, `flex-direction`, `gap`, `padding` |
| 70 | `h3` | — | Panel ConnectionGate | T | `color`, `font-size`, `font-weight`, `line-height` |
| 71 | `.connection-copy p` | — | Panel ConnectionGate | T | `color`, `font-size`, `line-height`, `margin-top`, `overflow-wrap` |
| 72 | `.agent-connect-button` | — | Panel ConnectionGate | T | `align-items`, `background`, `border-radius`, `color`, `display`, `font-size`, `font-weight`, `gap`, `justify-content`, `line-height`, `min-height`, `padding`, `width` |
| 73 | `.agent-connect-button:hover` | — | Panel ConnectionGate | T | `background` |
| 74 | `.agent-connect-button:active` | — | Panel ConnectionGate | T | `transform` |
| 75 | `.agent-connect-button:disabled` | — | Panel ConnectionGate | T | `background`, `color`, `opacity` |
| 76 | `.agent-connect-button svg` | — | Panel ConnectionGate | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 76 | `.connection-settings svg` | — | Panel ConnectionGate | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 77 | `.connection-settings` | — | Panel ConnectionGate | T | `align-items`, `align-self`, `border-radius`, `color`, `display`, `font-size`, `gap`, `justify-content`, `margin-top`, `min-height`, `padding` |
| 78 | `.connection-settings:hover` | — | Panel ConnectionGate | T | `background`, `color` |
| 79 | `button:focus-visible` | — | Panel ConnectionGate | T | `outline`, `outline-offset` |
| 80 | `.connection-wait` | — | Panel ConnectionGate | T | `animation`, `border`, `border-radius`, `border-top-color`, `flex`, `height`, `width` |
| 81 | `.compact` | — | Panel ConnectionGate | T | `gap`, `margin` |
| 83 | `.connection-wait` | @media (prefers-reduced-motion: reduce) | Panel ConnectionGate | T | `animation` |

### `apps/desktop/src/renderer/src/panels/agent/GhostCard.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 69 | `.ui-placement-ghost` | — | Panel GhostCard | T | `animation`, `background`, `border-radius`, `box-shadow`, `box-sizing`, `color`, `container-type`, `display`, `flex`, `flex-direction`, `isolation`, `min-height`, `overflow`, `pointer-events`, `position`, `user-select` |
| 88 | `.new-panel` | — | Panel GhostCard | T | `background`, `border`, `box-shadow` |
| 95 | `.section` | — | Panel GhostCard | T | `background`, `border`, `box-shadow`, `flex`, `min-height`, `width` |
| 103 | `.section .ghost-head` | — | Panel GhostCard | T | `height` |
| 104 | `.section .ghost-rows` | — | Panel GhostCard | T | `padding-top` |
| 106 | `.ghost-edge-field` | — | Panel GhostCard | T | `border-radius`, `inset`, `mix-blend-mode`, `overflow`, `pointer-events`, `position` |
| 114 | `:global(:root[data-theme='dark']) .ghost-edge-field` | — | Panel GhostCard | T | `mix-blend-mode` |
| 116 | `:global(.ghost-edge-field[data-ghost-renderer='static'])` | — | Panel GhostCard | T | `opacity` |
| 118 | `.ghost-head` | — | Panel GhostCard | T | `align-items`, `color`, `display`, `flex`, `font`, `gap`, `height`, `letter-spacing`, `min-width`, `padding`, `position`, `z-index` |
| 132 | `.ghost-head span` | — | Panel GhostCard | T | `overflow`, `text-overflow`, `white-space` |
| 133 | `.ghost-head i` | — | Panel GhostCard | T | `animation`, `background`, `border-radius`, `box-shadow`, `flex`, `height` |
| 141 | `.ghost-rows` | — | Panel GhostCard | T | `display`, `flex`, `flex-direction`, `gap`, `mask`, `min-height`, `overflow`, `padding`, `position`, `z-index` |
| 155 | `.ghost-row` | — | Panel GhostCard | T | `align-items`, `animation`, `display`, `gap` |
| 156 | `.ghost-row i` | — | Panel GhostCard | T | `background`, `border-radius`, `flex`, `height` |
| 157 | `.ghost-row b` | — | Panel GhostCard | T | `background`, `border-radius`, `flex`, `height`, `min-width` |
| 169 | `.ghost-row i` | @container (max-width: 168px) | Panel GhostCard | T | `display` |
| 184 | `.ui-placement-ghost` | @media (prefers-reduced-motion: reduce) | Panel GhostCard | T | `animation` |
| 185 | `.ghost-edge-field` | @media (prefers-reduced-motion: reduce) | Panel GhostCard | T | `display` |
| 186 | `.ghost-head i` | @media (prefers-reduced-motion: reduce) | Panel GhostCard | T | `animation` |
| 186 | `.ghost-row` | @media (prefers-reduced-motion: reduce) | Panel GhostCard | T | `animation` |

### `apps/desktop/src/renderer/src/panels/agent/Markdown.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 62 | `.agent-trace-code` | — | Panel Markdown | T | `background`, `border-radius`, `box-decoration-break`, `font-family`, `font-size`, `padding` |
| 63 | `:is(.agent-md-p, .agent-md-h, .agent-md-li, .agent-md-quote, .agent-md-pre) + :is(.agent-md-p, .agent-md-h, .agent-md-quote, .agent-md-pre)` | — | Panel Markdown | T | `margin-top` |
| 64 | `.agent-md-p` | — | Panel Markdown | T | `margin`, `overflow-wrap`, `white-space` |
| 64 | `.agent-md-h` | — | Panel Markdown | T | `margin`, `overflow-wrap`, `white-space` |
| 65 | `.agent-md-h` | — | Panel Markdown | T | `color`, `font-weight` |
| 66 | `.agent-md-h[data-level="1"]` | — | Panel Markdown | T | `font-size` |
| 66 | `.agent-md-h[data-level="2"]` | — | Panel Markdown | T | `font-size` |
| 67 | `.is-bold` | — | Panel Markdown | T | `font-weight` |
| 68 | `.is-struck` | — | Panel Markdown | T | `text-decoration` |
| 69 | `.agent-md-rule` | — | Panel Markdown | T | `border`, `border-top`, `margin` |
| 70 | `.agent-md-table-wrap` | — | Panel Markdown | T | `margin`, `max-width`, `overflow-x` |
| 71 | `table` | — | Panel Markdown | T | `border-collapse`, `font`, `width` |
| 72 | `th` | — | Panel Markdown | T | `border-bottom`, `overflow-wrap`, `padding`, `text-align` |
| 72 | `td` | — | Panel Markdown | T | `border-bottom`, `overflow-wrap`, `padding`, `text-align` |
| 73 | `th` | — | Panel Markdown | T | `font-weight` |
| 74 | `.is-italic` | — | Panel Markdown | T | `font-style` |
| 75 | `.agent-md-li` | — | Panel Markdown | T | `display`, `gap`, `min-width` |
| 76 | `.agent-md-li > p` | — | Panel Markdown | T | `flex`, `min-width` |
| 77 | `.agent-md-ordinal` | — | Panel Markdown | T | `color`, `flex`, `font-variant-numeric`, `min-width`, `text-align` |
| 78 | `.agent-md-bullet` | — | Panel Markdown | T | `background`, `border-radius`, `flex`, `height`, `margin-top`, `width` |
| 79 | `.agent-md-quote` | — | Panel Markdown | T | `border-left`, `color`, `margin`, `padding-left` |
| 80 | `.agent-md-pre` | — | Panel Markdown | T | `background`, `border-radius`, `box-shadow`, `color`, `font`, `margin`, `overflow-wrap`, `padding`, `white-space` |
| 85 | `.agent-md-link` | — | Panel Markdown | T | `color`, `text-decoration`, `text-decoration-color`, `text-underline-offset`, `transition` |
| 86 | `.agent-md-link:hover` | — | Panel Markdown | T | `text-decoration-color` |

### `apps/desktop/src/renderer/src/panels/agent/ModResult.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 37 | `.agent-mod-result` | — | Panel ModResult | T | `--mod-tint`, `align-items`, `background`, `border-radius`, `display`, `flex-wrap`, `gap`, `min-width`, `padding` |
| 38 | `.mod-mark` | — | Panel ModResult | T | `background`, `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 39 | `svg` | — | Panel ModResult | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 40 | `.mod-copy` | — | Panel ModResult | T | `display`, `flex`, `flex-direction`, `gap`, `min-width`, `overflow-wrap` |
| 41 | `small` | — | Panel ModResult | T | `color`, `font-size`, `line-height` |
| 42 | `b` | — | Panel ModResult | T | `color`, `font-size`, `font-weight`, `line-height` |
| 43 | `button` | — | Panel ModResult | T | `background`, `flex`, `padding` |
| 44 | `button span` | — | Panel ModResult | T | `color`, `margin-left` |
| 45 | `.mod-panels` | — | Panel ModResult | T | `display`, `flex-basis`, `flex-wrap`, `gap` |
| 46 | `.needs-attention` | — | Panel ModResult | T | `--mod-tint` |

### `apps/desktop/src/renderer/src/panels/agent/QuestionCard.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 92 | `.agent-question` | — | Panel QuestionCard | T | `background`, `border-radius`, `display`, `flex-direction`, `gap`, `min-width`, `padding` |
| 93 | `.agent-question.is-settled` | — | Panel QuestionCard | T | `background`, `border-radius`, `box-shadow`, `gap`, `padding` |
| 94 | `.agent-question-item` | — | Panel QuestionCard | T | `display`, `flex-direction`, `gap`, `min-width` |
| 95 | `.agent-question-header` | — | Panel QuestionCard | T | `color`, `font-size`, `font-weight` |
| 96 | `.agent-question-text` | — | Panel QuestionCard | T | `color`, `font-size`, `line-height`, `margin`, `overflow-wrap`, `white-space` |
| 97 | `.is-settled .agent-question-text` | — | Panel QuestionCard | T | `color`, `font-size` |
| 98 | `.agent-question-options` | — | Panel QuestionCard | T | `display`, `flex-direction`, `gap` |
| 99 | `.agent-question-option` | — | Panel QuestionCard | T | `align-items`, `border-radius`, `color`, `display`, `font-size`, `gap`, `padding`, `text-align`, `transition`, `width` |
| 104 | `.agent-question-option:hover` | — | Panel QuestionCard | T | `background`, `color` |
| 105 | `.agent-question-option.is-chosen` | — | Panel QuestionCard | T | `background`, `color` |
| 106 | `.agent-question-label` | — | Panel QuestionCard | T | `display`, `flex-direction`, `gap`, `min-width` |
| 107 | `.agent-question-option small` | — | Panel QuestionCard | T | `color`, `font-size`, `line-height` |
| 109 | `.agent-question-mark` | — | Panel QuestionCard | T | `border-radius`, `box-shadow`, `flex`, `height`, `margin-top`, `transition`, `width` |
| 113 | `.agent-question-option.is-multiple .agent-question-mark` | — | Panel QuestionCard | T | `border-radius` |
| 114 | `.agent-question-option:hover .agent-question-mark` | — | Panel QuestionCard | T | `box-shadow` |
| 115 | `.agent-question-option.is-chosen .agent-question-mark` | — | Panel QuestionCard | T | `background`, `box-shadow` |
| 116 | `.agent-question-hint` | — | Panel QuestionCard | T | `color`, `font-size`, `margin` |
| 117 | `.agent-question-input` | — | Panel QuestionCard | T | `background`, `border-radius`, `box-shadow`, `color`, `font-size`, `height`, `padding` |
| 121 | `.agent-question-input:focus-visible` | — | Panel QuestionCard | T | `box-shadow`, `outline` |
| 122 | `.agent-question-actions` | — | Panel QuestionCard | T | `display`, `gap`, `justify-content` |
| 123 | `.agent-question-actions .agent-btn:disabled` | — | Panel QuestionCard | T | `opacity`, `pointer-events` |
| 124 | `.agent-question-answer` | — | Panel QuestionCard | T | `color`, `font-size`, `margin`, `overflow-wrap`, `white-space` |
| 125 | `.agent-question-answer.is-missing` | — | Panel QuestionCard | T | `color` |

### `apps/desktop/src/renderer/src/panels/agent/ResultActions.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 119 | `.result-summary` | — | Panel ResultActions | T | `font-weight` |
| 120 | `.agent-run-details` | — | Panel ResultActions | T | `color`, `display`, `flex`, `flex-direction`, `font-size`, `gap`, `line-height`, `min-width`, `overflow-wrap` |
| 121 | `.agent-run-details:empty` | — | Panel ResultActions | T | `display` |
| 122 | `.agent-run-details p` | — | Panel ResultActions | T | `margin`, `white-space` |
| 123 | `.agent-review-markdown` | — | Panel ResultActions | T | `display`, `flex-direction`, `gap`, `min-width` |
| 124 | `.agent-run-details details` | — | Panel ResultActions | T | `color`, `font-size` |
| 125 | `.agent-run-files` | — | Panel ResultActions | T | `color`, `font-size` |
| 126 | `.agent-run-files ul` | — | Panel ResultActions | T | `display`, `flex-direction`, `gap`, `list-style`, `margin`, `padding` |
| 127 | `.agent-run-files li` | — | Panel ResultActions | T | `align-items`, `display`, `gap`, `min-width` |
| 128 | `.agent-run-file-name` | — | Panel ResultActions | T | `color`, `flex`, `min-width`, `overflow`, `text-align`, `text-overflow`, `white-space` |
| 129 | `.agent-run-files small` | — | Panel ResultActions | T | `flex`, `font-variant-numeric` |
| 130 | `.agent-run-files li > button:not(.agent-run-file-name)` | — | Panel ResultActions | T | `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 131 | `.agent-run-files li > button:hover:not(:disabled)` | — | Panel ResultActions | T | `color` |
| 132 | `.agent-run-files :global(.pm-icon)` | — | Panel ResultActions | T | `height`, `width` |
| 133 | `.agent-run-undo` | — | Panel ResultActions | T | `align-self`, `color`, `font-size`, `padding` |
| 134 | `.agent-run-undo:hover` | — | Panel ResultActions | T | `color` |

### `apps/desktop/src/renderer/src/panels/agent/SlashMenu.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 50 | `.agent-slash-menu` | — | Panel SlashMenu | T | `background`, `border`, `border-radius`, `box-shadow`, `box-sizing`, `color`, `font`, `inset`, `margin`, `overflow-y`, `padding`, `position` |
| 51 | `button` | — | Panel SlashMenu | T | `background`, `border`, `border-radius`, `color`, `display`, `flex-direction`, `font`, `gap`, `min-width`, `overflow-wrap`, `padding`, `text-align`, `width` |
| 52 | `button[aria-selected="true"]` | — | Panel SlashMenu | T | `background` |
| 52 | `button:hover` | — | Panel SlashMenu | T | `background` |
| 53 | `button:focus-visible` | — | Panel SlashMenu | T | `outline`, `outline-offset` |
| 54 | `small` | — | Panel SlashMenu | T | `color`, `font-size` |

### `apps/desktop/src/renderer/src/panels/agent/ThoughtRow.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 14 | `.agent-thought-prose` | — | Panel ThoughtRow | T | `color`, `font-size`, `line-height` |
| 15 | `.agent-thought-prose :global(.agent-md-h)` | — | Panel ThoughtRow | T | `color` |
| 15 | `.agent-thought-prose :global(.is-bold)` | — | Panel ThoughtRow | T | `color` |

### `apps/desktop/src/renderer/src/panels/agent/ThreadPicker.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 323 | `.thread-bar` | — | Panel ThreadPicker | T | `align-items`, `display`, `flex-shrink`, `gap`, `margin`, `min-width`, `padding` |
| 334 | `.thread-trigger` | — | Panel ThreadPicker | T | `align-items`, `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `flex`, `font`, `font-size`, `gap`, `height`, `min-width`, `padding`, `text-align` |
| 335 | `.thread-title` | — | Panel ThreadPicker | T | `flex`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 336 | `.thread-chevron` | — | Panel ThreadPicker | T | `flex-shrink`, `height`, `width` |
| 337 | `.thread-new` | — | Panel ThreadPicker | T | `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `flex-shrink`, `height`, `padding`, `place-items`, `width` |
| 338 | `.thread-trigger:hover:not(:disabled)` | — | Panel ThreadPicker | T | `color` |
| 338 | `.thread-new:hover:not(:disabled)` | — | Panel ThreadPicker | T | `color` |
| 339 | `.thread-new:hover:not(:disabled)` | — | Panel ThreadPicker | T | `background` |
| 340 | `:disabled` | — | Panel ThreadPicker | T | `cursor`, `opacity` |
| 341 | `svg` | — | Panel ThreadPicker | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 342 | `.thread-warning` | — | Panel ThreadPicker | T | `color`, `font-size`, `margin` |

### `apps/desktop/src/renderer/src/panels/agent/ToolActivity.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 181 | `.agent-tool-activity` | — | Panel ToolActivity | T | `border`, `color`, `display`, `font-size`, `interpolate-size`, `min-width` |
| 189 | `.agent-tool-activity::details-content` | — | Panel ToolActivity | T | `block-size`, `display`, `opacity`, `overflow`, `transition` |
| 196 | `.agent-tool-activity[open]::details-content` | — | Panel ToolActivity | T | `block-size`, `opacity` |
| 198 | `summary` | — | Panel ToolActivity | T | `align-items`, `border-radius`, `color`, `column-gap`, `display`, `grid-template-columns`, `line-height`, `list-style`, `max-width`, `min-width`, `padding`, `transition`, `width` |
| 213 | `summary:hover` | — | Panel ToolActivity | T | `background`, `color` |
| 214 | `summary:focus-visible` | — | Panel ToolActivity | T | `outline`, `outline-offset` |
| 215 | `summary::-webkit-details-marker` | — | Panel ToolActivity | T | `display` |
| 216 | `summary > span` | — | Panel ToolActivity | T | `min-width`, `overflow-wrap` |
| 217 | `summary em` | — | Panel ToolActivity | T | `color`, `font-size`, `font-style`, `font-variant-numeric`, `margin-left` |
| 218 | `.is-headerless` | — | Panel ToolActivity | T | `padding-top` |
| 219 | `.agent-tool-chevron` | — | Panel ToolActivity | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `transform`, `transition`, `width` |
| 224 | `details[open] > summary .agent-tool-chevron` | — | Panel ToolActivity | T | `transform` |
| 225 | `.is-error > summary > span` | — | Panel ToolActivity | T | `color` |
| 225 | `.is-error > summary em` | — | Panel ToolActivity | T | `color` |
| 227 | `.agent-tool-details` | — | Panel ToolActivity | T | `-webkit-user-select`, `color`, `display`, `flex-direction`, `gap`, `min-width`, `padding`, `user-select` |
| 237 | `.agent-tool-details > div` | — | Panel ToolActivity | T | `min-width` |
| 238 | `.agent-tool-details > .is-subtask` | — | Panel ToolActivity | T | `margin-left` |
| 239 | `.agent-tool-line` | — | Panel ToolActivity | T | `align-items`, `display`, `min-width` |
| 240 | `.agent-tool-line > .agent-tool-row` | — | Panel ToolActivity | T | `flex`, `min-width` |
| 241 | `.agent-task-stop` | — | Panel ToolActivity | T | `flex`, `font-size` |
| 242 | `.agent-task-error` | — | Panel ToolActivity | T | `color`, `font-size`, `margin` |
| 243 | `.agent-task-progress` | — | Panel ToolActivity | T | `color`, `font-size`, `margin`, `overflow-wrap` |
| 244 | `.agent-tool-row` | — | Panel ToolActivity | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `font`, `gap`, `margin`, `min-height`, `min-width`, `padding`, `position`, `text-align`, `transition`, `width` |
| 262 | `button.agent-tool-row:hover` | — | Panel ToolActivity | T | `background` |
| 262 | `.is-open > .agent-tool-line > .agent-tool-row` | — | Panel ToolActivity | T | `background` |
| 263 | `button.agent-tool-row:focus-visible` | — | Panel ToolActivity | T | `outline`, `outline-offset` |
| 264 | `.agent-tool-row > span` | — | Panel ToolActivity | T | `color`, `flex`, `font-weight`, `min-width`, `overflow-wrap` |
| 267 | `.agent-tool-row > span:has(+ .agent-tool-chip)` | — | Panel ToolActivity | T | `flex-shrink`, `overflow-wrap` |
| 268 | `.is-task > .agent-tool-line > .agent-tool-row > span` | — | Panel ToolActivity | T | `flex-shrink`, `overflow-wrap` |
| 269 | `.agent-tool-row > em` | — | Panel ToolActivity | T | `color`, `flex`, `font-size`, `font-style`, `font-variant-numeric`, `margin-left` |
| 270 | `.is-thought > .agent-tool-line > .agent-tool-row > span` | — | Panel ToolActivity | T | `color` |
| 271 | `.agent-tool-glyph` | — | Panel ToolActivity | T | `flex`, `height`, `width` |
| 271 | `.agent-tool-spinner` | — | Panel ToolActivity | T | `flex`, `height`, `width` |
| 272 | `.agent-tool-glyph` | — | Panel ToolActivity | T | `color`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `transition` |
| 273 | `.agent-tool-spinner` | — | Panel ToolActivity | T | `animation`, `border`, `border-radius`, `border-top-color` |
| 274 | `.agent-tool-row-chevron` | — | Panel ToolActivity | T | `fill`, `height`, `left`, `margin-top`, `opacity`, `position`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `top`, `transform`, `transition`, `width` |
| 279 | `button.agent-tool-row:hover > :is(.agent-tool-glyph, .agent-tool-spinner)` | — | Panel ToolActivity | T | `opacity` |
| 279 | `.is-open > .agent-tool-line > .agent-tool-row > :is(.agent-tool-glyph, .agent-tool-spinner)` | — | Panel ToolActivity | T | `opacity` |
| 280 | `button.agent-tool-row:hover > .agent-tool-row-chevron` | — | Panel ToolActivity | T | `opacity` |
| 280 | `.is-open > .agent-tool-line > .agent-tool-row > .agent-tool-row-chevron` | — | Panel ToolActivity | T | `opacity` |
| 281 | `.is-open > .agent-tool-line > .agent-tool-row > .agent-tool-row-chevron` | — | Panel ToolActivity | T | `transform` |
| 284 | `.agent-tool-chip` | — | Panel ToolActivity | T | `background`, `border-radius`, `box-shadow`, `color`, `display`, `flex`, `font`, `height`, `max-width`, `min-width`, `overflow`, `padding`, `text-overflow`, `white-space` |
| 300 | `.agent-tool-chip.is-prose` | — | Panel ToolActivity | T | `color`, `font-family`, `font-size` |
| 301 | `.is-failed > .agent-tool-line > .agent-tool-row > span` | — | Panel ToolActivity | T | `color` |
| 301 | `.is-failed > .agent-tool-line > .agent-tool-row > em` | — | Panel ToolActivity | T | `color` |
| 301 | `.is-failed > .agent-tool-line > .agent-tool-row > .agent-tool-glyph` | — | Panel ToolActivity | T | `color` |
| 303 | `.agent-tool-output` | — | Panel ToolActivity | T | `display`, `grid-template-rows`, `opacity`, `transition` |
| 309 | `.agent-tool-output.is-open` | — | Panel ToolActivity | T | `grid-template-rows`, `opacity` |
| 310 | `.agent-tool-output > div` | — | Panel ToolActivity | T | `min-height`, `overflow` |
| 311 | `.agent-tool-output pre` | — | Panel ToolActivity | T | `border-left`, `color`, `margin`, `overflow-wrap`, `padding`, `white-space` |
| 311 | `.agent-tool-thought` | — | Panel ToolActivity | T | `border-left`, `color`, `margin`, `overflow-wrap`, `padding`, `white-space` |
| 319 | `.agent-tool-output pre` | — | Panel ToolActivity | T | `font` |
| 320 | `.agent-tool-thought` | — | Panel ToolActivity | T | `color`, `font-size`, `line-height` |
| 321 | `.agent-tool-thought.is-task-result` | — | Panel ToolActivity | T | `color` |
| 323 | `.agent-tool-files` | — | Panel ToolActivity | T | `border-top`, `display`, `flex-wrap`, `gap`, `margin`, `padding-top` |
| 324 | `.agent-tool-file` | — | Panel ToolActivity | T | `background`, `border-radius`, `box-shadow`, `color`, `display`, `font`, `height`, `max-width`, `overflow`, `padding`, `text-overflow`, `white-space` |
| 331 | `.agent-tool-activity::details-content` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `transition` |
| 331 | `.agent-tool-chevron` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `transition` |
| 331 | `.agent-tool-row-chevron` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `transition` |
| 331 | `.agent-tool-output` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `transition` |
| 332 | `.agent-tool-details > div` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `animation` |
| 332 | `.agent-tool-file` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `animation` |
| 333 | `.agent-tool-spinner` | @media (prefers-reduced-motion: reduce) | Panel ToolActivity | T | `animation`, `border-top-color` |

### `apps/desktop/src/renderer/src/panels/agent/Turn.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 127 | `.agent-visual-attachments` | — | Panel Turn | T | `display`, `gap`, `margin-top`, `max-width`, `overflow-x`, `overscroll-behavior-x`, `padding` |
| 128 | `.agent-prompt-segment` | — | Panel Turn | T | `display` |
| 129 | `.agent-prompt-segment :global(.agent-md-p:only-child)` | — | Panel Turn | T | `display` |
| 130 | `.agent-prompt-signal` | — | Panel Turn | T | `border-radius`, `inset`, `overflow`, `pointer-events`, `position`, `z-index` |
| 131 | `.agent-prompt-signal :global(canvas)` | — | Panel Turn | T | `opacity` |
| 132 | `.agent-prompt-signal::before` | — | Panel Turn | T | `border-radius`, `box-shadow`, `content`, `inset`, `position` |
| 133 | `.agent-prompt-signal:global([data-glow-renderer="motion-gpu"])::before` | — | Panel Turn | T | `display` |
| 134 | `.agent-message-focus` | — | Panel Turn | T | `color`, `font`, `text-wrap` |
| 136 | `.agent-msg.user.is-steering` | — | Panel Turn | T | `margin-top` |

### `apps/desktop/src/renderer/src/panels/agent/UIPlacementGhost.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 84 | `:global([data-agent-editing])` | — | Panel UIPlacementGhost | T | `outline`, `outline-offset` |

### `apps/desktop/src/renderer/src/panels/agent/VisualReply.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 122 | `.visual-reply` | — | Panel VisualReply | T | `flex`, `min-height`, `min-width`, `width` |
| 123 | `.visual-attachment-header` | — | Panel VisualReply | T | `margin-bottom` |
| 124 | `.visual-attachment-header .agent-attachment-type` | — | Panel VisualReply | T | `white-space`, `width` |
| 125 | `.visual-attachment-open` | — | Panel VisualReply | T | `border-radius`, `height`, `padding-right`, `width` |
| 126 | `.visual-attachment-open:focus-visible` | — | Panel VisualReply | T | `outline`, `outline-offset` |
| 127 | `.visual-title` | — | Panel VisualReply | T | `flex`, `max-width`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 128 | `.visual-expand-icon` | — | Panel VisualReply | T | `color`, `display`, `flex`, `height`, `margin-left`, `place-items`, `transition`, `width` |
| 129 | `.visual-expand-icon :global(svg)` | — | Panel VisualReply | T | `height`, `width` |
| 130 | `.visual-attachment-open:hover .visual-expand-icon` | — | Panel VisualReply | T | `color` |
| 131 | `dialog` | — | Panel VisualReply | T | `background`, `border`, `color`, `display`, `inset`, `margin`, `max-height`, `max-width`, `padding`, `position`, `width` |
| 132 | `dialog[open]` | — | Panel VisualReply | T | `background`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `height`, `inset`, `margin`, `max-height`, `overflow`, `position`, `transform-origin`, `width` |
| 133 | `dialog::backdrop` | — | Panel VisualReply | T | `backdrop-filter`, `background` |
| 134 | `:global(:root[data-theme="dark"]) dialog::backdrop` | — | Panel VisualReply | T | `background` |
| 135 | `.visual-dialog-toolbar` | — | Panel VisualReply | T | `align-items`, `color`, `display`, `flex`, `font`, `gap`, `padding` |
| 136 | `.visual-controls` | — | Panel VisualReply | T | `align-items`, `display`, `flex`, `gap` |
| 137 | `.visual-controls :global(.pm-icon)` | — | Panel VisualReply | T | `height`, `width` |
| 138 | `.visual-zoom` | — | Panel VisualReply | T | `font-variant-numeric`, `min-width`, `padding` |
| 139 | `.visual-controls .iconbtn:focus-visible` | — | Panel VisualReply | T | `outline`, `outline-offset` |
| 140 | `.visual-loading` | — | Panel VisualReply | T | `color`, `font`, `margin` |
| 141 | `.visual-notice` | — | Panel VisualReply | T | `margin-bottom`, `min-width` |
| 142 | `dialog[open] .visual-notice` | — | Panel VisualReply | T | `flex`, `margin`, `max-height`, `overflow` |
| 143 | `dialog[open] .visual-loading` | — | Panel VisualReply | T | `margin` |
| 144 | `.visual-viewport` | — | Panel VisualReply | T | `background`, `border-radius`, `contain`, `overflow`, `overscroll-behavior`, `position`, `width` |
| 145 | `.visual-viewport.expanded` | — | Panel VisualReply | T | `flex`, `margin`, `min-height`, `width` |
| 146 | `iframe` | — | Panel VisualReply | C | `border`, `display`, `transform-origin` |
| 147 | `.visual-expand-icon` | @media(prefers-reduced-motion:reduce) | Panel VisualReply | T | `transition` |

### `apps/desktop/src/renderer/src/panels/agent/WorkingTimer.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 18 | `.agent-working-timer` | — | Panel WorkingTimer | T | `border-bottom`, `color`, `font-size`, `font-variant-numeric`, `line-height`, `margin`, `padding` |

### `apps/desktop/src/renderer/src/panels/easing/CurveEditor.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 110 | `.easing-graph` | — | Panel CurveEditor | T | `aspect-ratio`, `background`, `border-radius`, `display`, `height`, `outline`, `touch-action`, `width` |
| 111 | `.easing-graph:focus-visible` | — | Panel CurveEditor | T | `outline`, `outline-offset` |
| 112 | `.grid` | — | Panel CurveEditor | T | `fill`, `stroke`, `stroke-width` |
| 113 | `.curve` | — | Panel CurveEditor | T | `fill`, `pointer-events`, `stroke`, `stroke-width` |
| 114 | `.handle-line` | — | Panel CurveEditor | T | `fill`, `pointer-events`, `stroke`, `stroke-dasharray`, `stroke-width` |
| 115 | `.handle` | — | Panel CurveEditor | T | `fill`, `pointer-events` |
| 116 | `.point` | — | Panel CurveEditor | T | `fill`, `pointer-events` |
| 117 | `.hit` | — | Panel CurveEditor | T | `cursor`, `fill` |
| 118 | `.hit:active` | — | Panel CurveEditor | T | `cursor` |

### `apps/desktop/src/renderer/src/panels/generated/CurveControl.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 219 | `.generated-curve-canvas:focus-visible` | — | Panel CurveControl | T | `outline-width` |

### `apps/desktop/src/renderer/src/panels/shader/CompileStatus.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 52 | `.compile-status` | — | Panel CompileStatus | T | `min-width`, `overflow`, `text-overflow`, `white-space` |

### `apps/desktop/src/renderer/src/panels/shader/ShaderEditor.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 173 | `.shader-editor` | — | Panel ShaderEditor | T | `display`, `flex`, `min-height`, `position` |
| 180 | `textarea.code` | — | Panel ShaderEditor | T | `flex`, `min-height` |

### `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 153 | `.agent-launcher-wrap` | — | Component AgentLauncher | T | `-webkit-app-region`, `flex`, `margin-right`, `min-width`, `position` |
| 154 | `.agent-launcher` | — | Component AgentLauncher | T | `align-items`, `background`, `border`, `border-radius`, `box-shadow`, `box-sizing`, `color`, `cursor`, `display`, `font`, `font-size`, `gap`, `height`, `isolation`, `max-width`, `padding`, `position`, `text-align`, `touch-action`, `transition` |
| 164 | `.agent-launcher.compact` | — | Component AgentLauncher | T | `justify-content`, `padding`, `width` |
| 165 | `.agent-launcher:hover` | — | Component AgentLauncher | T | `background` |
| 165 | `.agent-launcher.on` | — | Component AgentLauncher | T | `background` |
| 166 | `.agent-launcher:active` | — | Component AgentLauncher | T | `transform` |
| 167 | `.agent-launcher:focus-visible` | — | Component AgentLauncher | T | `outline`, `outline-offset` |
| 168 | `:global(.agent-launcher.drag-src)` | — | Component AgentLauncher | T | `opacity` |
| 170 | `.agent-launcher-mark` | — | Component AgentLauncher | T | `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 171 | `.agent-launcher.compact:not(.on) .agent-launcher-mark` | — | Component AgentLauncher | T | `color` |
| 172 | `.agent-launcher-mark :global(.pm-icon)` | — | Component AgentLauncher | T | `fill`, `height`, `width` |
| 173 | `.agent-launcher-mark svg` | — | Component AgentLauncher | T | `fill`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 174 | `.agent-launcher-spinner circle` | — | Component AgentLauncher | T | `opacity` |
| 175 | `.agent-launcher-spinner` | — | Component AgentLauncher | T | `animation` |
| 176 | `[data-tone="done"] .agent-launcher-mark` | — | Component AgentLauncher | T | `color` |
| 177 | `.agent-launcher-check` | — | Component AgentLauncher | T | `animation`, `stroke-dasharray`, `stroke-dashoffset` |
| 178 | `.agent-launcher-dot` | — | Component AgentLauncher | T | `background`, `border-radius`, `height`, `width` |
| 179 | `[data-tone="attention"] .agent-launcher-mark` | — | Component AgentLauncher | T | `color` |
| 180 | `[data-tone="attention"] .agent-launcher-dot` | — | Component AgentLauncher | T | `animation` |
| 181 | `[data-tone="error"] .agent-launcher-mark` | — | Component AgentLauncher | T | `color` |
| 183 | `.agent-launcher-label` | — | Component AgentLauncher | T | `animation`, `flex`, `max-width`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 187 | `.agent-launcher-meta` | — | Component AgentLauncher | T | `color`, `flex`, `font-size`, `font-variant-numeric` |
| 188 | `.agent-launcher-count` | — | Component AgentLauncher | T | `background`, `border-radius`, `box-sizing`, `color`, `display`, `flex`, `font-size`, `font-variant-numeric`, `height`, `min-width`, `padding`, `place-items` |
| 195 | `.agent-launcher.receiving` | — | Component AgentLauncher | T | `animation` |
| 197 | `.agent-launcher-announce` | — | Component AgentLauncher | T | `clip-path`, `height`, `overflow`, `position`, `white-space`, `width` |
| 199 | `.agent-launcher-tasks` | — | Component AgentLauncher | T | `animation`, `background`, `border-radius`, `box-shadow`, `box-sizing`, `left`, `padding`, `position`, `top`, `width`, `z-index` |
| 206 | `.agent-launcher-tasks-head` | — | Component AgentLauncher | T | `color`, `font-size`, `margin`, `padding` |
| 207 | `.agent-launcher-task` | — | Component AgentLauncher | T | `align-items`, `background`, `border`, `border-radius`, `color`, `display`, `font`, `gap`, `padding`, `text-align`, `width` |
| 211 | `.agent-launcher-task:hover` | — | Component AgentLauncher | T | `background` |
| 212 | `.agent-launcher-task:focus-visible` | — | Component AgentLauncher | T | `outline`, `outline-offset` |
| 213 | `.agent-launcher-task-dot` | — | Component AgentLauncher | T | `animation`, `background`, `border-radius`, `flex`, `height`, `width` |
| 214 | `.agent-launcher-task-text` | — | Component AgentLauncher | T | `display`, `flex`, `min-width` |
| 215 | `.agent-launcher-task-title` | — | Component AgentLauncher | T | `overflow`, `text-overflow`, `white-space` |
| 215 | `.agent-launcher-task-step` | — | Component AgentLauncher | T | `overflow`, `text-overflow`, `white-space` |
| 216 | `.agent-launcher-task-title` | — | Component AgentLauncher | T | `font-size` |
| 217 | `.agent-launcher-task-step` | — | Component AgentLauncher | T | `color`, `font-size` |
| 230 | `.agent-launcher-spinner` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 230 | `.agent-launcher-label` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 230 | `.agent-launcher.receiving` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 230 | `.agent-launcher-tasks` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 230 | `.agent-launcher-task-dot` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 230 | `[data-tone="attention"] .agent-launcher-dot` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation` |
| 232 | `.agent-launcher-check` | @media (prefers-reduced-motion: reduce) | Component AgentLauncher | T | `animation`, `stroke-dashoffset` |

### `apps/desktop/src/renderer/src/shell/StatusBar.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 162 | `button.status-contribution` | — | Component StatusBar | T | `background`, `border`, `color`, `font`, `padding` |
| 169 | `button.status-contribution:hover` | — | Component StatusBar | T | `color` |

### `apps/desktop/src/renderer/src/shell/TaskPill.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 121 | `.task-wrap` | — | Component TaskPill | T | `-webkit-app-region`, `flex`, `margin-right`, `position` |
| 122 | `.task-pill` | — | Component TaskPill | T | `align-items`, `background`, `border`, `border-radius`, `box-sizing`, `color`, `cursor`, `display`, `font`, `font-size`, `gap`, `height`, `padding`, `transition` |
| 128 | `.task-pill:hover` | — | Component TaskPill | T | `background` |
| 128 | `.task-pill.on` | — | Component TaskPill | T | `background` |
| 129 | `.task-pill.on` | — | Component TaskPill | T | `border-color` |
| 130 | `.task-pill.done` | — | Component TaskPill | T | `border-color` |
| 131 | `.task-pill:focus-visible` | — | Component TaskPill | T | `outline`, `outline-offset` |
| 132 | `.task-mark` | — | Component TaskPill | T | `fill`, `flex`, `height`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 133 | `.task-ring-track` | — | Component TaskPill | T | `opacity`, `stroke` |
| 134 | `.task-ring` | — | Component TaskPill | T | `stroke`, `transition` |
| 135 | `.task-mark.spin` | — | Component TaskPill | T | `animation` |
| 136 | `.task-check` | — | Component TaskPill | T | `stroke`, `stroke-width` |
| 137 | `.task-pct` | — | Component TaskPill | T | `font-family`, `font-size`, `font-variant-numeric` |
| 138 | `.task-pill.done .task-pct` | — | Component TaskPill | T | `font-family` |
| 140 | `.task-pop` | — | Component TaskPill | T | `animation`, `background`, `border-radius`, `box-shadow`, `box-sizing`, `display`, `flex-direction`, `gap`, `padding`, `position`, `right`, `top`, `width`, `z-index` |
| 147 | `.task-rule` | — | Component TaskPill | T | `background`, `height` |
| 148 | `.task-row` | — | Component TaskPill | T | `display`, `flex-direction`, `gap` |
| 149 | `.task-head` | — | Component TaskPill | T | `align-items`, `display`, `gap`, `justify-content` |
| 150 | `.task-title` | — | Component TaskPill | T | `color`, `font-size`, `font-weight`, `min-width`, `overflow`, `text-overflow`, `white-space` |
| 151 | `.task-num` | — | Component TaskPill | T | `color`, `flex`, `font-family`, `font-size`, `font-variant-numeric` |
| 152 | `.task-done` | — | Component TaskPill | T | `fill`, `flex`, `height`, `stroke`, `stroke-linecap`, `stroke-linejoin`, `stroke-width`, `width` |
| 154 | `.task-bar` | — | Component TaskPill | T | `background`, `border-radius`, `display`, `gap`, `height`, `overflow`, `position` |
| 155 | `.task-bar.segments` | — | Component TaskPill | T | `background`, `overflow` |
| 156 | `.seg` | — | Component TaskPill | T | `background`, `border-radius`, `flex`, `height`, `overflow` |
| 157 | `.fill` | — | Component TaskPill | T | `background`, `border-radius`, `display`, `height`, `transition` |
| 161 | `.fill.indeterminate` | — | Component TaskPill | T | `animation`, `width` |
| 163 | `.task-foot` | — | Component TaskPill | T | `align-items`, `display`, `gap`, `justify-content` |
| 164 | `.task-detail` | — | Component TaskPill | T | `color`, `font-family`, `font-size`, `white-space` |
| 164 | `.task-meta` | — | Component TaskPill | T | `color`, `font-family`, `font-size`, `white-space` |
| 165 | `.task-detail` | — | Component TaskPill | T | `min-width`, `overflow`, `text-overflow` |
| 166 | `.task-detail.fade` | — | Component TaskPill | T | `-webkit-background-clip`, `background`, `background-clip`, `color` |
| 170 | `.task-action` | — | Component TaskPill | T | `background`, `border`, `border-radius`, `color`, `flex`, `font`, `font-size`, `padding` |
| 171 | `.task-action:hover` | — | Component TaskPill | T | `background` |
| 172 | `.task-action:focus-visible` | — | Component TaskPill | T | `outline`, `outline-offset` |
| 173 | `.task-meta` | — | Component TaskPill | T | `flex`, `font-variant-numeric` |
| 179 | `.task-mark.spin` | @media (prefers-reduced-motion: reduce) | Component TaskPill | T | `animation` |
| 179 | `.task-pop` | @media (prefers-reduced-motion: reduce) | Component TaskPill | T | `animation` |
| 180 | `.fill.indeterminate` | @media (prefers-reduced-motion: reduce) | Component TaskPill | T | `animation`, `translate` |
| 181 | `.fill` | @media (prefers-reduced-motion: reduce) | Component TaskPill | T | `transition` |
| 181 | `.task-ring` | @media (prefers-reduced-motion: reduce) | Component TaskPill | T | `transition` |

### `apps/desktop/src/renderer/src/shell/WhatsNew.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 34 | `.whats-new` | — | Component WhatsNew | T | `color`, `display`, `flex-direction`, `font-size`, `gap`, `line-height` |
| 35 | `.whats-new-lede` | — | Component WhatsNew | T | `color`, `margin` |
| 36 | `.whats-new-release` | — | Component WhatsNew | T | `display`, `flex-direction`, `gap` |
| 37 | `.whats-new-release + .whats-new-release` | — | Component WhatsNew | T | `border-top`, `padding-top` |
| 38 | `header` | — | Component WhatsNew | T | `align-items`, `display`, `gap`, `justify-content` |
| 39 | `.whats-new-version` | — | Component WhatsNew | T | `color`, `font-size`, `font-weight` |
| 40 | `.whats-new-date` | — | Component WhatsNew | T | `color`, `font-size`, `white-space` |
| 41 | `.whats-new-empty` | — | Component WhatsNew | T | `margin` |

### `apps/desktop/src/renderer/src/store/StoreIcon.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 13 | `img` | — | Component StoreIcon | T | `display`, `height`, `object-fit`, `width` |

### `apps/desktop/src/renderer/src/transcription/ModelMeters.svelte`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 20 | `.meters` | — | Component ModelMeters | T | `align-items`, `display`, `flex`, `gap`, `grid-template-columns` |
| 21 | `.meter` | — | Component ModelMeters | T | `display` |
| 22 | `.label` | — | Component ModelMeters | T | `color`, `font-size`, `line-height`, `text-align` |
| 23 | `.track` | — | Component ModelMeters | T | `background`, `border-radius`, `height`, `overflow`, `position` |
| 24 | `.fill` | — | Component ModelMeters | T | `background`, `border-radius`, `inset`, `position` |

### `apps/desktop/src/renderer/index.html`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 13 | `#boot-ghost` | — | Shell cold-start placeholders | T | `--boot-ink-1`, `--boot-ink-2`, `--boot-panel`, `--boot-shadow`, `animation`, `display`, `gap`, `grid-template`, `inset`, `opacity`, `padding`, `pointer-events`, `position`, `z-index` |
| 23 | `body:has(#projects-screen.on) #boot-ghost` | — | Shell cold-start placeholders | T | `visibility` |
| 23 | `body:has(#settings-screen.on) #boot-ghost` | — | Shell cold-start placeholders | T | `visibility` |
| 26 | `#app:not(:has(> #titlebar[data-svelte-shell])) ~ #boot-ghost` | @media (prefers-color-scheme:dark) | Shell cold-start placeholders | T | `--boot-ink-1`, `--boot-ink-2`, `--boot-panel`, `--boot-shadow` |
| 33 | `#boot-ghost>div` | — | Shell cold-start placeholders | T | `background`, `border-radius`, `box-shadow`, `display`, `flex-direction`, `min-height`, `min-width`, `overflow`, `position` |
| 34 | `#boot-ghost>div::before` | — | Shell cold-start placeholders | T | `background`, `border-radius`, `content`, `flex`, `height`, `margin`, `width` |
| 35 | `#boot-ghost i` | — | Shell cold-start placeholders | T | `animation`, `background`, `border-radius`, `display`, `flex`, `height`, `margin`, `width` |
| 36 | `#boot-ghost i:nth-of-type(4n+2)` | — | Shell cold-start placeholders | T | `width` |
| 37 | `#boot-ghost i:nth-of-type(4n+3)` | — | Shell cold-start placeholders | T | `width` |
| 38 | `#boot-ghost i:nth-of-type(4n)` | — | Shell cold-start placeholders | T | `width` |
| 39 | `#boot-ghost .boot-ghost-stage` | — | Shell cold-start placeholders | T | `border-radius`, `flex`, `height`, `margin`, `width` |
| 43 | `#boot-ghost` | @media (prefers-reduced-motion:reduce) | Shell cold-start placeholders | T | `animation` |
| 44 | `#boot-ghost i` | @media (prefers-reduced-motion:reduce) | Shell cold-start placeholders | T | `animation` |

### `apps/desktop/src/extensions/timeline/index.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 27 | `#panel-timeline` | — | Timeline header, controls, nested-composition tabs | T | `flex` |
| 28 | `#panel-timeline>.body` | — | Timeline header, controls, nested-composition tabs | T | `display`, `flex-direction`, `overflow`, `position` |
| 29 | `#tl-head` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `background`, `border-bottom`, `border-right`, `display`, `gap`, `height`, `justify-content`, `left`, `overflow`, `padding`, `position`, `top`, `width`, `z-index` |
| 30 | `#tl-head .iconbtn` | — | Timeline header, controls, nested-composition tabs | T | `background`, `border`, `border-radius`, `box-shadow`, `flex`, `height`, `width` |
| 31 | `#tl-head .iconbtn:hover` | — | Timeline header, controls, nested-composition tabs | T | `background` |
| 32 | `#tl-head .iconbtn svg` | — | Timeline header, controls, nested-composition tabs | T | `height`, `width` |
| 33 | `#tl-head[data-density="compact"]` | — | Timeline header, controls, nested-composition tabs | T | `gap`, `padding-inline` |
| 34 | `#tl-head[data-density="compact"] .iconbtn` | — | Timeline header, controls, nested-composition tabs | T | `height`, `width` |
| 35 | `#tl-head[data-density="compact"] .iconbtn svg` | — | Timeline header, controls, nested-composition tabs | T | `height`, `width` |
| 36 | `#tl-head[data-density="compact"] #tl-time` | — | Timeline header, controls, nested-composition tabs | T | `font-size`, `min-width` |
| 37 | `#tl-head .tl-group` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `display`, `gap`, `min-width` |
| 38 | `#tl-head .tl-graph-slot` | — | Timeline header, controls, nested-composition tabs | T | `margin-left` |
| 39 | `#tl-head .tl-graph-slot .iconbtn.on` | — | Timeline header, controls, nested-composition tabs | T | `background`, `color` |
| 40 | `#tl-head .tl-graph-slot .iconbtn.on:hover` | — | Timeline header, controls, nested-composition tabs | T | `background` |
| 41 | `#tl-head .tl-graph-slot` | — | Timeline header, controls, nested-composition tabs | T | `gap` |
| 42 | `#tl-head .tl-graph-slot>.iconbtn:not(.tl-view-button)` | — | Timeline header, controls, nested-composition tabs | T | `margin-right` |
| 43 | `#tl-head .tl-view-button` | — | Timeline header, controls, nested-composition tabs | T | `border-radius`, `color`, `width` |
| 43 | `#tl-head[data-density] .tl-view-button` | — | Timeline header, controls, nested-composition tabs | T | `border-radius`, `color`, `width` |
| 44 | `#tl-head .tl-view-button svg` | — | Timeline header, controls, nested-composition tabs | T | `height`, `width` |
| 44 | `#tl-head[data-density] .tl-view-button svg` | — | Timeline header, controls, nested-composition tabs | T | `height`, `width` |
| 45 | `#tl-head .tl-view-button.first` | — | Timeline header, controls, nested-composition tabs | T | `border-radius` |
| 46 | `#tl-head .tl-view-button.last` | — | Timeline header, controls, nested-composition tabs | T | `border-radius` |
| 47 | `#tl-head .tl-view-button:not(.on):hover` | — | Timeline header, controls, nested-composition tabs | T | `color` |
| 48 | `#tl-head .tl-graph-slot .tl-view-button.on` | — | Timeline header, controls, nested-composition tabs | T | `background`, `color` |
| 49 | `#tl-time` | — | Timeline header, controls, nested-composition tabs | T | `color`, `cursor`, `font-family`, `font-size`, `font-variant-numeric`, `letter-spacing`, `margin-left`, `padding`, `white-space` |
| 50 | `#tl-time.edit` | — | Timeline header, controls, nested-composition tabs | T | `color` |
| 51 | `#tl-canvas-wrap` | — | Timeline header, controls, nested-composition tabs | T | `flex`, `min-height`, `overflow`, `position` |
| 52 | `#tl-canvas` | — | Timeline header, controls, nested-composition tabs | T | `display`, `height`, `inset`, `position`, `width` |
| 53 | `#tl-canvas-wrap .tl-caption-editor` | — | Timeline header, controls, nested-composition tabs | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `cursor`, `font-family`, `font-size`, `line-height`, `margin`, `outline`, `overflow`, `padding`, `position`, `resize`, `z-index` |
| 55 | `#tl-comp-tabs` | — | Timeline header, controls, nested-composition tabs | T | `padding-right` |
| 56 | `#tl-mode` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `background`, `border-radius`, `display`, `gap`, `height`, `padding`, `position`, `right`, `top`, `z-index` |
| 57 | `#tl-mode .tl-mode-button` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `background`, `border`, `border-radius`, `color`, `cursor`, `display`, `font`, `font-size`, `gap`, `line-height`, `padding` |
| 58 | `#tl-mode .tl-mode-button svg` | — | Timeline header, controls, nested-composition tabs | T | `flex`, `height`, `width` |
| 59 | `#tl-mode .tl-mode-button:hover` | — | Timeline header, controls, nested-composition tabs | T | `color` |
| 60 | `#tl-mode .tl-mode-button.on` | — | Timeline header, controls, nested-composition tabs | T | `background`, `box-shadow`, `color` |

### `apps/desktop/src/extensions/timeline/comp-tabs.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 15 | `#tl-comp-tabs` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `background`, `border-bottom`, `display`, `flex`, `gap`, `height`, `overflow-x`, `overflow-y`, `padding`, `scrollbar-width` |
| 16 | `#tl-comp-tabs::-webkit-scrollbar` | — | Timeline header, controls, nested-composition tabs | T | `display` |
| 17 | `#panel-timeline>.body>#tl-head` | — | Timeline header, controls, nested-composition tabs | T | `top` |
| 18 | `.tl-comp-tab` | — | Timeline header, controls, nested-composition tabs | T | `align-items`, `background`, `border`, `color`, `cursor`, `display`, `flex`, `font-size`, `gap`, `max-width`, `padding`, `position` |
| 19 | `.tl-comp-tab:hover` | — | Timeline header, controls, nested-composition tabs | T | `color` |
| 20 | `.tl-comp-tab[aria-selected="true"]` | — | Timeline header, controls, nested-composition tabs | T | `color` |
| 21 | `.tl-comp-tab[aria-selected="true"]::after` | — | Timeline header, controls, nested-composition tabs | T | `background`, `border-radius`, `bottom`, `content`, `height`, `left`, `position`, `right` |
| 22 | `.tl-comp-tab>span` | — | Timeline header, controls, nested-composition tabs | T | `overflow`, `text-overflow`, `white-space` |
| 23 | `.tl-comp-tab>button` | — | Timeline header, controls, nested-composition tabs | T | `background`, `border`, `border-radius`, `color`, `display`, `flex`, `font-size`, `height`, `line-height`, `opacity`, `place-items`, `width` |
| 24 | `.tl-comp-tab:hover>button` | — | Timeline header, controls, nested-composition tabs | T | `opacity` |
| 24 | `.tl-comp-tab[aria-selected="true"]>button` | — | Timeline header, controls, nested-composition tabs | T | `opacity` |
| 24 | `.tl-comp-tab>button:focus-visible` | — | Timeline header, controls, nested-composition tabs | T | `opacity` |
| 25 | `.tl-comp-tab>button:hover` | — | Timeline header, controls, nested-composition tabs | T | `background`, `opacity` |

### `apps/desktop/src/extensions/viewer/index.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 7 | `.canvas-text-input::selection` | — | Viewer stage / source monitor / controls | T | `background`, `color` |
| 8 | `#panel-viewer` | — | Viewer stage / source monitor / controls | T | `background` |
| 9 | `#panel-viewer > .panel-move-handle` | — | Viewer stage / source monitor / controls | T | `background`, `border`, `border-radius`, `box-shadow`, `color`, `height`, `left`, `top`, `transform`, `width` |
| 13 | `#panel-viewer > .panel-move-handle:hover` | — | Viewer stage / source monitor / controls | T | `background`, `box-shadow`, `color` |
| 13 | `#panel-viewer > .panel-move-handle:focus-visible` | — | Viewer stage / source monitor / controls | T | `background`, `box-shadow`, `color` |
| 13 | `#panel-viewer > .panel-move-handle:active` | — | Viewer stage / source monitor / controls | T | `background`, `box-shadow`, `color` |
| 18 | `#panel-viewer .body` | — | Viewer stage / source monitor / controls | T | `display`, `flex-direction`, `overflow`, `padding` |
| 19 | `#stage` | — | Viewer stage / source monitor / controls | T | `flex`, `min-height`, `overflow`, `position` |
| 20 | `#stage-inner` | — | Viewer stage / source monitor / controls | T | `border-radius`, `box-shadow`, `corner-shape`, `outline`, `overflow`, `position` |
| 21 | `[data-preview-corners="rounded"] #stage-inner` | — | Viewer stage / source monitor / controls | T | `border-radius`, `corner-shape` |
| 22 | `#gl` | — | Viewer stage / source monitor / controls | T | `background`, `display`, `height`, `width` |
| 23 | `#overlay` | — | Viewer stage / source monitor / controls | T | `inset`, `pointer-events`, `position` |
| 24 | `#composition-recovery` | — | Viewer stage / source monitor / controls | T | `align-items`, `background`, `border`, `border-radius`, `box-shadow`, `color`, `display`, `font-size`, `gap`, `left`, `padding`, `position`, `top`, `transform`, `white-space`, `z-index` |
| 25 | `#composition-recovery[hidden]` | — | Viewer stage / source monitor / controls | T | `display` |
| 26 | `#composition-recovery b` | — | Viewer stage / source monitor / controls | T | `background`, `border-radius`, `color`, `font-weight`, `padding` |
| 27 | `#composition-recovery:hover` | — | Viewer stage / source monitor / controls | T | `background`, `color` |
| 28 | `#composition-recovery:focus-visible` | — | Viewer stage / source monitor / controls | T | `outline`, `outline-offset` |
| 29 | `#stage.fx-drop-over > #stage-inner` | — | Viewer stage / source monitor / controls | T | `box-shadow`, `outline`, `outline-offset`, `transition` |

### `apps/desktop/src/extensions/viewer/source-preview.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 22 | `#source-preview` | — | Viewer stage / source monitor / controls | T | `background`, `border-radius`, `display`, `inset`, `overflow`, `position`, `z-index` |
| 23 | `#source-preview[data-open="true"]` | — | Viewer stage / source monitor / controls | T | `animation`, `display` |
| 25 | `#source-preview .sp-media` | — | Viewer stage / source monitor / controls | T | `background`, `display`, `inset`, `place-items`, `position` |
| 26 | `#source-preview[data-kind="audio"] .sp-media` | — | Viewer stage / source monitor / controls | T | `background` |
| 27 | `#source-preview .sp-media img` | — | Viewer stage / source monitor / controls | T | `display`, `height`, `min-height`, `object-fit`, `width` |
| 27 | `#source-preview .sp-media video` | — | Viewer stage / source monitor / controls | T | `display`, `height`, `min-height`, `object-fit`, `width` |
| 28 | `#source-preview .sp-audio` | — | Viewer stage / source monitor / controls | T | `align-items`, `color`, `display`, `flex-direction`, `gap`, `max-width`, `min-width` |
| 29 | `#source-preview .sp-audio svg` | — | Viewer stage / source monitor / controls | T | `height`, `width` |
| 30 | `#source-preview .sp-audio .sp-name` | — | Viewer stage / source monitor / controls | T | `flex`, `max-width`, `padding`, `text-align` |
| 31 | `#source-preview .sp-foot` | — | Viewer stage / source monitor / controls | T | `align-items`, `background`, `border-radius`, `bottom`, `box-shadow`, `box-sizing`, `display`, `gap`, `left`, `padding`, `position`, `transform`, `width`, `z-index` |
| 32 | `#source-preview .sp-name` | — | Viewer stage / source monitor / controls | T | `color`, `flex`, `font-size`, `font-weight`, `min-width`, `overflow`, `padding-left`, `text-overflow`, `white-space` |
| 33 | `#source-preview .sp-close` | — | Viewer stage / source monitor / controls | T | `background`, `border`, `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 33 | `#source-preview .sp-play` | — | Viewer stage / source monitor / controls | T | `background`, `border`, `border-radius`, `color`, `display`, `flex`, `height`, `place-items`, `width` |
| 34 | `#source-preview .sp-close:hover` | — | Viewer stage / source monitor / controls | T | `background`, `color` |
| 34 | `#source-preview .sp-play:hover` | — | Viewer stage / source monitor / controls | T | `background`, `color` |
| 35 | `#source-preview .sp-close:active` | — | Viewer stage / source monitor / controls | T | `background` |
| 35 | `#source-preview .sp-play:active` | — | Viewer stage / source monitor / controls | T | `background` |
| 36 | `#source-preview :focus-visible` | — | Viewer stage / source monitor / controls | T | `outline`, `outline-offset` |
| 37 | `#source-preview .sp-close svg` | — | Viewer stage / source monitor / controls | T | `height`, `width` |
| 37 | `#source-preview .sp-play svg` | — | Viewer stage / source monitor / controls | T | `height`, `width` |
| 38 | `#source-preview .sp-scrub` | — | Viewer stage / source monitor / controls | T | `align-items`, `border-radius`, `display`, `flex`, `height`, `min-width`, `touch-action` |
| 39 | `#source-preview .sp-scrub i` | — | Viewer stage / source monitor / controls | T | `background`, `border-radius`, `height`, `width` |
| 40 | `#source-preview .sp-scrub i b` | — | Viewer stage / source monitor / controls | T | `background`, `border-radius`, `display`, `height`, `width` |
| 41 | `#source-preview [hidden]` | — | Viewer stage / source monitor / controls | T | `display` |
| 42 | `#source-preview[data-open="true"]` | @media(prefers-reduced-motion:reduce) | Viewer stage / source monitor / controls | T | `animation` |

### `apps/desktop/src/renderer/sandbox/boot.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 149 | `html` | — | Legacy Store view document | C | `background`, `margin`, `padding` |
| 149 | `body` | — | Legacy Store view document | C | `background`, `margin`, `padding` |
| 150 | `html` | — | Legacy Store view document | C | `overflow`, `scrollbar-width` |
| 150 | `html::-webkit-scrollbar` | — | Legacy Store view document | C | `display` |
| 151 | `html[data-noscroll]` | — | Legacy Store view document | C | `overflow` |
| 152 | `body` | — | Legacy Store view document | C | `-webkit-font-smoothing`, `-webkit-user-select`, `color`, `cursor`, `font-family`, `font-size`, `min-height`, `text-rendering`, `user-select` |
| 153 | `input` | — | Legacy Store view document | C | `-webkit-user-select`, `user-select` |
| 153 | `textarea` | — | Legacy Store view document | C | `-webkit-user-select`, `user-select` |
| 153 | `[contenteditable]:not([contenteditable="false"])` | — | Legacy Store view document | C | `-webkit-user-select`, `user-select` |
| 154 | `.pm-view-error` | — | Legacy Store view document | C | `color`, `font-size`, `line-height`, `padding`, `text-wrap` |

### `apps/desktop/src/renderer/src/panels/agent/visual-reply.ts`

| Line | Exact selector | Conditions | Surface | Class | Declared properties |
|---:|---|---|---|---|---|
| 62 | `:root` | — | Authored visual-reply HTML document | C | `--accent`, `--background`, `--foreground`, `--muted`, `color-scheme` |
| 62 | `html` | — | Authored visual-reply HTML document | C | `background`, `color`, `font` |
| 62 | `body` | — | Authored visual-reply HTML document | C | `box-sizing`, `margin`, `padding` |
| 62 | `*` | — | Authored visual-reply HTML document | C | `box-sizing` |
| 62 | `button` | — | Authored visual-reply HTML document | C | `font` |
| 62 | `input` | — | Authored visual-reply HTML document | C | `font` |
| 62 | `select` | — | Authored visual-reply HTML document | C | `font` |
| 62 | `a` | — | Authored visual-reply HTML document | C | `color` |

## Motion blocks

Keyframe names are source identifiers, not a requirement to preserve browser animation mechanics. All map to T native motion for their named surface; use C where they animate arbitrary legacy DOM content. Reduce Motion remains authoritative.

| Source | Line | Block |
|---|---:|---|
| `apps/desktop/css/account.css` | 75 | `@keyframes acct-spin` |
| `apps/desktop/css/account.css` | 85 | `@keyframes acct-caret` |
| `apps/desktop/css/app.css` | 694 | `@keyframes project-ghost-in` |
| `apps/desktop/css/app.css` | 695 | `@keyframes project-ghost-breathe` |
| `apps/desktop/css/app.css` | 696 | `@keyframes project-ghost-dim` |
| `apps/desktop/css/app.css` | 724 | `@keyframes modal-in` |
| `apps/desktop/css/app.css` | 725 | `@keyframes scrim-in` |
| `apps/desktop/css/app.css` | 733 | `@keyframes modal-scroll-fade` |
| `apps/desktop/css/app.css` | 899 | `@keyframes agent-message-send` |
| `apps/desktop/css/app.css` | 900 | `@keyframes agent-pending-enter` |
| `apps/desktop/css/app.css` | 903 | `@keyframes spatial-thinking` |
| `apps/desktop/css/app.css` | 906 | `@keyframes agent-word-roll` |
| `apps/desktop/css/app.css` | 932 | `@keyframes spatial-selection-confirm` |
| `apps/desktop/css/app.css` | 990 | `@keyframes agent-glow-in` |
| `apps/desktop/css/app.css` | 991 | `@keyframes agent-glow-pulse` |
| `apps/desktop/css/app.css` | 1017 | `@keyframes agent-live-pulse` |
| `apps/desktop/css/app.css` | 1018 | `@keyframes agent-spin` |
| `apps/desktop/css/app.css` | 1026 | `@keyframes agent-fade-up` |
| `apps/desktop/css/app.css` | 1027 | `@keyframes agent-pop-in` |
| `apps/desktop/css/app.css` | 1032 | `@keyframes agent-caret-in` |
| `apps/desktop/css/app.css` | 1041 | `@keyframes agent-pixel-on` |
| `apps/desktop/css/app.css` | 1055 | `@keyframes shimmer-sweep` |
| `apps/desktop/css/app.css` | 1081 | `@keyframes agent-jump-in` |
| `apps/desktop/css/app.css` | 1085 | `@keyframes agent-card-in` |
| `apps/desktop/css/app.css` | 1170 | `@keyframes agent-chip-in` |
| `apps/desktop/css/app.css` | 1313 | `@keyframes pm-menu-in` |
| `apps/desktop/css/app.css` | 1314 | `@keyframes pm-menu-out` |
| `apps/desktop/css/app.css` | 1489 | `@keyframes library-view-fade` |
| `apps/desktop/css/app.css` | 1490 | `@keyframes library-view-out` |
| `apps/desktop/css/app.css` | 1491 | `@keyframes library-screen-in` |
| `apps/desktop/css/app.css` | 1492 | `@keyframes library-screen-out` |
| `apps/desktop/css/store.css` | 331 | `@keyframes st-breathe` |
| `apps/desktop/src/renderer/src/controls/search-picker.css` | 16 | `@keyframes thread-pulse` |
| `apps/desktop/src/renderer/src/onboarding/welcome.css` | 38 | `@keyframes welcome-enter` |
| `apps/desktop/src/renderer/public/onboarding/onboarding.css` | 28 | `@keyframes onboarding-dim-out` |
| `apps/desktop/src/renderer/public/onboarding/onboarding.css` | 33 | `@keyframes onboarding-dim-in` |
| `apps/desktop/src/renderer/src/errors/ErrorFix.svelte` | 83 | `@keyframes fix-spin` |
| `apps/desktop/src/renderer/src/onboarding/SuitcaseTowers.svelte` | 98 | `@keyframes rise` |
| `apps/desktop/src/renderer/src/overlays/Toasts.svelte` | 222 | `@keyframes save-activity` |
| `apps/desktop/src/renderer/src/panels/agent/ConnectionGate.svelte` | 82 | `@keyframes connection-spin` |
| `apps/desktop/src/renderer/src/panels/agent/GhostCard.svelte` | 171 | `@keyframes ghost-breathe` |
| `apps/desktop/src/renderer/src/panels/agent/GhostCard.svelte` | 175 | `@keyframes ghost-line` |
| `apps/desktop/src/renderer/src/panels/agent/GhostCard.svelte` | 180 | `@keyframes ghost-reserve` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 219 | `@keyframes agent-launcher-spin` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 220 | `@keyframes agent-launcher-draw` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 221 | `@keyframes agent-launcher-pulse` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 222 | `@keyframes agent-launcher-swap` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 223 | `@keyframes agent-launcher-receive` |
| `apps/desktop/src/renderer/src/shell/AgentLauncher.svelte` | 228 | `@keyframes agent-launcher-tasks-in` |
| `apps/desktop/src/renderer/src/shell/TaskPill.svelte` | 175 | `@keyframes task-spin` |
| `apps/desktop/src/renderer/src/shell/TaskPill.svelte` | 176 | `@keyframes task-in` |
| `apps/desktop/src/renderer/src/shell/TaskPill.svelte` | 177 | `@keyframes task-sweep` |
| `apps/desktop/src/renderer/index.html` | 40 | `@keyframes boot-ghost-in` |
| `apps/desktop/src/renderer/index.html` | 41 | `@keyframes boot-ghost-breathe` |
| `apps/desktop/src/extensions/viewer/source-preview.ts` | 24 | `@keyframes sp-in` |
