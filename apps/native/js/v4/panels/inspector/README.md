# Properties built-in v4 panel

Design source: `docs/native/design/panels/inspector.md` and the shared panel
README, including its owner decisions. This directory is the Wave 4 inspector
lane. No existing consumer is removed. Visual output is provisional until Opus
review; Linux validation does not establish AppKit rendering or VoiceOver behavior.

## Integration boundary

The integration base has no `pm-panel-inspector` crate and no inspector model
contract under `docs/native/panels/`. The v4 host service exposes immutable
`project`, `selection`, `time`, and `evaluated` projections but deliberately does
not prescribe their application schemas. The application KernelData adapter is
also pending (`docs/native/v4-api.md`). A panel-owned adapter interface will
connect these projections and model-produced commands without inventing a kernel
command namespace or storing a second editable document in JS.

`cargo test --locked -p pm-panel-inspector` currently fails: package not found.
Changes to surface, UI-kit exports, shell activation, or kernel are outside this
lane. These dependencies must remain explicit integration gates.

## Rule-selected decisions

| Decision | Rule |
|---|---|
| Consume UI-kit components and pm-design token references; use neutral omissions for missing recipes | Design ownership, N1 |
| Keep model projection/command translation an injected boundary until the actual model exists | N3, single-writer kernel architecture |
| Register acknowledged native binding handles before enabling controls | N3/N8 |
| Retain all v1–3 consumers and existing example panel code | N4/N6, prune rule |
| Stable property IDs and six atomic ops, async host subscriptions and commands | N7/N8 |
| Never split a large selection edit into independent transactions to evade the 256-command host limit | N3, W075 atomic undo |

Implementation evidence, section coverage and API gaps will be recorded here as
this lane lands.
