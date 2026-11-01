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

## Recovery and safety limits

Every nonempty recovery result produces an AppKit sheet with Recover, Open as
copy and Discard. A base-mismatch recovery copy can open from its kernel library
slot; Discard moves that unopened copy to kernel trash. An adopted recovery can
be kept or saved to a new path. **Discard of already-adopted recovery is blocked
with a visible explanation and retains all data.** The kernel currently adopts
before returning and `ShellCall::Close` saves, so closing cannot safely emulate
Discard. Unrecoverable data is explicitly reported and kept.

Window close/quit currently retain journals rather than completing the full
kernel close/compaction workflow. Multi-window document ownership, clean
termination barriers, Finder/Dock open events and restoration remain incomplete.
Temporary layer selection is app-local because the actor has no selection
mutation call yet; it is not presented as complete kernel selection integration.
Work-area navigation currently uses composition boundaries.

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

## Decisions and picking rules

| Decision | Rule |
|---|---|
| Reuse the kernel lane's session, library and state APIs directly | §3.2 single writer; N5 prune/reuse instead of introducing another store |
| Small shell hooks; app owns temporary panel/controller code | Assigned lane ownership; §3.1 composition contract; P3 replaces built-ins |
| Use existing pm-design tokens and solid rows/panels | §3.10, mockup v2 and KISS; glass belongs to control surfaces |
| Direct Metal presenter and native transport layers | P-DEVICE and §3.3; no CPU viewer readback or per-frame GPUI notification |
| Asynchronous kernel calls and AppKit sheets; worker file/media/export IO | §3.2 asynchronous hosts; §5 main-thread budget |
| Isolated explicit native library; never discover a real profile | Explicit task safety rule and §6 beta/library isolation |
| Keep recovery data when no safe decision API exists | N4 no data loss; never silently discard acknowledged work |
| Report partial evidence instead of declaring Gate P1 | §7 requires the whole corpus workflow and §5 paired measurements |

## Remaining Gate P1 work and requested APIs

Kernel requests:

1. A generation-bound recovery-decision operation (Recover / OpenAsCopy /
   Discard), ideally before adopting/publishing recovered content. Discard
   needs to reopen the unchanged base and retire only the explicitly rejected
   journal, without the save side effect of Close.
2. Actor selection mutations that publish the resulting selection summary.
3. A quit/close coordinator or non-saving close/defer primitive, plus explicit
   completion of durability/compaction before application termination.
4. Expose session destination and resolved media metadata/catalog with open
   results, avoiding duplicate file reads/cache extraction in the host.

Engine-service requests:

1. A status/frame notification or native display-link feed so the host can stop
   its provisional polling timer while paused, while retaining responsive resize.
2. A documented composition-placement callback contract for output generation
   changes inside the shell's present transaction, including display/EDR changes.

Still open: broader corpus playback/scrub/export coverage, GUI drop/zoom/readout
and keymap edge cases, recovery Discard, library media fixtures, journal kill tests,
full-resolution export/visual checks, migration gates, and every §5 paired
latency/playback/A-V/CPU/GPU/energy/idle/memory budget. No release, tag or push.
