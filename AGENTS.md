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

## UI Geometry: Concentric Corner Radii

Keep corners concentric on closely nested surfaces using
`Outer R = Inner R + Padding`. Use `Radii::nested` or the corresponding
`pm-design` tokens so the inner radius follows
`Inner R = max(Outer R - Padding, min_radius)`.
