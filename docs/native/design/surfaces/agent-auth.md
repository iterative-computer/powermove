# Agent and accounts — remaining agent surfaces, providers, sign-in

Rows S057 S059 S060 S062 S063 S064 S065 S066 S070 S071 S072 S083 S084 S085
S086. Grammar: [README](README.md) §1–5. This file completes
[panels/agent-panel.md](../panels/agent-panel.md) and the Accounts section of
[panels/settings.md](../panels/settings.md); it does not repeat them. Where a
string here differs from those specs, this file wins because it was checked
against v1.2.0 line by line.

No new tokens. One new kit recipe: `AgentCard` (below), which names the shape
the Question card already uses so other cards stop restating it.

## Shared recipe: `AgentCard`

| Part | Spec |
|---|---|
| Shape | agent-panel *Question card*: `c.panel_secondary` docked / `c.panel` popover, `m.control_radius`, `m.border` `c.separator` inner stroke, padding `m.inset` |
| Eyebrow (optional) | `t.minimum` `c.muted`, sentence case, `m.gap` above the title |
| Title | `t.title` `c.text`; truncates to two lines with a tooltip |
| Rows | label `t.body` `c.secondary` leading, value `t.body` `c.text` trailing-aligned to the card inset, `m.row` tall, separated by nothing |
| Footer | trailing, `m.inset` above: push buttons at `m.control`; at most one accent default per card; a leading control sits at the card's leading inset |
| Narrow | below `m.agent_width`, footer buttons wrap to a second line, default button last |
| VoiceOver | one `AXGroup` labelled `{eyebrow}: {title}`; children in reading order |

## S057 Agent conversation — plan preview and step details

Ports `R/panels/agent/PlanPreview.svelte`, `ToolActivity.svelte`.

**Plan preview.** An `AgentCard` placed as the last conversation item while a
plan is pending. Eyebrow `Proposed change`.

| Plan kind | Title | Rows | Default button |
|---|---|---|---|
| Panels | `Panel arrangement` | one row per action: symbol `rectangle.3.group` `m.symbol_inline` `c.muted` + description | `Apply Panel Changes` |
| Interface | `Timeline redesign` | key · value per patch entry | `Apply Interface Edit` |
| Chrome | `Interface edit` | `Timeline surfaces` or `Corner style` · value | `Apply Interface Edit` |
| Scene | the plan's label | one row per command, description only | `Apply Scene Edit` |
| Workspace | the workspace name | dock id · panel names joined by ` · `; then section title · `{n} connected controls` | `Create Workspace` |
| Section | the section title | control label · connection or type | `Apply Section` |

Footer: `Dismiss` (push, closes the plan, focus to the composer), then the
default button. When the plan kind is Panels and access is Composition only, a
leading checkbox `Auto-Apply` (tooltip, off: `Review panel changes before
applying`; on: `Safe panel changes apply automatically`). v1.2.0 drew small
slider and curve glyphs beside section rows: dropped, the value text carries
it. While a plan is pending, `Switch Thread` is disabled with tooltip
`Finish applying the current change to switch threads`.

**Step details.** An expanded activity row (agent-panel *Work steps*) gains a
detail block under the row, indented `m.tree_indent`:

| Content | Spec |
|---|---|
| Command or argument lines | `MONO_FONT` `t.body` `c.secondary`, wraps |
| Tool output | agent-panel code block recipe, max height `m.palette_height`, then scrolls inside the block; copy button on hover |
| Subagent or task result | Markdown in `t.body` |
| Stop failed | Error state: `Couldn’t stop this task.` |
| Row from a reopened thread | same layout, no Stop. If the detail is not stored: `t.body` `c.muted` `Details aren’t available for this step.` |
| Loading stored detail | placeholder well at one `m.row`, then content; no spinner |

Keyboard: the disclosure line is a button (Space/Return toggles). VoiceOver:
`{tool label}, {state}`, expanded state as `AXExpanded`.

## S059 Provider connection gate

