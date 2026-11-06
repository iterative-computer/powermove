# Native first launch, consent and outcomes

Rows N001 N002 N003. Grammar: [README](README.md) §1–5. Contract:
[first-launch.md](../../first-launch.md), model `pm-first-launch::{run, Event, Outcome,
Step, secrets}`. These surfaces exist only in native. Everything here is written for a
person who is anxious about their work, so every screen says what's safe.

Naming: the existing app is **the original app** or `Powermove`; the beta is `Powermove
Native`.

No new tokens. Reuses the Welcome window frame (S107).

## The gate

The first-launch gate is the Welcome window frame, shown before any library opens,
because the shell keeps the library closed until the coordinator returns. Layout: mark,
`t.display` title, `t.reading` `c.secondary` line, status block, footer buttons trailing,
with a leading `Quit Powermove Native` wherever waiting is possible. Closing the window
quits the app. Quitting at any point is safe, and the next launch resumes.

## N001 Waiting and copying

| `Event` / state | Title | Line | Status block | Buttons |
|---|---|---|---|---|
| `WaitingForElectron`, local | `Quit Powermove to Continue` | `Powermove Native copies your library while the original app is closed. Save your work there and quit it.` | spinner + `Waiting for Powermove to quit…` | none; continues by itself |
| Owner unknown or on another Mac | `Powermove Might Still Be Open` | `It looks like the original app is using this library, possibly on another Mac. Close it there, then check again.` | | `Check Again` (default) |
| Checkpoint | `Copying Your Library` | `Your original app stays unchanged, and this copy never replaces it.` | step + `ProgressRow` | `Pause` |
| Interrupted, resumed | same | same | `Picking up where it stopped…` then the step | `Pause` |
| Paused | `Copying Is Paused` | `It picks up where it stopped. You can quit and come back.` | step, frozen `ProgressRow` | `Resume` (default) |
| Low disk | `Not Enough Space` | `Powermove Native needs {needed} free to copy your library, and this Mac has {available}. Nothing was deleted.` | | `Check Again` (default) |
| Failed | `Copying Stopped` | `Your original app and library weren’t changed. {ServiceErrorCode informative}` | `Details` disclosure | `Try Again` (default) |
| Profile refused | `Powermove Native Can’t Start` | `Its library folder isn’t usable. Your original app wasn’t changed.` | `Details` disclosure | `Quit Powermove Native` |

**Steps**, from `Event::Started` and `Progress`. Counts come straight from the event, so
no percentage is invented, and a step with no total is indeterminate:

| `Step` | Status |
|---|---|
| `Checkpoint` | `Taking a snapshot of your library…` |
| `Bootstrap` | `Preparing your library…` |
| `Library` | `Copying projects · {done} of {total}` |
| `Media` | `Copying media · {done} of {total}` |
| `LocalStorage` | `Copying settings…` |
| `Extensions` | `Copying extensions…` |
| `Secrets` | N002 |

Names and paths never appear here: the event carries counts only.

`Details` shows selectable `MONO_FONT` text and goes to the Error Log. No secret or user
path is included.

**Normal path.** With nothing to review, `Your Library Is Ready` is skipped and the editor
opens by itself. No click is needed unless encrypted values exist (N002) or a row below
has something to say.

**Later launches.** An unchanged original costs a metadata check. Otherwise its changes
come in before the library attaches, with no gate; if that passes one second, the Welcome
frame shows `Updating Your Library`, `Your original app stays unchanged.`, spinner +
`Bringing in your recent work…` and `Quit Powermove Native`. New media then lands behind
the open library. If the original app is running, the delta waits silently for a later
launch.

VoiceOver: the status block is a polite live region, throttled to one update per 5 s.
Title gets focus on each state change. Return activates the default.

## N002 Keychain consent

Appears only when encrypted values exist. The app never touches Keychain before the
person chooses to.

| State | Title | Line | Buttons |
|---|---|---|---|
| `NeedsSecretConsent{interrupted: false}` | `Keep Your Sign-Ins` | `secrets::PROMPT` verbatim: `Powermove Native can keep your extension secrets and sign-ins. Continue to allow one macOS Keychain request for Powermove Safe Storage. Your original app stays unchanged.` | `Not Now`, `Continue` (default) |
| Waiting for the system | same | same; status `Waiting for macOS…` | disabled |
| Denied or cancelled | `Keychain Access Wasn’t Allowed` | `Your sign-ins and extension values stay in the original app. You can sign in again here, or try again.` | `Continue Without Them` (default), `Try Again` |
| `interrupted: true` | `The Keychain Request Didn’t Finish` | `Powermove Native stopped before macOS answered. Continuing asks macOS one more time. Your original app stays unchanged.` | `Not Now`, `Continue` (default) |
| Partly imported | no extra screen; N003 lists the rest | | |

