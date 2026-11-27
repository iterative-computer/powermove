# Native shell (Phase 1, package 4)

The shell lives in three workspace crates:

- `pm-design`: serializable semantic colors, light/dark and Increase Contrast variants, type/optical-size metadata, spacing, radii, material policy and motion values. Views consume these tokens. System control geometry stays AppKit-owned.
- `pm-shell`: workspace model/persistence, ordered key resolver, frozen command/menu tables, GPUI editor/Settings shells, AppKit toolbar and below-band viewer slot, and two pooled native search windows.
- `pm-native-app`: `powermove-native`, the runnable entry point.

The binary sets a macOS 14 minimum at link time.

The shell does not depend on or modify the kernel, core, engine, spikes or vendor crates. `run_with_device` accepts the app/engine's Metal device and uses that same instance for GPUI and the viewer placeholder. Standalone `run` chooses the device once.

## Build and test

Run from the native workspace so Rust **1.96.1** is selected:

```sh
source engine/env.sh
export CARGO_TARGET_DIR="$HOME/Developer/pm-target"
cd apps/native
cargo build -p pm-native-app
cargo test -p pm-design -p pm-shell
cargo clippy -p pm-native-app -p pm-design -p pm-shell --all-targets -- -D warnings
cargo run -p pm-native-app --bin powermove-native -- --smoke
cargo run -p pm-native-app --bin powermove-native
```

Check disk first; this lane stops below 6 GB. Do not create another target or use `cargo clean`. On a shared desktop acquire the GUI lock before the last command. `PM_NATIVE_APPEARANCE=light|dark` overrides system appearance for inspection. `PM_NATIVE_SHELL_STATE` can point to an isolated layout JSON outside the repository.

## Implemented

### Workspace and panels

Default has a 300-point Project / Effect Controls tab group on the left, Viewer in the center, a 344-point Timeline beneath both, and a full-height 300-point right dock with flexible Properties above 268-point Effects & Presets. All bodies show their stable panel IDs. The viewer has a persistent, below-GPUI, RGBA16F/EDR-enabled CAMetalLayer placeholder; no media is presented and no color-accuracy claim is made.

Gradient, Animate, Shader Lab, Edit, Review and Focus retain the frozen tag's dock membership and sizes. Tabs select on click. Drag a panel tab to another header to join its tab group, or to the upper/lower body to insert a stacked group. Dragging retains the viewer host. Escape cancels the current placement/resize. Splitters drag, and after clicking a splitter, arrow keys resize by the token's keyboard step; Enter collapses a whole dock. Toolbar toggles reopen the side docks.

The layout model validates versions, unique panel/dock IDs, active tabs and finite dimensions. Moves validate a copy before replacing the current layout. Geometry scales fixed regions down in small windows rather than overlapping panels. Side docks remain full-height when the bottom dock exists.

Layouts are saved per project and workspace, with the active workspace remembered. The demo uses `shell-untitled`; `Editor::bind_project` is the integration seam for a real project identity. The default store is under the separate `Powermove Native/Shell` Application Support directory. Writes sync a temporary file and atomically rename it; a malformed or newer-version store is reported and never overwritten. Before saving, the shell reloads the latest store to preserve other windows' project/workspace records. No Electron profile or project data is touched.

### AppKit chrome and commands

The editor uses NSToolbar, system traffic lights/fullscreen, a common native tabbing identifier, native frame autosave/restoration flags, and `setDocumentEdited` with an Edited subtitle. The toolbar has Sidebar, document title, a centered eight-tool group, Agent, orange prominent Export and Inspector. There is no Settings gear. The app menu opens one Settings window with the shipped Project, General, Accounts, Transcription and Extensions section shells.

The native menu model includes every production item from the frozen `menu.ts` and all **35 W1 equivalents**, including menu-only window/tab actions and unaccelerated image-sequence/folder imports. About, Services, Hide, Quit, full screen, minimize, zoom and bring-to-front use AppKit selectors. Help adds the native searchable Help menu. Workspace selection and shell visibility/search commands are additional shell affordances. Development-only Electron reload/devtools are intentionally absent.

