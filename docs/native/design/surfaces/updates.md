# Updates, What's New and About

Rows S111 S112 S113. Grammar: [README](README.md) §1–5. Sources: `M/updates.ts`,
`R/shell/{app-updates.ts,WhatsNew.svelte,whats-new.ts}`, `M/whats-new.ts`, `M/menu.ts`.
The update sheet from [reviews/na-cloud-services.md](../reviews/na-cloud-services.md)
stands; this file adds every state around it. The native update route is intentional
and doesn't imitate Electron's. No new tokens.

## S111 App updates

Menu: `Powermove ▸ Check for Updates…`. Automatic checks are silent unless an update
is ready. Only a manual check reports *checking*, *up to date* and *couldn't check*,
as v1.2.0.

| State | Surface | Copy |
|---|---|---|
| Checking (manual) | small titled window `Software Update`, `m.sheet_width` wide: spinner + line, `Cancel` | `Checking for updates…` |
| Up to date | alert | `Powermove is up to date.` / `You’re running {version}.` / `OK` |
| Available | update sheet on the frontmost window (standalone window if none) | title `A New Version of Powermove Is Available`; line `{name} is now available. You have {version}.`; notes in `t.reading`, scrolling, height `m.palette_height`; links open the browser; `Install Update` (default), `Remind Me Later`, `Skip This Version` at the leading edge |
| Notes missing or unreadable | same sheet | `Fixes and improvements.` |
| Downloading | `Software Update` window | `Downloading {name}…`, `ProgressRow` determinate with `{done} of {total}`, `Cancel` |
| Verifying | same window | `Verifying…`, spinner, no Cancel |
| Ready | see below | |
| Check again while busy | alert | `An update is already in progress.` / `Powermove will let you know when it’s ready.` / `OK` |
| Couldn't check | alert | `Couldn’t check for updates.` / `Try again later. You can keep using Powermove.` / `OK` |
| Download failed | alert | `Couldn’t download the update.` / `Try again later. You can keep using Powermove.` / `OK` |
| Verification failed | alert, critical | `This update couldn’t be verified, so it wasn’t installed.` / `Try again later. You can keep using Powermove.` / `OK` |
| Offline, manual | alert | `Couldn’t check for updates.` / `Check your internet connection, then try again.` / `OK` |

**Ready.** v1.2.0 shows a toast with a per-version dismissal plus a persistent home
sidebar item.

| Place | Spec |
|---|---|
| Editor | viewer banner with close button: `Powermove {version} is ready to install`, action `Restart to Update`. Dismissal is remembered for that version |
| Project Library | inline notice, same copy and action; persists after the banner is dismissed |
| Menu | `Check for Updates…` becomes `Restart to Install Update…` |
| Manual check while ready | alert `An update is ready.` / `Quit Powermove normally to install it. Your session will be saved before quitting.` / `OK` |
| Next quit | installs silently after S002's save and quit barrier |

`Restart to Update` runs the normal quit path first, so S002 owns unsaved work and an
active export. Cancelling that path cancels the restart and leaves Ready in place. The
quit barrier never installs while a save or export is running.

Keyboard: alerts and sheets per grammar. The `Software Update` window closes with ⌘W,
which cancels. VoiceOver: progress is `ProgressRow`; `Ready` posts `.medium`.

## S112 What's New

Shown once after an update changed the version, as v1.2.0: on the first project window,
never during onboarding, N001, a recovery alert or another sheet. It waits for the next
free moment in that session; it isn't dropped.

Wide sheet (`m.sheet_width_wide`), title `What’s New in Powermove {version}`.

| Element | Spec |
|---|---|
| Line | `Powermove updated from {previous} to {current}.` |
| Releases | newest first, every release between the two versions. Each: name `t.title`, date `t.minimum` `c.muted` (`November 4, 2026` in the system's long format, omitted if unknown), notes as Markdown in `t.reading`, agent-panel Markdown recipe, no remote images. Empty notes: `Fixes and improvements.` |
| Height | scrolls inside, max window height − 2 × `m.sheet_inset × 2` |
| Footer | leading `View on GitHub`; trailing `Continue` (default) |

| State | Spec |
|---|---|
| Seen marker | written when the sheet is **shown**, so it never returns for that version even if dismissed by quit |
| Notes unavailable, offline | nothing is shown and the marker stays, so the next launch retries |
| Several windows | one window only |
| Skipped versions | shown together on the next update |

v1.2.0 has no menu entry for it; none is added. Esc and Return both continue.

## S113 About

`Powermove ▸ About Powermove` opens the standard panel
(`orderFrontStandardAboutPanel`), not a custom window.

| Field | Value |
|---|---|
| Icon, name | bundle values; the menu name is **Powermove** |
| Version | `Version {marketing version} ({build})` |
| Copyright | `NSHumanReadableCopyright`: the shipped string |
| Credits | `Credits.rtf`, system-rendered in its scroll area: a short line of thanks, then the bundled font and icon licence notices, in the same order as the v1.2.0 resource notices. Third-party Rust and JS notices come from the generated notices file. Links open the browser |

Light, dark and Increase Contrast come from the system panel. The RTF uses system
label colour, never fixed black, so it reads in dark mode. VoiceOver is system.

## Don't

- Don't show *checking* or *up to date* for automatic checks.
- Don't install during a save or export.
- Don't draw a custom About window or put marketing in Credits.
