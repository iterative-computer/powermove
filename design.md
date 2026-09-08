# Powermove design language

This is the implementation guide for every new Powermove interface. It turns the current codebase into a reusable design library so an agent can assemble a native-looking feature from existing parts instead of inventing a new visual system.

The design is **quiet, compact, precise, source-connected, and native to macOS**. Neutral surfaces carry the structure. Type and spacing carry most of the hierarchy. The active workspace accent marks primary creation, active editing, selection, and AI actions; orange is the default, while specialized workspaces may use another registered accent. Controls appear when useful and recede when idle.

## Start here: the agent recipe

For nearly every feature:

1. Find the closest existing surface in the component map below.
2. Let the dock system create panel chrome. Do not draw another card or title bar inside it.
3. Use `api.ui.controls` for fields and inspector-style controls.
4. Use tokens from `css/tokens.css`; never introduce a private palette, type scale, shadow system, or spacing system.
5. Use the existing icon set through `api.ui.icon(name)`.
6. Route document changes through `api.project.apply(...)`, a shared edit binding, or `PM.Edit.apply(...)`. One user action must equal one Undo step.
7. Implement resting, hover, active, selected, disabled, empty, loading, error, narrow, light, and dark states.
8. Hot-reload and verify the existing app window in place. Never restart Powermove for a panel-only change.

Preference order:

```text
existing component
  -> existing shared class/pattern
    -> small local composition of existing tokens
      -> new primitive only when no existing primitive can express the behavior
```

## Authority and source map

When this guide and the source differ, the current source wins. Update this guide with the same change.

| Layer | Source of truth | What it owns |
|---|---|---|
| Theme foundations | `css/tokens.css` | Color, surfaces, text, radii, type, density, shadows, motion |
| Shared app patterns | `css/app.css` | Shell, docks, panels, buttons, rows, lists, overlays, agent UI, project/library surfaces |
| Shared field behavior | `src/renderer/src/controls/` | Svelte fields, edit gestures, labeling, keyboard behavior |
| Panel helpers | `src/renderer/src/panels/` | Panel registration, simple lists, generated panels, media, agent and specialized panels |
| Dock chrome | `src/renderer/src/layout/` | Panel frame, header, movement, resizing, collapsing, persistence |
| Overlay behavior | `src/renderer/src/overlays/` | Menus, modal, palette, toast, focus and dismissal |
| Public extension API | `src/renderer/src/kernel/api.ts` | Versioned controls, panels, editing, icons, themes, menus, status |
| Best property-editor example | `src/extensions/inspector/` | Sections, rows, animated channels, source bindings |
| Best compact-list examples | `AssetsPanel.svelte`, `FxBrowserPanel.svelte`, `TakesPanel.svelte` | List selection, metadata, contextual actions, empty states |
| Best generated-tool example | `GeneratedPanel.svelte` and `panels/generated/` | Schema-driven native controls, previews, Apply/Reset |

Do not copy long blocks of CSS from one feature to another. If a pattern appears more than twice, promote it to the shared component or stylesheet that already owns that family.

### Code boundaries

- A built-in under `src/extensions/<id>/` may import only `powermove`, Svelte, and files inside its own extension folder. Compose host UI through the API it receives.
- A user or project extension follows the same rule and must never import a private renderer path.
- Renderer-owned panels under `src/renderer/src/panels/` may import the shared control and state modules directly.
- `api.host.pm` and `api.host.state` are temporary unstable escape hatches. Use them only where the versioned API does not yet expose a required behavior; isolate the use so it can be removed later.
- Prefer a contribution or a small same-id override over forking an entire built-in. Fork only when the whole surface must change.

## The product character

### The six visual rules

1. **Structure with surfaces, not outlines.** Use the window, panel, field, and floating surface ladder. Borders are hairlines for separation, not boxes around everything.
2. **Hierarchy with type and whitespace.** Headers are quiet. Section labels are smaller and more muted. Primary values remain readable.
3. **One accent at a time.** Orange is a signal, not decoration. Do not combine an orange background, orange border, and orange icon on the same control.
4. **Dense but not cramped.** Default rows are 30px; controls are 28px or 24px inside inspector rows. Information aligns to consistent tracks.
5. **Reveal controls on approach.** Destructive and secondary actions may fade in on hover/focus, but they must remain keyboard reachable.
6. **Editing is tangible.** Scrubbable values, selected rows, keyframe states, Apply/Cancel, previews, and Undo make the interface feel like an editor—not a settings form.

### Color roles

