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

## Layout (all three screens)

Same window and size as the gate (`welcome_width` × `welcome_height`). The step's
screens are anchored at the top (`Frame::top`): the title sits at the same height on
every screen, `toolbar` below the window's top edge, and the buttons sit at
the bottom, `sheet_inset + inset` above it, so nothing jumps between screens. The
content column is `sheet_width_wide`.

Rhythm: eyebrow, title and line are one header (8 pt apart, the line at 1.45 leading);
then `space.xxl` (24 pt) to the counts, `space.xxl` to the lists, `space.xl` (16 pt)
to the note and the buttons. Each screen leads with the outcome, shows the exceptions
in full, and lists the rest by name only.

- **Counts**: one card, the counts side by side, each figure at `display` size with its
  label under it in `c.secondary`; hairline dividers between counts only.
- **Exceptions**: a heading (`Needs a Look` when something couldn't load, else `Not
  Brought In`), then one card. Each row: symbol centred on the first line, the
  extension's display name, its reason on the next line in `c.secondary`. Rows are
  separated by air (6 pt padding top and bottom), not hairlines.
- **Names**: a heading (`To Bring In` / `Brought In`), then the display names (the
  manifest's `name`, else the folder's) in two columns, read down then across. Six show
  first; `Show All {n}` (accent) opens the rest, `Show Less` closes them.
- The exceptions and names scroll together in the room between the counts and the
  buttons. When there's more below, a fade (`form_row` high, window colour) sits at the
  bottom of that room; rows are never cut off without it.

## N004 Offer

| | |
|---|---|
| Title | `Bring In Your Extensions` |
| Line | `The original app has {n} extensions that aren’t in Powermove yet. Only extensions are copied. Your projects, settings and sign-ins stay as they are.` |
| Counts | `New`, `Updated`, `Already in Powermove`, `Skipped` (non-zero only) |
| Exceptions | `Not Brought In`: each skipped extension and why (`info.circle`) |
| Names | `To Bring In` |
| Note | Startup: `Choose Not Now to open your library as it is. You can bring them in later from Settings › Storage.` |
| Buttons | Startup: `Not Now`, `Bring In Extensions` (default). Settings: `Cancel`, `Bring In Extensions` (default; disabled when nothing is new) |

**Not Now** is remembered (`extension-import/preferences.json`) and opens the library
at once. Later launches don't look again. Bringing extensions in, from anywhere,
clears it, so something added to the original app later is offered at startup.

## N005 Bringing in

Title `Bringing In Your Extensions`, the same line. One card row with the running bar:
`Copying` then `Checking that it loads`, with the extension's display name as the
value. The engine copies every extension, then checks each one loads in a throwaway
runtime (never the open project), with its declared dependencies installed first. No
buttons: it is quick, and quitting is safe.

## N006 Summary

| Result | Title | Line starts |
|---|---|---|
| Something brought in | `Your Extensions Are In` | `Each one was checked and loads.` / `One couldn’t load. The reason is below.` / `{n} couldn’t load. The reasons are below.` |
| Nothing new | `Nothing New to Bring In` | `They were already in Powermove.` |
| Only failures | `Your Extensions Couldn’t Load` | as above |
| Stopped (folder unreadable, locked) | `Your Extensions Couldn’t Be Brought In` | `Nothing in your library changed.` |

Every line ends `Your original app stays unchanged.` Counts: `Brought in`, `Couldn’t
load`, `Already in Powermove`, `Skipped`. Exceptions in order: couldn't load
(`exclamationmark.triangle.fill`, `c.warning`), skipped, already here (`info.circle`).
Names: `Brought In`, with no reason repeated.

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
