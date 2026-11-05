# Cloud inspector integration

## 2026-11-05 — owned panel wiring

Branch `feat/na-cloud-inspector`, behavior reference
`c7f46a05641b23944d3692c9d80f9677c7e1ef1d`. Linux only; native presentation
remains provisional until macOS verification and Opus review.

The Properties header now routes its menu through the existing asynchronous
`effectMenu` shell service, then submits the chosen definition to the real
kernel actor. Row menus similarly use the existing `contextMenu` service.
Cancelled menus, retired document/selection leases and disabled actions do
not edit. Effect Controls native parameter fields now write through their
registered binding and current model projection; formerly they fell through
the Properties-only binding route. Effect diamonds use the published playhead
rather than time zero. Registry refresh discards stale presentation metadata
without deleting authored effects. Effect field handles include document identity.

Properties publishes existing Weight, Align, independent-corner, parenting,
matte-source, 3D, motion-blur, Skew and Scale-link controls where applicable.
Independent Corners seeds all corners from the current radius atomically.
Settings publishes the specified Color Depth choices and submits the existing
project-root command. Invalid depths refuse; Undo restores the previous tree.

Decisions and selecting rules:

- Kernel receipts remain the success boundary; no optimistic document writes
  or synchronous JS on the UI thread (N3/N7/N8).
- Menu results retain their originating selection/document, and effect bindings
  require registration plus current applicability and editability (N3/N4).
- Preserve absent/unknown fields and missing effect definitions; retire only
  presentation metadata whose registry consumer is gone (N4/N6, prune rule).
- Use the approved Settings/Properties specs and existing menu host operations;
  no new look, copy or gesture is inferred (N1, Opus design ownership).

`DESIGN-TODO(opus)`: blend/matte menu descriptors and the previously recorded
structured/contributed/variable-font-axis controls remain placeholders. Existing
font, fill, point-pad and expression native services still need their shell
handlers; the shared kit's missing recipes remain visible.

Remaining application blockers outside this lane's ownership:
`app/src/builtins.rs` still leaves Project import/relink/download/preview effects
unhandled and does not handle Inspector font/fill/point/expression services.
`app/src/viewer2.rs` and the viewer-tools/kernel transform host must connect
Anchor Point press/drag with evaluated world/local transforms, rollback and
pointer cancellation. The shell pasteboard route must arbitrate text/local
clipboard/Finder/image ownership and dispatch durable import/placement.
These are not closed by model tests or Linux execution.

Verification in progress; final counts and merge base will be appended.
