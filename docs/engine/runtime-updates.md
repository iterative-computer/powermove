# Agent runtime updates: scratch cleanup and test launches

`apps/desktop/src/main/runtime-updates.ts` downloads a newer Claude Code or
Codex runtime from npm when the bundled one is too old. This note covers two
fixes to that path: the scratch directories it leaked, and test launches that
triggered it.

## The leak

`installRelease` downloads the npm tarball into a `mkdtemp` directory in
`os.tmpdir()`, extracts it there, copies the binary into
`userData/runtimes/.versions`, and removes the directory in `finally`. When
the process is killed mid-install (the user quits during a download, a crash,
or an e2e or harness teardown), `finally` never runs. The directory is left
with `package.tgz` (about 122 MB for Claude Code) and often the extracted
`package/` as well, about 380-390 MB in total.

The automatic refresh (`refreshRuntime` in `codex/index.ts`) runs 20 s after
the first model-list request for each provider when that account is
connected. Every app launch that lived past 20 s with the developer's Claude
login started a download, and every launch killed before it finished leaked
one directory. Measured on the development Mac on 2026-10-23, before this
branch was merged: 45 `powermove-claude-*` directories and 1 `powermove-codex-*`
in `$TMPDIR`, 13 GB in total. 42 of them held `package.tgz`, and the newest were
12 minutes old (launches from other lanes still running without the switch).

## Scratch layout and the startup sweep

New downloads use `powermove-runtime-<provider>-XXXXXX`. Only
`installRelease` uses that prefix. Its first write is `owner.pid` (the
creating process id), followed by `package.tgz` and `package/`.

`configureRuntimeUpdates` (called once from `main/index.ts` at startup) starts
`sweepRuntimeScratch()` without awaiting it, so boot is not delayed. The sweep
does one `readdir` of the temp directory and `lstat` calls on matching names
only. `installRelease` awaits the sweep before creating its own directory, so
the sweep never races an install in the same process.

The sweep's rules:

- Symlinked entries are skipped (`Dirent.isDirectory()` is false for them),
  and so is anything that `lstat` doesn't report as a real directory. Markers
  are read with `lstat`, so a symlinked `owner.pid`, `package.tgz` or `package`
  never counts. `rm` does not follow links inside the directory.
- `powermove-runtime-<provider>-*`: reclaimed once the process named in
  `owner.pid` is gone (`process.kill(pid, 0)` fails with `ESRCH`; `EPERM`
  counts as alive). A live owner in another Powermove process sharing
  `TMPDIR` (the packaged app next to a dev build or an e2e launch) is never
  touched, however long its download stalls. If `owner.pid` is missing or
  unusable (killed between `mkdtemp` and the write, not a positive integer,
  pid 0 or 1, larger than 32 bytes, or a symlink), the directory is reclaimed
  only after an hour without activity. A recycled pid can only keep a dead
  directory, which is then reclaimed on a later launch.
- `powermove-<provider>-*` (builds before this change): editor runs share the
  prefix. These are the Claude run directory and process cwd (`claude/runner.ts`),
  Codex inputs (`codex/runner.ts`) and App Server directories
  (`codex/app-server-runner.ts`). So a directory is reclaimed only if it holds
  exactly the old layout: a regular `package.tgz` plus, optionally, a real
  `package/` directory, and nothing else. It also needs no activity for an hour.
  Activity is the newest mtime of the directory, `package.tgz` and `package/`,
  so an archive still being streamed by an old build keeps its directory.
  Editor-run directories always contain other files (`schema.json`,
  `reference-*.png`, `frame-*.png`, or whatever the agent wrote), and no old
  build wrote `owner.pid`, so a directory that has one is kept.
- Errors on one entry (a concurrent cleanup, permissions) are ignored and the
  sweep continues.

Production behaviour is otherwise unchanged. The download, checksum,
extraction, version check, atomic link swap and `finally` cleanup are the same.
The only additions are the `owner.pid` write, the different prefix, and the
install waiting for the startup sweep (one directory listing).

## Test launches no longer download runtimes

`POWERMOVE_DISABLE_RUNTIME_UPDATES=1` makes `refreshRuntime` return
immediately. Only the exact value `1` counts. It disables just the automatic
refresh. The explicit "Update runtime" action (`agentRuntimeUpdate` IPC) keeps
working, and `codex/index.test.ts` tests both behaviours. The switch is not
gated on `app.isPackaged`, matching the other test switches
(`POWERMOVE_BACKGROUND_TEST`, `POWERMOVE_FAKE_CHATGPT_STATUS`). A Finder or
Dock launch does not inherit a shell environment, so production cannot set it
by accident, and setting it only skips a background check.

Every launcher of the real app sets the switch:

- `apps/desktop/e2e/helpers/app.ts` (`launchApp`, all e2e specs that use it).
  A spec that tests runtime updates can opt back in by passing
  `POWERMOVE_DISABLE_RUNTIME_UPDATES: '0'` in `env`.
- The specs that call `_electron.launch` themselves:
  `e2e/onboarding-timeline-mode.spec.ts`, and the onboarding model-download
  test in `e2e/transcription.spec.ts`.
- The corpus parity harness (`engine/corpus/src/app.ts`, which also covers
  `electron-bridge.mts`), `scripts/gate-engine-switch-corpus.mjs`,
  `scripts/frame-bridge-measure.mjs` (app rows), `pm-wasm-bench.mjs`,
  `profile-autosave-main-thread.mjs`, `profile-desktop-memory.mjs`,
  `test-mcp.mjs` and `test-native-import-contract.mjs`.

`apps/desktop/scripts/runtime-update-launchers.test.ts` (vitest) guards this.
It finds every file under `apps/desktop/e2e`, `apps/desktop/scripts` and
`engine/corpus/src` that launches the app's main entry
(`_electron.launch({ args: [..., '.'] })` or `spawn(electron, ['.'])`, with
MCP stdio mode excluded), adds the launchers the pattern cannot see (listed in
`INDIRECT`), and fails if any of them omits the switch.

## Tests

- `src/main/runtime-updates.test.ts`: the sweep rules above (owner
  alive or dead, missing or bad owner, the one-hour boundary, an archive still
  streaming, the legacy layout against editor-run look-alikes, symlinks and
  wrong marker types, real liveness of the current process), plus an install
  that checks the prefix and `owner.pid` during the download, that the startup
  sweep ran first, and that a failed download removes its directory.
- `src/main/codex/index.test.ts`: the switch (unset, `0`, `1`) and that the
  explicit update still works.
- `e2e/runtime-scratch-sweep.spec.ts`: the real app, started with a private
  `TMPDIR` holding four seeded directories. It reclaims the dead-owner and
  legacy downloads, and keeps the live-owner install and the editor-run
  look-alike.
- `scripts/runtime-update-launchers.test.ts`: the launcher audit.

## What remains

- Legacy directories younger than an hour are not reclaimed at startup. They
  are reclaimed by a later launch more than an hour after their last write.
  New-prefix directories with a dead owner are reclaimed on the next launch.
- The sweep runs only at startup, not periodically. A long-running app does
  not reclaim directories that other processes leak while it is running.
- A new launcher outside the three audited roots, or one that starts the app
  in a way the audit pattern does not recognise (like `frame-bridge-measure`'s
  `-r` preload), has to be added to the audit by hand.
- Directories in a different `TMPDIR` than the one the app starts with (a
  harness that overrode `TMPDIR`) are only reclaimed by a launch that uses
  that `TMPDIR`.