`Continue` here is deliberately the default: it opens a system prompt, which is the
real decision, and the person answers there. The macOS Keychain alert is system-owned
and must not be imitated. Retries are always a fresh click, never automatic. `Not Now`
and `Continue Without Them` finish migration without secrets: S070 and S083 then show
signed-out states, and S011 shows *Needs re-entry*.

**Model gap for the lane:** `pm-first-launch` has `resume_with_secret_consent` but no
explicit decline. A lane needs one (`skip_secret_consent`) so `Not Now` can reach
`Complete`, and a later retry entry, Settings ▸ Accounts row `Bring Sign-Ins from the
Original App…`. Until it lands the lane stops at `DESIGN-TODO(opus)`: it may not hide
`Not Now`.

Never show a decrypted value, an item name beyond the prompt, or a count of secrets.

## N003 Outcomes

From `Outcome::Complete{projects, history_refusals, missing_projects, blocked_owners,
proxy_only, already_offline}`.

**Summary** — last gate screen. Title `Your Library Is Ready`, line `{n} projects were
copied. Your original app stays unchanged.` If all counts but `projects` are zero there
are no rows. Otherwise a grouped card with one row per non-zero count, omitting others:

| Count | Symbol | Row |
|---|---|---|
| `history_refusals` | `exclamationmark.triangle.fill` `c.warning` | `{n} projects came across without their Undo history. Their current state is complete.` |
| `missing_projects` | `c.warning` | `{n} projects couldn’t be read. They’re kept safely and weren’t changed.` |
| `blocked_owners` | `xmark.octagon.fill` `c.danger` | `{n} projects need media that went missing. They’ll ask you to find it.` |
| `proxy_only` | `info.circle` `c.secondary` | `{n} media files came across as proxies only. Exports using them show a warning until you relink the originals.` |
| `already_offline` | `info.circle` `c.secondary` | `{n} media files were already offline.` |

Buttons: `Show Report…` (with rows), `Open Powermove Native` (default). Onboarding is
skipped if the source completed it.

**Migration Report** — `Window ▸ Migration Report`, a standard window like the Error Log
(S053), available any time after migration.

| Section | Rows | Action |
|---|---|---|
| `Projects Without Undo History` | project name | `Open` |
| `Projects That Couldn’t Be Read` | project name | `Show in Finder` |
| `Waiting for Media` | project, count | `Relink…` |
| `Proxy-Only Media` | file, project | `Relink…` |
| `Kept Aside` | quarantined item, reason `Powermove couldn’t recognise this.` | `Show in Finder` |
| `Conflicts` | key or file; `Both versions were kept.` | `Show in Finder` |
| `Sign-Ins Not Copied` | account or extension display name only | `Set Up…` |

Footer: `Copy Report` (no paths beyond the Mac's own, no secrets), `Show Library in
Finder`. Empty section: omitted. Everything empty: `EmptyState` `checkmark.circle`,
`Nothing to Review`, `Everything came across.`

**Opening a blocked project** — alert: `Media for “{project}” is missing.` / `Powermove
Native can’t open this copy until the media is found. The original project wasn’t
changed.` / `Relink…` (default) · `Cancel`. Offline projects aren't blocked: they open
with S006's offline treatment.

**Proxy-only export** — Export sheet warning line, `c.warning` with symbol: `This uses
proxy-only media, so quality may be lower. Relink the originals for full quality.` Export
stays enabled.

**Conflict review.** Three-way merge isn't built, so conflicts are reported and both
sides kept. No merge UI ships. The row's action stays `Show in Finder`.

**Going back.** There is no reverse migration (owner decision, 2026-11-04), and no copy
may suggest one. Native never touches the original library, so the original app keeps
everything as it was at the copy. Work done in Native doesn't move back. Native's external
`.pmv` saves stay rollback-compatible by default, but a file using something the original
app can't read (new 3D, for one) won't open there.

Report footer adds `About Going Back…`, an alert:

| Part | Copy |
|---|---|
| Message | `Your original app still has everything from before.` |
| Informative | `Work you do in Powermove Native doesn’t move back to it. It can open a .pmv saved here only if it can read everything in the file. If it can’t, it refuses the file, and the file isn’t changed.` |
| Button | `OK` |

**Refusal in the original app.** This alert belongs to the Electron transition release, not
to Native. The original app checks a `.pmv` before opening it and refuses a file it can't
read fully; it never opens part of one, never migrates it and never writes to it.

| Part | Copy |
|---|---|
| Message | `“{file}” needs a newer version of Powermove.` |
| Informative | `This version can’t read it, so it wasn’t opened. The file wasn’t changed.` |
| Button | `OK` |

Project-library rows are unaffected, because Native's library is a separate copy.

## Don't

- Don't use a percentage the model doesn't report.
- Don't imitate the Keychain alert, or read Keychain without a fresh click.
- Don't show names or paths on the gate.
- Don't describe the copy as a move.
- Don't promise that work can go back, or that every `.pmv` from Native opens in the
  original app.
