# P2 extension checks — 2026-11-01

**OPEN / NOT PASSED.** No extension has accepted paired Electron/native traces,
visual oracles and complete scripted-interaction coverage. Successful activation
and component tests are not extension matches.

Reference: frozen `v1.2.0`, `07ed5fa17ee640659cb075f9cad1af2095011d60`.
The earlier check's executable and harness were brought forward from its branch;
they were not present in the fetched native-engine branch at continuation time.
The S1 sandbox archive and bindings passed all five checksum checks. Builds use
Rust 1.96.1, `engine/env.sh`, the shared target and the build lock. No release,
app update, tag or deployment was created.

## Population and current evidence

| Source | Total | Match | Mismatch | Can't run | Logic executed in real V8 | Activated |
|---|---:|---:|---:|---:|---:|---:|
| Owner installations | 44 | 0 | 0 | 44 | 44 | 43 |
| Local public corpus | 10 | 0 | 0 | 10 | 9 | 9 |
| Published Store, latest releases | 4 | 0 | 0 | 4 | 0 | 0 |
| **Total** | **58** | **0** | **0** | **58** | **53** | **52** |

Activation counts are from the rebuilt runner's complete one-case-at-a-time
headless V8 audit, after the kernel and compatibility fixes below.
The previous baseline activated 36 of the same 53 executable inputs. Populations
remain separate, without cross-source deduplication or unsupported-extension
exclusions. Owner discovery counts immediate manifest-bearing installations:
24 v1 and 20 v3. One local fixture declares permissions with apiVersion 1 and
is rejected by the frozen manifest parser; its rejection remains unchanged.
Store archives are compiled but never substituted into the trusted owner realm.
The refreshed anonymous Store catalog still has four latest releases; all four
compile, and all four remain blocked on the permissioned runtime connection.

These numbers do **not** mean all requested full comparisons completed. The
native capture driver still declines full capture: it has no integrated native
script/trace/screenshot endpoint. The new WebKit and Store components below do
not change that admission rule. Complete per-extension scripts/projects and the
permissioned Store facade/helper connection remain required.

## Activation investigation

All 17 original failures were reproduced before changes, using copied inputs
and the actual production bundle inside V8. Their immediate causes were:

| Family | Original failures | Immediate cause |
|---|---:|---|
| DOM panel | 11 | Missing panel open/close/isOpen backend |
| PM facade | 5 | Missing `PM.invalidate` |
| Lifecycle / DOM scheduling | 1 | Missing host animation-frame scheduler |

The final headless audit removes 16 of the 17 original activation failures.
Deeper failures exposed an absent icon helper/DOM string conversion and an
incorrect initial project/composition representation; both are fixed and
covered by passing regressions. One input still requires the unavailable shared
`ui.controls.SelectField` provider and its connected document/selection state.
The runner preserves the underlying reported error rather than only
returning the generic activation-error wrapper. No owner source, names,
identifiers, selectors or error strings enter tracked files.

No **measured cross-host mismatch** ranking exists yet. The table above is an
activation-blocker classification, not an Electron/native mismatch ranking.

## Adapter implementation and limits

- **Kernel:** the runner owns a disposable `pm-kernel::Kernel`, without a profile
  or journal directory. Calls use the native history/transaction authority;
  replies carry authoritative document, selection, workspace and history facts.
  The adapter handles direct writes, history begin/commit/cancel/rollback,
  undo/redo, external closures and cleanup pins, command batches through the
  existing native command planner, selection and workspace writes, plus the
  synchronous engine-query boundary. Immutable request/reply caching preserves
  retries and rejects transaction-ID reuse with different requests. Unsupported
  methods return acknowledged errors. Structural callback leases, complete PM
  facade methods, built-in provider closure and rendering backends remain open.
  Publications and history snapshots expose the legacy open-composition shape;
  direct-write operations are staged in that shape and translated back to the
  native canonical document before the authority applies them. The synthetic
  initial project now comes from the same complete factory as the reference.
- **Panels:** `CompatPanels` invokes actual contribution builds in the V8 DOM
  mirror, tracks open/hidden state, refreshes registry overrides and disposes
  owned roots/imperative cleanup functions. `PM.invalidate` flushes the DOM queue
  and marks presentation dirty; animation frames use runtime-owned timers.
  Opening a mirror contribution is not a WebKit paint acknowledgement.
  The icon helper is extracted unchanged from the frozen public source. Shared
  Svelte mounting/reactivity modules are loaded for the host even when extension
  source imports neither; optional browser modules remain demand-loaded.
