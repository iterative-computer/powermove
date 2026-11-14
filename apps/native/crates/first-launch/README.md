# Extension transfer at startup

For people whose native profile predates first-launch migration, offer to bring
extensions from the original Powermove app. This also works without migration.json.
Nothing transfers until the person chooses to continue. This crate provides the
mechanics; the app owns the startup and Settings screens.

## API

Use `pm_first_launch::extension_transfer` and the existing `pm_first_launch::Config`
(`electron_profile`, `native_profile`). Both paths are explicit. The source is read
only; do not launch Electron or connect to it.

```rust,ignore
let inventory = extension_transfer::scan(&config)?;
if inventory.should_offer() {
    // Show the offer. Inventory entries explain what is new, changed or skipped.
}
// Not Now:
extension_transfer::set_declined(&config.native_profile, true)?;
// Settings > Storage / Extensions can reset the choice, then show the offer:
extension_transfer::set_declined(&config.native_profile, false)?;
// On the worker, after the person chooses Bring Extensions:
let result = extension_transfer::run(
    &config,
    |p| send_progress(p),
    |dir, manifest| crate::extension_authoring::check(dir, manifest)
        .map(|_| "Loads.".into()),
)?;
```

* `scan(&Config) -> anyhow::Result<Scan>` is read-only, sync and does not execute
  extensions. Run it on a worker: it hashes extension files, so large libraries or
  network drives can take time. `Scan` has `found`, `new`, `changed`,
  `already_present`, `entries`, `source_issue`, `declined`.
* `ScanEntry { id, status: ScanStatus, reason }`; statuses: `New`, `Changed`,
  `AlreadyPresent`, `Skipped`. Hidden staging folders are omitted. Missing or
  unreadable source folders return `source_issue` and no offer. An individual
  unreadable/empty folder is skipped with a reason. `has_new_or_changed()` ignores
  Not Now; `should_offer()` also respects it.
* `declined(&Path) -> Result<bool>`, `set_declined(&Path, bool) -> Result<()>`.
  Not Now stays declined across launches, including when the source changes.
* `run(&Config, impl FnMut(Progress), check) -> Result<TransferResult>` performs an
  explicit action even if declined. `Progress { done, total, current }` uses folder
  IDs as names. Total is twice the entry count: copy then validate, including skips.
  All copies finish before validation starts, so dependencies are available. Callback runs
  on the import worker. `check: FnMut(&Path, &serde_json::Value) -> Result<String,
  String>` must validate in a disposable synthetic realm, never an open project.
* `TransferResult` contains `brought_in`, `already_present`, `could_not_load` and
  `entries: Vec<ExtensionResult>`. Each has `id`, `outcome` and a short `reason`.
  Outcomes: `BroughtIn`, `AlreadyPresent`, `CouldNotLoad`, `Skipped`. A validation
  error stays visible with its first-line reason. Copied files remain installed;
  another explicit run validates them again. Copy errors also produce an
  individual `CouldNotLoad` result; top-level source/locking errors return Err.

Call before starting extension hosts, or pause the destination hosts during the
worker and rescan afterwards. `first-launch.lock` excludes other migration workers,
not the app's extension hosts. The app should not run first-launch and this worker
at the same time. A repeated run copies only new/changed trees, preserves local
edits as a unit and revalidates identical installed extensions. No source deletion
propagates. Conflicts stay installed and get a skipped reason.

## Storage and scope

All engine writes are confined to `extensions/` (including hidden previous versions),
`extensions-provenance.json`, `extension-import/` (preferences, receipts, captured
source, last result) and `first-launch.lock`. There is no library/kernel attach,
project/settings writer or credential access. `existing_profile_import_and_write_scope`
tests project/history/settings/workspace/runtime credential sentinels unchanged.
The `check` callback is supplied by the app and must honor that same scope; use the
existing isolated `extension_authoring::check`, not a live-project MCP call.
  That single-extension check can reject a valid dependency on another extension;
  for dependency-aware results use a synthetic host with the complete installed
  library and return that extension’s health record.

Receipts live at `extension-import/state.json`; this never creates migration.json
or marks first launch complete. Existing migration receipts can seed the first
explicit transfer. The underlying first-launch extension importer still performs
staged publication, whole-tree conflict checks and provenance merging. Sources
and destination ancestors/trees refuse symlinks. Shared extension conflicts are
preserved; no shared or overlay directory is rewritten.

Maintenance CLI (close the destination app first):

```sh
pm-import-extensions --source '/path/to/original/profile' --profile '/path/to/native/profile'
```

The CLI copies only and explicitly reports that activation is unchecked. For real
load results use the app callback above. Tests: `cargo test -p pm-first-launch`.
Compatibility evidence for a person’s installed library belongs outside this
public repository; do not commit profile snapshots, private paths or reports.
