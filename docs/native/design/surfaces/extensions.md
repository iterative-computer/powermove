# Native: bringing in the original app's extensions

Rows N004 N005 N006. Grammar: [README](README.md) §1–5, and the gate in
[migration.md](migration.md). Model `pm-first-launch::surfaces::extensions`, engine
`pm-first-launch::extension_transfer` (scan, declined, run with progress, result with
reasons). App: `crates/app/src/extension_transfer.rs`, `first_run.rs`, `storage.rs`.

No new tokens. Reuses the Welcome window frame (S107) with an eyebrow,
`Extensions From the Original App`.

## When it shows

At startup, on a library that already exists (made here, or copied earlier), when
the original app has extensions the library doesn't have, or has changed since they
were brought in. The look is read-only and happens before any window: when there is
nothing to offer, nothing shows and the editor opens as before. A first launch that
copies the whole library brings extensions in itself, so this never follows it, and
it never interrupts onboarding.

It copies only extension folders and their receipts. Projects, settings and sign-ins
are never read for writing.

## N004 Offer

| | |
|---|---|
| Title | `Bring In Your Extensions` |
| Line | `The original app has {n} extensions that aren’t in Powermove yet. Only extensions are copied. Your projects, settings and sign-ins stay as they are.` |
| Card | counts, rows only when non-zero: `New`, `Updated in the original app`, `Already in Powermove`, `Skipped` |
| List | each skipped extension, `{id}: {reason}`, `info.circle`, `c.secondary` |
| Note | Startup: `Choose Not Now to open your library as it is. You can bring them in later from Settings › Storage.` |
| Buttons | Startup: `Not Now`, `Bring In Extensions` (default). Settings: `Cancel`, `Bring In Extensions` (default; disabled when nothing is new) |

**Not Now** is remembered (`extension-import/preferences.json`) and opens the library
at once. Later launches don't look again. Bringing extensions in, from anywhere,
clears it, so something added to the original app later is offered at startup.

## N005 Bringing in

Title `Bringing In Your Extensions`, the same line. One card row with the running bar:
`Copying` then `Checking`, `{at} of {count}`. The engine copies every extension, then
checks each one loads in a throwaway runtime (never the open project), with its
declared dependencies installed first. Status: `Copying {id}…`, `Checking that {id}
loads…`. No buttons: it is quick, and quitting is safe.

## N006 Summary

| Result | Title |
|---|---|
| Something brought in | `Your Extensions Are In` |
| Nothing new | `Nothing New to Bring In` |
| Only failures | `Your Extensions Couldn’t Load` |
| Stopped (folder unreadable, locked) | `Your Extensions Couldn’t Be Brought In` |

Line: `{n} brought in, {n} already in Powermove and {n} couldn’t load. Your original app
stays unchanged.` (parts that are zero are left out). Card: `Brought in`, `Already in
Powermove`, `Couldn’t load`, `Skipped`. List: every extension with its reason,
problems first: couldn't load (`exclamationmark.triangle.fill`, `c.warning`), skipped
and already here (`info.circle`), brought in (`checkmark.circle.fill`, `c.success`).

The list scrolls inside its card, which takes the room left under the counts; each
line wraps inside the card at any width, the symbol stays on line one, and the
buttons always stay on screen.

Buttons: Startup `Open Powermove` (default). Settings `Done`. Stopped: Startup `Try
Again`, `Open Powermove` (default), with Details; Settings `Close`, `Try Again`.

## Settings › Storage

In the `Original App` group, after `Space Used`:

- New or changed extensions: `Extensions`, value `{n} not in Powermove`, detail
  `Extensions you made or installed in the original app. Bringing them in copies only
  the extensions. Your projects, settings and sign-ins stay as they are.`, button
  `Bring In Extensions…`. Pressing it opens N004–N006 in their own window, titled
  `Bring In Extensions`, which closes (never quits). Afterwards every window rescans
  its extensions and Storage measures again.
- All brought in: `Extensions`, `All in Powermove`, no button.
