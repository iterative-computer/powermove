# Agent built-in v4 panel — 2026-11-01

**Partial implementation; blocked for native panel acceptance. All visual output is provisional
until Opus review.** Entry: `index.ts`. Source spec:
`docs/native/design/panels/agent-panel.md` and its shared README/owner decisions.
This lane changes only this directory, thin `pm-panel-agentpanel` adapter code
and the one required native lockfile dependency edge. No existing consumer is
removed; Effects, the 266 pt layer list and the absence of web export are retained.

The panel uses the UI kit for available controls and pm-design token references.
Structural `div`/`text` nodes describe containers and neutral placeholders; there
are no custom push buttons, text editors, glyph renderers or browser controls.
Unavailable controls are deliberately inactive. Their `metadata.apiGap` identifies
the exact missing API; metadata is diagnostic data, **not** an undocumented
renderer command. These trees cannot yet provide the complete specified UI.

## Model and host wiring

`pm_panel_agentpanel::v4` maps actual model names (not the spec's proposed names)
to a bounded `Snapshot` and id-based dispatch. `Window` pairs immutable archived
rows with their flattened `TranscriptIndex`: even a single 60k-step trace pages
three rows without serializing the entire trace. Cache it per thread/archive
publication on the worker; rebuild it after archive mutations, including in-place
question/result acknowledgements. A structural count change rejects an obsolete
window. `Window::count` reports the flattened logical count. Live streaming needs
no archive-index rebuild. The window limit is 256 serialized rows, **not a byte,
layout or latency certification**. Very large visible tool output still needs a
paged output/resource API.

`Draft::text` projects exact authored text without cloning attachment bodies.
Snapshots omit private answer drafts. Scoped archive ids keep duplicate legacy
step disclosures independent. Effect actions on ambiguous archived raw ids refuse
explicitly until the model exposes scoped effect commands; they never operate on
a different duplicate. All archive bytes remain in their existing store.

`dispatch_at` checks coordinator sequence, project, visible thread and run before
calling the existing model methods. Context time and fresh run/request/thread ids
come from the native coordinator. Returned `Effect`s remain pending for the
existing provider/shell/storage owner; no provider result or consent is inferred
from queue admission. HostRouter/HostClient separately enforce realm, generation,
document revision and actual host acknowledgements.

`HostAgentModel.connect(host, registration)` requires **application-registered**
read/dispatch command ids plus a generation-scoped publication callback. No agent
query exists in the current `HostQuery` enum. Route these two registered commands
through the real `HostDataService` backend on the model worker, publish only actual
model facts, and include the current publication sequence. Publish provider/model
changes even when document revision stays unchanged. `NativeFacts` supplies
connection, presentation, shell/input/accessibility observations and catalog copy.
The application must retain a failed/unsaved model and use its existing lifecycle
lease/storage owner; this extension opens no writer or profile.

The app must mount `AgentPanel` with the existing `createRoot` and a required error
handler, retire the model/root with their realm generation, and forward accepted
native actions asynchronously. The entry is ready for that registration; **the
application registration/effect runner is outside this lane and is not installed
here**. The UI-kit entry is imported directly because the current public `ui`
export still exposes the older component set.

`model-wire.rs` is a **test-only** backend: public HostClient → actual Rust
HostRouter/HostDataService → real Panel/adapter → six-op panel. Its Store, ids,
connection facts and pending-effect collector are synthetic. No account, provider,
files, consent dialog or project edits are simulated as successful. This executable
also projects the emitted commits through the real AccessKit tree builder.

## Spec section → status

| Section           | Done in this lane                                                                                                                                                                                        | Not done / acceptance blocker                                                                                                                                                                                         |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| §1 Presentation   | Popover default facts, popover/docked solid-content tokens, Pin/Close model effects                                                                                                                      | NSPopover/toolbar placement, height/min-width math, undoable pin/layout, drag previews, visibility toggle, launcher ring/dot/badge                                                                                    |
| §2 Model          | Actual Rust model adapter, explicit host publications, target/generation admission, intact effects                                                                                                       | App command registration, live provider/shell facts, thread summary/unread window service                                                                                                                             |
| §3 Anatomy        | Kit header actions, conversation descriptions, disclosures, timer, composer/attachment descriptions, Send/Stop, exact available copy                                                                     | Native composer, rich Markdown/selection, system push/default buttons, complete cards/results, pickers/menus, symbols, previews, activity grouping, scroll host                                                       |
| §4 States         | 23 op-tree snapshots: empty variants, connection gates, history loading, working/answer/stopping, hover/focus/selected/disabled/error/drop, multi-select N/A, 60k, narrow/docked and accessibility flags | Snapshots certify the partial tree only; native geometry, text selection, drop/paste, measured-height virtualisation, narrow wrapping/model-glyph collapse, loading timing and appearance still need host/kit support |
| §5 Interactions   | Supported header/attachment/Send/Stop/disclosure actions; actual model key/slash/IME/repeat/select-all/consent behavior tested through data service                                                      | Composer input/paste/drop/inline token driver, native key cancellation, file picking, full cards, drag/dock and fitted previews                                                                                       |
| §6 Keyboard/focus | Available controls' order/disabled guards; native model routes; hidden disclosure descendants removed                                                                                                    | Full tab sequence, native composer/selection ownership, global ⌘⇧K registry, layered Esc dismissal and focus restoration                                                                                              |
| §7 VoiceOver      | Actual AccessKit group/list/button roles, available labels, disabled state and descendant assertions                                                                                                     | AXRowCount, row role projection for raw message containers, text-area/hint semantics, row navigation, announcements and real VoiceOver                                                                                |
| §8 Appearance     | Token-only colors/type/radii/available dimensions; no decorative canvases                                                                                                                                | Exact derived geometry, control slots/material renderer, accessibility appearance, screenshots and Opus approval                                                                                                      |
| §9 Copy           | Spec strings and model/provider-authored content only; no new product copy                                                                                                                               | Unavailable controls' copy remains represented by placeholders until their API lands                                                                                                                                  |
| §10 Don't         | Popover default; solid content; replies have no bubbles; no decorative canvases; no retired consumer deletion                                                                                            | Platform shortcut/focus/popup proofs require the macOS integration                                                                                                                                                    |

## API requests — additive to the frozen contract

1. Kit native multiline composer: host binding for thread-local draft, IME/UTF-16
   selection, inline attachment tokens/arming/Undo, clipboard/file/layer drop and
   caret anchoring; modifier/composition-aware native default cancellation.
2. Kit system push/default/borderless buttons and agent-specific disclosure/card
   recipes. ToolbarItem is not substituted for push buttons. EmptyState needs the
   specified display typography slot; current kit fixes its heading typography.
3. Kit selectable Markdown/code/tables, media/resource thumbnails, output paging,
   SF Symbols and message alignment/max-width/token arithmetic/wrapping. No raw
   markup executes. Copy/selection must stay in native text, never layer commands.
4. Variable-height list API: stable item ids, measured-height cache and streaming
   invalidation, viewport plus one viewport overscan, scroll anchors/full extent,
   scroll-to-row/Latest, AXRowCount and accessibility row navigation. Current
   virtual-list only admits fixed `itemSize`; metadata does not implement this.
5. Thread/model picker publications, unread/running counts, confirmed archival,
   provider capability/display labels; access menu subtitles/warnings plus the
   native consent effect; approval decision commands and connection/settings/retry
   routes. Access is disabled here so a binding cannot bypass consent.
6. Isolated visual/fitted-attachment previews, leased resources, modal zoom/replay,
   result/frame/Finder/import actions and opener focus restoration.
7. Shell launch/popover/pin/drag/layout/visibility/keyboard integration; toolbar
   symbol/ring/dot/count rendering; system materials and accessibility notifications.
8. Actual host command/effect registration and native publication/lifecycle wiring;
   scoped model effect commands for duplicate archived ids. Row-count/announcement/
   message-row AX projection and real native control rendering remain host work.
9. Missing `metrics.composer_max_lines` and token arithmetic/style/slot APIs for
   the exact specified derived measurements. No fallback literal is substituted.

## DESIGN-TODO(opus)

All four are also recorded in `gaps.ts`; no extra product decisions are made:

1. §3 anatomy draws a header search item, but the header table and §6 tab order
   omit it. The neutral header omits search pending resolution.
2. Approve token names/expressions for literal-only strokes, attention/unread
   dots, code padding and the below-300-pt model-label collapse. The shared README
   prohibits substituting literal visual values; current wire styling has no
   approved token-arithmetic representation.
3. Reconcile §3's recommended/default single-choice buttons with the actual
   model/tag's explicit Answer behavior (choosing never submits). Question
   submission controls remain placeholders; the model contract is preserved.

4. The model restores interrupted reply questions as `closed`, but the spec
   provides only Answered/Skipped copy. Closed questions stay plain labelled
   placeholders with no submission controls pending the specified state/copy.

These items and all visuals require Opus review; API absence is not a request to
redesign the already specified controls.

## Rule-selected decisions

| Decision                                                                                                | Selecting rule                                                                               |
| ------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Retain model effects/actual acknowledgements and exact Unicode/newlines; refuse stale/ambiguous actions | N3/N7: authored intent and inspectable id-based admission                                    |
| Keep native coordinator, storage owner and kernel authority; enqueue without native-thread JS waits     | N8 and pm-kernel single-writer architecture                                                  |
| Use kit/token descriptions; explicit neutral placeholders for missing APIs                              | N1/N3 + design ownership: do not invent controls, measurements, strings or renderer behavior |
| Cache/window archived steps; avoid draft attachment-body copies                                         | N1/N2: large professional sessions must remain usable; physical latency remains unproved     |
| Leave every old consumer/archive/extension path intact                                                  | N4/N6 and prune rule; no data migration/deletion or contract cutover is authorized here      |
| Preserve popover + pin and shared owner decisions                                                       | N3: recorded authored/product intent wins; no Chromium decoration is reproduced              |

## Reproduction and macOS gate

Every shell: `source /workspace/.tools/activate.sh; cd /workspace/powermove`.
Runtime/test prerequisites: Rust 1.96.1, Bun 1.4.4 and the frozen package lock.
The own Rust example is declared in the model manifest; tests reference no spikes,
private files, GNU utilities, platform temp paths or installed user state.

```sh
cd apps/native
cargo test --locked -p pm-panel-agentpanel -p pm-surface
cargo clippy --locked -p pm-panel-agentpanel --all-targets --no-deps -- -D warnings
cargo fmt -p pm-panel-agentpanel --check
cd js/v4
bun install --frozen-lockfile
bun test
bunx tsc --noEmit -p panels/agent/tsconfig.json
bunx prettier --check 'panels/agent/*.{ts,tsx,json,md}'
bun run typecheck
bun run format
bun run build
```

On macOS, finish the missing host/kit integrations, then verify actual NSPopover/
dock behavior and layout Undo, native text/IME/dictation/pasteboard/selection and
shortcut ownership, all card/picker/preview interactions, sandbox/resource/focus
restoration, 60k measured-row scrolling/input latency and full VoiceOver. Capture
light/dark 2×, Increase Contrast, Reduce Transparency and Reduce Motion states,
confirm real hit rectangles and no UI-thread JS waits, then obtain Opus review.
Linux wire/model/AX descriptions do not certify those gates.

## Frozen API baseline — 2026-11-01

Merged `origin/feat/native-engine` at
`f7844ebc007c17900cb2d65f4ef63c6bd3f8b005`, including the recorded v4 API freeze
in `docs/native/p0-decisions.md`. This supersedes the integration branch as this
panel's API baseline. The frozen `v4-api.md` and `ui-kit.md` contracts match the
API implementation already consumed by this panel; no code adaptation, API shape
change, new dependency or lockfile update is required for this merge.

The API requests above are additive requests for their owning lanes. The panel
continues to use the frozen six-op protocol, existing kit components, asynchronous
HostClient/HostRouter/HostDataService boundary and explicit diagnostic metadata.
No change is made to `crates/surface` or `js/v4/src`. Missing native controls and
application registration remain acceptance blockers; the 40/40 headless API eval
does not certify completion of this panel or macOS rendering.

Rule-selected decision: consume the frozen contract unchanged and report missing
capabilities as additive API requests (orchestrator freeze directive; N3/N7).
Keep all existing consumers and asynchronous authority boundaries intact (N4/N6,
N8 and the prune rule). The four Opus TODOs and macOS verification list above are
unchanged.
