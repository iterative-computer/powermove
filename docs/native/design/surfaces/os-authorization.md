# OS authorization and notifications

Rows S114 N007. Grammar: [README](README.md) §1–5. Sources:
`M/agent-notifications.ts`, `R/panels/agent/notification-preferences.ts`,
`S/agent-notifications.ts`, `M/permissions.ts`, [os-base.md](../../os-base.md). System
prompts stay system-owned: Powermove words only the usage strings and what happens
after an answer. The Keychain outcome is N002 in [migration.md](migration.md); agent
computer access is S010. No new tokens.

## S114 Notifications

`UNUserNotificationCenter`, category per event. v1.2.0 notifies on two agent events and
always plays a sound.

| Event | Title | Body | Click |
|---|---|---|---|
| Agent asked a question | `Your agent has a question` | the question, plain text, system-truncated | bring the thread's window forward, open the agent to that thread, focus the question card |
| Agent finished | `Powermove` | `Your agent has finished. Your result is ready.` | bring the window forward, open the agent to that thread |

Both play the bundled sound `Little Victory (Deep)` as the notification's own sound, so
Focus and notification settings silence it too. v1.2.0 played it through a separate
process and ignored those settings.

| Case | Spec |
|---|---|
| Powermove frontmost, agent visible | no banner, no sound: the card is already on screen |
| Powermove frontmost, agent hidden | banner, via `willPresent`; toolbar dot still shows |
| Powermove in background | banner and sound |
| Same thread again | a new notification replaces the old one (thread id as identifier) |
| Run stopped by you | none |
| Thread deleted before click | opens the agent on the current thread |

**Asking.** Authorization is requested the first time a run is sent, not at launch, so
the request arrives with a reason. No pre-prompt.

**Outcome rows** — a new row in Settings ▸ General, group `Notifications`:

| Authorization | Row `Agent Notifications` detail | Control |
|---|---|---|
| Allowed | `Powermove tells you when your agent finishes or has a question.` | status `Allowed` |
| Not asked yet | same | status `Not Set Up` |
| Denied | `Notifications are off for Powermove in System Settings.` | `Open System Settings…` |
| Provisional | treated as Allowed | status `Allowed` |

Denied changes nothing else: toolbar dot, card and thread states still work, and no
alert ever appears for it. v1.2.0's shared list of other sounds has no UI, so no sound
picker is added.

VoiceOver: system. Notification text carries no secrets, and the body contains only
the question the agent wrote.

## N007 OS authorization outcomes

v1.2.0 asks for no camera, microphone or screen permission, and native adds none.
Chromium's `local-fonts` and `pointerLock` grants have no native equivalent to present:
CoreText and pointer association need no authorization.

| Capability | System surface | Outcome handling |
|---|---|---|
| Open or import a file | `NSOpenPanel` | a chosen file carries its access; nothing to word |
| Reopen a recent or project-linked file | security-scoped bookmark | stale or denied bookmark: S006 Relink for media; S004 failed open for projects |
| Save or export to a folder | `NSSavePanel` | S003 or Export error: `Powermove doesn’t have permission to save here. Choose another location.` |
| Documents, Desktop, Downloads access by path | TCC prompt, once | denied: same line as the owning surface, plus `Open System Settings…` when the system can show the pane |
| Removable and network volumes | TCC prompt | same |
| Agent shell or file work | TCC prompt arrives on behalf of Powermove | agent gets `File Access Was Denied` (S064); access mode doesn't change |
| Colour eyedropper | `NSColorSampler` | needs no permission; Esc cancels and nothing changes |
| Region capture (S066) | own window only | none |
| Local model server | Local Network prompt | denied: S071 Failed, plus detail `Allow Local Network for Powermove in System Settings.` |
| Notifications | above | above |
| Keychain | system alert | N002 and S011 *Needs re-entry* |
| Extension requests camera, microphone, screen | refused by the host | no system prompt; the extension's call fails; Error Log entry `{extension} asked for {capability}, which Powermove doesn’t allow.` |

**Usage strings** (Info.plist), complete list:

| Key | Copy |
|---|---|
| `NSDocumentsFolderUsageDescription` | `Powermove opens projects and media you keep in Documents.` |
| `NSDesktopFolderUsageDescription` | `Powermove opens projects and media you keep on the Desktop.` |
| `NSDownloadsFolderUsageDescription` | `Powermove opens projects and media you keep in Downloads.` |
| `NSRemovableVolumesUsageDescription` | `Powermove opens projects and media on external drives.` |
| `NSNetworkVolumesUsageDescription` | `Powermove opens projects and media on network drives.` |
| `NSLocalNetworkUsageDescription` | `Powermove connects to the local model server you set up.` |

[os-base.md](../../os-base.md) still lists placeholder `NSCameraUsageDescription` and
`NSMicrophoneUsageDescription` strings. Nothing shipped uses them, so they're removed.

**Shared recovery pattern.** One line at the failure, in the surface that failed, then
one action. Nothing pops up on its own and nothing retries in a loop. `Open System
Settings…` deep-links to the matching Privacy & Security pane. After the user returns,
the failed action isn't repeated automatically: they repeat it.

macOS verification: TCC, Keychain and Local Network behaviour must be checked on a
signed build; the table states intent.

## Don't

- Don't pre-prompt, or draw a fake permission dialog.
- Don't ask for a permission before the feature that needs it.
- Don't let an extension trigger a system prompt.