| Role | Token | Use |
|---|---|---|
| Window canvas | `--bg-window` | App background around docks |
| Rail | `--bg-rail` | Secondary side chrome |
| Main surface | `--bg-panel` | Standard panels and primary content |
| Alternate surface | `--bg-panel-2` | Flush panels, code/editor backgrounds |
| Recessed surface | `--bg-sunken` | Tracks, wells, thumbnails, segmented tracks |
| Resting row | `--bg-row` | Cards and list-row hover surfaces |
| Strong row hover | `--bg-row-hi` | Hover within a surfaced element |
| Field | `--bg-field` | Input wells and editable values |
| Floating surface | `--bg-float` | Menus, modals, raised controls, toasts |
| Primary text | `--tx` | Main labels and values |
| Secondary text | `--tx-2` | Standard labels and supporting content |
| Muted text | `--tx-3` | Metadata and secondary actions |
| Quiet text | `--tx-4` | Section labels, timestamps, placeholders |
| Primary action | `--accent` | Apply/create, active toggles, current edit |
| Accent hover | `--accent-hover` | Hover only for accent-filled actions |
| Accent wash | `--accent-dim` | Selected rows, soft active regions |
| Accent text | `--accent-tx` | Active icon/text on a neutral surface |
| Auxiliary blue | `--blue` | Registered semantic hue when a feature explicitly needs blue; not a second default accent |
| Destructive | `--danger` / `--red` | Delete and errors |
| Caution | `--warning` / `--yellow` | Warnings and pending external action |
| Success | `--success` / `--green` | Completed/live success state |

Never hardcode neutral grays. They must come from the surface ladder, the text ladder, or `rgb(var(--ink-rgb) / alpha)`. The default light and dark palettes deliberately use different temperature and contrast.

### Type

| Token | Value | Use |
|---|---:|---|
| `--f-ui` | SF Pro Text/system sans | All interface copy |
| `--f-mono` | SF Mono/system mono | Numeric values, shortcuts, code, time |
| `--fs-xs` | 11px | Section labels, metadata, shortcut hints |
| `--fs-sm` | 12px | Buttons, compact list copy, numeric fields |
| `--fs-md` | 13px | Default UI text and rows |
| `--fs-lg` | 15px | Modal and page headings |
| `--fs-xl` | 18px | Rare major heading only |
| `--fw-regular` | 400 | Body and metadata |
| `--fw-medium` | 500 | Controls, panel labels, selected emphasis |
| `--fw-semibold` | 600 | Modal/page headings; use sparingly |

Rules:

- Default to 13px/400. A control label usually uses 13px; its value often uses 12px mono.
- Use medium weight for interaction hierarchy, not bold blocks of copy.
- Use tabular numerals for time, measurements, performance, and numeric readouts.
- Truncate one-line labels with ellipsis and keep the full value in `title` or an accessible label.
- Let long explanatory copy wrap with `line-height: 1.45–1.6` and `text-wrap: pretty`.
- Do not use all caps except extremely short machine/status labels already present in the status bar.

### Spacing and size

| Token | Normal | Meaning |
|---|---:|---|
| `--row-h` | 30px | Property and list-row rhythm |
| `--hdr-h` | 38px | Docked panel header |
| `--ctl-h` | 28px | Button, chip, and field height |
| `--hit` | 28px | Minimum common pointer target |
| `--gut` | 10px | Internal gutter |
| `--pad` | 12px | Shell and panel outer padding |
| `--gap` | 10px | Dock/panel gap and common inline gap |
| `--val-w` | 128px | Inspector value track |

Use multiples already present in the feature family: 2, 3, 4, 6, 8, 10, 12, 14, 16, 18, 20, 24. Avoid one-off spacing values unless aligning with measured canvas geometry.

Density is global and automatic:

| Density | Row | Header | Control | Default text | Pad/gap |
|---|---:|---:|---:|---:|---:|
| Compact | 26px | 32px | 24px | 12px | 9px / 8px |
| Normal | 30px | 38px | 28px | 13px | 12px / 10px |
| Comfy | 34px | 42px | 30px | 14px | 14px / 12px |

Never freeze a component to normal-density metrics when the equivalent token exists.

### Radius and depth

| Token | Value | Use |
|---|---:|---|
| `--r-xs` | 4px | Tiny buttons, swatches, checks |
| `--r-sm` | 6px | Fields, rows, normal buttons |
| `--r-md` | 8px | Segmented tracks, menus, compact cards |
| `--r-lg` | 12px | Panels and large inset regions |
| `--r-xl` | 16px | Modals, libraries, major cards |
| `--r-pill` | 999px | Toggles and genuinely circular/pill controls |

For nested rounded surfaces whose edges visually follow one another:

```text
outer radius = inner radius + inset
```

Example: an 8px inset around a 16px inner preview needs a 24px outer radius. Do not apply this to unrelated children, standalone buttons, circles, or pills.

Use `--ctl-edge` for raised controls, `--ctl-edge-inset` for wells, `--shadow-panel` for dock panels, `--shadow-lift` for a nearby lifted control, and `--shadow-float` for overlays. Do not invent drop shadows locally.