Replay is scheduled on the next native run-loop turn, outside shell borrows. Its completion is acknowledged before later queued keys resume; composition hit testing reads a separate immutable geometry cell and does not enter the shell host.

All **195 W1 registry bindings** carry their original IDs, chord, command, arguments, priority, field, repeat and loose-match flags. Tests compare each cell with `w1-keymap.md`. Native events pass through a local pre-dispatch event monitor; consumed events do not reach menu equivalents. Declined events replay once to AppKit with a recursion guard. Native text responders receive text defaults after field-aware bindings decline; focused buttons keep Enter/Space. WebKit first responders retain their DOM-first route. S5 must explicitly forward unhandled WebKit keys.

Never-declines is limited to implemented shell commands. Other commands, including unknown and extension commands, are may-decline. Decisions retain a token without a deadline; later keyDown/keyUp events queue behind it. Escape and Command-period use independent resolution lanes with the same extension priority order; Command-Q bypasses the queue. Held-key filtering and shifted digit/bracket/slash/function-key handling follow W1. Editor actions currently log and decline. This is shell routing, not editing/playback parity.

### Materials and motion

The work surfaces are solid, with neutral grays, orange `#FF6019`, 12-point body type (11-point minimum), 12-point panel corners and 6-point gaps. GPUI uses `.SystemUIFont`, which the vendored macOS text system resolves with automatic optical sizing; AppKit text uses the system font at the token's size.

Palette and field menu are separate pre-created NSPanel child windows, with native search fields, filtered inset rows, a selection glider, pointer choice, wrapping arrows, Home/End, Enter, Escape/Tab, outside-click dismissal and responder restoration. Only one is open at a time. Native glass uses its supported `contentView`; pre-26 uses NSVisualEffectView. Reduce Transparency replaces the material with a solid NSBox, and Increase Contrast strengthens GPUI separators and material borders. Accessibility display-option notifications rebuild transient materials and GPUI tokens. System window-appearance notifications update the editor unless an explicit appearance override is active.

Entry uses the **0.32-second / 0.15-bounce** public CASpringAnimation initializer and scale **0.96 → 1**. Opacity exit is **0.12 seconds**; Reduce Motion uses a **0.15-second** fade. A presentation-layer value and a sampled damped-oscillator velocity seed interrupted spring entries. No per-frame JS or idle polling timer is introduced.

### Command routing

`Controller::command` asks `layer_commands::route` first. Layer commands plan off
the main thread and run as one Undo step; Timeline commands use the model's own
route; Cut, Copy and Paste use an app-local clipboard and try a focused text field
first. `layer_commands::disabled` recomputes on every publication and feeds
`MenuState.disabled`: menu rows and toolbar tools dim, the palette drops them, and
`Editor::command` declines them so their key reaches the focused control.
`UNAVAILABLE` lists rows with no working path yet, and
`every_keymap_and_menu_command_has_an_owner` keeps the tables complete.

Playback publishes to panels only at its start and end (`PanelClock`). In between
the playhead and timecode are CALayers, so no GPUI view repaints.

## Decisions and their picking rules

| Choice | Picking rule |
|---|---|
| Default uses tabs and 344/268 heights although the tag has stacked left panels and 340/380 heights. | Explicit package brief and approved design direction supersede the older preset. Other presets preserve the tag's behavior (N4/N6). |
| Work surfaces stay solid; materials occur only in native toolbar/search windows. | N9 / controls-above-content material rule. |
| Semantic serializable tokens, including all shell dimensions and motion. | One design module; shell chrome must remain translatable under T1. |
| Keep panel IDs and a retained below-band viewer host. | N7 inspection; composition contract and viewer ownership. |
| Resolve unknown commands conservatively and preserve ordered key replay. | Keyboard may-decline contract; never guess editor eligibility. |
| Separate layout store with validated atomic replacement. | N4 no data loss; shell cannot write the kernel's document/history. |
| Keep Electron consumers and all P0 sources intact. | N5 removes code only after its last consumer and replacement gate pass. |

## Evidence and remaining work

