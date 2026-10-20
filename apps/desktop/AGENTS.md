# UI preferences

Themes must fully unapply when disabled, removed, or reloaded. Follow "Reversible themes" in `docs/EXTENSIONS.md`: use owned theme registrations and workspace appearance changes, clean up every custom side effect, preserve later user edits, and verify the complete on/off lifecycle across workspaces and restarts. Compilation alone does not verify cleanup.

Prefer clean surfaces and spacing over decorative outline borders. Use borders only when they clarify an interactive control or otherwise serve a clear purpose; retain accessible focus indicators. Avoid redundant helper text and implementation details in product flows.

New and modified panels must match the native Powermove panels one to one unless the user explicitly asks for a different style. Always build from existing components (`src/renderer/src/controls`: `Row`, `Section`, `Segmented`, `TextField`, `NumField`, `SelectField`, `ToggleField`, color/fill/font fields, shared menus and icons) and copy the closest built-in panel's layout before writing new UI. Do not hand-roll outlined pills, accent-ringed toggles, bordered buttons, divider lines, nested cards, custom palettes, or helper/attribution captions. Compare the result against a built-in panel in light and dark themes before finishing. The extension-facing version of this rule is "Native panel design" in `docs/EXTENSIONS.md`.
