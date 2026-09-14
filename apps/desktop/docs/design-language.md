# Powermove interface language

Use flat, quiet surfaces throughout the desktop app, including panels, settings, project home, libraries, and onboarding.

- Buttons use solid tonal fills or transparent backgrounds. Do not add decorative gradients, bevels, top highlights, or drop shadows. The existing `--ctl-raised`, `--ctl-edge`, and `--shadow-raise` names remain compatible with extensions but now resolve to flat materials.
- Group related content with spacing and a subtle background. Error and result cards have no decorative outline. Recovery actions belong inside the card they act on.
- Reserve elevation for floating menus, dialogs, and drag previews. Color gradients are appropriate when they represent actual content, such as a color picker or an effect preview.
- Preserve clear selected, hovered, disabled, and keyboard focus states. Focus rings, selection outlines, input boundaries, and resize handles convey interaction and must remain legible.
- Use shared typography, spacing, radii, and theme tokens. Check both light and dark themes; do not hardcode a dark surface into a light panel.
- Fit controls to narrow panels. Model and reasoning labels may truncate, but their triggers and focus targets must stay reachable. Floating menus must respect viewport margins and only scroll when their content needs it.

Review the actual running UI at normal and narrow widths. Check project home, editor panels, settings, library, export, contextual menus, and error recovery. Keep model settings honest: expose supported reasoning choices and send the visible selection with the request.
