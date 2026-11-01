# Frozen-reference behaviour traces

The P0 recorder lives in [`tools/trace-recorder`](../../tools/trace-recorder). It is an optional build overlay, not a native implementation. The provisional reference is **`v1.2.0` (`07ed5fa17ee640659cb075f9cad1af2095011d60`)** pending the reference-build decision in [p0-decisions-for-jude.md](p0-decisions-for-jude.md). The format and recorder core contain no hard-coded reference version; the runner records the actual checkout revision and accepts `--ref` and `--reference-label`. Source adapters fail on unsupported source shapes rather than silently applying a patch to the wrong location.

This is the behaviour oracle for the history/keymap P0 slice. It does not establish complete C-MATRIX, native-kernel, compatibility-page or visual parity. Intentional native differences—selective cancellation, awaited async decline, bypass queues and the proposed direct-write barrier—must be classified separately. The reference recorder preserves the shipped implementation.

## Capture and removal

Use Node 22.18+ for the Playwright driver and Bun 1.3+ to install/build. Running the Playwright driver under Bun can stall the Electron startup handshake. The existing `apps/desktop/e2e/helpers/app.ts` is reused through a temporary Node-compatible transpilation; only module path handling, finite startup waits and failure cleanup are adapted. The application still uses the harness's isolated temporary profile and hidden window. `ELECTRON_RUN_AS_NODE` is deleted before launch.

From the implementation checkout:

```sh
df -h /
git worktree add --detach /tmp/pm-ref-v1.2.0 v1.2.0
cd /tmp/pm-ref-v1.2.0
bun install --frozen-lockfile
cd -
bun tools/trace-recorder/apply.ts /tmp/pm-ref-v1.2.0
node tools/trace-recorder/run.ts --check
bun test tools/trace-recorder/tests
node tools/trace-recorder/verify-off.ts /tmp/pm-ref-v1.2.0
```

Stop if free disk falls below 3 GB. Do not use an installed application profile or a user project. The runner creates synthetic projects and disposable DOM only. Build output, profiles, diagnostic logs and ordinary captures stay outside the implementation checkout. Dependencies and Electron require substantial disk space.

`run.ts` sets **`POWERMOVE_TRACE_RECORDER=1` at build time** and normally rebuilds. Both Vite adapters return no transformation when this flag is absent or `0`; the resulting application has no trace-controller global. `verify-off.ts` builds both modes, launches both hidden, compares history/edit behaviour and checks that the disabled build has no controller. It leaves a disabled build. Rebuild before recording again.

The overlay modifies only the reference checkout's Vite config, copies the adapter/runtime there and creates `e2e/helpers/app.trace.mjs`. It never commits in that checkout. Renderer instrumentation is applied during the enabled build, leaving original renderer source files untouched. It inserts method-body probes, not proxy objects or replacements for public API functions, callback closures, project objects or returned values. It adds no Promise handlers to application results, and rethrows the same error object. Instrumentation has observation overhead and changes reflected function source text; these traces are not timing measurements.

Options:

- `--scenario history-cancel,keymap-fields`: select synthetic cases.
- `--out /tmp/pm-traces`: choose a diagnostic output directory (default: a fresh temporary directory).
- `--update`: intentionally regenerate tracked goldens after review; every case must produce two identical runs first.
- `--check`: compare fresh captures byte-for-byte with committed goldens.
- `--skip-build`: reuse the build; a compiled adapter fingerprint check rejects stale/disabled instrumentation.
- `--all-events`: include bus emissions outside observed calls, for diagnostics. These captures cannot update/check deterministic goldens because background presentation scheduling varies.

Remove the overlay, then remove the reference worktree (no commits are needed there):

```sh
bun tools/trace-recorder/apply.ts /tmp/pm-ref-v1.2.0 --remove
git -C /tmp/pm-ref-v1.2.0 status --short
git worktree remove /tmp/pm-ref-v1.2.0
```

Removal restores the checked-out revision's Vite config and deletes only this overlay's files. An unrelated config edit is refused. Ignored dependencies/build output are disposable with this reference worktree. Never remove another lane's worktree or files.

## Recorded boundaries

