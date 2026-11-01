# Effects — v4 built-in panel (2026-11-01)

This directory owns the Effects browsing model, v4 surface, async data-service
adapter and Linux conformance tests. It does **not** register a replacement in the
application shell. All visual output is provisional until Opus review. The panel
ID is `fxbrowser`; the title is **Effects**, following the shared panel README's
owner decision, which supersedes the older title in effects-presets.md.

Authoritative design: `docs/native/design/panels/effects-presets.md` and its
shared README; components: `docs/native/ui-kit.md`. Behaviour was checked against
the tagged `FxBrowserPanel.svelte` and `fx-browser.svelte.ts`, W064 B413/B414 and
the _Effects browser_ keymap route. No existing consumer is removed.

## Entry and contracts

Call `createEffectsExtension` **in the v4 worker realm**, supplying the surface
transport, the public `HostClient`, registered local field bindings, a unique
subscription prefix, measured viewport row range and `EffectsHostAdapter`.
It returns the surface root (including native/headless ID dispatch), model,
semantic `inspect()` and async `dispose()`. Root/host cleanup both run on failure;
failed subscription cleanup stays retryable. This panel does not retire a shared
HostClient belonging to another panel.

If mounting and cleanup both fail, `EffectsMountError` retains both failures and
an explicit `retryCleanup()` function. The application must retry it or complete
native realm retirement; an unsuccessful detach is never reported as success.

The adapter is deliberately required: `registry.subscribe` publishes immutable
registered items/groups; `target(project, selection)` resolves the selected
effect-capable layer and its existing effect types; `add` returns the real
atomic kernel edit commands; `reveal` returns the actual Effect Controls command.
There is no invented production command, registry query or success backend.
Synthetic `fixture.edit` commands exist only in tests. The current public host
query vocabulary is project/selection/time/evaluated, without an effects registry.

Settings use the original `pm.fx.recent` key and a new app-UI key
`pm.fx.collapsed`, outside the project document/Undo. Native migration/settings
integration must map the legacy recent slot to this host setting. Both legacy
JSON-string values and arrays are accepted, with unknown IDs preserved. Reads
never rewrite or delete the source. Hydration cannot overwrite a newer local
collapse or an acknowledged apply. Writes serialize and failures remain visible
to `onError`; they do not undo an already accepted document edit.

All search/indexing runs in the worker, including >2,000 items. In-flight searches
keep accepted rows visible; epochs fence late query, registry and retired-realm
answers. The model caches a logical row index and materializes at most three
viewport lengths. `setViewport(start, size)` takes row counts, never invented
pixel metrics. Registry changes preserve the first visible ID when it survives.
`inspect().window` exposes full count, range and bounded rows. This is **not** a
native-scroll performance claim: the kit's virtual-outline integration is absent.

## Spec coverage

| Spec section      | Status                                                                                                                                                                                                                                                         |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 Model          | Done: scopes, query, registry/group order, persisted collapse, ≤8 recents, one selected ID, target eligibility port. Production registry/target wiring remains application work.                                                                               |
| §2 Anatomy        | Partial: title-only header, always-present field/scope, group/flat/search rows, token text highlighting, conditional Add, neutral selection. Native search recipe, symbols/disclosure/depth slots and menu separators are unavailable in the kit.              |
| §3 States         | All listed states have real wire/AccessKit snapshots, including explicit loading/drop/multi-select N/A states and bounded 60k rows. Native drag image, virtual scroller and visual acceptance are not done.                                                    |
| §4 Interactions   | Model done: select/apply/recents, query, disclosure, exact drag payload, context availability/add/reveal. Surface click/Add/double-click/search/scope are wired. Native drag, context-menu invocation and announcements need host APIs.                        |
| §5 Keyboard/focus | Model done: all listed keys, printable input transfer, scope-native ownership, modifiers/IME and repeat guards. Search → scope → list is the native focus order. Real focus/rings/IME need macOS. Kit row Enter/Space interception needs reconciliation below. |
| §6 VoiceOver      | Partial: actual AccessKit roles/names/selected/expanded/disabled/actions and focus order asserted. Kit lacks search-field role, AXRowCount/virtual row navigation, row custom Add action and live announcements.                                               |
| §7 Appearance     | Tokens only; solid panel, neutral selection, hover and Increase Contrast outlines/separators. No native rendering or theme/accessibility screenshot claim.                                                                                                     |
| §8 Copy           | Spec copy only; title follows owner decision. No invented failure UI. Preset spoken kind and unspecified failure presentation remain DESIGN-TODOs.                                                                                                             |
| §9 Don't          | No category rail/fx badge/accent selection/single-click apply. No web-export or unrelated layout changes.                                                                                                                                                      |

The N/A snapshots intentionally equal the synchronous/browsing state; they do not
introduce fake loading UI, a drop target or multiple selection. Drag snapshot
asserts the exact model payload; it does not claim an OS drag session ran.

## API gaps — owned by UI-kit, surface and application lanes

1. **Search and local fields:** kit `SearchField`/NSSearchField, exact placeholder,
   SearchInput AX role, progress in the trailing edge, and app-UI-only local
   bindings. Current `TextField` is a neutral placeholder with the correct label;
   document-channel registration is not a substitute. Native scope pop-up needs
   borderless inspector-choice presentation, separator support and content-fit
   sizing up to `metrics.value_column`.
2. **Outline/virtualization:** kit VirtualTree with token row height, depth and
   disclosure slots, `metrics.tree_indent`, one-viewport overscan, full logical
   scroll height, rangechange events and ID anchors. Native AXRowCount and
   offscreen row reveal must consume the logical index, not metadata alone.
   The `Tree` metadata here is inspectable extension data; the renderer does not
   interpret it. No fake itemSize literal or custom scrollbar is introduced.
