# Agent panel

Agent surface (`Surface::AgentPanel`), title **Agent**. Shared rules:
[README.md](README.md). Look: [language.md §12](../language.md#12-agent-panel).

Behaviour source: `v1.2.0:apps/desktop/src/renderer/src/panels/AgentPanel.svelte`,
`agent-shell.ts`, `agent/` (`Composer.svelte`, `Conversation.svelte`,
`Turn.svelte`, `ToolActivity.svelte`, `QuestionCard.svelte`,
`ResultActions.svelte`, `ThreadPicker.svelte`, `ModelPicker.svelte`,
`AccessPicker.svelte`, `ConnectionGate.svelte`, `SlashMenu.svelte`,
`AttachmentChips.svelte`, `VisualReply.svelte`, `WorkingTimer.svelte`,
`agent-state.svelte.ts`, `launcher-status.ts`) and
`legacy/assistant/spatial.ts` (access modes). Workflows W024–W033.

## 1. Presentation (v1.2.0 behaviour, HIG placement)

v1.2.0: the titlebar launcher opens the agent **as a popover under the
launcher, never in a dock**; pin or drag docks it into the workspace;
closing the docked panel returns it to the popover (A470, A592–A597).
Native keeps this exactly:

| Mode | Native form |
|---|---|
| **Popover** (default) | `NSPopover` from the toolbar **Agent** item (`sparkles`), behaviour `.transient` (Esc or a click outside closes it; the run continues). Size: width `m.agent_width`, height = min(2 × `m.agent_width`, window content height − `m.toolbar` − `m.status` − 2 × `m.gap`). Glass frame; **content on solid `c.floating`** (language §3: dense content inside glass is solid). |
| **Docked** | a right-dock panel (`m.agent_width` default width, resizable, min `m.agent_width − m.value_column`), solid `c.panel`, shares the dock with Properties/Effects like any panel (v1.2.0 "fills its column when docked alone and keeps its resized height beside another panel"). |

Pin button in the popover header → docks (undoable layout change). The
toolbar Agent item while docked toggles the docked panel's visibility.
Closing the docked panel (header `xmark` or dock menu `Close Panel`) returns
to popover mode. Dragging the popover's header out of the popover docks it
where dropped (dock previews from `pm-shell`). ⌘⇧K toggles the agent in its
current mode.

Toolbar Agent item states (launcher status, `launcher-status.ts`): idle
`sparkles` `c.secondary`, tooltip `Ask the Agent`; working `sparkles` in
`c.accent` with a circular indeterminate ring around it, tooltip
`Agent is working · {elapsed}`; needs attention (question/approval) adds a
`m.status_dot` `c.accent` dot top-right, tooltip `Agent needs your answer`;
background runs show a `t.minimum` count badge (`2`) in a `c.selected_fill`
capsule. Never more than one of ring/dot visible: attention wins.

## 2. Model contract (proposed names)

| State | Values |
|---|---|
| `AgentPresentation` | `Popover` · `Docked{dock}` |
| `Connection` | `Checking` · `SignInWaiting` · `Connected{provider, model, effort}` · `Unavailable{reason}` |
| `AccessMode` | `Editor` (Composition only) · `Supervised` · `Auto` · `Full` |
| `Phase` | `Idle` · `Working{started_at}` · `Steering` · `AwaitingAnswer` · `Stopping` · `Error{message}` |
| `threads` | `Thread{id, title, running: bool, unread: bool}` + `active` |
| `conversation` | `Turn{user: Message, items: [Text · Thought · Activity{rows} · Subagent{…} · Question · Approval · Visual · Result · Error]}` |
| `draft` | text with inline attachment tokens + `attachments` |
| `slash_menu` | `Option<{query, items, selected}>` |

## 3. Anatomy

```
┌ Agent ─────────── [Thread ⌄] [✎] [📌] ┐  header m.header
│                                        │
│            You  ┌──────────────────┐   │  user message: right, c.field bubble
│                 │ Make the headline│   │
│                 └──────────────────┘   │
│ › 4 steps · 3.2 s                      │  work steps disclosure (t.body c.muted)
│ Done. Headline now scales from 92 % …  │  agent text: plain on panel, t.reading
│ ┌ Apply the same entrance to Kicker? ┐ │  question card
│ │ It would start 0.2 s after…        │ │
│ │ [Apply to Kicker] [Not Now]        │ │
│ └────────────────────────────────────┘ │
│ ┌ Rendered result ── [Keep] [Undo] ──┐ │  result card
│                                        │
│ ┌────────────────────────────────────┐ │  composer (c.field)
│ │ Describe a change…                 │ │
│ │ [📎] [Composition only ⌄] [GPT-6.1 ⌄]  (↑) │
│ └────────────────────────────────────┘ │
└────────────────────────────────────────┘
```

| Element | Tokens |
|---|---|
| Header | README §2. Title `Agent`. Trailing: **Thread** pop-up (borderless, `t.body` `c.secondary`, current thread title truncated to `m.value_column`, chevron) → thread picker popover; **New Thread** (`square.and.pencil`, tooltip `New Thread`); **Pin** (`pin`, tooltip `Pin to Workspace`, popover only) or **Close** (`xmark`, docked only, tooltip `Close`). In the popover the whole header is also the drag handle (tooltip `Drag to move or pin to workspace`). |
| Conversation | scroll view, inset `m.inset`, gap between items `m.inset`, between turns `m.inset × 2`. Bottom anchored: new content keeps the view pinned to the latest unless the user scrolled up; then a **Scroll to Latest** button (`arrow.down`, `m.hit` round, `c.floating`, system shadow) appears centred `m.inset` above the composer. |
| User message | right-aligned, max width 84 % of the content width, `c.field` fill, `m.panel_radius` corners, padding `m.text_padding` vertical × `m.inset` horizontal, `t.reading` `c.text`. Attachments inside it as thumbnails (`m.row × 2` square, `m.control_radius`) above the text. No "You" label (alignment carries it); AX label `You said`. |
| Agent text | plain on the panel (no bubble), `t.reading` / `t.reading_leading`, `c.text`, full width. Markdown with the language's type scale: `#` → `t.heading`, `##`/`###` → `t.title`, lists with `m.inset` hanging indent, links `c.accent_text` underlined on hover, inline code SF Mono `t.body` on `c.field` (`m.inner_radius`, padding 0 × `m.inner_radius`), code blocks SF Mono `t.body` on `c.field` with `m.control_radius`, padding `m.text_padding`, horizontally scrollable inside the block only, copy button `doc.on.doc` top-right on hover (`Copy`). Tables: hairline `c.separator` grid, header `t.semibold_weight`. Local media references render as inline thumbnails (main's "show local media"). Streaming text appends in place (no reflow of earlier lines, A606). |
| Thought row | collapsed single line `t.body` `c.muted` with `brain` glyph: `Thinking…` while streaming, `Thought for {n} s` after (whole seconds, minimum 1; `Thought` when the span is unknown); disclosure expands the reasoning in `t.body` `c.secondary`. |
| Agent text, added by main | Task lists use a read-only `checkmark.square` / `square` glyph at `m.symbol_inline`. Ordered and nested lists indent `m.inset` per level. Quotes are `c.secondary` with a 2 pt `c.separator` bar at the leading edge. Unsafe HTML and URLs stay inert text. |
| Local media in replies | Under the prose of its message, `m.control_radius`, width to the content, click for the fitted preview sheet. Video and audio use the system player controls. Reads only files the reply was granted |
| Remote images | Not loaded by default: a `c.field` placeholder at `m.row × 2`, symbol `photo`, borderless button `Load Image`. Nothing is fetched until then |
| Work steps (activity) | finished work folds apart from the reply, and the elapsed time keeps counting across steering. One disclosure line per turn: `chevron.right` + `{n} steps · {elapsed}` `t.body` `c.muted`; expanded = list rows (`m.row`): tool symbol (`tool-icons.ts` mapping to SF Symbols) `m.symbol_inline` `c.muted`, label `t.body` `c.secondary`, trailing state: running = small spinner + **Stop** text button (`c.accent_text`), done = nothing, failed = `xmark.circle.fill` `c.danger` + `Failed`, stopped = `Stopped` `c.muted`, waiting = `Waiting` `c.muted`, continued = `Continued` `c.muted`. Expanded results are disclosure rows themselves, content `t.body` (SF Mono for raw output) on `c.field`, max 12 lines then scroll. |
| Subagent row | like an activity row with the subagent title, model in `t.minimum` `c.muted`, Stop while running; expanding shows its own steps and result (A607). |
| Working timer | under the last item while working: small spinner + `Working · {m:ss}` `t.body` `c.muted`, tabular. |
| Question card | `c.panel_secondary` fill (docked) / `c.panel` (popover, so it lifts off `c.floating`), `m.control_radius` corners, `m.border` `c.separator` inner stroke, padding `m.inset`. Title `t.title` `c.text`; detail `t.body` `c.secondary`; one group per question. Choosing an option **selects** it and never submits (v1.2.0 QuestionCard): round bullets for one answer, checkboxes for several. **`Answer`** is the default (accent) push button, disabled until something is chosen or typed; `Skip` (borderless) appears only for `reply` questions. Free-text answer: a one-line `NSTextField` `Type an answer` (`Or type an answer` when choices exist), Return submits (not ⇧Return, not during IME). After answering the card collapses to one line `t.body` `c.muted`: `Answered: {choice}` / `Skipped`; a question closed by an interrupted run collapses to `Not answered` (AX `Not answered: {question}`). |
| Approval card | same shape; title = the action (`Run “npm test”?`), detail = reason; buttons `Allow` (default), `Allow for This Thread`, `Deny` (borderless, `c.danger` text). Full-access warnings use `exclamationmark.triangle.fill` `c.warning` before the title. |
| Visual reply | card with a fixed-aspect preview (16:9, `m.control_radius`, solid `c.panel_secondary` behind while loading `Loading visualization…`), title `t.title`, trailing borderless buttons **Replay** (`arrow.counterclockwise`) and **Expand** (`arrow.up.left.and.arrow.down.right`). Expanded = a window-modal sheet sized to the window minus 2 × `m.inset × 2`, with zoom controls (`Zoom In`, `Zoom Out`, `Reset Zoom`) and `Close`; Esc closes and returns focus to the card. Failure: `The visualization couldn’t load.` `c.danger` with the triangle glyph. |
| Result card (reversible change) | `Reversible agent change` / `Rendered result` heading `t.title`; frames strip (thumbnails `m.row × 2` tall, `m.control_radius`, click = `View rendered composition at {t} seconds`); actions: **Keep Change** (default button), **Undo Change** (push button), **Add to Timeline** (when an artifact can be added; disabled `Already added to the timeline`), **Show in Finder** (borderless `folder`). |
| Error item | `exclamationmark.triangle.fill` `c.danger` + message `t.body` `c.text` + **Try Again** push button. |
| Composer | pinned to the bottom, inset `m.inset`, `c.field` fill (on `c.panel` docked / on `c.floating` in the popover add an `m.border` `c.separator` stroke so it reads), `m.control_radius` corners (language §12). Native `NSTextView`, `t.reading`, padding `m.text_padding` vertical × `m.inset` horizontal, min 2 lines, grows to `m.composer_max_lines` then scrolls. Placeholder (`c.muted`): `Describe a change…` idle, `Add direction…` while working. Bottom bar inside the composer (height `m.hit`): **Attach** (`paperclip`, tooltip `Add Attachments`), **Access** pop-up (borderless `t.minimum`: `Composition only` / `Supervised` / `Auto` / `Full access`, Full access in `c.warning` with the triangle), **Model** pop-up (borderless `t.minimum`: the model that runs, by name, never `Default`), **Effort** pop-up beside it (borderless `t.minimum`: the effort that runs, e.g. `Medium`; keeps its width while the model's name gives way; absent for a model with no effort levels), spacer, **Send** (`arrow.up.circle.fill` at `m.hit − 4` (20), `c.accent` when the draft is sendable, `c.muted` disabled) or **Stop** while working (`stop.circle.fill`, `c.text`, tooltip `Stop Current Run`). Tooltip on the text view: `Return to send · Shift-Return for a new line · / for shortcuts`. |
| Attachment chips | above the text inside the composer: capsule `c.panel` (docked) / `c.field`-on-`c.floating`, `m.field` tall, thumbnail or `doc` glyph, name `t.minimum` truncating, `xmark` remove (tooltip `Remove {name}`). Inline file tokens in the text are the same capsule inline; ⌫ once arms (ring `c.focus`), twice removes (A475). |
| Slash menu | glass child panel (palette recipe, language §9 Menus) anchored above the composer's caret line, width = composer width, rows `m.menu_row`, command `t.body` + description `t.minimum` `c.muted`, max 6 rows. |
| Thread picker | `NSPopover` from the Thread pop-up, `m.popover_width`; search field `Search threads…`; rows `m.row`: title `t.body`, running threads get a small spinner trailing, unread an `m.status_dot` `c.accent` dot; hover shows `trash` (`Delete Thread: {title}`); `New Thread` row at top with `square.and.pencil`. |
| Model picker | The window's field menu from the Model pop-up: `Provider` and `Model` sections, real models only; with none chosen, the provider's default model is ticked. Applies live. |
| Effort picker | The window's field menu from the Effort pop-up: `Reasoning Effort`, only the selected model's levels (`Low` · `Medium` · `Extra high` …as supplied); with none chosen, the model's default is ticked (the provider's, else Medium, else its first). A new model keeps the effort when it takes it, else runs at its own default. Applies live. |
| Access picker | `NSMenu` from the Access pop-up, header item (disabled) `How should agent actions be approved?`, then four items with subtitles: `Composition only` — `Edit the composition. No shell, files or web.`; `Supervised` — `Ask before anything outside the project sandbox.`; `Auto` — `Codex approves routine actions; Claude and API models still ask.`; `Full access` — `Run commands and edit files anywhere without prompts.` (warning symbol). Choosing `Full access` shows the Full Access `NSAlert` (language §9) first. |