Large app-chrome surfaces use the shared superellipse through `--ui-corner-smoothing`. Project artwork and exported composition geometry must never inherit UI corner styling.

### Motion

| Token | Value | Use |
|---|---:|---|
| `--dur-1` | 75ms | Hover, press, selection |
| `--dur-2` | 100ms | Menus, toggles, small surfaces |
| `--dur-3` | 160ms | Modal and larger reveal |
| `--ease` | spring-like ease-out | Entrances and spatial movement |
| `--ease-io` | balanced ease-in-out | Toggles and continuous state |

Motion confirms cause and effect. It must never be the only indicator. Respect `prefers-reduced-motion`; the global stylesheet reduces all motion and specialized components must also remain understandable without animation.

## Layout library

### App shell

The shell has four planes:

```text
44px native titlebar: project tabs + tool strip + global actions
main body: left dock | fluid center dock | right dock
22px status line
overlays: menus < drag previews < scrim/modal
```

The center dock consumes remaining width. Typical rail widths are 250–320px. Right-side property panels should generally stay at least 220px wide. Never hide the viewer from a workspace; it is the recovery surface for the editor.

### Docked panel

Register the body and let the layout system supply `.panel`, its header, title, grip, radius, shadow, scrolling, drag, resize, collapse, and persistence.

`PanelDefinition` options:

| Option | Use |
|---|---|
| `title` | Short noun: `Media`, `Inspector`, `Effects` |
| `icon` | Meaningful icon name, unique among nearby panels |
| `size` | Preferred fixed height in a vertical dock |
| `min` | Real content minimum; avoid oversized minimums |
| `flush` | Content visually merges with `--bg-panel-2` |
| `noscroll` | Body owns scrolling or is a canvas/editor |
| `headless` | Toolbar/canvas surface that must not show panel chrome |
| `hideMoveHandle` | Only when the surface has a deliberate alternate move affordance |
| `moveSlot` | Put the move handle inside a known body slot |

Panel rules:

- One header only. Do not put a second title/card at the top of the body.
- Header actions are 24px `iconbtn panel-action` controls on the right.
- The header label is quiet; actions are not a miniature toolbar.
- The body owns one scroll direction. Nested scroll regions need a clear reason.
- Use `padding: var(--pad) var(--pad) 24px` for an inspector body and 6–10px for compact lists.
- Prefer one flexible panel per dock so unused space always has an owner.

### Standard panel registration

```ts
import type { PowermoveAPI } from 'powermove';
import MyPanel from './MyPanel.svelte';

export default function activate(api: PowermoveAPI) {
  api.panels.register({
    id: 'my-feature',
    title: 'My feature',
    icon: 'sliders',
    component: MyPanel,
    size: 220,
    min: 140
  });
}
```

```svelte
<script lang="ts">
  import type { PanelProps, PowermoveAPI } from 'powermove';

  let { panelId, api }: PanelProps & { api: PowermoveAPI } = $props();
</script>

<div class="my-panel" data-svelte-panel={panelId}>
  <!-- Body only. DockLayout owns the frame and title. -->
</div>
```

## Component library

### Icons

Always use the kernel icon set. It is a consistent filled Phosphor-style set and automatically inherits color.

```svelte
{@html api.ui.icon('plus')}
```

Current useful names:

```text
music film image puzzle note speedometer sliders timeline tools dot missing
play pause prev next home project plus x undo redo panel grip up export clock
search list trash more chev chevD eye eyeoff lock link magnet layers wand grid
cursor hand zoom type shape solid frame code cam graph bezier gear sparkle sun
moon diamond enter return
```

Every icon-only button needs `aria-label` and usually `title`. Normal icon size is 13–15px. Do not use emoji, text glyphs, or a second icon library.

### Icon button: `.iconbtn`

Use for a single compact action in a header, toolbar, row, or overlay.

```html
<button class="iconbtn" type="button" title="Add effect" aria-label="Add effect">
  <!-- api.ui.icon('plus') -->
</button>
```

States:

- Rest: transparent, `--tx-2`.
- Hover: `--ink-1`, primary text.
- Press: scale to `.94`, `--ink-2`.
- Selected: `.on`, neutral selected surface. Use accent text only for a special active tool.
- Disabled: native `disabled` or `.dis`; opacity `--disabled`.
- Panel header: add `.panel-action`; it becomes 24px with a 13px icon.

### Button: `.btn`

Use for dialog actions and explicit rectangular commands.

| Variant | Class | Use |
|---|---|---|
| Default | `.btn` | Normal action |
| Quiet | `.btn.ghost` | Cancel, tertiary action |
| Primary | `.btn.pri` | One decisive action per local surface |
| Destructive text | `.btn.danger` | Destructive action; confirm if material |

Do not put two primary buttons in one action group. Prefer verb labels: `Apply`, `Import`, `Save take`, `Delete`.