The pinned-toolchain build, Clippy with warnings denied, and 15 model/token/Metal capture tests pass. The hidden `--smoke` run also passes: AppKit builds all six toolbar items, both pooled child windows, the edited state and the RGBA16F viewer layer with activation prohibited and all windows invisible. This validates initialization, not native appearance or event ordering. Renderer tests paint the same `Editor` GPUI tree at Retina scale in four modes, assert actual interior/border pixels and save:

- `/tmp/pm-shots/shell-headless-light.png`
- `/tmp/pm-shots/shell-headless-dark.png`
- `/tmp/pm-shots/shell-headless-reduce-transparency.png`
- `/tmp/pm-shots/shell-headless-increase-contrast.png`

The headless toolbar row is a solid textual capture surrogate. These are **renderer captures, not AppKit screenshots**: native tabs/fullscreen/restoration, complete slot input/accessibility and measured animation continuity still need broader GUI coverage. The solid GPUI dark and Reduce Transparency captures should be identical. Capture tests do not claim native material pixels.

An unlocked GUI-lock-owned smoke pass also produced native window screenshots:

- `/tmp/pm-shots/shell-native-dark.png`
- `/tmp/pm-shots/shell-native-light.png`
- `/tmp/pm-shots/shell-native-palette.png`
- `/tmp/pm-shots/shell-native-field-menu.png`

The native toolbar layout and orange prominent Export were inspected in light/dark. Command-K opened the pooled palette; search and Return switched Focus/Default workspaces. The field menu opened through View, and Down/Enter chose Scale. Settings opened and closed with Command-comma. Command-Q quit. These sequences were rerun after fixing native replay/hit-test re-entry. The app was closed and the GUI lock released. This is interaction smoke evidence, not a full W1/composition/performance gate.

Remaining integration/gates:

- Wire command eligibility/completion, project titles/URLs/dirty state, project ownership, save/close safeguards, recents, Dock behavior and full document-session restoration to the kernel/app lifecycle. Shell frame restoration and native tab readiness do not implement W001/W002 end to end.
- Replace logged editor/field commands and labeled bodies with the P3 built-ins; connect the S3 viewer presenter and explicit extended-linear color space. No engine frames, export, or playback are present.
- Complete native drag-to-new-edge-dock/drag previews, keyboard focus traversal of individual splitters, arbitrary theme selector/state translation and workspace authoring/import UI. Current drag targets join/stack within existing docks.
- Connect trusted-runtime watchdog notices using JS execution time (2/8 seconds) and failure responses; the shell resolver itself has no wall-clock expiry.
- Validate real native menu/text/key ordering, search interactions, motion interruption and live accessibility/appearance changes. No performance or full W1 parity gate is inferred from model tests.
- Interleaved foreign-child accessibility, transformed/clipped slot input and resize lag remain dependent on the open S2 composition gate. The shell publishes its retained native viewer to the existing composition accessibility hook; a nested GPUI/foreign-child reading-order walkthrough still needs verification. No vendor patch was made. If the direct-root-only accessibility splice cannot interleave the viewer at its panel position, P-A11Y needs nested foreign-child insertion support in the vendor lane.

The shared Cargo.lock already has changes from other lanes. This package commits only its three crate directories and this document; workspace lock reconciliation belongs to the integration lane. Nothing is pushed or released.

## P-TEXT integration (2026-11-01)

`pm_shell::text::TextField::new(kind, label, initial, tokens, window, cx)` provides
primary names, multiline captions/composer text, and dense inspector text/numbers
using `gpui_macos::native_text::TextKind`. Keep the entity alive for the visible
field; drop it when its panel is removed. All kinds reserve a fixed GPUI rectangle.
Dense fields draw their resting value in GPUI, then place a borderless AppKit field
in that rectangle on click. Return/blur commits; Escape restores the old value.
Invalid/non-finite numbers restore the previous value only at commit, so typing
and IME composition are not filtered. `TextEvent` exposes changes/commits/cancel.

