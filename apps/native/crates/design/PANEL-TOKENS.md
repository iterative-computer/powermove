# Panel token additions — 2026-11-01

These append the values in docs/native/design/panels/README.md §6 without
changing existing tokens or rewriting the shared generated table. Their role
and values are checked by tests/panels.rs; the existing serialized token
snapshot includes them in every appearance.

| Token | Value | Role |
|---|---|---|
| metrics.kind_badge | 18 | Timeline/Project kind badge square |
| metrics.thumbnail_min | 120 | Minimum Project thumbnail width |
| metrics.status_dot | 6 | Open-composition/attention/insertion dot |
| colors.media_badge | #000000 | Over-media capsule RGB in both schemes |
| metrics.media_badge_opacity | 0.55 | Over-media capsule alpha in both schemes |

The existing colour patch contract is RGB. Alpha is a separate metric so the
new colour does not change the representation of existing theme values. The
native card recipe combines these. UI-kit-2 should reuse these entries if its
parallel work adds the same names; existing values are unchanged.
