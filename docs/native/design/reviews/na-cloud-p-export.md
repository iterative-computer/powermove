# Design review: `feat/na-cloud-p-export`

Reviewer: design lead · 2026-11-01 · Lane head `3448dc146` (diff vs
`origin/feat/native-engine`). Read against [panels/export-sheet.md](../panels/export-sheet.md),
[panels/README.md](../panels/README.md) (owner decision 4: no Code export),
[panels/settings.md §3](../panels/settings.md) (group recipe),
[ui-kit.md](../../ui-kit.md), [review-checklist.md](../review-checklist.md) and
`panels/export/__snapshots__/panel.test.tsx.snap`. Code and snapshots only.

**Verdict: APPROVE as an integration candidate. Merge
`origin/feat/na-design-fix-p-export` (`3ebdd8521`), not the lane head.** That
branch is the lane plus the **Fixed** items, merged with
`origin/feat/native-engine`. The members conflict is resolved as a union.
`bun test panels/export` passes there (47/47, snapshots regenerated, wire
executable built). TypeScript and Prettier are clean.

## Item by item

| # | Spec | Finding | Status |
|---|---|---|---|
| 1 | Owner decision 4 | Three kinds only (`Video` · `Images` · `Project`); no Code/web path anywhere | APPROVE |
| 2 | §2 title / kind | `Export`, `t.heading`; segmented `Export type`, initial focus; descriptions exact | APPROVE |
| 3 | §2 rows | Label, detail and control were three loose siblings; the control wasn't in a trailing column | **Fixed**: row = leading stack (label `t.body` `c.text`, detail `t.minimum` `c.muted`) + trailing control cluster, `space-between`. Column width `m.sheet_value_column` still metadata (F2) |
| 4 | §2 FieldSupport | Rows shown per format exactly (ProRes/MP4/WebM/PNG/Still/Project); captions only with captions layers; `Caption File` video only | APPROVE |
| 5 | §2 option labels | `Half · {w}×{h}`, `Custom size…`, fps list, `Work area · not set`, `Draft · 4 Mbps`…; `Choose Preset` | APPROVE |
| 6 | §2 custom size | Both wells were labelled `Size` | **Fixed** (TODO 3): `Width` / `Height` (AX `Width, 1920 pixels`); `×` and `link` between; error `Enter a size from 16 to 8192 px.` |
| 7 | §2 Project body | Capsule labels were plain text on a fill | **Fixed**: `m.capsule_radius`, `m.gap` horizontal padding |
| 8 | §2 colour note | Exact string, `t.minimum` `c.muted`, bottom of Presets | APPROVE |
| 9 | §2 footer | Summary, note and buttons were stacked | **Fixed**: summary (+ note) leading, `Cancel` then the default button trailing |
| 10 | §2 default button | `Export Video` / `Export Frames` / `Export Frame` / `Save Project File`; one primary | APPROVE |
| 11 | §3 progress | `Exporting “{name}”`; preview on `c.viewer`; `Frame {i} of {total} · {pct} %`; ETA rule (≥ 8 frames and ≥ 1 s; min/s); no fake % while preparing | APPROVE |
| 12 | §3 progress line | ETA was on its own line under the label | **Fixed**: label leading, ETA trailing on one line; progress footer button trailing |
| 13 | §3 failure | Triangle + `Export failed: {reason}`; `Close` + `Try Again` (default) | APPROVE |
| 14 | §5 keyboard | Esc = Cancel (settings only), Return default, ⌘. cancels progress, Return in the preset name saves; ordinary `.` never suppressed | APPROVE |
| 15 | §2 group card | `c.field`/`c.floating` card, `m.group_radius`, inset hairlines are metadata only | CHANGES REQUIRED (kit `FormGroup`, F2) |
| 16 | Checklist §5 | `SheetAnchor`, system placement, not dismissible outside; progress not dismissible by Esc | APPROVE |
| 17 | Checklist §1, §7 | Tokens only; copy verbatim (curly quotes, `×`, `…`) | APPROVE |

## DESIGN-TODO(opus) answers (spec updated on `feat/na-design4`)

1. **Try Again.** It re-runs the **same frozen job to the same destination**
   (overwriting the partial output), with no save panel and no settings
   sheet. The person already chose both, and v1.2.0 retried in place. One
   exception: if the failure reason is about the destination (not writable,
   disk full, folder missing), `Try Again` reopens the save/choose-folder
   panel as a sheet on the progress sheet, pre-filled with the old name.
2. **Failures outside rendering.**
   - **Planning failed** (no summary can be computed): the footer summary
     becomes `exclamationmark.triangle.fill` `c.warning` +
     `Can’t export with these settings: {reason}` (`t.body`). The default
     button is disabled until a setting changes and planning succeeds.
   - **Preset save failed**: a `t.minimum` `c.danger` line under
     `Save as Preset`, `Couldn’t save the preset. Try again.` The name stays
     in the field. **Preset list failed to load**: the `Saved Preset` pop-up
     is disabled with detail `Couldn’t load your saved presets.`
   - **Ambiguous transport ack** (start sent, no acknowledgement): stay in
     `Preparing…` with the indeterminate bar. If the host then reports the
     job lost, go to the failure state with reason
     `Powermove lost contact with the export engine. Try again.` No toasts.
3. **Custom size names.** `Width` and `Height`, unit `px` (spoken `pixels`).
   Done on the fix branch.

## Sol follow-ups (precise)

| # | Owner | Change |
|---|---|---|
| F1 | kit | `PushButton{role}` (shared request) for `Cancel`, the default button, `Close`, `Try Again`, `Save`. |
| F2 | kit + pm-design | `FormGroup{title}` card recipe and a sheet form-row recipe: `m.sheet_value_column` trailing column (pop-ups fill it, checkboxes at its trailing edge), min height `m.form_row`, inset hairlines; tokens `m.sheet_inset`, `m.sheet_value_column`, `m.sheet_width_wide`, `m.group_radius`, `m.form_row`. |
| F3 | kit | Pop-up option subtitles (disabled-format reasons), text-field placeholder (`Preset name`), help text, polite announcements, progress value description. |
| F4 | export lane | Implement TODO 1 (retry in place + destination exception) and TODO 2 (planning, preset and transport failures). |

## Sign-off

| Lane | Reviewer | Date | Result |
|---|---|---|---|
| na-cloud-p-export | design lead (Opus) | 2026-11-01 | APPROVE via `feat/na-design-fix-p-export` `3ebdd8521`; F1–F4 open |
