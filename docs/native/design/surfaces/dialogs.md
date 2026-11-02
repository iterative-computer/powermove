# Editing dialogs — composition, pre-compose, keyframes, expression, presets

Rows S021 S022 S029 S036 S043. All are sheets (README §2) or alerts
(README §1) on the editor window. Fields use inspector grammar and the
shared value column `m.sheet_value_column`; number wells are the kit
`NumberField` (scrubbable, arithmetic, units in `c.muted`).

## S021 New Composition / Composition Settings

Ports `R/legacy/ui/compositions.ts`, `R/legacy/ui/project-settings.ts`
(same form as [library.md S018](library.md#s018-new-project-sheet)).

| | New Composition | Composition Settings |
|---|---|---|
| Entry | `Composition ▸ New Composition…` (⌘N, w1-keymap M026) | `Composition ▸ Composition Settings…`, Properties composition section `Settings…` |
| Title | `New Composition` | `Composition Settings` |
| Name default | `Comp {n}` (uniqued) | current |
| Default button | `Create` | `Apply` |

Rows: `Name`, `Resolution`, `Size`, `Frame Rate`, `Duration`, `Background`
with S018's controls, ranges and error copy. Settings-only warning (warning
style, under Duration): `{n} layers extend past the new duration. They’re kept but won’t play past the end.`
Changing size offers no rescale (shipped behaviour: layers keep coordinates).
`Apply` is one undo step `Composition Settings`. Stale target → README §2.

## S022 Pre-compose and delete composition

**Pre-compose sheet** (`Pre-compose…`, ⇧⌘C, w1-keymap K025/M028):

| Element | Spec |
|---|---|
| Title | `Pre-compose` |
| `Name` | text field, default `Pre-comp {n}` (first free), selected |
| Option group (`NSButton` radios, each with a `t.minimum` `c.secondary` description under the title) | `Leave all attributes in “{layer}”` — `Creates a composition with only this layer’s source in it. The new composition becomes the source of the layer.` (disabled, description `Available for a single layer with a source.`, when not applicable) · `Move all attributes into the new composition` — `Places the selected layers together into a new composition.` (default) |
| Checkboxes | `Adjust composition duration to the time span of the selected layers` (enabled only with Move) · `Open New Composition` |
| Buttons | `Cancel` · `Pre-compose` |

One undo step `Pre-compose`. Stale target (project or active composition
changed) → README §2 (replaces the shipped thrown error).

**Delete composition** — alert:

| Part | Copy |
|---|---|
| Message | `Delete “{name}”?` |
| Informative | used elsewhere: `It’s used in {n} other compositions. Those layers are removed too. You can undo this.` · otherwise `You can undo this.` |
| Buttons | `Delete` (destructive) · `Cancel` (default) |

## S029 Keyframe dialogs

Menu labels (keyframe context menu, Title Case): `Edit Value…`,
`Keyframe Velocity…`, `Scale Keyframes…`. All three: width `m.sheet_width`,
one undo step named for the dialog, `Apply` default, invalid → field Error
and `Apply` disabled (no toasts). Locked layers' keys are excluded; if none
remain the menu item is disabled.

**Edit Value** — title `Keyframe Value`, line `{property} · {n} keyframes`
(`{property} at {timecode}` for one). One row labelled with the property;
two-part properties show two wells; units as the inspector. Several keys with
different values: wells empty with placeholder `Mixed`; leaving one empty
leaves that component unchanged. Error `Enter a number.`

**Keyframe Velocity** — title `Keyframe Velocity`, line `{n} keyframes`.

| Group (`t.title` sub-heading) | Rows |
|---|---|
| `Incoming` | `Speed` (`{unit}/s`) · `Influence` (`%`, 0.1–100) |
| `Outgoing` | `Speed` · `Influence` |

Mixed → `Mixed` placeholder as above. Error
`Enter a speed, and an influence from 0.1 % to 100 %.` Applying sets
Bézier interpolation and turns off auto-Bézier (shipped behaviour).

**Scale Keyframes** — title `Scale Keyframes`, line
`Scales from the earliest selected keyframe.` Rows `Time` (`%`, default 100,
> 0) and `Value` (`%`, default 100; disabled with tooltip
`Only number values can be scaled.` when no numeric keys are selected).
Collision → Error under Time
`This would overlap another keyframe or go past the layer’s end.`

## S036 Expression editor

Ports `R/legacy/core/expression.ts` and the shipped modal title
`Expression · {layer} · {property}`. Sheet, width `m.sheet_width_wide`.

| Element | Spec |
|---|---|
| Title / line | `Expression` · `{layer} · {property}` `t.reading` `c.secondary` |
| Editor | `NSTextView` (P-TEXT), `MONO_FONT` `t.body`, `c.field` well, `m.control_radius`, height 10 lines, scrolls; no line numbers, no syntax colouring, smart quotes/dashes/substitutions **off**, spell checking off |
| Insert | `NSPopUpButton` pull-down `Insert ⌄` above the editor, trailing: the shipped expression presets, inserted at the caret |
| Result line | under the editor, `t.minimum`: ok → `c.muted` `At the playhead: {value with unit}`; error → panels/README Error state `Line {n}: {message}` |
| Buttons | leading `Remove Expression` (danger text, only when one exists) · `Cancel` · `Apply` (disabled while there is a syntax error; runtime errors allow Apply and keep the error shown in the Properties row) |

Keyboard: Return/Tab type into the editor; ⌘Return applies; ⌃Tab leaves the
editor; Esc cancels. Evaluation is debounced to 250 ms off the main thread.
VoiceOver: editor `Expression for {property}`; result line is announced on
change (`.medium`).

## S043 Export presets

Extends [panels/export-sheet.md](../panels/export-sheet.md) Presets section
(`Saved Preset` pop-up, `Save as Preset` field + `Save`).

| Action | Surface / copy |
|---|---|
| Save with a name that exists | alert (sheet on the Export sheet): `A preset named “{name}” already exists. Do you want to replace it?` · buttons `Replace` · `Cancel` (default) |
| Manage | the `Saved Preset` pop-up ends with separator, `Rename “{current}”…`, `Delete “{current}”` (only when a saved preset is chosen) |
| Rename | alert with an accessory `NSTextField` (width 240): message `Rename Preset`, buttons `Rename` · `Cancel` |
| Delete | alert: `Delete the preset “{name}”?` · `Delete` (destructive) · `Cancel` (default). Deleted presets are archived in app storage (N4). |
| Persistence failure | inline Error under the Presets section: `Couldn’t save the preset.` + `ServiceErrorCode` informative |
| Queued export | Queue rows and actions are owned by the promedia review (Background Tasks replaces the Render Queue); nothing added here. |
