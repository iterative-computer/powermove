# Powermove agent instructions

Explain technical work in plain language. Assume the user has very little coding experience.

## Interface-panel changes

Interface-panel changes must update in place without reopening Powermove. Keep the current app process, window, and editing session open. This includes panel components, registration, layout, styling, and built-in panel extensions under `src/extensions/`.

Use the existing renderer hot reload or extension reload path and verify the requested change in the already-open Electron window. Do not quit, relaunch, or force a whole-window reload just to display a panel change. If the live update fails, investigate it instead of masking the problem with a restart. If a change genuinely cannot take effect without restarting, explain why and ask the user first.

Preserve unrelated working-tree changes, and commit only files that belong to the requested change.