## 4. States

| State | Visual |
|---|---|
| **Empty (new thread)** | centred in the conversation area: title `Make Your Next Move` `t.display` (SF Pro Display) `c.text`, line `t.reading` `c.secondary`: `Your ideas. Editable motion.` (Composition only), `Create, research, and build with your project.` (other modes), `Build and improve your extensions.` (extension context). Up to three suggestion buttons below (borderless, `t.body` `c.accent_text`) when the model offers them. Composer focused. |
| **Connection: checking** | conversation area shows a small spinner + `Checking connection…`; composer disabled. |
| **Connection: sign-in** | `Waiting for sign-in…` + spinner + `Open Settings` button. |
| **Connection: unavailable** | Empty-state layout: symbol `bolt.horizontal.circle`, title `Connect Your Agent`, line = reason (`ChatGPT couldn’t be reached.` / `Connect an API, Ollama, or LM Studio in Settings.`), buttons `Try Again` (default) and `Connection Settings…`; composer disabled with placeholder `Connect an agent to start`. |
| **Loading thread history** | previous messages render as they load; a top spinner row `Loading earlier messages…` while older pages load on scroll-up. |
| **Working** | Send → Stop; placeholder `Add direction…` (typing steers the current run, A611); working timer; toolbar ring. |
| **Awaiting answer** | question/approval card is the last item, composer still usable (typing answers free-text questions); toolbar dot. |
| **Stopping** | Stop button disabled with spinner; `Stopping…` in the timer line. |
| **Hover** | code block copy button; thread rows trash; activity rows `c.hover`. Messages have no hover fill. |
| **Focus** | composer has the system focus ring (orange) inside its radius; cards' buttons system rings; conversation is a focus stop (VoiceOver/keyboard scrolling). |
| **Selected** | text selection only (native); ⌘C copies text, never layers (A610). |
| **Disabled** | Send disabled for an empty draft (no text, no attachments). |
| **Error** | error item (above); persistence failure banner at top of the conversation: `Thread history couldn’t be saved. Keep this window open.` (`c.warning` triangle, `t.body`). |
| **Drag/drop** | files/images dragged over the agent: composer gets the drop-target treatment (`c.accent_wash` + 1.5 pt `c.accent` stroke) and text `Drop to attach` `t.body` `c.accent_text` centred; drop attaches. Layers dragged from the timeline attach as layer references (token capsule). |
| **Multi-select** | N/A. |
| **60 k messages** | variable-height virtualisation (README §4.1): measured heights cached per item id; streaming item measured each frame only while visible. |
| **Narrow (docked at min width)** | everything wraps; no horizontal scroll except inside code blocks and tables (A626). Composer bottom bar drops the Model pop-up label to the model glyph only (`cpu`) whenever the docked panel is narrower than `m.agent_width`. |

