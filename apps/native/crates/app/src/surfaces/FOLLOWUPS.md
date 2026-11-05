# Design review follow-ups — 2026-11-01

Scope: `na-cloud-kit2` and the seven `na-cloud-s-*` reviews, on top of their
merged Opus fixes. All visual output remains provisional for Opus review.

| Review | Implemented in the owned surface paths | Acceptance still requiring macOS / another owner |
| --- | --- | --- |
| Kit2 | Native secure-input admission retains exact private bytes under backpressure; explicit worker-only secret boundary; existing `editing::native` supplies NSColorSampler; native pop-up measurement; precise/discrete wheel-unit and momentum publication; native overlay fades from kit visibility deadlines and motion tokens. | Shell must attach native controls, feed wheel phase/scroll facts to NumericWheel and schedule its rest deadline / OverlayVisibility idle deadline. Persistent Keychain worker remains a service-owner integration. Light/dark × Increase Contrast captures and device/momentum checks require macOS. |
| Kit2 Settings | Components already exist on the merged base. | Review items 8–10 require changing `js/v4/panels/settings/**`, outside this lane. Replace its placeholders with Switch/SecureField/ColorWell/PushButton/FormGroup/FormRow and text-run emphasis in that lane. |
| Authority | Request-fenced Reveal completion and exact Reveal/Remove/deletion-failed copy; unreadable Reveal transitions to Needs re-entry; failed Remove retains saved state; safe acknowledgment prompt; native setup factory uses sheet tokens and native secure fields, never a surface value. | Routing and real agent/Store/env/Keychain worker attachment require app manifests, main/session controllers and service crates outside ownership (the authority review’s items 1–4). Native setup factory/input-target attachment is ready for that controller. Check secret visibility, copy-concealed, focus/Tab and Keychain persistence on macOS. |
| Library | Every Answers item: linked size control initially on, empty/loading symbols/AX, creation buttons and Escape gate, real drop-frame arithmetic and exact unsupported-rate error, descending numeric sorts/reversal/popup checks, partial facts, clearing nonmatching duplicate search before select/rename. GAPS removed. Native New Project factory, Library window/collection/list and measured card factory use the new tokens. | Shell binds its existing native delegate/input infrastructure to the controller, supplies system relative dates/predecoded images, mounts EmptyState and the toolbar items from the existing model. Native grid slot/badge geometry remains the DESIGN-TODO below. Test 60k virtualization, selection, type-select, search/rename and create failure on a Mac. |
| Media | Stale sequence banners use The media / The project with their articles; no arbitrary supplied noun. Native image-sequence sheet uses sheet inset/value-column tokens, validation, leading tertiary action and Cancel/default equivalents. | Mount sheet and bind stable native IDs in the media controller; verify owning-window geometry, validation, file-picker return and stale close on macOS. |
| Recovery | Real EmptyState symbol slot (`exclamationmark.triangle`) with warning severity; kit resolves c.warning; requiredSymbol placeholder removed. | Check SF Symbol, single AXGroup and high-priority announcement in mounted native panels. |
| Dialogs | Disabled Leave Attributes label; Time/Value errors; line-less diagnostics; retained pending preview and unavailable copy; empty/unchanged/duplicate preset rename semantics. Explicit default cell for Return and Escape key equivalent for Cancel. | Run the real main-thread probe below; no worker-thread skip counts as key-dispatch evidence. Shell's preset accessory edit notification applies RenameValidation, then republishes the native alert. |
| Documents | DocumentNotice uses the kit Banner with exact severity symbols, viewer/inline placement and generation-qualified action/dismiss callbacks. | Verify material, native symbol sizes/colours, four-second passive lifetime, persistent action banners and AX priority with the shell’s kit host. |
| Editing | Path plans use gizmo_handle/tangent_end/gizmo_keyline instead of keyframe subtraction. PathControls renders the kit ViewerGizmo at exact evaluated display positions, with stable path/vertex/handle IDs, selected/hovered states and only eligible tangents. Kit resolves selected/tangent colours and over-image dark palette. | Viewer host mounts PathControls with its publication and native input queue; verify actual drawing, hit rectangles, transforms/zoom and light/dark × Increase Contrast captures on macOS. |

## Remaining design placeholder

`DESIGN-TODO(opus): specify the Library thumbnail slot aspect / row height and
badge insets for unknown or mixed-aspect projects.` The native collection does
not invent square/16:9 slots. It leaves a neutral empty collection until the
host supplies the approved measured item size. `grid_card` requires a measured
thumbnail size; the duration badge is omitted unless its measured frame is
supplied. All colours, radii, type and border recipes are already specified.

## Decisions and rules

- N1/N3: use the approved answers, exact copy and actual host tokens, including
  SMPTE dropped-label rejection. No Chromium/Electron quirk was copied.
- N3: retain pending expression output; do not invent source line numbers,
  missing facts, zero dimensions or a thumbnail aspect ratio.
- N4/N6 and prune: no existing consumers, projects, opaque authored values,
  settings or extension APIs are removed. Presets still archive before removal;
  credential changes remain explicit worker operations.
- N7: native controls carry stable identifiers, actions retain target generation,
  and the Linux harness compiles the actual production surface modules.
- N8: controls do no IO/JS; secure input uses try_send and returns the private
  edit on full/disconnected queues. Device conversion, menu measurement and
  overlay animation run natively, without waiting for JavaScript.
- Ownership: Settings replacement, shell routing and service implementations
  are handed off explicitly rather than editing other lanes’ files. No app
  dependency or production lockfile change was needed.

## Reproduce verification

From the repository root after the requested activation:

```sh
mkdir -p work
rustc --edition=2024 apps/native/crates/app/src/surfaces/followups_check.rs -o work/followups-check
work/followups-check prepare
CARGO_TARGET_DIR="$PWD/apps/native/target" work/followups-check test
CARGO_TARGET_DIR="$PWD/apps/native/target" work/followups-check clippy
work/followups-check fmt
cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-surface
cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-surface --all-targets -- -D warnings
cargo fmt --manifest-path apps/native/Cargo.toml -p pm-surface --check
bun test --cwd apps/native/js/v4
bun run --cwd apps/native/js/v4 typecheck
```

The app binary unconditionally links GPUI/Metal/AppKit, so the Linux runner uses
an isolated scratch manifest over the real sources. Preparation copies the native
lockfile and verifies that all resulting dependency pins are already in it;
only the disposable harness root is added. Every test/check is locked. There are
no spike/vendor fixtures, system temporary-path ownership assumptions or external
media-tool dependencies. Rust is 1.96.1; no new workspace crate was added.

On a real Mac, run `CARGO_TARGET_DIR="$PWD/apps/native/target" work/followups-check macos`.
This is a harness-free main-thread executable: it must really dispatch Escape
and Return to default Cancel and inspect native setup/New Project/sequence/Library
controls and system menu measurement. Off macOS the driver refuses this command;
it does not report a skipped probe as a pass. `work/followups-check check-macos`
checks all target source, including this probe, for Apple Silicon. Linux cross
checking additionally needs Apple-capable clang/llvm-ar for existing sha2-asm;
it is source/type/Clippy evidence, not a GUI or runtime result.
