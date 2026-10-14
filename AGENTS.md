# Public repository safety

This repository is public. Never commit credentials, authentication/session
cookies or tokens, private keys, production database URLs, user data dumps,
private deployment reports, screenshots, or local operational notes.
Keep sensitive configuration outside Git; use Worker/GitHub secrets as appropriate.
Public API origins and public client/site keys are not authentication secrets.

Before every commit, inspect the staged file list and diff and run
`gitleaks git --pre-commit --staged --redact --ignore-gitleaks-allow`.
Set up the local hook with `git config core.hooksPath .githooks`.
Do not disable the hook or weaken secret scanning to make a check pass.
Historical `.gitleaksignore` entries are reviewed synthetic fixtures and
code false positives, scoped to exact commit fingerprints—not whole files.
If a real secret is exposed, stop propagating it and revoke/rotate it;
deleting the current file does not remove it from history.

Do not create a release tag, publish an app update, or dispatch the release
workflow without explicit user authorization. The secret-scan workflow is
security validation only and does not build or publish software.

# Windows app

The Windows app is the Svelte renderer in WebView2 over a Rust host. To add
native behaviour, register the channel in `crates/powermove-host` with the
request/response shape from `apps/desktop/src/shared/ipc.ts`; the host logs
`unhandled channel <name>` for any channel the renderer calls that nothing answers.

Put renderer changes made for Windows behind `desktopHost()` / `hello.desktop`
in `host/web-bridge.ts`, so the Mac app and `powermove serve` keep their behaviour.

Debug builds of `powermove-windows` expose the WebView over CDP when
`POWERMOVE_CDP_PORT` is set, for end-to-end checks with Playwright's
`connectOverCDP`. Answer native file dialogs by sending Win32 messages to the
Powermove-owned dialog window, so input never reaches other apps on the desktop.
