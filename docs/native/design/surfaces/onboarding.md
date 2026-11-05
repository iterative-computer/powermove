# Onboarding

Rows S107 S108 S109 S110. Grammar: [README](README.md) §1–5. Sources:
`M/onboarding.ts`, `R/onboarding/{welcome.ts,TimelineStep,AgentStep,TranscriptionStep,
WorkspaceStep}.svelte`, `R/onboarding/{workspace-import.ts,SuitcaseTowers.svelte}`,
`apps/desktop/src/renderer/onboarding/welcome.html`.

Order is fixed, as v1.2.0: Welcome → Timeline → Agent → Speech Model → Workspace.
Only the last step begins. Native first launch (N001) runs before all of this; if the
profile was migrated, onboarding records come across and these steps don't appear.

No new tokens.

## S107 First run and Welcome window

v1.2.0 plays a full-screen overlay animation of about 9.8 s, then opens a 900 × 640
welcome window. Native keeps the moment but not the takeover (owner decision,
2026-11-04): the animation plays **inside** the Welcome window, which needs no
transparent screen-sized overlay, and it can't hide other apps. The overlay isn't
ported: no borderless window, no screen-sized surface, and the Welcome window is on
screen from the first frame.

| Part | Spec |
|---|---|
| Window | standard titled window, full-size content, title hidden, close button only, centred on the active display. Size `m.welcome_width` × `m.welcome_height` (below), matching v1.2.0's 900 × 640 |
| Ground | solid `c.window`, no card; content centred, max width `m.sheet_width_wide` |
| Mark | Powermove mark; the v1.2.0 animation plays once, 9.8 s, on `c.window`, clipped to the window's content area and scaled to fit it. Title and button sit above the animation and are visible throughout |
| Title | `t.display`, `Welcome to Powermove.` on two lines as v1.2.0 |
| Button | default accent `Begin`, enabled from the first frame. Return or a click skips the animation |
| Reduce Motion | no animation; the final frame shows at once with `mo.reduced` cross-fade |
| Progress | no step dots: v1.2.0 has none and the Back button already orients |

Step anatomy (S108–S110): `chevron.left` `Back` borderless top-leading (AX as listed),
`t.display` title, `t.reading` `c.secondary` intro on two lines, choice area, primary
default button, optional secondary below it. On each step the title receives focus,
so VoiceOver reads it first.

| State | Spec |
|---|---|
| Window closed mid-flow | nothing is saved and onboarding runs next launch, as v1.2.0 (the marker is written only on completion) |
| Completion | Welcome window closes, the editor opens, focus goes to it; no animation |
| Revisit | no menu entry: v1.2.0 has none. Each choice stays changeable where the step says |
| Light and dark | follows the system appearance, read once on open and live afterwards |

Two new tokens for the pm-design lane:

| Token | Value | Role |
|---|---|---|
| `metrics.welcome_width` | 900 | Welcome window width |
| `metrics.welcome_height` | 640 | Welcome window height |

## S108 Timeline style

Title `Pick Your Timeline.` Intro `Work the way you already do.` / `You can switch
anytime from the timeline panel.` Back AX `Back to Welcome`.

Two choice cards side by side, each a radio in a group labelled `Timeline style`, each
`m.group_radius`, `c.panel`, with a 2 pt `c.accent` ring when selected and `Layers` or
`Tracks` in `t.title` below.

| Choice | Illustration rows (timeline clip recipe, label colours, not interactive) |
|---|---|
| `Layers` | `1  Title`, `2  Lower third`, `3  B-roll`, `4  Interview`, `5  Music` |
| `Tracks` | `V2`: Title, Lower third · `V1`: Interview, B-roll · `A1`: Music |

Illustrations are static. Default `Layers`. Button `Continue`. ←/→ change the choice;
Return continues. AX: `{name}, timeline style`, illustration hidden from VoiceOver.

## S109 Agent and speech model

**Agent**. Title `Pick Your Agent.` Intro `It builds what you ask for, right in the
editor.` / `You can switch anytime from the agent’s composer.` Back AX `Back to
Timeline Choice`.

Radio rows (`m.row × 2`), Settings grouped-form card:

| Row | Detail | Trailing |
|---|---|---|
| `ChatGPT` | `Uses your ChatGPT plan through Codex.` | model pop-up, AX `ChatGPT model` |
| `Claude` | `Uses your Claude plan through Claude Code.` | model pop-up, AX `Claude model` |
| `API or Local Model` | `Any OpenAI-compatible API, Ollama or LM Studio. Connect it in Settings.` | none |

Focusing a model pop-up selects its row. Choosing doesn't sign in: the S059 gate does
that on first use. Button `Continue`.

**Speech Model**. Title `Pick Your Speech Model.` Intro `Captions and your agent can
transcribe speech on this Mac.` / `It’s optional, and the download runs while you
work.` Back AX `Back to Agent Choice`.

| State | Spec |
|---|---|
| Models listed | S091 row recipe without meters; `Recommended` tag. Primary `Download`, or `Continue` when the selection is ready or already downloading. Secondary `Not Now` |
| Models unavailable | line `Speech models can be downloaded later from Settings.`; only `Continue` |
| Start failed | Error under the list `Couldn’t start the download. You can download it later in Settings.`; `Continue` stays available |
| Note | `Runs on this Mac. Your audio is never uploaded. More models are in Settings.` |

The download outlives onboarding and appears in Background Tasks.

## S110 Bring My Workspace

Step and General Settings entry (`Bring My Workspace…`). Title `Grab Your Suitcase.`
Intro `Bring your panels and tools along.` / `Your agent will unpack them in
Powermove.` Above the title, a source line with the text `After Effects` (AX `From
After Effects`). v1.2.0's small `Ae` badge is dropped so no third-party mark is
imitated. The decorative suitcase towers are not ported: they're a canvas animation
that says nothing, and language §1 asks for quiet. Back AX `Back to Speech Model
Choice`.

| Control | Copy |
|---|---|
| Primary | `Bring My Workspace` |
| Secondary | `Start Fresh` |
| Busy | status `t.body` `c.secondary`, spinner: `Preparing your workspace agent…` / `Opening Powermove…`; both buttons and Back disable |
| Failed | Error `Couldn’t open your workspace. Try again, or start fresh.`; controls return |

**Handoff** (`workspace-import.ts`): the choice is an opt-in held until an agent is
connected.

| Step | Surface |
|---|---|
| Editor opens | a blank project if none is open; agent opens in app context, new thread, scope Workspace |
| Not connected | composer holds the draft `Bring my After Effects workspace into Powermove.`, S059 gate shows; the request is **not** sent until connected, and never if the draft was edited |
| Connected | the draft sends itself once |
| Agent needs folder access | normal approval card, then the system open panel (N007) |
| Scanning, unpacking | normal activity rows in the conversation |
| Finished | S063 panel results; imported items appear under Panel Library ▸ After Effects |
| Nothing found | agent reports it in plain text; no alert |
| From Settings | identical, without the onboarding steps. Failure is the inline notice `Couldn’t start the workspace transfer. Try again.` |

Keeping this agent-driven means no bespoke chooser exists. v1.2.0 lists only After
Effects; no other source app is added.

## Don't

- Don't cover other apps with a screen-sized overlay.
- Don't sign in, download or import before the user's explicit choice.
- Don't add step dots, tips or a tour.
- Don't send the import draft if it was edited.
