# Native app integration (P1)

The runnable `pm-native-app` now composes `pm-kernel`, `pm-engine-service` and
`pm-shell`. This is a Phase 1 editing surface. **Gate P1 is still open.**

## Run in an isolated library

From the repository root:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd apps/native
cargo build -p pm-native-app
"$CARGO_TARGET_DIR/debug/powermove-native" --profile /tmp/pm-native-profile /tmp/project.pmv
```

Only use disposable copies for verification. The default native library is
`/tmp/powermove-native-integration`; `--profile` or `PM_NATIVE_PROFILE` selects
an explicit separate native destination. No Electron profile is discovered,
migrated or selected implicitly. The shell layout uses that destination too;
NSUserDefaults frame autosave is disabled when this isolated layout is supplied.
The app links a development FFmpeg runtime search path from `PM_FFMPEG_DIR`,
plus the future bundle's `../Frameworks` path. No release bundle is produced.

## End-to-end implementation

- Launch path, File → Open (an asynchronous NSOpenPanel sheet), and GPUI
  external `.pmv`/JSON file drops reach `KernelHandle::shell(Open)`. Stable
  file identities reconnect journals across launches. Failed opens preserve
  the current editor. Switching saves/closes the previous kernel session.
- The Project slot lists the open composition's layer names. Its Library &
  recent projects view uses kernel recents and opens native library slots.
  File → Open Recent and the Projects shortcut show this browser stub.
- The engine's Metal device also initializes GPUI (P-DEVICE). Its retained
  presenter attaches to the shell's below-slot CAMetalLayer. The app configures
  extended-linear P3/RGBA16F/EDR through engine-service. A clipped neutral
  container supports Fit, 50% and 100% without changing the composition's aspect.
  Drawable sizes use backing scale. Display/headroom and pixel-size changes
  reach `Viewer::configure` through the native transport feed.
- Space toggles playback; PageUp/PageDown and Command-arrow bindings step
  frames (Shift steps ten); Home/End seek boundaries. J/K/L try the engine's
  state-dependent shuttle first, then decline to the frozen resolver. Control-J
  starts reverse from pause. Temporary visible-event navigation uses layer
  boundaries. The existing keymap owns field/repeat/priority handling.
- Timeline bars represent layer intervals and select/seek on click. A native
  CALayer playhead and timecode follow engine clock samples without notifying
  the GPUI tree on each tick. The current feed uses a provisional 16 ms timer;
  it is not certified display-link pacing or an idle-budget pass.
- Properties shows the selected name, opacity and X/Y position. A value drag
  begins/steps/commits one kernel gesture; a click opens an asynchronous AppKit
  text-field sheet for precise entry. Edits use typed, id-addressed kernel
  commands; published snapshots update the viewer. Work surfaces consume
  pm-design tokens and carry a visible temporary Phase 1 label.
- Named Undo/Redo menu items use published kernel history. Save/Save As use
  kernel session calls and the existing PMV writer; successful publications
  update native title/represented URL/edited dot. UI facts use StateTransaction,
  preserving other viewer/transport members, outside document undo.
- Export starts an original-only PNG-sequence job in the same executable's
  `--engine-job` child. Serialization/spawn/wait are on a worker. The child
  owns fonts, rendering and atomic publication. No-clobber/failure behavior is
  inherited from engine-service. Other delivery formats and export controls
  are not provided by this temporary UI.

The additive shell seam is `integration::Integration`/`Factory`, exported native
Host/Command access, custom panel bodies, command completion, external paths,
project layout identity and observation. The shell retains its standalone
entrypoints, layout structure, native menus, keyboard resolver and slot owner.

## App MCP transport

The application executable accepts the shipped `--powermove-mcp` flag and serves
newline-delimited JSON-RPC on stdio. With an app already using the selected native
profile, it proxies to that app's authenticated loopback descriptor. With no host,
it starts a windowless kernel host; an optional launch path opens a disposable
project copy before the first request. Both use the same actor-backed connection.
No Electron profile or installed app is discovered implicitly.

```sh
"$CARGO_TARGET_DIR/debug/powermove-native" --powermove-mcp --profile /tmp/pm-native-mcp /tmp/project-copy.pmv
```

Windowed and windowless hosts publish a private, mode-0600 connection descriptor
under the explicit profile and remove their own descriptor on orderly shutdown.
A stale descriptor produces an error; it is never silently replaced or used to
start a second writer. Diagnostics go to stderr; stdout contains JSON-RPC only.

All 67 frozen tools are listed unchanged. The 13 existing P1 bodies are bound to
the app actor: library/project operations, session start/finish, project/timeline
inspection, selection, typed commands/strips, save and guarded rollback. The other
54 tools keep their explicit unavailable errors. This does not implement extension
activation, rendering, surface automation or the P2 headless-extension gate.

Each connection binds an immutable `Origin::Mcp` actor identity and pins the
current generation-bound document session. Reads consume published snapshots;
command planning reuses pm-mcp's frozen planner and state/timeline digests. UI and
MCP selection share kernel publications. MCP writes fold into an active UI gesture
under ordinary kernel rules. Folded/interleaved runs are never snapshot-restored
over foreign work or regrouped into a different history entry. Uninterrupted runs
can group at commit; rollback and disconnect use identity/revision guards.

File saves use the worker save handoff and the current kernel destination. MCP
project switches notify the editor, which adopts the publication and resolves its
media on a worker. The host remains available with the last editor window closed.
MCP calls are refused while the native close decision is pending. A nonblocking
app fence prevents a project open/close from replacing a session during a tool
call; ordinary UI gestures never take that fence. Window activation binds MCP to
the focused document, and project-switch notices reach each registered editor.

## Native close, quit and recovery

Traffic-light close, File close and Command-W use GPUI's cancellable window-close
hook. Command-Q and native termination requests enter the app coordinator before
GPUI's non-cancellable shutdown notification. Quit visits registered editor
windows before requesting final compaction; cancellation or a busy editor stops
the quit. The app retains and forwards the existing application delegate's other
selectors.

Dirty documents show an asynchronous AppKit sheet: **Save / Don’t Save / Cancel**.
Return saves, Escape cancels and Command-D chooses Don’t Save. Saving uses the
worker handoff. A save in flight refuses close/quit and shows a progress note;
an open edit, project open or recovery choice also defers closing. Failed saves,
recovery-copy fallbacks and durability failures retain the window with a message.
Don’t Save calls kernel `DiscardClose`: rejected work is archived, never deleted,
and the original project is not saved. Close waits on `DurabilityBarrier`; quit
also requires `PrepareQuit` compaction and durability to succeed.

Launch recovery uses `PrepareOpen`, then a native **Recover / Open as Copy /
Discard** sheet before adoption. The chosen single-use ticket goes directly to
`DecideRecovery`. Discard archives only rejected recovery data and opens the
unchanged base. Cancellation or a failed choice preserves recovery data. The UI
uses published session destination/media and kernel selection; no local document
or selection writer is introduced.

**On-screen checks for this addition are UNVERIFIED.** The Mac was locked; no
window or sheet was shown for verification. Run the prepared disposable fixture
script on an unlocked Mac:

```sh
apps/native/crates/app/tools/check-lifecycle-ui.sh close
apps/native/crates/app/tools/check-lifecycle-ui.sh recovery
```

Repeat recovery for each choice, and close for traffic lights, Command-W and
Command-Q, including Cancel, save-flight feedback and native Quit with no editor
window. Multi-window document ownership/restoration, Finder/Dock open events and
full native accessibility/menu ordering remain broader integration gates.

## Reproducible smoke

```sh
bun apps/native/crates/app/tools/prepare-corpus.ts /tmp/pm-native-corpus
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
"$CARGO_TARGET_DIR/debug/powermove-native" --gate-p1-smoke /tmp/pm-native-corpus/integration-manifest.json
```

Preparation reads engine/corpus and emits only into the requested `/tmp`
directory. Dependency selection includes explicit fixture extensions, foreign
effects and unknown extension-layer definitions; built-in native 3D definitions
are retained. The manifest and generated corpus stay outside Git. The Rust
harness opens through the kernel actor, renders frame 0 and the midpoint at
320×180 through engine-service Headless, requires complete finite pixels,
changes one static opacity, saves a PMV into a temporary directory, reopens,
and checks the exact stored value and layer identity. Animated keyframes may
continue to override the static value, so this check deliberately reads `v`.
It prints ordinal-only pass/fail lines and final counts and fails on any error.
This smoke is not a visual oracle, playback test, full-resolution export check,
or performance benchmark.

Verification on 2026-11-01:

- Pinned-toolchain app build passes, including the additive shell changes.
  App/shell Clippy with `--no-deps --all-targets -- -D warnings` passes. A
  workspace-dependency lint attempt finds 13 preexisting kernel lint errors;
  kernel files are outside this lane and were not changed.
- The app session integration test passes (1/1): a three-update opacity gesture
  creates exactly one named undo step; library save clears dirty; PMV Save As
  and reopen retain the edited value and undo back to the original value;
  recents are populated.
- Full corpus smoke: **251 passed, 0 failed, 4 extension-dependent skipped,
  255 total**. The selector includes the three deliberately absent definition
  fixtures alongside the explicitly installed extension fixture. No identifiers
  or generated documents are tracked.
- Shell/design regression suite: **15 tests passed** (2 token tests, 1 Metal
  renderer test and 12 shell model/key/layout tests).
- GUI-lock-owned verification opened a real project from the launch path,
  selected a layer, entered opacity 37, saved, undid back to 100, redid to 37,
  and saved a new PMV through Save As. The edited dot appeared after the edit
  and cleared on save. A numeric drag was undone with one Undo and the precise
  field confirmed 37 again. File Open displayed the NSOpenPanel and cancelled
  without replacing the current project. Space playback and boundary keys
  were exercised. PNG-sequence Export completed and published **90 PNG frames**
  into temporary storage.
- On-screen captures (outside Git): `/tmp/pm-shots/integ-edit-save.png` shows
  the selected layer, opacity 37, matching dimmed viewer output and timeline;
  `/tmp/pm-shots/integ-export.png` shows the same editor after Export completed.
  `/tmp/pm-shots/integ-final.png` shows the saved PMV reopened, opacity 37,
  and a seek to a different animated frame with a matching playhead/readout.
  These verify the real AppKit/GPUI/Metal app, not a mockup or headless surrogate.
- The recovery sheet was observed after interrupting the isolated test app;
  Recover reopened its project. That is smoke evidence, not a journal-kill gate.

## Verification of MCP and lifecycle wiring

Headless verification for this addition:

- `cargo test -p pm-native-app -p pm-mcp`: 14 app unit tests, 2 app integration
  tests and 25 existing MCP tests pass. The executable test creates a disposable
  corpus-project copy, spawns `powermove-native --powermove-mcp`, and runs initialize,
  tools/list, start_session, get_project_state, apply_commands and both commit and
  rollback finishes. It repeats through a second executable attached to the live
  host's authenticated loopback descriptor and checks orderly descriptor cleanup.
- App tests verify MCP origin in durable history, live-gesture folding, normal
  history grouping, no-op baselines, interleaved rollback refusal, disconnect,
  reopened-session fencing, editor permissions, shared selection/timeline/strips,
  library operations, worker saves and the app session-transition fence.
- Lifecycle tests verify original PMV bytes and library content remain unchanged
  after Don’t Save followed by quit compaction; rejected journals are archived;
  active saves refuse close/quit; completion permits them; dirty/open-edit states
  and cancelled AppKit responses cannot silently choose Save or Recover.
- Shell/design regression suite: 15 tests pass. These are headless checks, including
  renderer capture, and do not verify the new native sheets on screen.

The kernel-owned discard-close/durability additions are a prerequisite; no kernel
or core source was edited by the app lane. The app lane's only MCP seam changes
are `transport.rs` (generic connection factory on workers), `views.rs` (shared
snapshot-data digests) and `lib.rs` (export the existing strips planner). No shell
files changed. Shared Cargo.lock reconciliation remains with the integration owner;
the only app dependency addition is pm-mcp.

## Decisions and picking rules

| Decision | Rule |
|---|---|
| Reuse the kernel lane's session, library and state APIs directly | §3.2 single writer; N5 prune/reuse instead of introducing another store |
| Small shell hooks; app owns temporary panel/controller code | Assigned lane ownership; §3.1 composition contract; P3 replaces built-ins |
| Use existing pm-design tokens and solid rows/panels | §3.10, mockup v2 and KISS; glass belongs to control surfaces |
| Direct Metal presenter and native transport layers | P-DEVICE and §3.3; no CPU viewer readback or per-frame GPUI notification |
| Asynchronous kernel calls and AppKit sheets; worker file/media/export IO | §3.2 asynchronous hosts; §5 main-thread budget |
| Isolated explicit native library; never discover a real profile | Explicit task safety rule and §6 beta/library isolation |
| Recovery choices and Don’t Save archive through kernel-owned operations | N4 no data loss; generation fencing; no app-owned file deletion |
| One app actor for stdio, loopback, windowed and windowless access | N7 agent-native; §3.8 MCP; §3.11 headless; §3.2 single writer |
| Immutable MCP actor identity; preserve ordinary gesture folding | Kernel slice 3 identity/restore contracts and §3.2.1; D10 stays off |
| Native cancellable sheets and termination guard | §3.10 native behaviors; Save / Don’t Save / Cancel; main-thread waits stay asynchronous |
| Keep 54 pending tools explicit and leave extension activation to P2 | N3 correctness over imitation; existing pm-mcp implementation boundary |
| Report partial evidence instead of declaring Gate P1 | §7 requires the whole corpus workflow and §5 paired measurements |

## Remaining Gate P1 work and requested APIs

Kernel requests:

1. Broader multi-window document ownership and restore coordination. The app now
   consumes explicit recovery choices, selection, published destination/media,
   worker save handoff, discard-close and durability/quit APIs.

Engine-service requests:

1. A status/frame notification or native display-link feed so the host can stop
   its provisional polling timer while paused, while retaining responsive resize.
2. A documented composition-placement callback contract for output generation
   changes inside the shell's present transaction, including display/EDR changes.

Still open: broader corpus playback/scrub/export coverage, GUI drop/zoom/readout
and keymap edge cases, library media fixtures, journal kill tests,
full-resolution export/visual checks, migration gates, and every §5 paired
latency/playback/A-V/CPU/GPU/energy/idle/memory budget. No release, tag or push.


## Unlocked lifecycle UI verification (2026-11-01)

Disposable `check-lifecycle-ui.sh` close and recovery fixtures ran on the actual
AppKit/GPUI app with isolated `/tmp` profiles. Dirty work was created through the
owned fixture's native MCP connection; the decisions, shortcuts and close buttons
below were operated on screen. No installed app/profile was controlled.

App set A, present and untouched: Finder, Aside, Grok Bot, Notion, Mail,
Claude, T3 Code (Nightly), installed Powermove/helpers, RobloxPlayer,
RobloxMenuBar and figma_agent. Every result below retains this app-presence
caveat; load is the observed one-minute average before → after the case.
Existing Powermove process count changed from seven to five without this lane
closing them. No owned benchmark instance remains. All projects and profiles
were disposable public/synthetic fixtures under `/tmp`.

| Check | Result | Conditions |
|---|---|---|
| Command-W, dirty close sheet | PASS Save / Don’t Save / Cancel shown | A; lifecycle observed load range 5.75–16.67 |
| Escape / cancel focus | PASS sheet cancelled; editor remained dirty; native sheet exposed Cancel as focused | A; same lifecycle load range |
| Command-Q + Command-D | PASS quit sheet, Don’t Save and final process exit | A; discard checkpoint load 16.67; high-load caveat retained |
| Don’t Save durability | PASS original SHA-256 unchanged after quit; discarded journal/checkpoint archive exists | A; same discard checkpoint |
| Return / Save | PASS wrote PMV4 and closed editor | A; same lifecycle load range |
| Saved file reopen | PASS fresh isolated profile opened saved PMV; edited layer present in live app state and screenshot | A; ending load 6.67 |
| Red traffic-light close | PASS native close-button AXPress opened dirty sheet; Don’t Save closed editor | A; ending load 6.67 |
| Recover | PASS sheet choice restored “Recovered work” in the window-bound app state | A; same lifecycle load range |
| Open as Copy | PASS restored recovered layer; original JSON still contains only base layer | A; same lifecycle load range |
| Discard recovery | PASS recovered layer absent; discarded archive exists | A; same lifecycle load range |
| Quit with no editor + connected MCP helper | PASS helper initialized while last editor was closed; native Command-Q exited with that client connected | A; final load 5.75 |
| Save-in-flight refusal/progress note | UNVERIFIED; the tiny fixture saved too quickly for a deterministic busy-window check | A; same lifecycle load range |

Native keyboard shortcuts were sent through System Events to the explicitly
frontmost owned process; direct process-posted modifier events were not used as
passing shortcut evidence. Fresh AX button titles/close subroles were resolved
for each action. Saved reopen used a fresh disposable profile.

Window-only captures outside Git: `/tmp/pm-shots/onscreen-lifecycle-close-sheet.png`,
`onscreen-lifecycle-recovery-sheet.png` and `onscreen-lifecycle-saved-reopen.png`.
No GUI app owned by the user was quit. The owned native/VoiceOver/Quickstart
processes are gone, U.S. input is restored, and the external display's original
mode 150 / 240 Hz is verified. Reference worktrees/node_modules and the shared
release directory are absent; approximately 15 GiB is free. Multi-window ownership,
save-flight feedback, broader shortcut/focus coverage and P1's physical gates
remain open. No product code, push, tag, release or workflow dispatch.

## Save-in-flight close/quit on screen (2026-11-01, 09:53–09:54)

Release `powermove-native` built at `c1dbf3bc9` (09:48), fresh `/tmp` profile and
project per run, GUI lock `onscreen`, one-minute load 4.5 → 9.6. To make the save
long enough to hit, the disposable project had 60,001 shape layers (8.9 MB
JSON); a windowless save of it took 0.914 s. Steps per run:

1. Open the project and wait for the window title (12–13 s).
2. Over the app's own `--powermove-mcp` connection to the same profile: run
   `start_session`, `add_layer "Flight edit"`, `finish_session commit`, then
   `start_session` again.
3. Start `save_project` (worker save handoff) and send Command-W or Command-Q
   through System Events 54–344 ms after the save began.

| Check | Result |
|---|---|
| Command-W during save (save 1.611 s) | **PASS refusal:** the window stayed open, the process stayed alive, the save completed, and the file is PMV4 containing `flight-layer`. |
| Command-Q during save (save 1.209 s) | **PASS refusal:** the process stayed alive through the save. |
| Quit after the save | PASS: Command-Q exited 0 with no dirty sheet. The project lock heartbeat was released (`pid: 0`). |
| Progress/refusal note visible | **FAIL / open.** No “Saving in progress…” (or “A project operation is in progress…”) text appeared in captures 150 ms after the key or 0.8 s after the save. The status bar kept `Opened · Writable`. In the Command-W run, the during-save capture is byte-identical to the pre-save capture, so the window did not repaint at all. Which guard refused is unknown: `CloseState::Saving` and the MCP operation fence both apply while an MCP save runs. |
| MCP edit visible in the editor before saving | **FAIL / open on this fixture.** About 2 s after the committed MCP edit, the window showed neither `Flight edit` nor the edited state, and it still did not after the save. The saved file does contain the layer. A first attempt that edited while the project was still “Opening…” did show `Untitled — Edited`. It is unknown whether a small project refreshes; that check was queued but the Mac locked (~10:17) first. |

Likely area for the open rows: `Controller::refresh` (mirror sync and layer query
over 60k layers on the main thread) and status-bar repaint after
`lifecycle::request_close` sets `message`. This needs instrumentation (timestamp
the `refresh` duration, and `cx.notify` → shell paint) before any fix.

Reproduce: the local untracked driver `/tmp/pm-onscreen/lifecycle/saveflight.py
<binary> <layers> close|quit|timeonly` creates everything under
`/tmp/pm-native-saveflight.*` and writes window captures to
`/tmp/pm-onscreen/lifecycle/` (outside Git). Run it under one GUI lock. Use
`layers=0` for the small-project refresh check.
