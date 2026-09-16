# Saving Powermove projects

- **Command+S** saves the current editable project as a `.pmv` file. The first save asks for a name and location. Later saves update that file, including after restarting Powermove.
- **Shift+Command+S** (File → Save Project As…) chooses a new destination. Future saves use that destination. The original file is left alone.
- **Command+O** opens a project file. Layers, animation, nested compositions, project notes, and embedded imported media remain editable. Opening a copy creates a separate local project so it cannot replace an existing tab accidentally.
- An unsaved dot on a project tab means the current edits are not in its project file. Hover the tab to see its file location. Local recovery does not clear that dot.
- Closing a project or quitting asks whether to Save, Cancel, or Don’t Save when needed. Cancelling either the prompt or the file picker keeps the document open. Don’t Save leaves the project file unchanged; the local recovery copy remains available in Projects.

Each replacement saves the previous complete file beside it with a `1` suffix (`Demo.pmv1`). You can open that backup with Command+O, then use Save As to give the recovered version its own `.pmv` name. Writes use a flushed temporary file followed by an atomic rename. If a backup or write fails, Powermove reports the error and does not report the project saved. If the file has been moved, removed, or changed by another app or open copy, use Save As; Powermove will not silently overwrite those changes.

Project files embed imported media with **no application-imposed total size cap**. Native Save and Open transfer media in 1 MiB chunks backed by temporary disk files, without assembling a whole-project buffer. Available disk space and filesystem limits still apply; a replacement save also needs space for staging and its backup. Document metadata is parsed in memory (the PMV3 format uses a 32-bit metadata-header length). Legacy JSON files remain readable but their embedded base64 media still requires in-memory parsing. Missing sources retain their editable references. Generated effects and panels still need their corresponding extensions installed on another computer.

Local recovery remains in the app’s own storage and is separate from deliberate saves. Playing, seeking, selecting layers, and moving panels do not constitute project-file edits. Unsaved edits are retained locally for recovery, but a `.pmv` file changes only when you explicitly save it.

Native save/open/close handlers are installed at app launch. An already-running development session must be restarted with the user’s approval to activate changes to those handlers. Tests run only in the hidden Electron harness with a fresh temporary profile; native dialog decisions are substituted there while the actual file reads/writes and renderer commands run unchanged.

### Undo and redo persistence

Saved `.pmv` files and local project sessions retain the bounded project-edit
history and its current undo/redo position. Grouped agent edits remain a single
step, and the file includes media referenced only by history (for example, a
deleted image). Opening an older file without history starts a fresh timeline.
Session-only selection and interface callbacks are not included in the file.
