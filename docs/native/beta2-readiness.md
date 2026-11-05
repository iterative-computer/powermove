# Beta readiness preparation — cloud beta2

2026-11-04 (America/Chicago). Synthetic Linux evidence only. No signing,
notarization, publishing, workflow dispatch, real profile or Keychain access.
This lane does not declare beta/cutover readiness.

The worker bridge in `pm-first-launch::integration` implements the existing
`pm-distribution::update_handoff::HandoffHost` using the real first-launch
coordinator. Direct DMG, skipped/invalid TR and verified TR all run the importer
and a fresh final delta before reading owner evidence. Electron lock waiting,
secret consent, missing/refused history and owner blocking remain explicit.
`Evidence` is an application capability held under source/import leases; it must
supply fresh generations, signed identities, state/secret closure and current-state
Restore proof. No production success implementation is provided. The isolated
beta can finish importing while cutover remains blocked. Poll scheduling uses the
existing 30-second/four-hour intervals.

`restore::merge` supports forward and reverse *current-state store* inputs with
missing/null distinction, tombstones and member-level `ext.<id>` merge. Atomic
project/history/workspace values retain both sides in conflict records. The
kernel owner must first flush/compact current journals and capture current state
under its leases. `prepare_store` writes a new rollback-readable store through
`LibraryWriter` and archives complete input maps, unknown values and conflicts.
An interrupted `.preparing` attempt is retained, then archived by a retry. This is
store preparation, not a complete profile/media/Keychain Restore implementation.
No source profile is replaced. Never pass decrypted secrets to store maps.

First-launch capture failures, stale capture attempts and duplicate completed
deltas now remain archived. Failed lower-level snapshots and unreadable sampled
partials also remain intact; an incomplete snapshot cannot be admitted as complete.
No source, base, history, quarantine file or previous attempt is pruned.

## Sparkle public-API mismatch and required integration

`first-launch::sparkle::Driver` implements the existing `Installer` trait over an
injected Sparkle capability. It checks exact version/URL/length/EdDSA enclosure
selection and streams a fresh SHA-256/size verification before delegating stage.
The native capability owns authentication, extraction, signatures/DR, platform,
expanded-space checks, tickets and persisted recovery. No macOS success double is
compiled into production. Tests exercise enclosure mismatches, changed bytes,
interrupted extraction, retained archives, pending recovery and explicit install.

The existing `Installer::stage(archive) -> ticket` API is **not implemented by
Sparkle 2.10.1's public API**. `SPUUpdater` owns its own download/extraction cycle;
`SPUUserDriver` receives callbacks and reply continuations. It offers no public
"adopt this externally downloaded ZIP" entrypoint. Calling an imagined selector
or returning a staged ticket before the authenticated callback would be incorrect.
This lane cannot truthfully call its policy bridge a linked production installer.

Required outside owned paths: distribution/services must expose an asynchronous
Sparkle-owned download path (or obtain an upstream supported archive-adoption
API), with policy/enclosure binding and freshly checked migration/save barriers at
all side effects. Shell must retain its custom `SPUUserDriver` on the main run loop,
enqueue Rust worker events and enforce normal quit. In particular, Sparkle may
install after ordinary termination once extraction starts; holding only the
`showReadyToInstallAndRelaunch` reply is insufficient to guard every quit. Recovery
must query installer state and verify the installed signed identity, not infer
success from a cached in-memory ticket. Public header evidence, inspected at the
pinned tag: [SPUUpdater](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUpdater.h),
[SPUUserDriver](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUserDriver.h),
[SPUUpdaterDelegate](https://github.com/sparkle-project/Sparkle/blob/2.10.1/Sparkle/SPUUpdaterDelegate.h).

## Decisions and selecting rules

- Always rerun the final delta after bootstrap/consent and then obtain owner
  evidence; TR receipts never skip data work (N3/N4).
- Readiness remains false without signed identity/full closure/current Restore
  evidence; passing import models is not release proof (N1/N4).
- Archive failed and duplicate attempts, including partial snapshots (N4).
- Preserve missing versus null and concurrent values; merge extension members,
  keep authored arrays/history atomic (N3/N4/N6).
- Use the kernel writer for a new Restore store; require the owning actor's
  compaction barrier and preserve old inputs (single writer/N4/prune rule).
- Implement exact policy validation over a macOS capability, and report the
  unsupported Sparkle adoption API explicitly (N3/N4/N8). No invented selector,
  permissive installer or synchronous main-thread wait.

`DESIGN-TODO(opus)`: N001–N007 first-launch/update/consent/conflict/Restore
presentation, text, actions, progress and accessibility await specs. Only stable
existing model states/events are supplied. Existing secret-prompt copy remains
provisional; no new copy, spacing, colour, layout or interaction is introduced.