## 5. Interactions

| Gesture | Result |
|---|---|
| Return in composer | send once (repeat Enter ignored; IME composition never submits) |
| ⇧Return | newline |
| `/` at line start | slash menu; ↑/↓ wrap, Return/Tab insert, Esc dismiss |
| ⌘A / ⌘V / ⌘C | native text (draft), never layer commands |
| Paste/drop file | attachment |
| Click Stop | stop the run (and its subagents) |
| Click work-steps line | expand/collapse |
| Click Pin | dock |
| Drag header (popover) | move → dock preview → drop docks; Esc cancels |
| Click image attachment | fitted preview sheet; Esc closes and restores focus |

## 6. Keyboard and focus

⌘⇧K toggles the agent and focuses the composer. Tab order: header (Thread,
New Thread, Pin/Close) → conversation (one stop; ↑/↓ scroll by line, PageUp/
PageDown by page; Tab inside moves through interactive items in cards in
reading order) → composer → Attach → Access → Model → Send/Stop. Esc: closes
the slash menu → closes an inner popover (thread/model) → (popover mode)
closes the agent popover and returns focus to where it was. Editor shortcuts
never fire while the composer is focused (W010).

## 7. VoiceOver

| Element | Role | Label / value |
|---|---|---|
| Panel/popover | group | `Agent conversation` |
| Conversation | list (`AXRowCount`) | items: `You said: {text}`; `Agent: {text}`; `4 steps, 3.2 seconds, collapsed`; `Question from the agent: {title}` |
| Live updates | streaming text is **not** read per token; when a turn completes, announce `Agent replied` (polite); questions announce `The agent has a question` (assertive) |
| Composer | text area | `Message Powermove agent`; hint `Return to send, Shift-Return for a new line` |
| Send / Stop | button | `Send` / `Stop current run` |
| Access | pop-up | `Agent access, Composition only` |
| Model | pop-up | `Model, {model}` |
| Effort | pop-up | `Reasoning effort, {effort}` |
| Thread | pop-up | `Switch thread, {title}`; background runs `2 threads running` |
| Activity Stop | button | `Stop {tool}` |
| Attachment chip | button | `{name}, attachment`; action `Remove` |