### Chip/action: `.chip`

Use inside compact panels for a standalone action.

| Variant | Class | Use |
|---|---|---|
| Default | `.chip` | Compact raised action |
| Quiet | `.chip.ghost` | Low-priority action |
| Full width | `.chip.wide` | Empty-state or end-of-list action |
| Primary | `.chip.solid` | Creation/apply action |

Properties-panel chips use the flat `--bg-field` surface without raised edges or shadows. Full-width actions align to the content gutter with no horizontal margins. Hover uses `--bg-row-hi`; keyboard focus stays visible.

Use `.btn` in modal/page action bars and `.chip` inside compact panel content. Do not mix them arbitrarily in the same group.

### Segmented control: `.segmented`

Use for 2–4 mutually exclusive sibling views, never for independent toggles.

The internal `Segmented.svelte` provides `role="tablist"`, roving tab focus, arrow keys, Home/End, and `.on`. Its props are:

```ts
options: Array<{ id: string; label: string }>
value: string
onChange(id: string): void
label?: string
```

Selected segments are raised neutral surfaces. They are not accent-filled.

Inside a dialog or overlay the control takes the Settings tab material: a `--bg-field` track with `--ctl-edge-inset`, 28px segments, and a `--bg-float` selected segment lifted by `--ctl-edge`.

### Section: `Section.svelte` / `.sec`

Use to divide a property panel by whitespace and a quiet label.

```svelte
<Section title="Transform" />
```

Sections do not need cards or divider lines. The standard margin is intentionally generous above and tight below. Use sentence case.

### Property row: `Row.svelte` / `.row.split`

The property row is the most important reusable layout primitive.

```svelte
<Row label="Opacity">
  <NumField {PM} get={() => opacity} edit={opacityEdit} min={0} max={100} unit="%" />
</Row>
```

It supplies:

- a label in flexible left space;
- a fixed `--val-w` right value track;
- shared labeling context for field accessibility;
- consistent height, hover wash, truncation, and alignment.

Use the optional `left` snippet for a stopwatch, disclosure, or small state control. Use `onLabel` only when dragging/clicking the label has a meaningful editor behavior. Do not hand-build property rows unless the content genuinely cannot fit this model.

### Numeric field: `NumField.svelte`

Use for all quantities.

Important props:

| Prop | Meaning |
|---|---|
| `get` | Current source-connected value |
| `edit` | Shared edit binding |
| `min`, `max` | Real domain bounds |
| `step` | Keyboard/scrub step |
| `speed` | Horizontal scrub sensitivity |
| `unit` | Visible `px`, `%`, `s`, `°`, etc. |
| `precision` | Explicit decimal precision when needed |
| `label` / `ariaLabel` | Accessible name outside `Row` or for paired fields |

Behavior is part of the design:

- Drag horizontally to scrub.
- Click, focus, Enter, or F2 to edit text.
- Accept arithmetic such as `+10`, `*2`, and expressions.
- Arrow keys step; Shift multiplies by 10; Option/Alt makes pointer scrubbing precise.
- Escape cancels; Enter/blur commits.
- A drag is one edit gesture and one Undo step.

Never replace this with a plain `<input type="number">` in an editor surface.

### Text field: `TextField.svelte`

Use for short source-connected strings. Props include `align` and `mono`. The field writes during the gesture, commits on blur/Enter, and cancels on Escape.

Use a plain `<textarea>` only for genuinely multiline content. It must stop global shortcuts, preserve text selection, and commit through one edit transaction.

### Select field: `SelectField.svelte`

Use for a compact choice in a property row. Options may be strings or `{ v, label }`. Values use mono type by default to match other wells.

Use `Segmented` instead when there are very few sibling modes and instant visibility is valuable. Use a searchable picker/menu for long collections.

### Toggle field: `ToggleField.svelte`

Use for a boolean property. It renders a 32×18 pill and supports `aria-pressed`. The active track uses the accent. Do not use checkboxes for ordinary inspector booleans.

Checkboxes remain appropriate in multi-select pickers where each row independently joins a set.

### Color field: `ColorField.svelte`

Use for a single solid color. The resting well shows a 16px swatch and uppercase hex value. Its picker provides presets, direct hex input, focus trapping, Apply/Cancel, outside dismissal, and Escape.

Do not create inline browser color inputs. Keep color changes previewed in the dialog and committed as one Undo step on Apply.

### Fill field: `FillField.svelte`

Use when a value may be solid, linear gradient, radial gradient, or none. It owns the full fill workbench: saturation/value, hue, channels, palette, stops, stop positions, angle, Add stop, Apply, and Cancel.

Use `ColorField` when gradients are not part of the data model. Do not expose a complex fill picker for a simple color property.

### Font field: `FontField.svelte`