Ports `R/panels/agent/ConnectionGate.svelte`. Replaces the three *Connection*
rows in agent-panel §4. Gate content is centred in the conversation area on
the panel material, `m.popover_width` wide. The composer stays visible,
disabled, placeholder `Connect an agent to start`.

Top: `SegmentedControl` `Codex` · `Claude` · `API / Local` (AX group
`Connection provider`). Switching provider re-evaluates the gate at once.

| Status | Heading (`t.heading`) | Line (`t.reading` `c.secondary`) | Button |
|---|---|---|---|
| Checking | `Checking Connection…` | `Checking your {provider} connection.` | `Checking…` disabled, small spinner leading |
| Not connected | `Connect {provider}` | `Use your {provider} subscription to power the Powermove agent.` | `Connect {provider}` (default) |
| Connecting | `Finish Connecting {provider}` | status detail, else `Finish signing in in your browser.` | `Waiting for Sign-In…` disabled, spinner |
| Reconnecting | `Finish Connecting {provider}` | `Connection interrupted. Reconnecting…` | `Waiting for Sign-In…` disabled, spinner |
| API / Local not set up | `Bring Your Own Model` | `Use an API, Ollama, or LM Studio. Set up your model in Settings.` | `Open Settings` (default) |
| Unavailable | error item (S064 *Error notice*); fallback title `{provider} Couldn’t Start`, fallback text `Powermove couldn’t start its {provider} service.` | | `Try Again` |

Below, except for API / Local not set up: borderless `Connection Settings…`
(opens Settings ▸ Accounts). The heading and line are one polite live region.

## S060 Thread history

Ports `R/panels/agent/ThreadPicker.svelte`. Completes agent-panel *Thread
picker*.

| Item | Spec |
|---|---|
| Thread pop-up, background count | capsule after the title: `m.status_dot` `c.accent` + count `t.minimum` tabular; tooltip `{n} thread still working` / `{n} threads still working` |
| Blocked | pop-up and New Thread disabled while a plan applies; tooltip `Finish applying the current change to switch threads` |
| Ordering | most recent activity first, as v1.2.0 |
| Default title | `New Thread`, replaced by the derived title after the first message. v1.2.0 has no rename control; none is added |
| Row while running | title with spinner trailing; trash tooltip `Delete Thread and Stop Its Run` |
| No search match | row area `t.body` `c.muted` `No threads match “{query}”.` |
| Large history | README §4 list rules |
| Save failure | agent-panel banner, unchanged |

**Delete** — alert, sheet on the editor window (the popover stays open behind
it):

| Part | Copy |
|---|---|
| Message | `Delete “{title}”?` |
| Informative (idle) | `The thread leaves this list. Powermove keeps an archived copy on this Mac.` |
| Informative (running) | `This stops its run. The thread leaves this list, and Powermove keeps an archived copy on this Mac.` |
| Buttons | `Cancel` (default) · `Delete Thread` (destructive) |

