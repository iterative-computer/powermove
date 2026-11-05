# Store and publishing

Rows S073 S075 S076 S077 S078 S079 S080 S081 S082. Grammar:
[README](README.md) §1–5. Permission sheet, Full Access alert and stored
decisions are [reviews/na-cloud-storehost.md](../reviews/na-cloud-storehost.md)
and stand unchanged; S012 owns uninstall values; S011 owns Set Up. Sources:
`R/store/`, `M/cloud/publish.ts`, `R/legacy/ui/extension-settings.ts`,
`R/shell/{extension-update-notices,fork-updates,update-fork}.ts`.

Vocabulary: the UI says **the Store** with a capital in Title Case positions and
`the store` in sentences, as v1.2.0 does. `Trust…` means giving full access.

No new tokens. New kit recipes: `ListingRow`, `DetailFacts` (below).

## Store window

v1.2.0 draws the Store as a full-screen overlay on the editor. Native: one
standard document-independent window, like App Store, so browsing never covers
work.

| Part | Spec |
|---|---|
| Open | `Window ▸ Store`; `Open Store` in Settings ▸ Extensions; `Your Library` in the account menu; Project Library sidebar. No default shortcut |
| Frame | titled, unified toolbar, default (`m.sheet_width_wide + m.sheet_width`) × (1.5 × `m.sheet_width`), min `m.settings_width` × `m.settings_height`, frame autosaved; one instance |
| Toolbar leading | back button `chevron.left` (Detail, kind page, maker page; ⌘[ and Esc go back) |
| Toolbar title | `Store`; on a kind page the kind; on a maker page `@{handle}` |
| Toolbar centre | `SegmentedControl` `Discover` · `Library` (AX group `Store pages`) |
| Toolbar trailing | search item, `Search extensions`; `Publish` (`plus`, label shown); account button (S085) |
| Content | solid `c.panel`, one scroll view, inset `m.sheet_inset` |
| ⌘F | focuses search. ⌘W closes |
| Not configured | when the Store host is unavailable, the whole content is `EmptyState` from S075 *Offline* and the toolbar controls disable |

`ListingRow`: `m.row × 2` tall. Icon `m.row + m.inset` square at
`m.control_radius`; name `t.body` `t.semibold_weight`; verified seal
`checkmark.seal.fill` `c.accent_text` at `m.symbol_inline` (AX `Verified`);
second line `t.minimum` `c.muted` `by {handle}` (button) and tagline, truncating.
Trailing action per S076 table, as a small `plus` button (AX `{action}: {name}`)
or `checkmark` when installed (AX `Installed`). Hover `c.hover`; click opens
Detail. Listings are laid out in a grid with minimum column width
`m.sheet_width − m.sheet_inset`.

## S075 Discover, search and kinds

| View | Spec |
|---|---|
| Featured band | one band `c.panel_secondary`, `m.group_radius`, `m.row × 6` tall. Eyebrow `Curated by Powermove` (collections from a kind: `Collection`) `t.minimum` `c.muted`; title `Featured this week` or the plural kind, `t.display`; up to five listing icons in a row, each a button `Open {name}`; `Explore` push button. Arrows `chevron.left`/`chevron.right` and page dots trailing (AX `Previous collection`, `Next collection`; dots are a tab list labelled `Collections`). Slides change only on request, never by timer. v1.2.0's lifted icon sizes stay; colours come from the listing icons alone |
| Shelf | heading `t.heading`, count `t.minimum` `c.muted`, trailing `Show All` link opening the kind page. Up to six rows |
| Kind page | title in toolbar, heading `{Kind plural}` with count, full grid, `Show More` (`Loading…` while fetching) at the end |
| Maker page | listings by that maker sorted by installs, `@{handle}` + seal |
| Search | live, debounced as v1.2.0; heading `Results for “{query}”` + count. Max 120 characters. README §4 rule 5 applies: results stay until replaced |

Kinds in order: `Effects`, `Transitions`, `Panels`, `Themes`, `Commands`,
`Layers`, `Tools`. There is no sort control in v1.2.0; none is added.

| State | Spec |
|---|---|
| Loading | `ListingRow` placeholder wells at real geometry, six rows; AX busy `Loading` |
| Empty search | `EmptyState` `magnifyingglass`, `No Results for “{query}”`, `Try a shorter name, a maker’s handle, or a kind such as “transition”.` |
| Empty Store | `EmptyState` `shippingbox`, `Nothing Here Yet`, `Extensions people publish show up here. Make one with your agent and publish it from your Library.` |
| Empty kind | same title, `No one has published {kind plural in lower case} yet.` |
| Empty maker | `Nothing on the Store`, `{handle} hasn’t published anything that’s listed.` |
| Offline | inline Error `Can’t reach the Store. Check your connection.` + borderless `Retry`, in place of the failed shelf only; the rest stays |
| Other failure | same placement with `actionErrorText` copy: `Too many requests. Wait a moment and try again.`, `This version of Powermove is too old for the Store. Update Powermove and try again.`, `Something went wrong. Try again.` |

Keyboard: Tab order toolbar → band → shelves; within a shelf, arrows move between
rows, Return opens. VoiceOver: each shelf is an `AXList` labelled by its heading.

## S076 Extension detail

Replaces the list in the same window (a push: `mo.enter`, cross-fade under Reduce
Motion). Scroll position of the list is restored on back.

| Region | Spec |
|---|---|
| Header | icon `m.row × 3`; name `t.display`; tagline `t.reading` `c.secondary`; maker line; access summary lines (below); primary action trailing |
| Access summary | Store-host labels: `Uses the network`, `Uses the clipboard`, `Imports files`, `Edits your project`, `Needs full access to Powermove` (warning symbol + `c.warning`). AX group `Access` |
| Description | Markdown, `t.reading`, agent-panel Markdown recipe, no images fetched from unlisted origins |
| Notes | `t.reading` `c.secondary` lines per state table |
| What changed | see S078 |
| Versions | heading `Versions` (owner) or `Version History`; rows: version `t.body` tabular, date `t.minimum` `c.muted`, note. Five shown, `Show All` / `Show Fewer`. Withdrawn rows prefix `Withdrawn` in `c.warning` with symbol |
| Setup | heading `Setup`, `Set Up…` when installed; rows `{label}`, hint, key in `MONO_FONT` `t.minimum`. Note: installed `Stays on this Mac. Never included when you publish or share this extension.`; not installed `You can set these values when you install. They stay on this Mac.` |
| Details | `DetailFacts`, below |
| Source | file list (`Tree`), click shows the file in a read-only `MONO_FONT` text view on `c.field`; binary files show `{size}` only |

`DetailFacts`: two-column key/value, key `t.body` `c.secondary`, value `t.body`
`c.text`, `m.row` rows. In order: `Made by`, `Kind`, `Version`
(`{installed} installed` + aside `{available} available`), `Updated` (`Today`,
`Yesterday`, else system short date), `Identifier` (selectable), `Forked from`,
`Includes`, `Opens without asking`, `Compatibility`. Sandbox note beneath:
`This extension runs in a sandbox with the permissions listed above.` or
`This extension needs full access to Powermove. Only trust code from people you trust.`

**The one action** — order of precedence exactly as `detailAction`:

| Situation | Button | Style |
|---|---|---|
| Not here, no declared values | `Install` | default |
| Not here, declared values | `Install and Set Up` | default |
| Installing | `Installing…`, spinner | disabled |
| Built in | `Built In` | quiet text |
| Here, needs full access | `Trust…` | default |
| Here, values missing | `Set Up…` | default |
| Your fork, on the original's page | `Forked` | quiet text |
| Newer version | `Update` | default |
| Newer version, you changed files | `Update…` | default |
| Yours, changed since publishing | `Publish…` / `Publish Update…` | default |
| Up to date from the Store | `Installed` | quiet text |
| Off | `Turn On` | push |
| Otherwise | `Open` | push |
| Newer app needed | `Install` disabled; note `A newer version of Powermove is needed.` | disabled |

Second row of the header when installed: `Uninstall…` (push), and the quiet
`Publish…` when `secondaryPublish` offers it.

| Note | Copy |
|---|---|
| Installed, no trust | `It stays off until you trust it. Trusted extensions run with the same access as the app: your projects, files you open, the network, and your account.` (warning) |
| Removed from Store | `This extension is no longer on the Store. It stays on this Mac, but won’t get updates.` |
| Trusted | `Trusted · runs with full access.` + link `Revoke Trust` |
| Built-in lineage | `Forked from Powermove’s built-in {id} {version}` |

Install outcomes go to a one-line inline notice at the top of the content (README
§3): `{name} is installed.`, `{name} is installed. It stays off until you give it
full access.`, `{name} has full access.`, `{name} no longer has full access.`,
`{name} is uninstalled.` Failures use the same notice with Error copy and no
alert: `This extension is no longer on the Store.`, `This extension isn’t on the
Store.`, plus S075 failures. `Couldn’t turn {name} on. Try again.` and
`Couldn’t turn {name} off. Try again.` for the switch.

Uninstall alert: message `Uninstall {name}?`; informative Store item `Its files
are removed from this Mac. You can install it again from the Store.`; yours
`Its folder moves to the Trash.`; buttons `Cancel` (default) · `Uninstall`
(destructive). v1.2.0 said “deleted… can’t be undone”; native archives it (N4).

## S077 Library

Sections in order; a section with no items is omitted unless stated.

| Section | Items | Empty |
|---|---|---|
| `Needs Attention` + count | anything `needsAttention` | omitted |
| `Installed` + trailing `Check for Updates` | Store items | `EmptyState` `Nothing Installed Yet`, `Extensions you install from the Store live here, and update when their makers publish.`, button `Discover Extensions` |
| `Made by You` + count | local items | `EmptyState` `Nothing Made Yet`, `Ask your agent for an effect, a panel or a theme. What it makes shows up here, ready to publish.` |
| `Remembered` + count (signed in) | account installs not on this Mac | row action `Install`. Omitted when `Remember Installs on This Account` is off |

Row = `ListingRow` with: lineage `forked from {handle}/{slug}` in the second
line; third line status `t.minimum`, `c.warning` with `exclamationmark.triangle.fill`
when it needs attention, else `c.muted`. Trailing: action, kit switch (AX
`{name}: on`), `ellipsis.circle` (AX `More actions for {name}`).

Status strings: `No longer on the Store`, `Needs full access`, `Needs setup`,
`You changed the files · {version} is beside your folder`, `You changed the files
· Update to {version}`, `Installed {old} · Update to {new}`, `Published {version}
· Changed since`, `Published {version}`, `You changed the files`, `Installed
{version}`, each with ` · Trusted` appended when trusted.

More menu (`NSMenu`): `Check for Updates` (Store items) · `Trust…` or `Revoke
Trust` · `Sandbox Compatibility Check…` (local) · `Show in Finder` · separator ·
`Uninstall…`.

Check result, inline notice: `Everything is up to date.`, `One update is
available.`, `{n} updates are available.` Signed out: `Remembered` is absent and a
one-line link `Sign in to keep your installs across Macs.` follows `Installed`.

## S073 Extensions in Settings

Completes settings.md *Extensions*, whose filter labels are replaced by v1.2.0's
groups in order. Pop-up `All Types`, `Effects`, `Panels`, `Appearance`, `Layer
Types`, `Media`, `Editor Tools`, `Other Extensions`. Beside it a segmented state
filter with live counts: `All {n}`, `Enabled {n}`, `Off {n}`, `Needs Attention {n}`.

List rows group under a heading (AX `{group}, {n} extensions`), large groups
collapse with a disclosure; row detail line shows `Needs attention` state in
`c.warning` with the error, or `Set up` note, else the description.

| State | Copy |
|---|---|
| None | `No extensions installed.` |
| No match | `No extensions match your search or filters.` + `Clear Filters` |
| Host unavailable | `Extensions are unavailable in this session.` |
| Load failed | Error `Couldn’t load extensions.` + `Try Again` |
| Switch failed | Error line on the row `Couldn’t update this extension.`; switch reverts |

**Detail pane** additions, top to bottom: state line (`Needs attention` etc.);
`Replacement` line when Powermove now includes it, tone `c.secondary`:
`Powermove now includes this.`; `Effect Categories`, `Panel Categories`,
`Appearance`, then one row each for `Panels`, `Commands`, `Shortcuts`, `Effects`,
`Transitions`, `Layers`, `Themes`; `Values` (`Set Up…` default when waiting, else
`Variables…`); `Extension Files` (`Show in Finder`, path `t.minimum` `c.muted`);
`Based On` · the forked-from identity when present;
`Delete Extension` (`Delete…`, `c.danger`). v1.2.0 has no fork or repair action
here: forking belongs to the agent, repair to S082.

`Delete…` alert: message `Delete {name}?`; informative `It moves to the Trash.`,
plus `Projects using its layer types or effects may need it reinstalled to
render.` when it declares them; buttons `Cancel` (default) · `Delete`
(destructive). v1.2.0 confirmed inline; alert grammar replaces it. Then S012.

Focus returns to the row's disclosure or switch after back, as v1.2.0 tracks it.

## S078 Updates, forks and comparison

**After-update notices** (`extension-update-notices.ts`): v1.2.0 sticky corner
toasts become one viewer banner each, with the close button, queued (README §3).

| Kind | Banner | Action |
|---|---|---|
| Compatibility | `{name} reported a problem after updating to Powermove {version}. Review the extension before using it.` | `Review Extensions` |
| Now included | `Powermove now includes {name}’s features in {built-in}.` | `Review Extensions` |

Dismissal is remembered per notice key, as v1.2.0.

**Pending fork updates** (`fork-updates.ts`): banner `{n} extensions need updating`
or `1 extension needs updating`, action `Update`. In the Project Library the same
line is an inline notice; it never covers New Project.

| Fork update step | Copy |
|---|---|
| Running | banner `Updating {name}…` with spinner, sticky |
| Done, clean | banner `{name} is updated.` |
| Needs the agent | banner with the reason + action `Update with Agent` |
| Checks failed | banner `{name} couldn’t pass the update checks. Your current version was kept.` |
| Restart required | banner `Restart Powermove to update this extension.` |
| Agent failed | banner `Couldn’t start the extension update.` |

**Update with local changes** — alert on the Store window:

| Part | Copy |
|---|---|
| Message | `Update {name} to {version}?` |
| Informative | `You changed its files, so your folder stays as it is. The new version is saved beside it for your agent to merge.` |
| Buttons | `Save New Version` (default) · `Cancel` |

Outcome notice when a merge conflicts: `Update available; you and the author
changed the same lines. The new version is beside your folder for your agent to
merge.`

**What Changed** section in Detail: heading `What Changed` with summary
`t.minimum` `c.muted` (`{n} added · {n} changed · {n} removed`, or `No file
changes`), trailing `Hide`. Rows: path `MONO_FONT` `t.body`, status word `Added`,
`Changed`, `Removed` in `c.secondary` (never colour alone). For a fork, the
manifest row adds `Names what it was forked from`. Selecting a text row shows a
read-only side-by-side `MONO_FONT` comparison beneath, no colour fills beyond
`c.field`, changed lines marked by `+`/`−` in the gutter. Loading = placeholder
wells. Failed: Error + `Retry`.

## S079 Publish sheet

Wide system sheet (`m.sheet_width_wide`), scrolling content, footer pinned.
Title by case: `Publish {name}`, `Publish Your Version of {name}` (fork, first
publish), `Publish an Update to {name}` (later). Fork line: `Forked from
{coordinate} {version}. It goes on the Store under your name, linked to the original.`

Groups (panels/README rows in sheet grammar), heading `t.title`:

| Group | Row | Control | Help line (`t.minimum` `c.muted`) |
|---|---|---|---|
| Release | `Version` | text field, tabular | `Last published {version}`, else `The number people see on the Store.` |
| | `What’s New` | multi-line text, 3 lines | `Optional. Shown on the Store with this version.` / `Shown on the Store with this version.` |
| Listing (first publish) | `Name` | text field | `How it’s listed on the Store.` |
| | `Tagline` | text field | `One line under the name.` |
| | `Kind` | pop-up, S075 kinds, singular | `Where it’s filed in Discover.` |
| | `Visibility` | pop-up `Public` · `Unlisted` | `Shown in Discover and search.` / `Only people with the link can find it.` |
| | `Icon` | drop well, S081 | `Any image. It’s cropped square and sized for the Store.` |
| Sandbox | status | S082 *Status row* | |
| Permissions to Declare | one row per finding | read-only | fix text from the check |
| Possible Secrets | row per finding: path `MONO_FONT`, `line {n}` | see below | `{kind}. If it isn’t a secret, say why.` |

Possible secrets is the publish safety gate: a blocked finding keeps `Publish…`
disabled, with no override. A reviewable finding has a text field, placeholder
`Why it’s safe`, and blocks until filled. Finding kinds: `Looks like a key or
token`, `Private key`, `Access token`, `AWS access key`, `GitHub token`, `GitLab
token`, `Slack token`, `Stripe key`, `Google API key`, `OpenAI API key`,
`Anthropic API key`, `Possible secret`.

Validation is live but shown after the first edit of a field: `Keep the name under
{n} characters.`, `Keep the tagline under {n} characters.`, `Keep this under {n}
characters.`

Footer: `Cancel` · `Publish…` (default). Disabled while busy, blocked, or until the
sandbox check passes. Error line above the footer for server errors, plus
`Extensions can have at most 400 files. Remove some and try again.`,
`Extensions can be at most 8 MB. Remove large files and try again.`, `This
extension’s folder is empty.`, `This extension has no manifest.json.`, `Powermove
couldn’t read this extension’s folder.`

## S080 Preparing, confirming, publishing, withdrawing

**Preparing**: the sheet opens at once with title `Publish {name}`, line `Getting
it ready…`, one row `Preparing` · `Reading the folder and checking what’s on the
Store.` with spinner. `Publish…` disabled, `Cancel` live.

**Native confirmation** — `NSAlert` on the sheet, kept from v1.2.0 so the last
step is outside anything an extension or agent can draw:

| Part | Copy |
|---|---|
| Message | `Publish {coordinate} {version}?` |
| Informative | `Snapshot {12-character id}. {n} files, {size}.` + ` Forked from {coordinate} {version}.` |
| Buttons | `Publish` (default) · `Cancel` |

**Progress**: form disabled; footer status with `ProgressRow`, tabular:
`Waiting for you to confirm…`, `Checking what the Store already has…`, `Uploading
{done} of {total} files…`, `Publishing…`. Cancel works until `Publishing…`, then
disables.

**Done**: content replaced. `checkmark.circle.fill` `c.success`, title `Published
{coordinate} {version}`, line: new public `It’s on the Store now, in Discover and
search.`; new unlisted `It’s on the Store now. Anyone with its link can install
it.`; update `It’s on the Store now. People who have it get the update the next
time Powermove checks.`; `Done` default.

**Failed**: form returns with the Error line; `Publish…` retries. Not signed in:
S083 opens first. No handle: S084 handle step. Generic: `Powermove couldn’t publish
from this window. Try again.`

**Nothing to publish** — `Publish` in the toolbar with no eligible items: inline
notice `Nothing to publish yet. Extensions you or your agent make show up under
Yours.`

**Withdraw a version** (`yank`). v1.2.0 confirms inline; native uses an alert.

| Part | Copy |
|---|---|
| Message | `Withdraw {version}?` |
| Informative | `People who have it keep it; nobody new gets it.` |
| Buttons | `Cancel` (default) · `Withdraw` (destructive) |

Row shows `Withdrawing…` while busy. Notice `Withdrew {version}.` Failure notice:
`You can only withdraw versions of your own extensions.`, else S075 failures.

**On the Store** block in Detail for owners: `Installs`, `Forks`, `Visibility`
in `DetailFacts`. **Publish** block: `Will Publish As` · `@{handle}/{id}`, `Choose a
handle to publish`, or `Sign in to choose a handle`, with note `Publishing puts the
source on the Store under your name. Setup values stay on this Mac.`

## S081 Icon cropper and pickers

**Icon well** (in S079): `c.field` well `m.row × 3` square, `m.control_radius`.

| State | Spec |
|---|---|
| Empty | `Drop an image here` `t.body` + `or click to choose one` `t.minimum` `c.muted`; click opens `NSOpenPanel` for images |
| Drag over | Drop target; text `Drop to Use This Image` |
| Ready | preview + `Icon ready` + `Drop another image to replace it.`; buttons `Remove`, `Adjust…`, `Change…` |
| Too small | Error `Choose an image at least {n} pixels on its short side.` |
| Unreadable | Error `Powermove can’t read this image. Choose a PNG, JPEG or WebP.` |
| Bad size after encode | Error `Choose a PNG of 256 KB or smaller.` |

**Cropper** replaces the well inline, not a further sheet: image in a `m.row × 6`
square frame with a square mask; area outside the crop dims with `c.media_badge`
at `m.media_badge_opacity`; title `Frame Your Icon`, line `Drag to move it. Scroll
or use the slider to zoom.`; `NSSlider` `Zoom` (disabled when the image can't
zoom); buttons `Cancel`, `Use Icon` (`Preparing…` while encoding).

Keyboard: arrows move (⇧ ×10), `+` and `−` zoom, Return confirms, Esc cancels. AX
`Icon framing. Drag or use the arrow keys to move, + and − to zoom.`

**Search picker** (project and publish pickers; `SearchPicker.svelte`): `NSPopover`
at `m.popover_width`, thread-picker recipe: search field, rows with title and
trailing meta `t.minimum` `c.muted` (`New`, `Update`). ↑/↓ move, Return chooses, Esc
closes. Empty: `No matches for “{query}”.`, or the caller's empty line. Choice
failed: Error line under the field, popover stays. Placeholder for publishing:
`Search extensions to publish…`.

## S082 Test in Sandbox

**Sheet** from `Sandbox Compatibility Check…`: title `Check {name} in the Sandbox`,
line `Runs it with only the permissions in its manifest, the way it runs for
people who install it.`, the Status row, footer `Done` (default). The same Status
row is the Sandbox group in S079.

**Status row**:

| State | Title | Line | Actions |
|---|---|---|---|
| Running | `Checking Compatibility` | `Running it in the sandbox with the permissions it declares.` | spinner |
| Couldn't run | `Couldn’t Check` | the reason, or `Powermove couldn’t run the sandbox check in this window. Close this and try again.` | `Check Again` |
| Skipped | `Full Access` | `Needs full access: installs only after the user trusts it.` | none |
| Passed | `Compatible with the Sandbox` | `It runs with only the permissions it declares.` | none; `checkmark.circle.fill` `c.success` |
| Failed | `Not Compatible with the Sandbox` | `The agent can fix this for you, or fix it yourself and check again.`; without an agent `Fix {these lines} and check again.` | `Check Again`, `Fix with Agent` (default; `Opening Agent…` while opening) |
| Fix wouldn't start | as Failed | Error `The agent couldn’t start the repair. Try again.` | same |

Failed findings list below as `t.body` rows with the `exclamationmark.triangle.fill`
glyph, one per `sandboxCheckLines` string, copy verbatim from
`R/store/sandbox-check.ts` (these name API members for extension authors, so they
are developer-facing on purpose). Copyable with ⌘C. `Fix with Agent` opens S064.
A cancelled check returns to Couldn't run with `The check was cancelled.`

## Don't

- Don't auto-advance the featured band or add store art gradients.
- Don't let any override skip Possible Secrets or the native publish confirmation.
- Don't use toasts: inline notice in Store and Settings, banners over the viewer.
- Don't say `Delete` permanently when the item moves to the Trash or an archive.
