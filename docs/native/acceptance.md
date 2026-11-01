# W1 native acceptance harness

`apps/native/crates/acceptance` (`pm-acceptance`) consumes the authoritative
W1 workflow/keymap Markdown and e2e-triage CSV at compile time. The catalog retains
83 groups, 1,344 rebased A/B variants, 195 bindings and all 396 triage rows
(including the frozen 370-spec roster). Post-tag appendix claims stay separate.

Decisions: inventory membership is not execution evidence (N1/N3); no binding is
marked passed by a replacement dispatcher or a fake shell (N3/N6); all original
requirements and source provenance remain addressable (N4/N7). No Electron
quirk is reproduced and no existing code is pruned.

Runner and first measured coverage are being added in the next lane commit.