A host can insert `field.accessibility_item()` in its committed slot reading order.
Without an explicit anchor, live text slots append to the foreign-child order.
Native placement inherits rectangular clipping; arbitrary affine transforms and
arbitrary shape clips are still open S2 work. Do not cache/remove/hide an owning
panel without updating or dropping its native text entities: staged native geometry
is retained until republished, as with the host's other composition slots.

Text editing uses the system field editor / `NSTextView`, preserving native marked
text, selection, undo and responder-chain services. The shell's key resolver and
pooled menu navigation let marked-text events go straight to AppKit. Native Edit
menu items expose spelling, Writing Tools and Emoji & Symbols. Dictation uses the
system text-input command/shortcut; its microphone/account availability is not
asserted by the screen-free tests.

The pinned build, 24 Rust tests (including real Metal captures and cache replay),
and AppKit text/AX probe pass. This run's captures use
`/tmp/pm-shots/ptext-shell-headless-*.png`. Real Japanese/Pinyin conversion,
Services, dictation, Writing Tools, emoji, keyboard traversal and the visible dense
swap remain unverified after the session locked. Exact runnable checks and the
30-second manual IME check are in [S2](s2-gpui-ce.md).

## Shell 2 (2026-11-01, lane `feat/na-shell2`)

**Docking.** Drag any panel tab: the shell shows an accent-tinted drop preview and a
ghost label. Targets (`Layout::drop_target`): a header joins that tab group at the
pointer's slot (same group = reorder); a body's upper/lower half inserts a group
before/after; a 16-point zone at the left/right/bottom edge docks there, creating the
dock if it does not exist and reopening it if collapsed. Escape restores the layout
from before the drag. Moves are validated on a copy (`move_panel_to`). Tabs have a
hover close button (not the viewer), a right-click menu (Close, Dock Left/Right/Bottom,
Save Layout…, Reset Workspace) and a header **+** menu that adds a closed panel to that
group. Headers follow the mockup: sunken segmented strip for tab groups, semibold title
for single panels.

**Saved layouts.** Window ▸ Layouts lists named layouts (shared by every project),
Save Layout… (an NSAlert sheet with a name field), Delete Layout ▸ and Reset Workspace.
A saved layout replaces the current workspace's arrangement. The store is re-read
before every write so other windows' records survive; old stores without `saved` load.

**Windows, tabs and restoration.** `session.json` (beside the layout store; the app
puts both in its profile) records each editor window's id, frame, project, workspace,
native tab-group leader, full-screen state and the key window. At launch the shell
reopens all windows when the system keeps windows on quit (`NSQuitAlwaysKeepsWindows`),
otherwise only the key window's arrangement (`PM_NATIVE_RESTORE=all|one` overrides).
Frames are kept on screen; tabs rejoin their leader. Closing a window removes it;
quitting keeps all. AppKit restoration and the single frame-autosave name are off (one
autosave name cannot serve several windows). New windows cascade from the key window;
Window ▸ New Tab opens a native tab. Document restoration remains the app's job.

**Menus.** New Layer and Timeline menus and View ▸ Tools carry the old app's W1
registry chords as **display-only** key equivalents: an NSMenuDelegate returns NO from
`menuHasKeyEquivalent:…`, so the ordered resolver stays the only key path and a
declined bare key (`v`, Space, `B`) reaches the focused text field instead of a menu
(rule: old app wins on shortcuts/behaviour, HIG on placement). File ▸ Projects… (⌘P),
Edit ▸ Deselect All (⇧⌘A), View ▸ Command Palette… (⌘K) and Agent (⇧⌘K) are ordinary
equivalents with the same replay semantics as the 35 Electron accelerators. Window gains
Show in Workspace ▸ (panel toggles with checkmarks), Workspaces ▸ (checkmark), Layouts ▸,
New Tab, Merge All Windows, Show Tab Bar and Show All Tabs. Sidebar/Inspector items read
Show/Hide per state; tool items show a checkmark.

**Toolbar.** Title is semibold per the mockup; Sidebar and Inspector are on/off glass
buttons that reflect dock visibility; the tool group follows keyboard/menu/palette tool
commands.

