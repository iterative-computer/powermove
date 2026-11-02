# Authority — agent access, extension values, removal

Rows S010 S011 S012. Grammar: [README](README.md) §1–4. Consistent with
[panels/agent-panel.md](../panels/agent-panel.md) (inline approvals, Access
picker) and [reviews/na-cloud-storehost.md](../reviews/na-cloud-storehost.md)
(Full Access alert, permission sheet). Authority prompts are **never**
remembered silently, never default to granting, and never appear without a
user action or agent request behind them.

## S010 Agent computer access and standing Full Access

Ports `M/codex/consent.ts` (copy verbatim, button order per HIG). Model:
`pm-agents::approvals`, `pm-panel-agentpanel` access commands.

**Standing Full Access** (choosing `Full access` in the Access picker) —
alert, **critical**, sheet on the editor window (the popover closes first):

| Part | Copy |
|---|---|
| Message | `Give Powermove agents full access to your Mac?` |
| Informative | `Agents will run commands and change files anywhere on this Mac without asking, until you choose another access level or quit Powermove.` |
| Buttons | `Cancel` (default, Esc) · `Allow Full Access` |

**Per-project computer access** (an agent run needs access outside the
composition and no standing grant exists) — alert, warning style, sheet on
the project's window:

| Part | Copy |
|---|---|
| Message | `Allow computer access for “{project}”?` |
| Informative | the request summary from the agent (≤ 3 lines, plain text; longer text truncates with `…` and the full text goes to the run's activity row) |
| Buttons | `Cancel` (default) · `Allow This Run` |

States: while either alert is up the agent shows its `Awaiting answer`
state; `Cancel` returns `granted: false` and the run shows the Approval card's
denied row. Standing grant visible as the Access pop-up value `Full access`
with `exclamationmark.triangle.fill` `c.warning`; it ends on quit (shipped
behaviour) — the next launch starts at the previous non-Full level.

Don't: put either decision inline in the conversation; make `Allow` the
default; add "Don't ask again" to the per-project alert.

## S011 Extension values — setup, secrets, missing values

Ports `R/vars/SetupSheet.svelte`, `R/vars/setup.ts`, `M/env/ipc.ts`.

**Set Up sheet** — opened from Settings ▸ Extensions `Set Up…`, from a
panel's missing-values state, or when an extension asks (`requestValues`):

| Element | Spec |
|---|---|
| Title | `Set Up {extension}` |
| Line | `{extension} needs these values to work.` (only when one is required and missing) |
| Rows | label = declared label (`t.body` `c.secondary`); value column `m.sheet_value_column`: plain → `NSTextField`; secret → `NSSecureTextField` with trailing borderless `eye` button (tooltip `Show {label}` / `Hide {label}`). Optional rows add `t.minimum` `c.muted` `Optional` under the label. Declared help text appears as `t.minimum` `c.muted` under the field. |
| Already set (secret) | value column shows `••••` (AX `{label} is set`) and three borderless buttons `Reveal`, `Change`, `Remove` |
| Needs re-entry (Keychain item unreadable) | warning line under the row `Enter this again; the saved value can’t be read.` and an empty field |
| Note | `t.minimum` `c.muted` at the bottom: `Stays on this Mac. Never included when you publish or share this extension.` |
| Buttons | `Cancel` · `Save` (disabled until every required value is present; label `Saving…` while writing) |
| Error | write failed → Error line above the footer `Couldn’t save these values. {ServiceErrorCode informative}` |

**Reveal** — alert: message `{label}`, informative = the value in full
(the user asked to see it), buttons `Done` (default) · `Copy`. Copy writes to the pasteboard with
`NSPasteboard` concealed type.

**Remove** a single value — no confirmation (re-enterable); the row becomes
empty.

**Missing required values** in a running panel: the panel content is the
`EmptyState` `Set Up {Extension}`, line `{extension} needs a few values before it can run.`, button `Set Up…`.

**Agent-requested secret** stays the Agent spec's inline secret question card;
it never opens this sheet.

Don't: show secrets in logs, tooltips or the Error Log; prefill a secret field
with a placeholder that looks like a value; store values in the project.

## S012 Removal confirmations

**Uninstall an extension that has saved values** — after the uninstall
choice (Settings ▸ Extensions `Uninstall…` or Store), alert:

| Part | Copy |
|---|---|
| Message | `Also delete the values you entered for {extension}?` |
| Informative | `If you keep them, they’re used again when you add this extension back.` |
| Buttons | `Keep` (default) · `Delete` (destructive) |

The uninstall itself is an archived release (`ArchivedRelease`,
`ArchiveReason`), so it asks nothing else. `Delete` removes the Keychain
items (secrets are the one thing N4 archiving does not keep). An extension
without saved values uninstalls with no alert.

**Delete a saved workspace / layout** (`WorkspacesPanel`, Window ▸
Workspace) — alert (shipped confirmation kept): message
`Delete the workspace “{name}”?`, informative
`Your panels stay as they are. You can undo this.`, buttons `Delete`
(destructive) · `Cancel` (default). `Edit ▸ Undo Delete Workspace` restores
it; `LayoutStore` keeps the deleted JSON archived (N4).

**Delete a local (user-made) panel or extension project in the Library
screen**: same as a project (library.md S005): `Move to Trash` alert, then
`Delete Immediately…` critical alert in Recently Deleted.