Use for font family. It provides a searchable menu, font previews, current selection, truncation, keyboard search/choose, outside dismissal, and font loading through `PM.Fonts.ensure`.

Pair it with a separate weight `SelectField`, as `ContentSection.svelte` does.

### Animated channel row: `ChannelRow.svelte`

Use for a keyframable numeric property in the inspector. It composes:

```text
stopwatch | label | numeric value | optional keyframe diamond
```

It also owns expression state, timeline reveal, graph-editor access, reset, easing menu, and keyframe add/remove. Scale uses the same row with linked X/Y behavior. Do not recreate any of those behaviors in a feature-specific field.

### List row: `.lyr`

Use for compact text-first lists such as takes, workspaces, layers, or simple effects.

```html
<div class="lyr simple-panel-row">
  <button class="simple-row-action" type="button">
    <span class="sw2"></span>
    <span class="nm">Item name</span>
    <span class="idx">Meta</span>
  </button>
  <button class="stopwatch" aria-label="Delete item">…</button>
</div>
```

Use `.sel` or an ARIA-selected attribute for selection. Keep row actions invisible or quiet until hover/focus when the action is secondary.

### Media/card row: `.asset-card`

Use when a narrow-panel list benefits from a visual preview and two lines of metadata.

```text
38px preview | name + metadata | contextual actions
```

It already defines hover, selection, truncation, image/icon preview, and reveal-on-hover actions. For a text-only collection, use `.lyr` instead.

### Page/card collection: `.library-*` and `.ps-*`

Use these only for full-page or full-overlay collections, not ordinary dock panels.

- `.library-card`: persistent catalog item with thumbnail, metadata, actions.
- `.ps-card`: project card with 16:9 preview, selected state, grid/list variants.
- `.library-sidebar` / `.ps-sidebar`: 168–240px navigation rail.
- `.library-grid` / `.ps-grid`: responsive `auto-fill` grid.
- `.library-empty` / `.ps-empty`: large-page empty state.

Cards use `--r-xl`; their thumbnails use smaller nested radii. Hover may lift by 1px. A dock panel should almost never contain a grid of large library cards.

### Empty state: `.empty`, `.asset-empty`, `.library-empty`

Choose by surface size:

| Surface | Pattern | Copy structure |
|---|---|---|
| Small panel | `.empty` | One useful sentence |
| Visual list | `.asset-empty` | Icon + short heading + one sentence |
| Full page/overlay | `.library-empty` | Icon + heading + explanation + optional one action |

State what the user can do next. Avoid celebratory illustrations and decorative filler.

### Search field

Use the closest existing family:

- Inspector/list panel: `.fxb-search`.
- Panel library: `.panel-library-search`.
- Full library/project page: `.library-search` or `.ps-search`.
- Modal/form: `.field` around an input.

All use `--bg-field`, 32–34px height, a 14–15px search icon, and an accent-aware edit state. Do not place a large web-style search box in a narrow dock.

### Menu: `api.ui.menu(...)`

```ts
api.ui.menu(anchor, [
  { header: 'Arrange' },
  { label: 'Move up', kb: '⌥↑', run: moveUp },
  '-',
  { label: 'Delete', disabled: !canDelete, run: remove }
]);
```

The shared menu provides viewport clamping, keyboard navigation, separators, headings, shortcuts, selected checks, disabled items, focus restoration, and dismissal. Never build an absolutely positioned menu locally.

Menus are opaque `--bg-float`, borderless, clipped to `--r-md`, and use `--shadow-float`. Curve/easing menus may use the established 4-column visual grid.

### Modal: `api.ui.modal(...)`

```ts
api.ui.modal({
  title: 'Delete media?',
  width: 420,
  body: 'This removes every layer using the file. You can undo this.',
  actions: [
    { label: 'Cancel' },
    { label: 'Delete', pri: true, run: remove }
  ]
});
```

The modal owns scrim, focus trap, Escape, initial focus, `--r-xl`, floating shadow, body scroll, and action alignment. Use 400–540px for common dialogs. Put the primary action last. Do not use a modal for a reversible, low-risk action that can happen directly.

Settings is the reference dialog, and every other popped surface follows it. Any sheet that floats over the app — including the full-overlay `#library-screen` — is `--bg-float` with `--r-xl` and `--shadow-float` and **no border**; the float shadow already carries the hairline. Its scrim is the modal's scrim (`rgb(var(--ink-rgb) / .32)` with a 3px blur, `rgb(0 0 0 / .5)` in dark), its title is `--fs-lg` semibold at `-.01em`, its view switcher is the Settings tab bar, and its fields are 32px `--bg-field` wells that show `--focus-ring` on focus. Do not give a floating surface its own scrim color, outline, field height, or focus treatment.

A dialog that can be reached from more than one place (menu item, shortcut, titlebar button) opens once. Reuse the open instance and move it to the requested tab instead of stacking a second copy — `PM.SettingsUI.open(tab)` is the reference.

