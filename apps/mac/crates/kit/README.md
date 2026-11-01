# Powermove component kit

`pm-kit` draws editor controls with GPUI 0.3.8. It does not own a document,
transport, or undo history. Native window chrome belongs to `pm-glass`.

Call `pm_kit::init(cx)` once when starting the application to install text-input
key bindings. Create retained controls with `cx.new`, and add their entities to
panels. Give every control its visible row label with `.label(...)`.

```rust,ignore
let opacity = cx.new(|cx| {
    NumField::new(100.0, window, cx)
        .label("Opacity")
        .range(0.0, 100.0)
        .unit("%")
        .on_edit(move |event, cx| {
            // Begin: start a document preview transaction.
            // Change(value): dispatch the live value without adding history.
            // Commit(value): finish one undo step.
            // Cancel(before): cancel the transaction and restore its snapshot.
        })
});
```

All value controls expose `.on_change` and `.on_commit` separately, plus the
complete `.on_edit` stream. Cancellation also calls `.on_change` with the
restored value for local-state consumers. Document consumers should use the
phase stream instead of treating every change as an independent command.
`.on_begin` can refuse a gesture when a document transaction is already busy.
`binding::CommandBinding` adapts these hooks to a typed command sink, with the
legacy default `interface` origin and optional preparation callback.

Use `.set_value(value, cx)` to refresh an idle control from the document.
An active gesture owns its local draft. For animated controls, `.live(getter)`
reads the current evaluated value when an edit starts. `logic::PlaybackReadout`
provides the old 15 Hz playback sampling rule; the host owns transport ticks.

Controls include numbers and visual sliders/dials, native single-line text,
segmented/checkbox toggles, typed selects, segmented choices, searchable font
lists, color and fill pickers, gradient ramps, and point pads with exact axis
fields. Layout components include Row, Section, Disclosure, BackTitle,
ActionButton, keyframe actions, menu items, and tooltips.

Numbers support arithmetic and relative operators, units, precision, mixed
values, modifier scrubs, horizontal wheel gestures, and Escape rollback.
Typed values deliberately bypass scrub bounds. Point steps are independently
anchored to each range minimum. Linked-scale math captures the starting ratio
for the entire gesture, including changes through zero.

The picker retains Display P3 values in its draft and serializes clipped sRGB
hex on commit, including alpha only below opaque. Fill modes are Solid, Linear,
Radial, and None; gradients retain stable stop identifiers and up to eight stops.
Supply `.on_sample` from the AppKit bridge to enable system screen sampling.
Font loading and font-family inventories also belong to the host.

`Appearance::System` resolves the current window appearance. Explicit light and
dark variants support previews. Dark surfaces are neutral grey. Regular, small,
and mini controls use the system font and concentric radii. `native::reduce_motion`
reads the macOS setting; `motion` provides reduced-motion-aware page parameters.

The `gallery` example shows Live, Mixed, and Disabled controls and all sizes.
Its theme button toggles light/dark; `--light` starts with the light theme.

From the macOS workspace, use the shared target directory:

```sh
CARGO_TARGET_DIR=~/Developer/pm-target ~/.cargo/bin/cargo build -p pm-kit --examples
CARGO_TARGET_DIR=~/Developer/pm-target ~/.cargo/bin/cargo test -p pm-kit
```

The kit supplies control behavior and transaction boundaries. Hosts still own
property-to-command mapping, actual keyframe edits, document undo/redo, transport
invalidation, font loading, and asynchronous system screen sampling. Native IME
and VoiceOver behavior require hands-on validation in addition to the test
input handler's UTF-16 composition checks. Animated roll text, sidebar glide,
and global tooltip warm handoff are not implemented by the current primitives.
