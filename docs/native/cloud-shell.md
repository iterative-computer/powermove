# Cloud shell integration

## 2026-11-05 — production docking and portable shell verification

Reference: main `c7f46a05641b23944d3692c9d80f9677c7e1ef1d`.
Read HANDOFF item 6, int1/int2-fixes, surfaces and the shell/document design
specifications before these changes. This note records this lane only; no
strict-acceptance total or Mac surface acceptance is promoted.

U01 now connects the existing app's GPUI drag handlers to stack moves (empty
header) and single-panel moves (tab). The actual layout API resolves header
and middle-body grouping, 30% top/bottom bands, Viewer non-grouping in both
directions, source suppression, original-layout destination indices and atomic
refusal. It retains stack order/active tab and preserves the remaining selected
tab when a preceding tab moves. Keyboard dock submenus resolve stable panel IDs.
Overflow retains the active tab, folds trailing tabs into More Panels, and uses
`metrics.hit` for minimum segment width. Maximize follows the group across tab
switches, has a Restore header control and Window menu commands, and ends on
layout changes. Bottom dock floor comes from header/ruler/track-row tokens.

U10 receives real recursive NSMenu submenus, Show All, Delete, and one New Window
entry. AppKit remains responsible for menu tracking, type-select and text services.
Existing native text/panel consumers remain. This is partial U01/U10 work, not a
closed row: physical gestures, AX, focus, motion and Opus review remain mandatory.

The shell's portable modules are now its production library on Linux as well as
Mac. AppKit/Metal/GPUI dependencies and capture targets remain macOS-only; they
are not replaced with simulated passing captures. Linux runs the real layout,
keymap, menu, session and theme code, with synthetic fixtures. No workspace
member, dependency version or lockfile change is needed for target gating.

Decisions: N3 selects main's authored docking behavior and Opus's native anatomy;
N1/N2 retain bounded geometry and accessible keyboard alternatives; N4 keeps
historical Viewer stacks loadable while rejecting new invalid grouping; N7 uses
panel IDs for commands; N8 keeps this path Rust-only. The prune rule retains all
existing APIs and Mac presentation consumers. No new design placeholder or
DESIGN-TODO(opus) is introduced.

Validation: `cargo test --locked --manifest-path apps/native/Cargo.toml -p pm-shell`
passes 39 tests (13 new docking/menu cases plus 26 existing portable tests).
`cargo clippy --locked --manifest-path apps/native/Cargo.toml -p pm-shell --all-targets -- -D warnings`
and `cargo fmt --manifest-path apps/native/Cargo.toml -p pm-shell --check` pass.
The Apple-target shell check is BLOCKED in the existing gpui_ce_media build
script: neither SDKROOT nor xcrun can locate a macOS SDK. Consequently the changed
GPUI/AppKit code still requires Mac build, tests and runtime verification; Linux
success is not evidence for those assertions.
