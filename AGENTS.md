# Powermove agent instructions

Explain technical work in plain language. Assume the user has very little coding experience.

## Interface-panel changes

After changing anything that affects a visible interface panel, fully quit and reopen the Powermove development app before reporting the work as complete. This includes panel components, registration, layout, styling, and built-in panel extensions under `src/extensions/`.

Start the fresh app from the repository root with `npm run dev`, then verify the requested change in the newly opened Electron window. Hot reload, unit tests, or a browser preview alone do not satisfy this requirement.

Preserve unrelated working-tree changes, and commit only files that belong to the requested change.