**Palette.** Ranks prefix > word start > substring > subsequence (`gtst` → Go to Start);
right-aligned shortcut glyphs; dynamic Show/Hide panel, saved-layout, Save Layout… and
Reset Workspace rows. A key palette panel now routes commands to its editor window.

**Materials.** Unchanged rule: work surfaces and drop previews stay solid tints; glass
only on the toolbar controls, palette/field menu (NSGlassEffectView `contentView`, or
NSVisualEffectView before macOS 26, solid under Reduce Transparency).

**Evidence.** 44 Rust tests across pm-shell, pm-design and the S2 spike, including 11
Shell 2 model tests and a headless capture of the drag preview and drop. On-screen
before/after screenshots (temp profiles) are in `/tmp/pm-shell2/`. Known gap: windows
restore as one native tab group (verified: two windows, one hidden as a tab), but the
native **tab bar did not appear** with this window style (unified-compact toolbar,
transparent titlebar); switching tabs by Show Next Tab was not tested. Needs a fix in the
window chrome or an explicit tab-bar accessory.


## Native tab-bar vendor fix (2026-11-01, P-120 lane)

The vendor `GPUIWindow` override of `addTitlebarAccessoryViewController:` hid
and reduced every native accessory to zero height, assuming an application-owned
tab strip. That override is removed: AppKit now owns its native tab-bar accessory.
The S2 `--verify-tab-bar` regression probe creates one GPUI window with a transparent
titlebar and a unified-compact NSToolbar, invokes the same `toggleTabBar:` selector
as Window ▸ Show Tab Bar, checks both AppKit's visibility flag and a non-hidden,
positive-height accessory, then hides the bar again and quits. No replacement
accessory or visual design change is introduced. See the P-120 validation in S2.

## Shell keyboard access and AX inspection

Tab / Shift-Tab walk the native toolbar, then each visible panel in visual
(top-to-bottom, left-to-right) order, then splitters and the status workspace
pop-up. Each panel is a focus group: its container, tabs/close buttons and header
controls precede its content. Control-Tab / Control-Shift-Tab (the design-language
keys) and Control-F6 / Control-Shift-F6 cycle panel groups; collapsed/closed panels
are skipped. In the native tool group use the arrow keys to select a tool.
Enter/Space activate a focused control. Splitter arrows resize; Enter collapses
a dock splitter. GPUI focus uses the existing `colors.focus` ring, AppKit controls
the system ring.

Save Workspace cycles Name → Cancel → Save → Name, including reverse traversal.
Return saves and Escape cancels. The existing palette row is labelled Save Layout…
(it opens Save Workspace; visible copy is unchanged in the accessibility lane).
Command-K opens the palette at Search; Tab moves
through filtered rows and returns to Search, Shift-Tab reverses, arrows select,
Return executes, Escape dismisses and restores the previous responder. Marked
text continues through AppKit's input-method path.

Inspect the **disposable test app only** (no production profile or project):

```sh
mkdir -p /tmp/pm-a11y
swiftc apps/native/scripts/shell-ax.swift -o /tmp/pm-a11y/shell-ax
/tmp/pm-a11y/shell-ax <test-pid> --main
/tmp/pm-a11y/shell-ax <test-pid> --sheet --expect-focus Cancel
/tmp/pm-a11y/shell-ax <test-pid> --palette --expect-focus Search
# Starts/quits its own temporary apps and owns the GUI lock:
python3 apps/native/scripts/check-shell-startup.py --binary <dev-bundle-binary> --runs 3
```

The external AXUIElement probe dumps actual roles, labels, values, focus and
selected states, excludes the system menu bar/Recent Items, and returns nonzero
for assertion failures. `--main` assumes the default workspace with no project;
`--sheet` and `--palette` inspect those open surfaces. Accessibility permission
is required; absence returns exit 2 without prompting or changing settings.
These assertions verify AX exposure, independently of a spoken VoiceOver pass.
Keep raw dumps/screenshots outside Git under `/tmp/pm-a11y/`. Screen verification
must hold `/tmp/pm-shots/gui.lock` and first check `/tmp/pm-shots/is_locked.sh`.
