# Design review: `feat/na-cloud-p-agent`

Reviewer: design lead · 2026-11-01 · Lane head `270fd8f8f` (diff vs
`origin/feat/native-engine`). Read against [panels/agent-panel.md](../panels/agent-panel.md),
[panels/README.md](../panels/README.md) (owner decisions), [ui-kit.md](../../ui-kit.md),
[review-checklist.md](../review-checklist.md) and the op-tree snapshots
(`panels/agent/__snapshots__/view.test.tsx.snap`). Code and snapshots only: no
app build, no screenshots.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-agent` (`31081ba31`), not the lane head.** That
branch is the lane plus the fixes marked **Fixed** below, merged with
`origin/feat/native-engine`. `bun test panels/agent/view.test.tsx` passes there
(37/37, snapshots regenerated). The model-wire tests need the full
`pm-panel-agentpanel` build and were not re-run on the reviewer's machine; the
fix touches only `view.tsx`. The Agent is **not shippable UI** until the kit
items in *Sol follow-ups* land. The lane was right not to fake the composer,
push buttons or pickers.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §1, owner decision 2 | Popover is the default; Pin/Close dispatch shell effects; popover content on `c.floating`, docked on `c.panel` | APPROVE |
| 2 | §3 header, README §2 | Thread/New Thread/Pin sat right after the title, not trailing | **Fixed**: header `space-between`, buttons grouped trailing |
| 3 | §3 user message | Bubble was full width, left-aligned | **Fixed**: list item is a row justified `end`. The 84 % max width is still missing (F3) |
| 4 | §3 thought row | `Thought for 3.217 s` (raw ms ÷ 1000); with no timestamps the label was the whole reasoning text | **Fixed**: whole seconds, minimum 1 (`Thought for 3 s`, v1.2.0 `activity-rows.ts`); fallback `Thought` |
| 5 | §3 question card | Questions sat in a group labelled `Answer` beside an `Answer` button (two "Answer" stops for VoiceOver); `Skip` was shown for every question; no padding on the vertical axis | **Fixed**: one labelled group per question; `Skip` only for `transport: "reply"` (v1.2.0 QuestionCard); card padding `m.inset` on all sides (TODO 2) |
| 6 | §3 approval card | Bare group, no card shape; `Deny` not danger | **Fixed**: same card as questions; `Deny` text `c.danger` |
| 7 | §3 error item | Message in `c.secondary` | **Fixed**: `t.body` `c.text` beside the triangle |
| 8 | §3 activity | Failed state had no symbol and wasn't danger-coloured | **Fixed**: `xmark.circle.fill` metadata + `c.danger` `Failed` |
| 9 | §3 work steps | Each tool is its own disclosure. The spec has one `{n} steps · {elapsed}` disclosure per turn | CHANGES REQUIRED (F5) |
| 10 | §3 result card | Body text came before the `Rendered result` heading; the heading is always `Rendered result` | Order **Fixed**. Heading choice is CHANGES REQUIRED (F6) |
| 11 | §3 visual reply | Failure had no triangle | **Fixed** (symbol metadata) |
| 12 | §4 loading history | Spinner labelled `Agent conversation` | **Fixed**: `Loading earlier messages…` (new copy, added to the spec) |
| 13 | §4 stop failure | `Couldn’t stop this task.` had no symbol | **Fixed** |
| 14 | §4 drag/drop | `Drop to attach` in muted/reading | **Fixed**: `t.body` `c.accent_text`. 1.5 pt stroke pending the token (TODO 2) |
| 15 | §7 VoiceOver | Panel, inner wrapper and list were all `Agent conversation` groups | **Fixed**: the inner layout wrapper has no role. Panel = group, conversation = list |
| 16 | §7 slash menu | The menu group reused the composer label `Message Powermove agent` | **Fixed**: `Shortcuts` (matches the tooltip's "/ for shortcuts") |
| 17 | §4 empty / connection | Copy exact; `Connect Your Agent` lacks `bolt.horizontal.circle`; empty title is `t.heading`, not `t.display` | CHANGES REQUIRED (kit F2) |
| 18 | §7 Thread pop-up | AX label is `Switch Thread`, should be `Switch thread, {title}` | CHANGES REQUIRED: the snapshot carries no thread title (F7) |
| 19 | §3 composer | Access pop-up disabled until the consent path exists. Correct: a binding must not bypass Full Access consent | APPROVE |
| 20 | Checklist §1 tokens | No literal colours/sizes in view code; `accent` only on marks | APPROVE |
| 21 | Checklist §7 copy | All strings verbatim; ellipses U+2026; no developer words visible (`apiGap` is metadata) | APPROVE |
| 22 | §8, §10 | No atmosphere/ghost/glow canvases; replies not bubbled; no glass inside | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

1. **Header search.** There is no header search. The `[⌕]` in the §3 diagram
   was a drawing error; thread search lives inside the thread picker
   (`Search threads…`). The diagram is corrected.
2. **Tokens for literal-only values.**
   - 1 pt strokes (bubbles/composer in the popover, cards, Increase Contrast
     edges) = `metrics.border` + `colors.separator`. Increase Contrast already
     makes `m.border` 2, which is the specified 2 pt composer stroke.
   - Drop-target stroke = new `metrics.drop_stroke` (1.5; 2 under Increase
     Contrast). This applies to README §3 everywhere, not only the agent.
   - Attention / unread dot (6 pt) = new `metrics.status_dot` (6). The README
     §3 insertion-line circle uses the same token.
   - Text padding `m.gap + 2` (bubble, composer, code block) = new
     `metrics.text_padding` (8). Inline code horizontal padding =
     `m.inner_radius` (4), vertical 0.
   - Question/approval card padding becomes `m.inset` on all sides. That drops
     the `m.inset − 2` arithmetic, which bought nothing.
   - Model-label collapse: the threshold is `m.agent_width` (360), not 300.
     The label collapses to `cpu` whenever the docked panel is narrower than
     its default width. The host reports it (`atMinimumDockWidth` → rename
     `narrow`).
   All three new tokens are added to [README §6](../panels/README.md#6-token-additions-and-changes-requested).
3. **Single-choice buttons vs explicit Answer.** v1.2.0 wins on behaviour
   (README precedence). Choosing an option **selects** it (round bullet for
   one answer, square for several) and never submits. `Answer` is the default
   (accent) button, disabled until something is chosen or typed. `Skip` (a
   borderless button) appears only for `reply` questions. The spec's
   "recommended choice is the default button" is withdrawn; §3 is updated.
4. **Closed (interrupted) questions.** They collapse like answered ones, to
   one `t.body` `c.muted` line `Not answered` (v1.2.0 QuestionCard copy). AX
   label `Not answered: {question}`. Done on the fix branch.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F1 | kit | System push button `PushButton{role: default · normal · destructive · borderless}` at `m.control` (shared request with Settings, Export, Effect Controls). Replace every `Push` placeholder, the result actions and `Try Again`/`Connection Settings…`/`Open Settings`. |
| F2 | kit | `EmptyState` gets `symbol` (2 × `m.symbol_panel`, `c.muted`) and `titleStyle: "heading" · "display"`. Agent empty uses `display`; `Connect Your Agent` uses `bolt.horizontal.circle`. |
| F3 | kit | Width ratio (`maxWidthRatio: 0.84`) or a `bubble` recipe for the user message. Paddings `metrics.text_padding` × `m.inset`. |
| F4 | pm-design | Add `metrics.status_dot` 6, `metrics.text_padding` 8, `metrics.drop_stroke` 1.5 (IC 2), `metrics.composer_max_lines` 10 (already requested). |
| F5 | agent lane + model | Group a turn's tool/task rows under one disclosure `{n} steps · {elapsed}` (`t.body` `c.muted`, AX `4 steps, 3.2 seconds, collapsed`); the per-tool rows become its children. Elapsed uses one decimal under 10 s, whole seconds above. |
| F6 | agent lane | Result heading: `Reversible agent change` + body `The editable Powermove result is ready to review.` for an editable (panel-run) result; `Rendered result` + `The rendered change is ready.` for a rendered run (v1.2.0 `ResultActions.svelte`). Requires the model to publish which kind `nativeResult` is. |
| F7 | agent lane + model | Publish the active thread title; Thread pop-up label `Switch thread, {title}`, text = title truncated to `m.value_column`; `{n} threads running` when background runs exist. |
| F8 | agent lane | Header trailing inset `m.inset − (m.hit − m.symbol_panel) ÷ 2` once the kit has token arithmetic (shared with every panel header). |
| F9 | kit/host | Composer (`NSTextView`), variable-height list, pickers, previews, launcher ring/dot/badge, announcements: the lane's API requests 1–8 stand as written. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-agent | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-agent` `31081ba31`; F1–F9 open |
