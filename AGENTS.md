# Powermove agent instructions

Explain technical work in plain language. Assume the user has very little coding experience.

## Interface-panel changes

Interface-panel changes must update in place without reopening Powermove. Keep the current app process, window, and editing session open. This includes panel components, registration, layout, styling, and built-in panel extensions under `src/extensions/`.

Use the existing renderer hot reload or extension reload path and verify the requested change in the already-open Electron window. Do not quit, relaunch, or force a whole-window reload just to display a panel change. If the live update fails, investigate it instead of masking the problem with a restart. If a change genuinely cannot take effect without restarting, explain why and ask the user first.

Preserve unrelated working-tree changes, and commit only files that belong to the requested change.

## Background-only testing

Never open or show a new window for testing, including headed browsers, detached DevTools, or a visible second copy of Powermove. Use `npm test`, `npm run typecheck`, and the hidden Electron harness (`npm run test:e2e`). Custom Electron tests must set `POWERMOVE_BACKGROUND_TEST=1`, `POWERMOVE_DEVTOOLS=0`, and `POWERMOVE_USER_DATA` to a new absolute temporary directory. Never use the user's live profile for tests.

Screenshots, clicks, persistence, and Apply/Undo checks can run in the hidden renderer. If background testing cannot cover an OS-specific interaction, explain the gap and coordinate with the user; never silently fall back to a visible test window. Background test processes may restart their isolated session, but must never restart or reload the user's existing editing window.

Ask the user whether to commit completed changes; stage only the intended changes.
