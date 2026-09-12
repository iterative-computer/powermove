# Agent testing without visible windows

Run checks from the Powermove source checkout, not an Agent Workspace (which does not contain the app's package scripts). The desktop app lives in `apps/desktop` of the bun workspace; run its scripts from that directory (or with `bun run --cwd apps/desktop <script>` from the repository root):

```sh
bun run test
bun run typecheck
bun run test:e2e -- e2e/agent-threads.spec.ts
```

Use the package scripts, not `bunx vitest`: `bunx vitest` runs Vitest under bun's runtime instead of Node and skews a calibrated performance test. Pass Vitest files or flags after `--`, for example `bun run test -- src/main/storage.test.ts`.

The Electron helper in `e2e/helpers/app.ts` always enables background testing. It creates a temporary profile, renders the real app with `show: false`, disables focus, taskbar presence, and DevTools, and keeps painting active. Playwright can still click controls, type, inspect editable project data, capture screenshots, and test persistence by restarting only its isolated copy. The HTML report never opens automatically. Deterministic media fixtures for import and export coverage live in `e2e/fixtures/`.

Custom launchers must set `POWERMOVE_BACKGROUND_TEST=1`, `POWERMOVE_DEVTOOLS=0`, and `POWERMOVE_USER_DATA` to a new absolute temporary directory. Use the helper whenever possible. Never launch a headed browser or visible copy as a fallback, and never supply a live profile. Background mode refuses the default/live profile. It is a window-launch guard for this app, not an OS sandbox for arbitrary shell commands or third-party apps.

Report exactly what was tested. Hidden-renderer screenshots and interactions prove renderer behavior, not native focus, file dialogs, permission prompts, or the already-open desktop instance. Those require explicit user coordination.

Thread controls save separate conversations, drafts, attachments, and focus choices per project. Titles come from the first user message. A thread keeps its own Codex session for each authority. Finish or stop a run before switching. Pending edits/checkpoints are retained during a switch only while the document revision is unchanged, and are not restored across app launches.

Component and extension hot updates are the preferred path for the live editor. When a core engine change has no safe hot replacement, preserve the current editing session, report the pending update, and ask before restarting. Do not mistake a successful background check for a live update.