## 8. Appearance

- **Docked**: solid `c.panel`; user bubbles `c.field`; cards
  `c.panel_secondary`.
- **Popover**: glass frame (system), content `c.floating`; user bubbles and
  composer `c.field` with 1 pt `c.separator`; cards `c.panel`.
- **Light/dark**: tokens only. No "atmosphere", "ghost field" or prompt-glow
  canvases (v1.2.0 decorative GPU effects): dropped — language §1.1, the work
  is the colour. The working state is the toolbar ring + timer.
- **Increase Contrast**: bubbles and cards get 1 pt `c.separator` strokes;
  composer stroke 2 pt.
- **Reduce Transparency**: popover frame solid `c.floating`.
- **Reduce Motion**: streaming text appears in chunks without the reveal
  animation; disclosure without spring.

## 9. Copy

| Where | String |
|---|---|
| Title | `Agent` |
| Header tooltips | `Switch Thread`, `New Thread`, `Pin to Workspace`, `Close`, `Drag to move or pin to workspace` |
| Toolbar item | `Ask the Agent`; `Agent is working · {m:ss}`; `Agent needs your answer` |
| Empty | `Make Your Next Move`; `Your ideas. Editable motion.` / `Create, research, and build with your project.` / `Build and improve your extensions.` |
| Composer | `Describe a change…`, `Add direction…`, `Connect an agent to start`; tooltip `Return to send · Shift-Return for a new line · / for shortcuts`; `Add Attachments`, `Stop Current Run`, `Drop to attach` |
| Connection | `Checking connection…`, `Waiting for sign-in…`, `Connect Your Agent`, `ChatGPT couldn’t be reached.`, `Connect an API, Ollama, or LM Studio in Settings.`, `Try Again`, `Connection Settings…`, `Open Settings` |
| Activity | `{n} steps · {elapsed}`, `Thinking…`, `Thought for {n} s`, `Thought`, `Loading earlier messages…`, `Working · {m:ss}`, `Stopping…`, `Stop`, `Stopped`, `Failed`, `Waiting`, `Continued`, `Couldn’t stop this task.` |
| Question | `Answer`, `Skip`, `Choose any that apply`, `Type an answer`, `Or type an answer`, `Answered: {choice}`, `Skipped`, `Not answered` |
| Approval | `Allow`, `Allow for This Thread`, `Deny` |
| Results | `Reversible agent change`, `Rendered result`, `Keep Change`, `Undo Change`, `Add to Timeline`, `Already added to the timeline`, `Show in Finder`, `The editable Powermove result is ready to review.`, `The rendered change is ready.` |
| Visual reply | `Loading visualization…`, `Replay`, `Expand`, `Zoom In`, `Zoom Out`, `Reset Zoom`, `Close`, `The visualization couldn’t load.` |
| Threads | `Search threads…`, `New Thread`, `Delete Thread: {title}` |
| Access | `How should agent actions be approved?`, `Composition only`, `Supervised`, `Auto`, `Full access` + the four subtitles in §3 |
| Errors | `Thread history couldn’t be saved. Keep this window open.` |
| Scroll | `Scroll to Latest` |
| Slash menu (AX group) | `Shortcuts` |

## 10. Don't

- Don't put the conversation on glass; don't use glass buttons inside the
  popover (glass on glass).
- Don't put the agent's own replies in bubbles.
- Don't show approvals or questions as modal alerts (except the Full Access
  consent).
- Don't ship the atmosphere/ghost/prompt-glow canvases.
- Don't let the composer steal editor shortcuts when it is not focused, or
  let editor shortcuts fire while it is.
- Don't open the agent docked by default; v1.2.0 opens it as a popover.