### Toast: `api.ui.toast(...)`

Use for brief confirmation, completion, or recoverable error after an action. Keep the message short and specific: `Added Gaussian Blur`, `Notes saved`, `Copy an effect first`.

Success/status toasts use a green dot; errors are detected and use red. Sticky toasts are reserved for information that requires attention. Do not toast every keystroke, hover, selection, or autosave tick.

### Command palette

Register commands through `api.commands`; they appear in the shared palette. Do not create feature-local command launchers. Categories are quiet 11px text, labels are 13px, and shortcuts are 11px mono.

### Status item

Use `api.status.register` for a small global fact that remains useful while editing.

```ts
api.status.register({
  id: 'my-feature-state',
  side: 'right',
  text: () => active ? 'Feature on' : null,
  title: 'Feature status'
});
```

Keep status text terse. It should not compete with the editor or become a button bar.

### Code editor

Use `.code` for plain shader/code text and `.codebar` for compile state/actions. It uses the alternate panel background, SF Mono, a 1.6 line-height, and accent caret. Code areas own their scrolling and should normally register their panel with `noscroll`.

### Generated tool controls

When an agent-generated tool can be expressed as schema, prefer `GeneratedPanel.svelte` over custom UI. Supported controls:

```text
slider text color fill toggle select button readout curve
```

Generated controls automatically compose the same `Row` and field primitives. Bind every project-facing control to a real source target. Use local state only for tool configuration that is not document state.

For a complex action:

1. Let controls change local configuration.
2. Show a structured preview through `PreviewList.svelte`.
3. Apply the complete source edit atomically.
4. Make the whole result one Undo step.

### Agent/chat primitives

The agent rail is a specialized surface, not a general style source.

- User messages may use the compact right-aligned `.agent-bubble`.
- Assistant responses remain flat on the panel surface for reading hierarchy.
- Tool/activity rows are muted; only the newest live action pulses.
- The composer is the strongest rounded surface in the rail.
- The send button becomes accent-filled only when sendable.
- Stop is always available during a live run.
- Attachment names truncate inside the rail and expose the full name on hover.

Do not copy chat bubbles, 20px composer radii, or streaming animation into ordinary editor panels.

## Editing and behavior contract

Visual consistency without edit consistency is a broken Powermove feature.

### Document edits

Use, in order:

1. `api.project.apply(commandOrCommands, meta)` in extensions.
2. A binding from `api.ui.controls.binding` in built-in property panels.
3. `PM.Edit.apply(...)` only in internal/legacy seams.

Never mutate document fields directly unless the code contains a clearly documented migration exception.

One user intent equals one history entry:

- pointer scrub: begin → repeated writes → commit;
- text edit: begin on focus → write on input → commit on blur/Enter;
- complex picker: edit a draft → Apply once;
- multi-property transform: one command array with one label;
- cancel/Escape: restore the original state and cancel history.

### Preview, Apply, Undo

Use preview-first UI when an operation is expensive, broad, generated, or hard to infer from one control. A preview must explain the intended source changes, not merely show decorative loading. `Apply` changes real project data. Undo returns to the exact previous state in one step.

### Selection

- Neutral `--ink-2` is the standard chrome selection.
- `--accent-dim` plus a quiet accent ring is for an actively editable object/effect.
- The timeline also follows the active workspace accent. Do not hardcode a separate playhead/selection color.
- Preserve multi-selection. Do not silently collapse it unless the operation is defined for one item.
- Selection rows need `aria-selected`; multi-select containers need `aria-multiselectable`.

### Loading and progress

Use a restrained spinner, a changing status line, or the existing panel placement ghost. Loading overlays must not block interaction unless the underlying operation truly cannot be used. The UI placement ghost is click-through, and it stands exactly where the panel will: a new panel's ghost takes a real slot in its dock, so the surrounding panels settle into their final sizes while it is still being built, and a ghost for an existing panel is pinned inside that panel. It previews the shape that is coming — a panel header naming the work, then skeleton rows filling the height it was given — over a Motion GPU field: a slow flowing, breathing violet light that shares the shake ripple's palette. It falls back to the plain static card whenever WebGPU is unavailable or reduced motion is preferred.

Never show a decorative “sneak peek” that looks like generated content but is not real content.

### Destructive actions

- Reveal delete controls on hover/focus for list rows.
- Use the danger color for the destructive action, not the entire dialog.
- Confirm when deletion cascades, cannot be undone, or removes external data.
- State the impact in plain language and mention Undo when available.

## Interaction state matrix

Every interactive component must define these states where applicable:

