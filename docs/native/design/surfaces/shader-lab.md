# Shader Lab

Row S096. Grammar: [README](README.md) §1–5. Sources:
`R/panels/ShaderPanel.svelte`, `R/panels/shader/{ShaderEditor,CompileStatus}.svelte`.
Uniform controls stay in Properties ([inspector.md](../panels/inspector.md)). No
new tokens.

## S096 Shader Lab panel

Dockable panel `shader`, title `Shader Lab`. It edits the selected shader layer, or
the first in the project when none is selected, as v1.2.0.

```
┌ Shader Lab ───────────────────────────────┐ header
│ 1  void mainImage(out vec4 c, vec2 uv) {  │ editor, fills the panel
│ 2    c = vec4(uv, 0.5, 1.0);              │
│ ⚠ 3  }                                    │ gutter marks the failing line
├───────────────────────────────────────────┤
│ ✓ Compiled · 3 uniforms  [Presets ⌄] [Compile] │ code bar, m.header
└───────────────────────────────────────────┘
```

| Element | Spec |
|---|---|
| Editor | native `NSTextView`, `MONO_FONT` `t.body`, `dense_leading`, on `c.field`, no rich text, smart quotes and substitutions off, spelling off. Line numbers in a gutter, `t.minimum` tabular `c.muted`, right-aligned, `c.panel_secondary`. Inset `m.text_padding`. Horizontal scroll, no wrap |
| Syntax colour | none. v1.2.0 has none, and language §1 keeps colour for the work |
| Status | leading in the bar, `t.body`. Success: `checkmark.circle.fill` `c.success` + `Compiled · {n} uniforms`. Failure: `exclamationmark.triangle.fill` `c.danger` + first line of the error, truncated to the column, full text as tooltip |
| Presets | `NSPopUpButton`, pull-down style, title `Presets`, items from the shipped preset list, AX `Shader preset` |
| Compile | standard push button `Compile`, tooltip `Compile · ⌘Return`. Not accent: the one prominent button belongs to sheets and the toolbar |
| Line mark | when the compiler reports a line, gutter shows `exclamationmark.triangle.fill` `c.danger` at `m.symbol_inline`, with the message as tooltip; AX reads `Error on line {n}: {message}` |

| State | Spec |
|---|---|
| No shader layers | `EmptyState` `chevron.left.forwardslash.chevron.right`, title `No Shader Layers`, line `This project does not contain any editable shader layers.` |
| Compiling | status holds its previous text; under 0.5 s nothing changes, then small spinner + `Compiling…` |
| Layer removed while open | moves to the next shader layer or the empty state; text edits are already committed, so nothing is lost |
| Locked layer | editor read-only; status `Unlock the layer to edit its shader.` `c.muted` |
| Disabled bar | no layer: bar hidden |

Editing: typing writes to the layer as one live gesture, undo name `Edit Shader`, so
⌘Z steps by gesture rather than by key. Choosing a preset replaces the source as one
undo step `Shader Preset`. The renderer keeps showing the last good program while
source is invalid.

Keyboard: ⌘Return compiles. Tab inserts two spaces. **Esc arms the exit:** the next
Tab leaves the editor for the code bar, and any other key disarms it, so keyboard
users are never trapped. Editor shortcuts never fire while the editor has
focus (W010 rule, as the agent composer). IME and Emoji & Symbols stay available.

VoiceOver: text area `Shader source code`, description `Tab inserts two spaces. Press
Escape, then Tab to leave the editor.` The status is a
polite live region; failures post `.high`.

Increase Contrast: gutter gains an `m.border` `c.separator` trailing edge.

## Don't

- Don't add syntax colours or an autocomplete popup.
- Don't blank the viewer on a compile error.
- Don't use the accent for Compile.