v1.2.0 said “This cannot be undone.” Native archives removed threads (N4,
[agents.md](../../agents.md)), so the copy must not claim permanence. On Cancel
focus returns to the search field. After a delete focus returns to the search
field, or the picker closes if no threads remain. There is no restore UI yet;
see [README §6](README.md#6-owner-questions-raised-by-these-specs).

## S062 Visual replies and generated-UI placement

Ports `R/panels/agent/VisualReply.svelte`, `UIPlacementGhost.svelte`,
`GhostCard.svelte`. Completes agent-panel *Visual reply*. Agent-authored
content stays in its isolated view; only host chrome is specified.

| State | Spec |
|---|---|
| Inline loading | `Loading visualization…` over `c.panel_secondary` (agent-panel) |
| Start timeout (8 s) | notice in the card: title `Visualization Unavailable`, line `The visualization didn’t start. Replay it or ask the agent to fix it.`, button `Replay` |
| Artifact missing or unreadable | same title, line `The visualization couldn’t load.`, button `Replay` |
| Reopened thread | the card re-creates the view from the stored artifact, entering Inline loading. A stored artifact that no longer exists shows the missing state. No auto-replay when the card scrolls back into view |
| Expanded sheet controls | leading to trailing: `Zoom Out`, zoom percentage button (`100 %`, tabular, click = `Reset Zoom`), `Zoom In`, `Replay`, `Close`. Range 50 % to 200 % in 25 % steps; the end button disables at each limit. `Replay` is hidden in the error states |
| AX | card group `{title} preview`; sheet labelled `Expanded {title}`; buttons append the title, `Zoom In {title}` |

**Placement card** — while a run is building UI, its destination shows where
the result will land, so the agent's work is visible in place.

| Placement | Spec |
|---|---|
| New panel section (insert) | a block inserted at the target position in the panel: `m.row × 2` tall, `c.panel_secondary`, `m.control_radius`, small spinner + `Building {label}…` `t.body` `c.muted` |
| New dock panel | a normal panel card in the target dock with the same line centred (Loading state, no heading) |
| Existing element | element gets a `m.drop_stroke` `c.accent` inner stroke following its radius, plus the tooltip `Updating {label}…`. No fill change |
| Target can't be resolved | nothing is drawn in the workspace; the line appears only as a row in the conversation. Never cover a whole panel |
| Run ends, stops or fails | the card or stroke disappears (`mo.exit`); the real result, if any, takes its place |

The v1.2.0 ghost-field canvas is not ported (agent-panel §8). Placement cards
are decoration of a running state, so they take no focus and no Tab stop;
VoiceOver gets one polite announcement `Building {label}`.

## S063 Agent results for extensions and runs

Ports `R/panels/agent/ModResult.svelte`, `ResultActions.svelte`.

**Panel result** — an `AgentCard`, compact: symbol at `m.symbol_panel`
leading (`square.stack.3d.up` created, `square.and.pencil` edited), eyebrow,
name as title, button trailing.

| Result | Eyebrow | Button |
|---|---|---|
| Created | `Panel created` | `Open` |
| Edited, one panel | `Panel edited` | `Open` |
| Edited, several panels | `{n} panels edited` | one `Open {panel}` per panel, wrapping |

The card appears only when the result is ready **and** the panel still exists,
as v1.2.0 does: an errored or removed result shows no card, and the agent's own
text reports it. `Open` reveals the panel in Library ▸ Panels (S101). AX for
`Open`: `Open {name}`.

**Run results** — completes agent-panel *Result card*.

| Case | Spec |
|---|---|
| Layout/panel run | eyebrow `Reversible agent change`, line `This is also in the normal ⌘Z Undo history.`, buttons `Undo Change` · `Keep Change` (default) |
| Supervised run | eyebrow `Rendered result`, title `Review the Actual Result`, the review text, line `Powermove source changes from this run are one ⌘Z Undo step.` when reversible, then the frames strip |
| Autonomous run | no eyebrow; frames behind disclosure `View Rendered Frames`; borderless `Undo Change`, tooltip `Restore this run’s project and app-extension changes` |
| Review failed | warning line (Error state, `c.warning`) with the review error; frames still shown |
| Artifact row | name (click: image = preview S065, other = Show in Finder), `Add to Timeline` (disabled while importing; AX when added `{name} is already on the timeline`), `folder` button `Show in Finder` |
| Added | viewer banner `Added {name} to the timeline` |

**Project handoff.** A run keeps its project. Switching context during a run
shows the viewer banner `Finish or stop the current agent run before switching
projects.` When the run's project is in a background tab the activity row
reads `Ready to continue in this project tab`.

## S064 Repair flow and error notices

Ports `R/panels/RepairAgent.svelte`, `repair-agent.svelte.ts`,
`R/errors/ErrorNotice.svelte`, `ErrorFix.svelte`, `presentation.ts`.

**Error notice** — every failed turn, and S059's unavailable state. Three
weights, as v1.2.0's `NoticeKind`:

| Kind | When | Look |
|---|---|---|
| `plain` | the agent simply answered | normal agent text, no card |
| `alert` | the agent could not do it; project intact | `exclamationmark.triangle.fill` `c.warning` + text; no diagnostics |
| `error` | editor or transport failed | `AgentCard`, `exclamationmark.triangle.fill` `c.danger`, title, message, `Details` disclosure, actions |

`Details` expands selectable `MONO_FONT` `t.body` `c.secondary` text with a
borderless `Copy`, and the same text goes to the Error Log (S053). Titles and
messages port the `presentError` table with Title Case titles and Mac wording:

| Title | Message |
|---|---|
| `Update {runtime} to Use This Model` | `This model needs a newer version than the one Powermove has. Install the update, then try again.` Rows `Installed` · `{version}`, `Required` · `{version} or newer` when known |
| `Codex Needs Node.js to Start` | v1.2.0 text verbatim |
| `Agent Session Couldn’t Reopen` | `Try sending your request again. If it repeats, restart Powermove to reconnect the agent.` |
| `Conversation Is Busy` | `Wait for the current run to finish, or stop it before sending your request again.` |
| `Couldn’t Reopen the Conversation` | `Send your request again. If this continues, start a new conversation.` |
| `Agent Is Temporarily Unavailable` | `Wait a moment, then send your request again. Check your account if you’ve reached a usage limit.` |
| `Sign-In Needs Attention` | `Reconnect your agent account in Settings, then send your request again.` |
| `The Request Timed Out` | `Check the current result before trying again; the agent may have completed part of the work.` |
| `Your Disk Is Full` | `Free up some storage, then try the operation again.` |
| `File Access Was Denied` | `Choose a location you can access, or check Powermove’s file permissions in System Settings.` |
| `Connection Interrupted` | `Check your internet connection, then try again.` |
| `The Agent Couldn’t Finish the Result` | `Review any changes already made, then send your request again.` |
| `Something Needs Attention` | the readable error when short, else `Try again. If it keeps happening, choose Window ▸ Error Log for details.` |

**Fix action** (`ErrorFix`) appears only when the notice names a runtime
Powermove can install. It is the card's default button:

| Phase | Button / line |
|---|---|
| Ready | `Install the Latest {runtime}`, or `Fix It for Me` for the Node.js case |
| Installing | button disabled, spinner, `Installing {runtime}…` |
| Done | `checkmark.circle.fill` `c.success` `{runtime} {version} is installed. Reconnecting…` (`Try again to continue.` when the surface cannot reconnect itself) |
| Failed | Error state, the failure or `The update couldn’t finish.`; button `Try the Fix Again` |

**Repair window** — opened by `Fix with Agent` in S082 (and the Sandbox group of
S079), one per extension; reopening focuses the existing one. v1.2.0 used a draggable floating panel. Native
uses a standard titled utility window (`NSPanel`, utility style, closable,
resizable, frame autosaved), title `Repair Extension`, subtitle
`{extension}`, default `m.agent_width` × 2 × `m.agent_width`, content on solid
`c.panel`.

| State | Spec |
|---|---|
| Opening | the request is sent at once; agent-panel conversation and work-steps recipes |
| Running | composer placeholder `Working on the repair…`; Stop visible (AX `Stop repair`) |
| Sandbox re-check | activity line `Checking the repaired extension in the sandbox…`, then if needed `Repairing the remaining sandbox issue…` |
| Idle | placeholder `Ask about this repair…` |
| Not connected | agent reply style `Connect your agent to repair this extension.` + button `Connection Settings…` |
| Failed | error notice with `Try Again` (disabled while running) |
| Couldn't verify | alert notice `Sandbox compatibility couldn’t be verified for this extension.` |
| Stopped or unfinished | alert notice `The repair didn’t finish.` |
| Closing while running | no alert: the window hides and the run continues, as a closed popover does. Reopening shows it |

Tab order: conversation → composer → Attach → Send/Stop. ⌘W closes. Attachment
failure: banner-free inline Error above the composer `Couldn’t attach {name}.`
AX window title `Repair agent: {extension}`.

## S065 Attachment preview

Ports `R/panels/agent/attachments.ts`. Sheet on the editor window, opened by
activating an image attachment or a rendered frame. Non-image attachments do
not preview: activating them is Show in Finder.

| Element | Spec |
|---|---|
| Frame | system sheet; content width = min(image width at 1×, window content width − 2 × `m.sheet_inset × 2`) with height likewise, never upscaled, never below `m.sheet_width` |
| Title | `t.heading`, the attachment name, truncating in the middle; rendered frames read `Rendered composition at {t} seconds` |
| Image | fitted, centred on `c.viewer`, `m.control_radius`; checkerboard (`m.checker`) behind transparency |
| Footer | leading `Show in Finder` (when a source or artifact file exists); trailing `Done` (default) |
| Loading (bytes not yet read) | Loading state in the image area, sized `m.sheet_width` × `m.sheet_width ÷ 2` |
| Unavailable | `EmptyState` symbol `photo`, title `Image Unavailable`, line `{name} isn’t on this Mac anymore.`; footer `Done` only |
| Show in Finder failed | Error line above the footer `Couldn’t show {name} in Finder.` |

Esc or Return closes; focus returns to the chip or thumbnail that opened it
(tracked by id, so a re-rendered row still receives it). AX: image labelled
`{name}`; opener labels `View {name}` and `Show {name} in Finder`.

## S066 Region selection

Ports `activate()` and the overlay in `R/legacy/assistant/spatial.ts`.
**v1.2.0 has the code but no caller reaches it**, so this surface has never
been in users' hands. It is specified so the row can close; a lane must not
build it until the owner answers [README §6](README.md#6-owner-questions-raised-by-these-specs).

Entry (when built): `View ▸ Ask About Area…`, no default shortcut (⌘⇧K stays
the agent toggle). The overlay is a borderless transparent child window over
the editor window.

| Phase | Spec |
|---|---|
| Selecting | whole window dims with `c.media_badge` at `m.media_badge_opacity`; capsule hint at top centre, dark-scheme tokens, `Type a prompt or drag to select an area`; crosshair cursor |
| Dragging | rectangle cut out of the dim, `m.gizmo_keyline` `c.accent` edge. The Electron ink stroke and ripple canvas are not ported; under Reduce Motion nothing changes because nothing animates |
| Too small or a click | a plain click cancels. A drag under `m.hit` either way keeps the overlay and changes the hint to `Drag across a larger interface area` |
| Composing | `NSPopover` anchored to the region, `m.agent_width` wide: target line, text view, footer |
| Target line | `t.minimum` `c.muted`: `Selected · {panel}`; without a region, `Powermove agent · full composition` |
| Text view | placeholder `How should this area change?`, or `Full composition` with no region |
| Footer | leading status `t.minimum` `c.muted` `Return to send`; trailing `Cancel`, then accent `arrow.up.circle.fill` (AX `Send`) |
| Disclosure | under the text view, `t.minimum` `c.muted`: `Your agent gets a screenshot of this area.` |
| Capture failed | Error state, `c.warning`: `Couldn’t capture this area. Your agent gets the panel’s details instead.` Sending stays available |
| Sent | overlay closes; the message appears in the normal conversation with the screenshot as an attachment |

Capture reads Powermove's own window only, so no Screen Recording permission is
involved and nothing outside the window can be sent. Esc steps back one phase:
Composing → Selecting → closed. Keyboard alternative: in Selecting, Tab moves a
highlight across panels and Return selects the highlighted panel; VoiceOver
reads `{panel}, select area`.

## S070 Subscriptions and external clients

Ports `R/legacy/ui/chatgpt-settings.ts`, `M/claude-desktop-mcp.ts`. Completes
settings.md *Subscriptions*. One row per service, `{name}` = `ChatGPT` or
`Claude`, `{runtime}` = `Codex` or `Claude Code`.

| State | Detail line | Status | Button |
|---|---|---|---|
| Checking | `Checking your {runtime} sign-in…` | spinner | `Connect` disabled |
| Not connected | `Use your {name} subscription with Powermove.` | none | `Connect` |
| Opening browser | `Opening {name} sign-in…` | spinner, `Waiting…` | `Connect` disabled |
| Waiting | status detail, else `Finish signing in in your browser.` | spinner, `Waiting…` | `Cancel` |
| Device code offered | detail `Enter this code in your browser:` then the code in `t.title` tabular, selectable, with borderless `Copy` | spinner, `Waiting…` | `Cancel` |
| Connected | identity (email, plan), else `Your {name} subscription is ready to use.` | `checkmark.circle.fill` `c.success` `Connected` | `Disconnect` |
| Connected, not removable | same | same | none; v1.2.0 showed a disabled `Connected` |
| Disconnecting | `Disconnecting {name} from Powermove…` | spinner | disabled |
| Unavailable | the failure | `exclamationmark.triangle.fill` `c.danger` `Unavailable` | `Retry` |
| Restart needed | `Restart Powermove once to finish installing {name} connection.` | `exclamationmark.triangle.fill` `c.warning` `Restart needed` | none |

`Disconnect` asks nothing; it is reconnectable. Status text is a polite live
region. Never show the browser-delivered token or a raw URL.

**External clients** — new group `External Clients`, shown only when a
supported client is installed. v1.2.0 registers Powermove with the Claude
desktop app silently at launch and has no UI, so this row is the native
addition that makes the change visible and reversible.

| State | Row | Detail | Control |
|---|---|---|---|
| Registered | `Claude Desktop` | `Claude can use Powermove tools on this Mac.` | switch on |
| Removed by you | same | same | switch off, never re-registered silently |
| Failed | same | Error state: `Couldn’t update Claude’s settings. Check that Claude is installed, then try again.` | switch off + `Retry` |

Writes back to the client's configuration are atomic and preserve its other
entries. The default state is an owner question (README §6).

## S071 API or local model

Ports `R/legacy/ui/compatible-settings.ts`. Completes settings.md *API or Local
Model*: row order and copy there stand; v1.2.0's lower-case labels are
Title Case as listed there. Presets fill the URL and leave the key alone.

| State | `Status` row | Button |
|---|---|---|
| Not set up | `Not connected. Start your local server, or enter your provider’s details.` | `Test and Connect` |
| Field missing | same; invalid field shows Error under it: `Enter a URL that starts with http:// or https://.` / `Enter the model name.` | disabled |
| Testing | spinner, `Testing the connection…` | `Testing…` disabled, fields disabled |
| Connected | `checkmark.circle.fill` `c.success` `Connected model: {model}` | `Disconnect` |
| Failed | Error state `Couldn’t connect. Check the address and model.` | `Test and Connect` |
| Needs restart | `c.warning` `Restart Powermove to enable API and local connections.` | disabled |

`API Key` detail follows the three strings in settings.md: saved key, local URL,
remote URL. A saved key never appears; the field stays empty with placeholder
`Saved`. `Disconnect` clears the active connection and keeps the fields; the
key is removed from Keychain only by clearing the field and testing. Editing
any field after Connected returns the button to `Test and Connect` while the
previous connection keeps working until the test succeeds. Focus order: type,
URL, key, model, images, button. Status is a polite live region.

## S072 Agent runtime install and update

Ports `M/runtime-updates.ts`, `M/codex/connection-status.ts`. Runtimes install
into Powermove's own storage. No separate window: state appears in the S059 gate,
the S070 row and the S064 fix action.

| State | Gate / Settings copy | Control |
|---|---|---|
| Needed, first connect | `Powermove needs to install {runtime} first.` | `Install {runtime}` |
| Downloading | `Downloading {runtime}…` | determinate `ProgressRow` when size is known, else spinner; `Cancel` |
| Verifying and activating | `Installing {runtime}…` | spinner, no Cancel |
| Checksum mismatch | Error `The download didn’t match its published checksum, so it wasn’t installed.` | `Try Again` |
| Offline | `c.warning` `Connect to the internet to install {runtime}.` | `Try Again` |
| Registry problem | Error `The update service returned something unexpected. Try again later.` | `Try Again` |
| Disk full | S064 `Your Disk Is Full` | `Try Again` |
| Update in background | nothing: new runs use the new version | none |
| Update failed in background | previous version keeps working; Error Log entry only | none |

An interrupted activation leaves the previous runtime in place, so no state
ever reads as uninstalled.

## S083 Sign in and create account

Ports `R/cloud/SignInSheet.svelte`. One system sheet at `m.sheet_width` that
changes step in place (height animates per `mo.enter`; cross-fade under Reduce
Motion). Opened from Settings ▸ Accounts, the account menu, and the Store when an
action needs an account. Default mode is sign in.

**Start step**

| Element | Copy |
|---|---|
| Title | `Sign In to Powermove` / `Create Your Powermove Account` |
| Line | `Publish and manage your extensions on the Store.` / `Everything you publish to the Store lives under your handle.` |
| Provider buttons | full-width push buttons, provider mark leading: `Continue with {provider}` |
| Separator | hairline with `or` `t.minimum` `c.muted` centred |
| Email | `NSTextField`, placeholder `Email address`, full width |
| Email button | `Continue with Email`, disabled until the address has a valid shape; `Sending Code…` while sending. This is the accent default |
| Legal | `t.minimum` `c.muted`, sentence with links `Terms` and `Privacy Policy` (`c.accent_text`), opening the browser |
| Switch | `New to Powermove?` + link `Create an Account` / `Already have an account?` + link `Sign In` |
| Footer | `Cancel`, leading |

**Browser step**: `Back` (borderless, `chevron.left`) leading in the footer.
Title `Finish Signing In in Your Browser`, line `Powermove opened {provider} in
your browser. When you’re done there, you’ll come straight back here.`
Centre: spinner + `Waiting for {provider}…`. Footer trailing `Cancel`.

Errors, Error state under the control that failed:

| Case | Copy |
|---|---|
| Browser wouldn't open | `Couldn’t open {provider}. Try again.`, spinner replaced by `Try Again` |
| Email rejected | `Couldn’t send a code to this address. Check it and try again.` |
| Offline | `Couldn’t send a code. Check your connection and try again.` |
| Rate limited | `Too many attempts. Wait a minute, then try again.` |
| Old client | `This version of Powermove is too old to sign in. Update Powermove, then try again.` |
| Return link failed | `Couldn’t finish signing in. Try again.` |

Success: the sheet closes and a banner (viewer) or inline notice (Library,
Settings) says `Signed in as {name}`. If the account has no handle, the sheet
goes to the handle step instead of closing. An auth link arriving with no sheet
open is ignored apart from an Error Log entry.

Focus: first provider button; in the browser step, `Cancel`. Esc cancels. The
sheet's progress line is a polite live region.

## S084 Email code and handle

**Code step**: `Back` as above. Title `Check Your Email`, line `Enter the
6-digit code sent to {email}.` (address in `t.semibold_weight`).

| Element | Spec |
|---|---|
| Code field | one `NSTextField` drawn as six `m.row`-square cells on `c.field`, `m.control_radius`, `t.heading` tabular digits, `m.gap` between. Real single field so paste, autofill from Mail and VoiceOver work (AX `Verification code`, `{n} of 6 digits`). `oneTimeCode` content type |
| Submit | automatic on the sixth digit; no button. Spinner replaces the resend line while checking |
| Resend | borderless: `Resend in {n} s` disabled, tabular, then `Resend Code` |
| Wrong code | Error `That code didn’t match. Check it, or send a new one.`; field clears and refocuses |
| Expired | Error `That code has expired. Send a new one.` |
| Couldn't check | Error `Couldn’t check the code. Check your connection and try again.` |

**Handle step**: also entered directly by `Choose Handle…`, and on resume after a
sign-up interrupted before a handle was saved. Title `Choose Your Handle`, line
`It’s your name on the Store. Extensions you publish live under it, and it
can’t be changed later.`

| Element | Spec |
|---|---|
| Field | row `Handle`, `@` prefix in `c.muted` inside the well, lower-cases as typed |
| Hint under field | `t.minimum` `c.muted` `Lowercase letters, numbers and hyphens.` |
| Invalid | Error `Use 2 to 39 characters, starting with a letter or number.` |
| Taken / reserved | Error `@{handle} is taken. Try another.` / `@{handle} is reserved. Try another.` |
| Preview | `t.minimum` `c.muted` `Your extensions publish as {handle}/my-extension`, handle in `MONO_FONT` |
| Footer | `Not Now` (leading), `Choose Handle` (default; `Saving Handle…` while saving) |
| Failed | Error `Couldn’t save your handle. Check your connection and try again.` |

`Not Now` closes signed in without a handle; the Store and account menu keep
offering `Choose Handle…`.

## S085 Account menu

Ports `R/cloud/AccountButton.svelte`, `Avatar.svelte`, `account.ts`. v1.2.0
draws its own menu with a note that native menus are for right-click; native
inverts this per language §9: a real `NSMenu`. The Settings gear and sidebar
row have no native equivalent, so the account button lives in two places:
trailing in the Store window toolbar and in the Project Library window toolbar.

| State | Button | Menu |
|---|---|---|
| Unknown | spinner at `m.symbol_toolbar` | none |
| Signed out | `person.crop.circle` + `Sign In…` | none, opens S083 |
| Signed in | avatar, `m.symbol_toolbar + m.gap` circle; image, else first letter on `c.selected_fill`; AX `Account: {name}` | below |

Menu: disabled header `{name}` with `@{handle}` as a second line → separator →
`Choose Handle…` (no handle only) → `Your Library` → `Account Settings…` →
separator → `Sign Out`. `Remember Installs on This Account` stays in Settings.
The same items are in `File ▸ Account` for keyboard users.

`Sign Out` asks nothing. Failure: alert `Couldn’t sign out.`, informative
`Try again.`, button `OK`. A failed avatar image falls back to the letter with no
error.

## S086 Delete account and registry switch

Both are system alerts in v1.2.0 and stay so.

**Delete Account** — critical, sheet on the Settings window:

| Part | Copy |
|---|---|
| Message | `Delete your Powermove account?` |
| Informative | `Your published extensions stay on the Store under your handle. This can’t be undone.` |
| Buttons | `Cancel` (default) · `Delete Account` (destructive) |

While deleting the row shows a spinner and `Deleting…`. Failure: alert
`Couldn’t delete your account.`, `ServiceErrorCode` informative, `OK`. Success:
signed-out state plus the inline notice `Your account was deleted.`

**Registry switch** — critical when moving away from Powermove Cloud:

| Part | To a custom registry | Back to Powermove Cloud |
|---|---|---|
| Message | `Use the registry at {origin}?` | `Use Powermove Cloud again?` |
| Informative | `You’ll be signed out. The Store will show extensions from this registry, and signing in sends your account details to it. Only use a registry you trust.` | `You’ll be signed out, and the Store will show extensions from Powermove Cloud.` |
| Buttons | `Cancel` (default) · `Use This Registry` | `Use Powermove Cloud` (default) · `Cancel` |

Going back is safe, so its action is the default; the move away is not. An
invalid origin never reaches the alert: `pm-services` validation shows the Error
state on the entry field. A registry fixed by the environment shows the row
disabled with detail `Set by this Mac’s environment.`

## Don't

- Don't add a rename control v1.2.0 doesn't have, or say deletion is permanent.
- Don't show tokens, API keys, raw auth URLs or transport text outside `Details`.
- Don't port the ghost field, ripple or ink canvases.
- Don't sign the user out or switch registry without the alert.
- Don't build S066 before the owner decides.