| State | Visual behavior |
|---|---|
| Rest | Mostly transparent or quiet field surface |
| Hover | One alpha step stronger; primary text becomes clearer |
| Pressed | Small scale or darker wash; no long animation |
| Selected/on | Neutral raised/wash state; accent only as a meaningful signal |
| Editing | Floating/field surface plus one accent edge or control-owned focus state |
| Disabled | `opacity: var(--disabled)` and no pointer action |
| Keyboard focus | Component-owned state or `--focus-ring`; never a browser-default outline |
| Busy | Disable duplicate action, show compact spinner/status, retain Stop/Cancel if supported |
| Error | Danger text or low-alpha danger wash; preserve readable neutral background |
| Empty | Guidance and at most one obvious next action |

The global stylesheet removes default outlines. Therefore, any custom keyboard control must provide a visible component-owned focus state. Reuse `box-shadow: var(--focus-ring)` for rows/buttons that need an explicit ring.

## Accessibility and keyboard contract

Native-feeling does not mean pointer-only.

- Use real `button`, `input`, `select`, and `textarea` elements.
- Icon-only actions require `aria-label`; tooltips do not replace accessible names.
- Use `role="listbox"`/`option` for selectable collections and roving `tabindex` for compact lists.
- Arrow keys navigate lists, segments, menus, and tabs. Home/End jump to boundaries.
- Enter activates the primary row action. Space selects/toggles when expected.
- Escape cancels editing or dismisses the current overlay.
- Modal and picker focus stays trapped until dismissed and returns to the trigger.
- Text inputs stop editor shortcuts from stealing typing.
- Announce nonvisual feedback through `.panel-sr-only` with `role="status"` or `aria-live`.
- Do not depend on color alone. Pair status color with text, shape, icon, or placement.
- Keep secondary actions reachable with `:focus-within`, not hover alone.

## Responsive and narrow-panel behavior

Panels can be moved and resized. Assume every panel will eventually be narrower than expected.

- Every flex child that may truncate needs `min-width: 0`.
- Use `overflow: hidden; text-overflow: ellipsis; white-space: nowrap` for one-line identity text.
- Keep action groups `flex: none` so they do not crush labels unpredictably.
- Use the `--val-w` track for inspector rows; only change it at a deliberate panel breakpoint.
- Avoid fixed content widths inside dock panels.
- Full-screen libraries may switch to a single-column grid below 760px.
- Test at the panel definition’s minimum width and in compact density.

## Copy language

Powermove copy is direct and editor-specific.

- Panel titles: nouns (`Media`, `Takes`, `Inspector`).
- Commands: verbs (`Import media`, `Add effect`, `Save take`).
- Section labels: concrete groups (`Transform`, `Content`, `Layer`).
- Empty states: explain what belongs here or the next action.
- Errors: say what failed and what the user can do.
- Avoid “Oops,” marketing language, exclamation marks, and vague labels like `Submit` or `Continue` when a concrete verb exists.

Use the multiplication sign `×`, middle dot `·`, ellipsis `…`, and macOS shortcut glyphs consistently where the existing surface uses them.

## Feature recipes

### Recipe: property panel

Use when editing selected or project-level values.

```svelte
<script lang="ts">
  const { Row, Section, NumField, SelectField, ToggleField } = api.ui.controls;
  const PM = api.host.pm as Record<string, any>;
</script>

<div class="insp" data-svelte-panel={panelId}>
  <Section title="Appearance" />
  <Row label="Opacity">
    <NumField {PM} get={() => opacity} edit={opacityEdit} min={0} max={100} step={1} unit="%" />
  </Row>
  <Row label="Blend mode">
    <SelectField {PM} get={() => blend} edit={blendEdit} options={blendModes} />
  </Row>
  <Row label="Enabled">
    <ToggleField {PM} get={() => enabled} edit={enabledEdit} />
  </Row>
</div>
```

Use the same section order as the Inspector when possible: content → transform/animation → effects → layer/advanced.

### Recipe: compact list panel

Use when the panel manages named items.

```svelte
<div class="simple-panel-list" data-svelte-panel={panelId}>
  {#each items as item (item.id)}
    <div class="lyr simple-panel-row" class:sel={item.id === selectedId}>
      <button class="simple-row-action" type="button" onclick={() => select(item.id)}>
        <span class="nm">{item.name}</span>
        <span class="idx">{item.meta}</span>
      </button>
      <button class="stopwatch" type="button" aria-label={`Delete ${item.name}`} onclick={() => remove(item)}>
        {@html api.ui.icon('x')}
      </button>
    </div>
  {:else}
    <div class="empty">No items yet.</div>
  {/each}
  <button class="chip wide" type="button" onclick={add}>{@html api.ui.icon('plus')} Add item</button>
</div>
```

Add roving keyboard focus when the list is selectable, and expose row status through a screen-reader-only live region.

### Recipe: searchable browser

Use a header action to reveal search. Keep the body as grouped sections of compact rows. Flatten groups while a query is active. Enter/click performs the default action; drag may provide a secondary editor-native path if the item can be dropped meaningfully.

