# Design review: `feat/na-cloud-p-project`

Reviewer: design lead · 2026-11-01 · Lane head `35422ce53` (diff vs
`origin/feat/native-engine`). Read against [panels/media-browser.md](../panels/media-browser.md),
[panels/README.md](../panels/README.md), [ui-kit.md](../../ui-kit.md),
[review-checklist.md](../review-checklist.md) and
`panels/project/tests/__snapshots__/view.test.tsx.snap`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-project` (`4a17ea785`), not the lane head.** That
branch is the lane plus the **Fixed** items, merged with
`origin/feat/native-engine` (members conflict resolved as a union). On macOS:
`bun test panels/project` 53/53 (snapshots regenerated) and
`cargo test` for `pm-project-panel-check` 13/13. TypeScript, Prettier and
rustfmt are clean.

**macOS test bug found and fixed.** The lane's `project-model-host` test
binary blocked on the kernel from the main thread. The kernel asserts against
that on macOS (`crates/kernel/src/actor.rs`), so 9 model tests panicked here
even though they passed on Linux. The host loop now runs on a worker thread.
The same bug was in Effect Controls' model wire; it is fixed there too (see
that review).

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | §2 header | Tab group `Project · Effect Controls`; `Search Media` / `Add` tooltips; Add menu items, order and ⌘N/⌘I | APPROVE |
| 2 | §2 header | Search and Add sat right after the tab group, without symbols | **Fixed**: trailing cluster; `magnifyingglass`, `plus` |
| 3 | §2 search row | Toggled by the header button and ⌘F; Esc clears, then closes; IME guarded. `TextField` stands in for `NSSearchField` with no placeholder | APPROVE as interim (F1) |
| 4 | §2 path bar | Crumbs were bare buttons: no `chevron.left` on `Media`, no ` / ` separators | **Fixed**: chevron on the root crumb; `c.muted` ` / ` between crumbs; current crumb `c.text`, others `c.secondary`; roving ←/→; crumbs are drop targets |
| 5 | §2 grid | Row virtualisation with native column count and row height; one viewport of overscan requested by rows; 4 096-item page cap | APPROVE |
| 6 | §2 thumbnail, badges | Poster well on `c.field`; duration and status text in `c.selected_text`. No `c.media_badge` capsule yet: the token is still missing, and the lane was right not to substitute it | APPROVE as interim (F2) |
| 7 | §2 hover actions | Correct per status (Show in Finder / Add to Timeline, Locate File…, Download / Retry Download, Delete Media), but no symbols | **Fixed**: `folder`, `plus`, `link`, `icloud.and.arrow.down`, `trash` |
| 8 | §2 meta | Formatter matches v1.2.0 (`MOV · 1920×1080 · 0:12 · 48 MB`, `SVG`, `{n} tris`, `Ready to use`, `{n} items`/`1 item`); status details in `c.danger` / `c.muted` | APPROVE |
| 9 | §6 VoiceOver | Asset cards spoke raw floats (`12.04 seconds`, `45.776… megabytes`) | **Fixed**: rounded seconds (`1 second` / `12 seconds`) and the meta line's KB/MB value spoken as `kilobytes`/`megabytes` |
| 10 | §2 selected | Name capsule `c.selected_fill` + `m.capsule_radius`; the 2 pt accent ring and the capsule padding are renderer work | APPROVE as interim (F3) |
| 11 | §2 selection bar | Count and Delete were packed together | **Fixed**: count leading with `m.inset` padding, Delete (`trash`, `c.danger`) trailing |
| 12 | §2 drop overlay | The card itself was filled with `c.accent_wash` | **Fixed**: the wash fills the area around a solid `c.floating` card with `plus`. The card's `t.title`/`t.minimum` typography waits on kit F2 (Agent review) |
| 13 | §3 empty states | Copy exact (`No Project`, `No Media` + `Import…`, `Empty Folder`, `No media matches “{query}”`) but no symbols | **Fixed**: `photo.on.rectangle`, `photo.on.rectangle.angled`, `folder` |
| 14 | §3 loading | No panel spinner; `Loading media…` only after 0.5 s empty; cards in Loading state | APPROVE |
| 15 | §3 rename | In-place field, Return commits, Esc cancels, blur commits, IME guarded | APPROVE |
| 16 | §4 menus | Labels, groups, separators and shortcuts per §4 (Title Case, `Show in Finder`) | APPROVE |
| 17 | §5 keyboard | Grid keys, ⌘A, F2, Esc go to the model; wrap and ⇧ extend are model-owned | APPROVE (grid-row ↑/↓ F5) |
| 18 | Checklist §1, §7 | Tokens only (lane guard enforces it); copy verbatim; no list/sort controls, no import modal | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

1. **`Availability::Unknown`.** Readiness simply hasn't been checked yet.
   Show nothing that claims a state:
   - the poster if one exists, else the kind glyph on `c.field`;
   - no status capsule;
   - meta shows only the known parts, never `Ready to use`;
   - AX adds no status word;
   - hover actions limited to `Delete Media`;
   - not draggable to the timeline until it resolves.

   If it stays unknown for more than 2 s, it is presented as Loading.
2. **Worker/transport failure.** Keep the last accepted view. Disable every
   card action and the header Add button. Show one Error line at the top of
   the content: `exclamationmark.triangle.fill` `c.warning`, `t.body`
   `c.warning`, text `Project media isn’t updating right now.` Announce it
   once. It clears when publications resume. Details go to the diagnostic
   callback only.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F1 | kit | `SearchField` (shared with Effects): system small `NSSearchField`, placeholder `Search media`, trailing progress. |
| F2 | pm-design + kit | Tokens `metrics.thumbnail_min`, `colors.media_badge`, `metrics.kind_badge`, `metrics.status_dot` (open-composition dot). A media-card recipe: 16:9 thumbnail, `m.gap − 1` badge inset, capsule badges on `c.media_badge`, round over-image buttons (`m.hit`) bottom-right. |
| F3 | kit | Selection ring (2 pt `c.accent`, `m.control_radius + 2`; 3 pt under Increase Contrast), focus ring outside it, name capsule padding `m.gap − 2`, middle truncation that keeps the extension. |
| F4 | model | Search through descendants with the folder path in results; `Changed{source}` publication (meta in `c.warning`); determinate cloud progress; comp background/thumb; Title Case undo names (`Delete Media`, `New Folder`, `Rename Folder`, `Delete Folder`, `Move to Folder`, `Replace Media`). |
| F5 | model | ↑/↓ move by a grid row (needs the column count from the native layout); O(1) select-all ("all except"). |
| F6 | project lane | TODO 1 (Unknown) and TODO 2 (transport Error line). |
| F7 | host | Delete alert as specified (`Delete “{name}”?`, Cancel default, Delete destructive); drag image with count badge; marquee/autoscroll geometry. |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-project | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-project` `4a17ea785`; F1–F7 open |