| Boundary | Recorded observations |
| --- | --- |
| Public trusted `history` facade and `PM.hist` | `do`, `begin`, `commit`, `cancel`, `external`, `selection`, `undo`, `redo`; underlying scoped begin/track/rollback/change checks, clear and undo-if-top |
| Trusted `edit` facade and `PM.Edit` | `apply`, `mutate`, `begin`, `dispatch`, `commit`, `cancel`; exact callback result identity, `EditResult`, boolean, undefined and thrown error forms |
| `EditGesture` | `begin`, `write`, `commit`, `cancel`, `once`, with gesture/binding identity and command/set/local mode |
| Workspace | Public `workspace.mutate` and underlying `WS.mutate`, including thrown callbacks and returned workspace identities |
| Events | Legacy and kernel bus emissions in original order, before delivery to existing listeners; nested emissions retain their enclosing call id |
| State | Live project and selection identity/structure, pending history depth/snapshot/scopes, stack cursor, document patches, external closure handles, cleanup handles and byte count, before/after each call and at explicit checkpoints |
| DOM/keymap | Window capture; the real `registries.ts` window bubble handler; ordered candidates, owner/priority/chord, field/repeat exclusions, pre-dispatch cancellation, actual command result, key-up; synthetic target/ancestor and later-window listeners show DOM order |
| Menus | Real MenuItem undo/redo → existing IPC → renderer command path; trusted Chromium Cmd-Z via `webContents.sendInputEvent`; main-process `before-input-event` routing and ignore-menu-shortcuts decisions |
| Direct writes/touch | Project identity/value snapshots at observed boundaries; `PM.touch`; explicit `PM.proj.direct` operation probes in synthetic scenarios |

**Direct-write limit:** C-MATRIX found no general write barrier in this tag. Installing proxies or setters would change the live-reference contract. Therefore arbitrary unwrapped assignments are observed at the next recorded boundary, not intercepted individually; multiple writes between boundaries can collapse. The synthetic direct-write/async cases put explicit observational spans around their assignments, preserving the assigned return value and retained reference. This is not proof of a general barrier, dirty-path validation, resync, or native-write races. A future reference with a real barrier needs a flush adapter. No native admission policy is installed in Electron.

The deterministic golden event scope is **all emissions inside observed synchronous calls**, plus explicit input/menu/callback/driver checkpoints. It excludes unrelated bus emissions between calls. The full `--all-events` mode records those emissions too, in actual observed order. Promise continuations are not implicitly awaited by the recorder; scenarios keep a capture open and explicitly mark the continuation when needed. No cross-process total order is invented: renderer records are ordered at observation time; `main.observations` contains main-process observations in their own array order, collected after the driven IPC/input step.

## JSON-lines format, version 1

UTF-8; one JSON object per line; LF terminated; no wall-clock record timestamps, absolute source paths or error stacks. The header has no sequence number. All subsequent rows have contiguous `seq` numbers starting at 1. Object/property order follows JavaScript enumeration order; it is not alphabetically sorted. Captures carry the actual reference revision, recorder/scenario source hash, compiled adapter hash, driver, input kind, role, API-version scope, event scope and `synthetic:true`.

```json
{"type":"header","format":"powermove.behavior","version":1,"scenario":"example","synthetic":true}
{"seq":1,"type":"call","call":1,"parent":null,"name":"history.cancel","args":{"$id":"o1","$type":"array","values":[]},"state":null}
{"seq":2,"type":"return","call":1,"value":{"$type":"undefined"},"state":null}
{"seq":3,"type":"end","valid":true,"calls":1}
```

This abbreviated example shows row structure; real captures contain full metadata and state.

- `call`: increasing `call` id, enclosing `parent` call or null, `name`, structural `args`, pre-call `state`.
- `return` / `throw`: matching call id, structural `value` / `error`, post-call `state`. Nested calls complete in stack order. Returns are not awaited or coerced by the probe.
- `state`: explicit label and state inside `data`; initial/final checkpoints are always present.
- Other `type` values (`event.legacy`, `event.kernel`, `key.candidate`, `key.result`, `dom.*`, `gesture.context`, `menu.received`, `main.observations`, `driver.*`, `callback.*`) carry `parent` and structural `data`.
- `end`: `valid` and completed call count. An observation/serialization failure makes it invalid. A missing/invalid end is never an accepted oracle.

Structural values:

| Encoding | Meaning |
| --- | --- |
| JSON primitive | Ordinary null, boolean, finite number or string |
| `{"$id":"o1","$type":"object","props":{...}}` | Stable live object identity and its current enumerable own data properties; repeated observations keep the id and resnapshot values |
| `{"$ref":"o1"}` | Alias already visited in this value graph (including a cycle) |
| `array` / `map` / `set` | Identity plus ordered `values` or `entries` |
| `function` | Identity, name and arity; no source text or invocation by the serializer |
| `promise` | Identity only; no `.then` or rejection handler added |
| `error` | Identity, name and message; stack omitted |
| `undefined`, `number`, `bigint`, `symbol` | Explicit tags; preserve undefined, non-finite numbers, negative zero, big integers and distinct symbol identities |
| `accessor` | Getter/setter handles; getters are not invoked |
| `key` / `element` | Keyboard flags/phase/default cancellation and minimal DOM identity/target descriptors; no DOM-tree dump |
| `{"$token":"id:1"}` | Generated model/history identifier, deterministically assigned by encounter, without changing the application's string |
| `json-string` | A returned serialized project string represented as a normalized JSON data tree; decoded data has no invented live identities |
| `{"$volatile":"wall-clock-ms"}` | Epoch-millisecond provenance `at` field; absolute clock values are excluded from comparison |