`FxBrowserPanel.svelte` is the reference.

### Recipe: complex picker

Use a local draft inside a shared modal/picker surface:

1. Copy the source value into `draft` when opened.
2. Let every control edit only `draft`.
3. Preview the draft immediately inside the picker or canvas.
4. Cancel discards it.
5. Apply sends one source command.
6. Return focus to the trigger.

`FillField.svelte` is the reference.

### Recipe: page-sized library

Use a rail + main content plane, not a panel nested in a panel:

```text
search
scope/navigation       page title · count · view/sort/actions
                       responsive card grid or list
```

Keep filters in the rail, collection actions near the page title, and item actions on the card. `#library-screen` and `#projects-screen` are the references.

### Recipe: result from an agent/generated operation

Use a compact, structured preview with a title, message, and list of changes. Error previews use a quiet danger wash. Do not dump raw JSON, logs, or code into the normal panel unless the feature is a developer tool.

## Things not to introduce

- A second design token file or feature-specific theme.
- Tailwind-style arbitrary colors in component markup.
- New fonts for app chrome.
- Gradient borders, glass blur, or decorative glows on ordinary panels.
- Cards inside cards inside a docked panel.
- Permanent action bars when contextual actions can live in the header or appear on approach.
- Orange on every selected control.
- A hardcoded alternate selection color when the workspace accent already carries that state.
- Browser-default number, color, range, dialog, or context-menu UI when a Powermove primitive exists.
- A custom popup without shared dismissal, focus, and viewport behavior.
- A feature that mutates project state outside the edit boundary.
- A control that only works with a mouse.
- A panel that requires a full app restart to show a renderer/interface change.

## Implementation checklist

Before coding:

- [ ] Identify the closest reference component.
- [ ] Decide whether this is a dock panel, overlay, page, or canvas control.
- [ ] List every document edit and its Undo label.
- [ ] Choose existing tokens and icon names.
- [ ] Decide the empty, loading, error, and narrow states.

While coding:

- [ ] Reuse `api.ui.controls` or the exact internal component.
- [ ] Use `api.ui.icon` for icons.
- [ ] Keep one body scroll owner.
- [ ] Add accessible labels and keyboard behavior.
- [ ] Bind values to real source; keep only tool configuration local.
- [ ] Make a continuous gesture one history transaction.
- [ ] Support Cancel/Escape without leaving partial state.
- [ ] Use tokenized colors, metrics, radii, shadows, and motion.

Before handing off:

- [ ] Verify normal and minimum panel width.
- [ ] Verify compact, normal, and relevant comfy density.
- [ ] Verify light and dark; use high contrast when the change touches tokens/focus.
- [ ] Verify rest, hover, pressed, selected, focused, disabled, empty, loading, and error states.
- [ ] Verify keyboard navigation and focus restoration.
- [ ] Verify Apply, Cancel, one-step Undo, and Redo.
- [ ] Verify the existing visible app updated in place; do not restart it.
- [ ] Run focused unit tests, `npm run typecheck`, and the hidden Electron harness as appropriate.
- [ ] Keep all test windows hidden and use isolated `POWERMOVE_USER_DATA`.
- [ ] Review the diff for hardcoded color, spacing, radius, shadow, icon, and direct document mutation.

## Fast reference by task

| Need | Reuse |
|---|---|
| New dock panel | `api.panels.register` + body-only Svelte component |
| Numeric property | `Row` + `NumField` |
| Keyframable property | `ChannelRow` |
| Boolean property | `Row` + `ToggleField` |
| Short choice | `SelectField` or `Segmented` |
| Color | `ColorField` |
| Gradient/fill | `FillField` |
| Font | `FontField` + weight `SelectField` |
| Short text | `TextField` |
| Multiline text | tokenized textarea + one edit transaction |
| Compact named list | `.lyr` |
| Media list | `.asset-card` |
| Large collection | `.library-card` / `.ps-card` |
| Search | closest `.fxb-search`, `.panel-library-search`, `.library-search`, or `.field` pattern |
| Icon-only action | `.iconbtn` + `api.ui.icon` |
| Panel action | `.iconbtn.panel-action` in the dock header |
| Explicit action | `.btn` in dialogs/pages, `.chip` inside panels |
| Context menu | `api.ui.menu` |
| Dialog | `api.ui.modal` |
| Brief feedback | `api.ui.toast` |
| Global command | `api.commands.register` |
| Global compact fact | `api.status.register` |
| Schema-generated tool | `GeneratedPanel` control schema |
| Complex generated edit | local controls → structured preview → atomic Apply → one Undo |

The goal is not for every feature to look identical. The goal is for every feature to feel as if it was always part of the same editor: the same materials, rhythm, controls, edit semantics, keyboard model, and restraint.