- **WebKit:** `MacDomHost::new_remote_dom` accepts the host-built production
  `PageApplier` bundle, using a dedicated document per realm. `RemoteDomPanel`
  validates mutation batches through `pm-remote-dom::Router` on a routing worker
  before queueing admitted batches to AppKit. The UI only presents admitted
  batches and retains the front batch on backpressure. Actual page feedback
  enters a bounded routing mailbox; its default handler explicitly refuses an
  unattached mailbox. The router then forwards validated feedback to a bounded
  realm-owner queue. Queue failure does not advance sequence state; only page
  feedback advances the applied sequence. Detached pages may acknowledge/layout
  but cannot forward input.
  The CLI still does not create this host or run a complete capture script.
- **Store:** `StoreSession` reads installed release state and publisher-bound
  decisions, refuses pending consent and full-access substitution, and rechecks
  live revision/release/permissions for method calls. The WebKit constructor
  retains data stores by the full profile-scoped identity, with a separate key
  namespace for the legacy constructor. These are platform admission components;
  the confined Store helper/facade and runtime connection to the durable
  StoreKernel are still required before any public Store extension can be counted
  as executed.

The recorder still supports its documented native subset: basic state,
paths/rectangles, transforms, pinned drawImage sources and pixel readback.
Complete DOM-canvas integration, typed ImageData, text/paint/encoding and engine
rendering remain open. No Canvas or visual tolerance exception was introduced.

## Small synthetic reproductions

`element.innerHTML = 42` reproduced a parser TypeError in the V8 DOM mirror,
while the independent DOM produced `"42"`. The setter now performs DOMString
conversion before parsing, including null-to-empty and Symbol rejection. The
regression covers ordinary elements, templates and SVG. The earlier explicit
browser-bundle export and comment-Range UTF-16 fixes remain in place.

Passing native tests cover stored/canonical composition translation, native
history round trips and external closures, transaction retry identity, selection
refusal, presentation demand, bounded DOM admissions/feedback and Store session
permission revocation. The production page-shell/applier event round trip also
passes in the headless DOM harness. The real WebKit probe compiles; it has not
been launched in background mode.

## Harness and resource handling

The shared GUI lease now waits and retries every 60 seconds instead of skipping
a busy lane. Waiting occurs outside the capture execution timeout. Screen state
is checked after acquisition; the lock file is never removed. A free slot was
obtained after waiting. The disposable frozen reference was instrumented and
built; an existing matching Electron 44 binary avoids a second installation.

An initial all-source harness invocation completed one reference pair, which
failed repeat determinism, before disk pressure prevented later captures. It
produced no accepted paired extension oracle. Disk pressure must not be counted
as a completed extension attempt: the corrected runner pauses below 6.5 GiB and
resumes at or above 6.5 GiB. The build wrapper uses the same revised thresholds and
retries interrupted builds. These limits were explicitly revised during this
continuation. Inputs remain one case at a time and are deleted after use.

Build and capture entry points also honor `/tmp/pm-shots/PAUSE_LOCAL`, checking
every five minutes. A new local pause stops the owned process group, releases the
GUI lease, and retries the interrupted attempt after the pause ends. No lock or
pause file is removed. Both capture roles also wait while
`/tmp/pm-shots/USER_ACTIVE` exists, and stop their owned capture if it appears.
Background builds/tests use `nice -n 20`, four Cargo build jobs and four Rust test
threads. The full GUI comparisons are queued until owner availability returns.

Latest completed checks: compatibility 176 pass; remote DOM 39 pass; both
typechecks pass; capture harness 21 pass; native V8 runner 14 pass; WebKit host
headless contracts 25 pass; Store contracts 90 pass. The build-tool-to-V8 CLI
integration probe passes with the complete initial project and shared Svelte
runtime. Formatting and all-target Clippy (`--no-deps -- -D warnings`) pass for
the runner, WebKit host and Store host packages.
No Electron/native window, screenshot or screen comparison was started during
background validation. Complete paired comparison validation remains pending.

Only anonymised aggregates and original synthetic probes belong in Git. Raw
sources, reports, traces, screenshots and selectors stay under disposable paths;
the installed profile is read-only. The final gate still requires completed
repeatable captures for every source, complete interaction coverage, and fixes
or explicit `can't run` evidence for every remaining case.