3. **Row content:** kit/system SF Symbol slots (`wand.and.rays`, `sparkles`,
   `plus`, `exclamationmark.triangle.fill`) and native focus/hover/pressed/transition
   recipes. EmptyState lacks its specified symbol slot. Warning glyph and token
   colour are pending together, avoiding a colour-only error substitute. Add is
   a neutral icon-button placeholder until the plus-symbol slot exists. Native
   pointer exit must clear hover; the present API only supplies pointer movement.
4. **Input:** `TreeItem` currently invokes its selection callback on Enter/Space
   before forwarding keys. The panel-level Return handler applies an item, but
   group Return/Space can toggle and item Return can select as a side effect.
   Need a kit keyboard-selection policy/activation callback so the model is the
   sole arbiter. Native host must map double-click to the v4 `doubleclick` event,
   preserve clickCount and suppress repeated Add activations. No DOM `dblclick`
   alias is assumed.
5. **Drag/context/AX:** typed drag-source descriptor carrying MIME
   `application/x-powermove-fx`, copy operation and `{kind:'effect',id,label}`,
   native capsule image and source treatment; contextmenu callback/native NSMenu
   with separator; custom row action `Add to selected layer`; medium-priority
   sequenced native announcement sink. `model.drag/context/contextAction` and
   `announcement` are executable semantic contracts until these APIs exist.
6. **Production data:** authorized registry publications, real effect/preset edit
   encoding and reveal command, kernel-backed local settings/bindings, and shell
   v4 panel registration. Actor consumption must revalidate the named target,
   current definition/permissions, project generation/revision and any required
   selection guard. Separate project and selection subscriptions do not expose a
   joint revision to the callback; the application must make target eligibility
   coherent. Tests prove the real Rust router/data-service boundary using its
   synthetic backend, **not** real effect insertion, Undo or durable profile writes.

These gaps cannot be repaired by editing the owned panel directory alone. They
are not permission to edit `surface`, `js/v4/src`, the shell or designer tokens.

## DESIGN-TODO(opus)

- Specify the spoken kind/label for supplied presets (§6 only specifies an effect
  row). The neutral preset row speaks its registered label without claiming it is
  an effect. Presets still obey the specified effect drag-payload contract.
- Specify presentation/copy/retry/focus for rejected applies, failed search and
  settings persistence errors. The spec only defines an effect-definition load
  failure. Until decided, errors go to the structured host `onError` boundary and
  no invented product string or failure surface is shown.

## Decisions and selecting rules

| Decision                                                                                                        | Rule                                                                               |
| --------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Title Effects; layer-list 266 pt, agent popover/pin and removed web export receive no changes here              | Owner decisions; design ownership and lane scope                                   |
| Single click browses; only explicit activation edits; eight recents, registered group order, label-or-ID search | N3: authored intent from W064/spec/tag                                             |
| Registry and target/editor adapters are required; no synthetic runtime definitions/commands                     | N1/N3/N6: real registered definitions and extension overrides remain authoritative |
| Atomic HostClient edits await actual acknowledgements before recents/success; settings are separate             | Kernel single writer; N3/N4/N7                                                     |
| Keep unknown recent/collapse IDs, source settings and every old panel/compat provider                           | N4/N6 and prune-only-after-last-consumer                                           |
| Worker search, bounded surface, epoch fencing and ID-based inspection                                           | N2/N7/N8: native main thread never waits on JS; no Node/Chromium in runtime        |
| UI kit/token gaps remain neutral and explicit; no custom native-control imitation                               | Opus ownership; N1/N3 and quirk policy                                             |
| Test-only conformance executable belongs to the native workspace; only its lock entry is added                  | Orchestrator merge-ready rules 2/6; reproducible locked builds                     |

## Validation and macOS gates

Run from `apps/native/js/v4` after activating the repository toolchain:

```sh
bun install --frozen-lockfile
bun test panels/effects
bunx tsc --noEmit -p panels/effects/tsconfig.json
bunx prettier --check panels/effects/*.ts panels/effects/*.tsx panels/effects/tsconfig.json panels/effects/README.md
bun test
bun run typecheck
```

The scoped tests build `conformance` with `cargo build --locked` and replay every
snapshot through the real Rust wire/kit/AccessKit implementation. The browser
bundle also mounts/filters without process, Buffer, window or document.

From `apps/native`:

```sh
cargo test --locked -p pm-surface -p pm-panel-effects-conformance
cargo fmt -p pm-panel-effects-conformance -- --check
cargo clippy --locked -p pm-panel-effects-conformance --all-targets --no-deps -- -D warnings
```

macOS must verify the real AppKit/GPUI layout/control/drag/menu/IME/focus/VoiceOver
integration, target/edit/Undo/settings durability, full virtual scroll/AX row
navigation at 60k, worker/UI scheduling with no main-thread JS waits, and the Opus
review checklist (light/dark, contrast, transparency, motion, 1×/2× hit geometry).
Linux snapshots are semantic conformance evidence, not those rendering gates.

Linux validation after merging both required bases: **65 panel tests**, including
**17 accepted wire/AccessKit snapshots**; **237 total v4 tests** (panel tests
included); **95 existing pm-surface Rust tests**. The conformance executable is
exercised by the panel tests and adds no Rust unit-test count. No failures or
skips. Scoped/full TypeScript and Prettier, browser builds, package rustfmt and
strict all-target Clippy pass. Cold probe compilation was also exercised during
module setup, outside per-test interaction timeouts. Cargo.lock adds only this
test package; no resolved dependency version changes.
