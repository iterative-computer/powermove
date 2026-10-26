# Saving Powermove projects

- **Command+S** saves the current editable project as a `.pmv` file. The first save asks for a name and location. Later saves update that file, including after restarting Powermove.
- **Shift+Command+S** (File → Save Project As…) chooses a destination and builds a fresh project file. Future saves use that destination. When a different destination is chosen, the original file is left alone. Imported files remain linked to their originals; pasted/generated media and media needed by Undo/Redo are embedded.
- **Export → Project** produces a portable copy with all available media embedded. Use this when sending the project to another computer or archiving it independently of its source files.
- **Command+O** opens a project file. Layers, animation, nested compositions, project notes, and embedded imported media remain editable. Opening a copy creates a separate local project so it cannot replace an existing tab accidentally.
- An unsaved dot on a project tab means the current edits are not in its project file. Hover the tab to see its file location. Local recovery does not clear that dot.
- Closing a project or quitting asks whether to Save, Cancel, or Don’t Save when needed. Cancelling either the prompt or the file picker keeps the document open. Don’t Save leaves the project file unchanged; the local recovery copy remains available in Projects.

Successful saves do not retain full project backup generations. A full rollback copy exists only during publication and is removed after a successful commit; an uncertain or interrupted save keeps its recovery copy. Writes use a flushed temporary file followed by atomic publication. macOS saves use copy-on-write file clones where supported; other volumes use ordinary copies. Both the file and directory are flushed; macOS requests `F_FULLFSYNC` where supported. Backups from older versions remain readable and retain the existing conservative cleanup rules.

If a backup or write fails, Powermove reports the error and does not report the project saved. Files changed outside Powermove are checked before preparing the save and again before publication. On macOS, atomic exchange retains and verifies the displaced file before releasing it; a conflicting version is restored or preserved at a recovery path reported in the error. A failed private file-association update is repaired from a commit journal at next launch. If the project file was saved but local recovery storage subsequently fails, the app distinguishes those outcomes.

Files imported from disk are read from their original source. Only a small link record is stored in Application Support, and ordinary native saves retain that path instead of embedding the file again. Moving or deleting a source leaves an editable missing-media item; use **Locate File…** to relink it. A missing link does not prevent saving unrelated edits. Existing owned media migrates to a link only after the complete original matches its stored checksum. Browser duplicates are removed only after their full contents match a durable native record.

Pasted, generated and converted media still need a durable owned copy. Portable project export and browser save embed all available media with **no application-imposed total size cap**. Native Save and Open transfer embedded media in bounded chunks without assembling a whole-project buffer. Save writes directly into a temporary file beside the chosen destination, eliminating a separate upload spool. Available disk space and filesystem limits still apply; unsupported clone volumes need space for full staging and transaction rollback copies. Generated effects and panels still need their corresponding extensions installed on another computer.

Desktop Save writes the incremental **PMV4** format. Unchanged media keeps its existing byte ranges; only the new project/history snapshot, changed media, and a checksummed index are written into the cloned file. Media storage revisions change on every replacement, including same-size replacements; sampled import fingerprints are not used to prove that saved bytes are unchanged. Opening verifies the saved document and media checksums. Saving also retains whole-file external-change verification, so it still reads existing bytes even when it does not rewrite them.

Old snapshots and unreferenced media are compacted when their space exceeds both 16 MiB and half the retained media size. Compaction occurs inside the temporary file and retains media needed by Undo and Redo. Document JSON is parsed in memory and is limited to a 32-bit byte length; the separate PMV4 index is limited to 64 MiB and 100,000 media records. Legacy PMV3 and JSON files remain readable and upgrade on their next desktop save. Older Powermove versions cannot read PMV4; the portable project export and browser save fallback continue to produce PMV3 for compatibility.

Local recovery remains in the app’s own storage and is separate from deliberate saves. Large project, session, history, journal and take records use a lossless gzip JSON envelope on disk; IPC and exported documents remain ordinary project JSON. Existing records are compressed when loaded or next written, through the same atomic persistence queue. Playing, seeking, selecting layers, and moving panels do not constitute project-file edits. Unsaved edits are retained locally for recovery, but a `.pmv` file changes only when you explicitly save it.

### First-launch storage optimization

The storage update starts a one-time background migration five seconds after
initial project recovery. `storage-upgrade-v1.json` checkpoints completed stages:
recovery compression, the current origin's older browser media, original-file
verification, then duplicate backup and idle cache cleanup. Packaged and local
development origins are tracked independently because IndexedDB is origin scoped.
Completion is keyed to the migration rather than every app version.

Browser media is deleted only after its complete checksum matches a durable,
readable native copy. Missing native payloads are uploaded before the old record
is retired. Conflicting older bytes are preserved. Existing project and Undo
records supply original paths; matching native copies become links, and only
proven unreferenced duplicates are removed. Source files, distinct earlier backup
versions, uncertain-save recovery copies, artifacts and conversations stay.

Each stage checkpoints only after its writes succeed. An interrupted stage is
safe to repeat; busy AI work or temporary errors pause the task and retry after a
minute, or on next launch. A main-process lease prevents two windows running the
browser migration together and is released on renderer loss or navigation.
The existing task indicator shows progress. A persistent completion notice shows
the approximate reduction in app-data file sizes and links to Storage settings;
its acknowledgment is saved so it does not repeat on later launches.

Native save/open/close handlers are installed at app launch. An already-running development session must be restarted with the user’s approval to activate changes to those handlers. Tests run only in the hidden Electron harness with a fresh temporary profile; native dialog decisions are substituted there while the actual file reads/writes and renderer commands run unchanged.

### Undo and redo persistence

Saved `.pmv` files and local project sessions retain the bounded project-edit
history and its current undo/redo position. Grouped agent edits remain a single
step, and the file includes media referenced only by history (for example, a
deleted image). Opening an older file without history starts a fresh timeline.
Session-only selection and interface callbacks are not included in the file.