Generated identifiers used as object keys are encoded as `@token:id:N`; literal keys starting with `@` gain one leading `@`. Tokens are allocated for reachable initial ids and newly generated ids. Synthetic projects use a fixed creation timestamp. Authored times, revisions and ordinary numbers remain exact. Arrow probes record their declared arguments after parameter defaults (rest parameters retain the supplied list); ordinary methods/functions use the original `arguments` list. Public facade rows and nested implementation rows are both intentional.

The core uses data descriptors to avoid invoking getters. Arbitrary hostile Proxy traps/custom intrinsic overrides are not a proven zero-observation-effects surface. The committed fixtures contain ordinary synthetic records. This recorder is reference-test tooling, not a security boundary or an extension sandbox.

## Replay contract

[`validate.ts`](../../tools/trace-recorder/validate.ts) rejects unsupported versions, missing/invalid completion, sequence gaps and unbalanced/mis-parented calls. Future pm-kernel/compat replay should:

1. Decode stimuli from `driver.*` and root API calls; provide scenario callback/closure handles rather than attempting to execute function source from a trace.
2. Map `oN`, symbol and logical-id tokens onto replay-local identities. Check alias equality and retained-old references independently of structural equality.
3. Compare call admission, return/error forms, nesting, ordered emissions, immediate reads, project/selection changes and undo/redo effects. Adapt reference debug state to each host; do not require Rust to copy internal JavaScript allocation or function names.
4. Keep the reference observation and any explicitly approved native difference side by side. Do not rewrite a tag golden to make a new policy pass.

An automated native replay engine is subsequent work. Byte comparison is currently between repeated Electron captures from the same recorder/reference, not between Electron internals and Rust.

## Synthetic scenarios and evidence

The committed set contains **11 traces / 1,093 rows**. Each was produced twice identically in a hidden Electron 44 session on the pinned tag. The in-scenario assertions verify the recorded outcomes; the core/overlay tests also exercise error/return identity, cycles, getters, Promise handling, invalid captures, parameter preservation and the disabled transform.

| Scenario | What it establishes | Rows |
| --- | --- | ---: |
| `history-nesting` | Nested snapshots; do during begin; commit depth; undo/redo; exact callback return; thrown rollback/rethrow | 144 |
| `history-cancel` | Edits kept, entry dropped, undefined return | 12 |
| `edit-cancel` | Empty cancel preserves project reference; changed live cancel reverts; no-live false | 96 |
| `edit-admission` | Nested source begin throws; mutate refuses before action during live edit; scoped begin returns null; malformed/no-op/non-tree/stale/empty cases | 96 |
| `external-history` | Invalid callbacks, independent push during begin, inline undo/redo, cleanup, cursor movement before external throw | 55 |
| `gesture-modes` | Command/set/local paths; set cancel keeps writes; command cancel reverts; once returns; commit | 212 |
| `workspace-direct` | Workspace return/throw, partial write path, direct assignment result/identity, touch without undo | 23 |
| `keymap-fields` | Field exclusion, inFields decline/handle, ordinary pre-cancellation, Escape and Cmd-period | 105 |
| `keymap-order` | Target/window order, decline→next, repeat filtering/handling, Promise-false treated as handled, pending Promise followed by Escape/Cmd-period, stop vs prevent propagation | 187 |
| `menu-equivalents` | Real menu undo/redo and trusted Cmd-Z, routing decision, renderer receipt and key-up | 141 |
| `history-async` | Exact Promise return, commit before continuation, later raw write outside the committed entry | 22 |

Remaining unverified: native/compat replay; native queue/watchdog/bypass semantics; physical AppKit menu selection and text defaults/IME; every C-MATRIX command, Store iframe forwarding, popout documents, hostile argument objects, arbitrary direct-write interception and barrier/race cases. Escape/Cmd-period observations describe the tag's immediate dispatch, not proof of the proposed native bypass queue. Reference choice remains provisional until the owner resolves Q1. Keep the frozen reference/overlay until its last comparison consumer and parity gate are retired.
