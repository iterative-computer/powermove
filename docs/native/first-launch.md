# Native first launch

Existing users update and are done. `apps/native/crates/first-launch`
(`pm-first-launch`) copies the original app's profile on a worker. `app/src/first_run.rs`
runs that flow behind the Welcome window (`shell/src/welcome_view.rs`). The source
profile and user projects are read-only inputs.

## Paths

`pm_first_launch::default_paths` is the one layout both sides use: source
`Application Support/Powermove`, library `Application Support/com.zellzoi.powermove.native-beta`.
`--profile` or `PM_NATIVE_PROFILE` isolates a run from both; it only migrates when
`PM_NATIVE_ELECTRON_PROFILE` names a source, which is how synthetic profiles run.

## What a launch does

`detect` reads local metadata and picks one path before any window opens:

| Launch | Behaviour |
|---|---|
| `Fresh` | Onboarding (S107–S110) with the first-run animation, then the editor. |
| `Migrate` | The gate: wait for the original app, copy, Keychain consent if encrypted values exist, outcomes. Onboarding follows only if the source never finished it. A clean outcome opens the editor with no click. |
| `Later` | No gate. Changes in the original app merge before the library attaches, then new media lands behind the open library. An unchanged source costs a metadata walk. |

The library, and the MCP server after it, attach only after this. `--powermove-mcp`
refuses while a copy is pending, because a library made first would block the copy.

## Bootstrap

1. Refuse a linked, foreign or overlapping destination. If the source's `SingletonLock`
   has a live local owner, wait and watch it; a remote or unreadable owner needs
   `Check Again`. The lock is never touched.
2. Capture an immutable checkpoint: file families, Chromium snapshots, availability
   inventory, all hashed with BLAKE3. Source changes during capture fail the capture.
3. Import through `LibraryWriter`: settings and `ext.*` exact, workspaces migrated, agent
   owners remapped to stable copy ids, every project with its history, names unchanged.
   Trash stays trash. Refused histories and unreadable slots stay in the checkpoint.
4. Stream media into the host layout, BLAKE3 verified. The closure records verified,
   proxy-only, offline, missing, unclassified and poster outcomes.
5. Import localStorage, register extensions (`extensions-shared` read-only, plus an
   overlay), then secrets.

Every step publishes durably, so a kill or `Pause` resumes where it stopped. Quitting is
always safe. Disk needs 6 GiB plus worst-case bytes; low space stops with real numbers.

Secrets: the Keychain is read only after a click for that request (`secrets::PROMPT`),
never on retry or relaunch. `Not Now` finishes without it. Settings ▸ Accounts ▸ `Bring
Sign-Ins from the Original App…` is the later retry, through the attached kernel.

## Deltas

`delta_store` merges everything but media, library detached. `delta_media` brings in
media and promotes the capture to the next base, library attached. Each is resumable:
`first-launch-deltas/pending` is reused, and every action is idempotent.

Three way against the newest merged checkpoint, by logical key, project, file path and
localStorage key:

| Original app | Native | Result |
|---|---|---|
| unchanged | anything | native stays |
| changed | unchanged | change comes in, deletion included |
| changed | changed | native stays; the original's version stays in its checkpoint; conflict recorded |

Rules: `ext.*` objects merge one member at a time. A project edited on both sides keeps
both, as two library projects. Trash follows only while the copy is untouched. A project
permanently deleted there stays here. Settings go through the kernel, because its durable
record outranks the store at attach. Extension copies are resealed after a merge.
No checkpoint is ever rewritten. A capture that adds nothing to its base is dropped, so a
run of the original app costs one capture at most. Conflicts appear in Window ▸ Migration Report.

## Decisions

| Decision | Rule |
|---|---|
| A clean outcome skips the summary | "Existing users just update and done" (N4). Spec updated. |
| Project names stay exactly as they were | N3; the copy suffix was a beta artefact. Copy ids stay private. |
| Merge settings before attach, media after | The library opens without waiting for media; nothing races the kernel's writer. |
| Unchanged source skips capture by metadata stamp | A launch must not hash gigabytes. |
| Files in `Fresh`-started libraries stay out of migration | A library with no `migration.json` was made here; a copy would overwrite that work. |

## Verification

Synthetic profiles only: `first-launch/tests/{first_launch,delta,beta,main_delta,surfaces}.rs`
and `app/src/first_run_tests.rs`, which drive the real coordinator and worker through
`Flow`. `shell/tests/welcome.rs` drives the window with real input events.

Not proven: the animation on a screen, the AppKit menu and window lifecycle, a signed
Keychain ACL, a real profile end to end, Chromium scope recovery, TR receipts. Restore,
reverse migration and conflict merge UI don't exist, by owner decision.
