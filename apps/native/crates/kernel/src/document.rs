//! The document authority: [`DocumentStore`].
//!
//! One store per open project. It owns the canonical `pm_model` document and
//! is the only place it changes. Every change, whether a command, an undo,
//! a redo or a gesture step, follows the same path:
//!
//! 1. **Stage** on a scratch copy of the tree (copy-on-write: only the path
//!    to each change is copied). Each operation's inverse is captured from
//!    the scratch tree right before the operation applies.
//! 2. **Validate** the whole result: every touched composition still has an
//!    id-keyed `layers` array, the open composition exists.
//! 3. **Publish** only if everything succeeded: a new [`Snapshot`] at the
//!    next revision, the [`Change`] (its operations), the query mirror
//!    updated, then subscribers notified. A failure anywhere publishes
//!    nothing; the document, revision, history and dirty state are exactly
//!    as before.
//!
//! Revisions only increase. Undo and redo publish *new* revisions whose
//! operations restore old content (boundary doc, section 6 step 7).

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Weak};

use pm_model::ops::diff_node_ops;
use pm_model::patch::{Patch, PathPart};
use pm_model::{Document, Map, Node, NodeKind, Op, Value};

use crate::commands::{Command, Origin};
use crate::error::{ProjectIoError, TransactionError};
use crate::history::{History, HistoryMark, next_token};
use crate::model::{self, LayerInfo};
use crate::project_io::save::HistoryToWrite;
use crate::project_io::{
    self, MediaTable, OpenedProject, ProjectFile, Publish, SaveOptions, SavePlan, SaveReceipt,
    SaveTarget,
};
use crate::queries::QueryContext;
use crate::selection::Selection;

/// An immutable view of the document at one revision. Cheap to clone and
/// `Send + Sync`: hand it to render, audio and export threads.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// The document session (changes on open; never reused).
    pub doc_id: u64,
    pub revision: u64,
    pub document: Arc<Document>,
}

impl Snapshot {
    pub fn root(&self) -> Option<&Node> {
        self.document.root()
    }

    /// The open composition id (`compId`), `None` for root-level layers.
    pub fn open_comp(&self) -> Option<String> {
        self.root().and_then(model::open_comp_id)
    }

    /// The id the engine renders the open composition by: `compId`, or the
    /// project `id` when the layers sit at the root (as the engine resolves it).
    pub fn render_comp(&self) -> String {
        self.root()
            .and_then(model::render_comp_id)
            .unwrap_or_default()
    }

    /// Layers panel rows of a composition (`None`: the open one).
    pub fn layers(&self, comp_id: Option<&str>) -> Vec<LayerInfo> {
        self.root()
            .map(|r| model::list_layers(r, comp_id))
            .unwrap_or_default()
    }
}

/// Why a revision was published.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeCause {
    /// A committed transaction ([`DocumentStore::execute`], [`DocumentStore::transact`]).
    Edit {
        label: String,
        origin: Origin,
    },
    Undo {
        label: String,
    },
    Redo {
        label: String,
    },
    /// One live step of a gesture (no undo step yet).
    GestureStep {
        gesture: GestureId,
    },
    /// A cancelled gesture restoring its starting values.
    GestureCancel {
        gesture: GestureId,
    },
    /// [`DocumentStore::restore_project`].
    Restore {
        label: String,
        origin: Origin,
    },
    /// A kernel write (`crate::tx`): applied now, recorded in history when
    /// its open entry closes (or never, for direct writes outside one).
    Write {
        label: String,
        origin: Origin,
        /// The open entry the write folded into.
        entry: Option<u64>,
    },
    /// A kernel cancel reverting a lease owner's operations.
    Revert {
        label: String,
    },
    /// Journal recovery replaying a record.
    Recover,
}

/// One published revision: what readers need to follow incrementally.
#[derive(Clone, Debug)]
pub struct Change {
    pub doc_id: u64,
    /// The revision these operations apply on top of.
    pub base_revision: u64,
    pub revision: u64,
    /// The operations, in order.
    pub ops: Arc<[Op]>,
    pub cause: ChangeCause,
}

/// What subscribers hear, synchronously, after the store is fully updated.
#[derive(Clone, Debug)]
pub enum StoreEvent {
    /// A new revision is published (the query mirror already follows it).
    Changed(Arc<Change>),
    SelectionChanged,
    /// Same composition revision, a new immutable notes publication.
    MetadataChanged {
        sequence: u64,
    },
    /// The undo stack changed (labels, can-undo, can-redo).
    HistoryChanged,
    /// The unsaved-changes state flipped.
    DirtyChanged(bool),
    /// A save finished and was adopted ([`DocumentStore::finish_save`]).
    Saved {
        revision: u64,
    },
}

/// Handle returned by [`DocumentStore::subscribe`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Subscription(u64);

/// An open gesture ([`DocumentStore::begin_gesture`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GestureId(u64);

/// Knobs of [`DocumentStore::transact_with`].
#[derive(Clone, Debug, Default)]
pub struct TransactOptions {
    /// The undo-menu label (default: the first command's).
    pub label: Option<String>,
    pub origin: Origin,
    /// Refuse with [`TransactionError::Stale`] unless the document is at
    /// this revision (optimistic concurrency for agents and MCP).
    pub expected_revision: Option<u64>,
}

/// A committed transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    pub revision: u64,
    /// The undo record's id.
    pub record: u64,
}

struct Gesture {
    id: GestureId,
    label: String,
    origin: Origin,
    base: Arc<Document>,
    base_token: u64,
    scopes: Vec<Vec<PathPart>>,
}

/// A transaction staged on a scratch tree, not yet published.
struct Staged {
    /// Immutable 3D import capabilities, admitted on a worker before planning.
    media: Vec<project_io::MediaEntry>,
    document: Document,
    forward: Vec<Op>,
    inverse: Vec<Op>,
    /// Where each inverse was captured (for the leaf-patch file form).
    scopes: Vec<Vec<PathPart>>,
    changed: bool,
}

/// A durable state change, in order, for the kernel journal
/// ([`DocumentStore::enable_tap`]).
#[derive(Clone, Debug)]
pub(crate) enum Tap {
    /// A published revision.
    Rev {
        base: u64,
        revision: u64,
        ops: Arc<[Op]>,
    },
    /// A document record pushed (discarding redo, then trimming).
    Push(crate::history::HistoryRecord),
    /// An external record pushed.
    PushExternal(crate::history::HistoryRecord),
    Undo {
        external: bool,
    },
    Redo {
        external: bool,
    },
    /// `count` records from the top replaced by `record`.
    Squash {
        count: usize,
        record: crate::history::HistoryRecord,
    },
    Clear,
    Metadata {
        ops: Arc<[Op]>,
    },
}

/// Most changes kept for [`DocumentStore::changes_since`].
const CHANGE_LOG: usize = 256;

type Listener = Box<dyn FnMut(&StoreEvent) + Send + 'static>;

/// A file's `history` member, the open composition it was judged with, and
/// whether it needs an archive.
type ArchiveJudgment = (crate::history::CompactJson, Option<String>, bool);

/// The single document authority. See the crate documentation for the
/// threading model.
pub struct DocumentStore {
    runtime_state: Option<crate::state::StateSnapshot>,
    text_host: Option<crate::text_host::SharedTextHost>,
    doc_id: u64,
    current: Arc<Document>,
    file: ProjectFile,
    history: History,
    /// Whether the history now differs from the file's `history` member.
    history_owned: bool,
    /// Whether the file's `history` member needs an archive before a save
    /// replaces it ([`Self::file_history_needs_archive`]), for the member
    /// and open composition it was judged with.
    archive_judged: Arc<std::sync::Mutex<Option<ArchiveJudgment>>>,
    selection: Selection,
    queries: QueryContext,
    gesture: Option<Gesture>,
    state_token: u64,
    saved_token: u64,
    /// Changes history cannot return from: direct writes, Undo that skipped a
    /// stale operation. History position alone proves nothing past one.
    outside_writes: u64,
    saved_outside_writes: u64,
    metadata_token: u64,
    saved_metadata_token: u64,
    save_metadata_token: u64,
    log: VecDeque<Arc<Change>>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
    save_ticket: Weak<()>,
    /// Durable changes since the last [`take_tap`](Self::take_tap), when
    /// enabled (the kernel journal).
    tap: Option<Vec<Tap>>,
    upgraded_on_open: bool,
}

impl std::fmt::Debug for DocumentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentStore")
            .field("doc_id", &self.doc_id)
            .field("revision", &self.revision())
            .field("path", &self.file.path)
            .field("dirty", &self.is_dirty())
            .finish()
    }
}

fn new_doc_id() -> u64 {
    next_token()
}

impl DocumentStore {
    // ── construction ───────────────────────────────────────────────────────

    /// Adopts a project read by [`project_io::open_project`] (cheap: the tree
    /// was built on the opening thread). The store starts clean.
    pub fn open(opened: OpenedProject) -> DocumentStore {
        let OpenedProject {
            file,
            document,
            history,
            ..
        } = opened;
        // A replayable history can still contain unknown metadata. Keep its
        // original representation until an actual history mutation (N4).
        DocumentStore::from_parts(file, document, history.unwrap_or_default(), false)
    }

    /// A never-saved project from a project value (stored or canonical form).
    pub fn new_untitled(project: Value) -> DocumentStore {
        let document = Document::new(Some(model::to_canonical(project)), 1);
        DocumentStore::from_parts(ProjectFile::untitled(), document, History::new(), true)
    }

    fn from_parts(
        file: ProjectFile,
        document: Document,
        history: History,
        history_owned: bool,
    ) -> DocumentStore {
        let upgraded_on_open = crate::commands::scene3d::needs_upgrade(&document, &history);
        let (mut document, history) = if upgraded_on_open {
            crate::commands::scene3d::upgrade_document_history(document, history)
        } else {
            (document, history)
        };
        // Native saves preserve the original effect-generation evidence even
        // though the envelope's own application version changes on Save.
        let version = file
            .envelope
            .extra
            .get("effectDefinitionSavingVersion")
            .or(file.envelope.version.as_ref());
        document.set_saving_app_version(version.and_then(|v| match v {
            Value::String(v) => v.as_str().map(str::to_owned),
            _ => None,
        }));
        DocumentStore::assemble(
            file,
            Arc::new(document),
            history,
            history_owned,
            upgraded_on_open,
        )
    }

    /// A planning copy of a published document (an agent's batch resolved
    /// command by command): it shares the snapshot's tree, so making it costs
    /// nothing however large the project; untitled, without history on disk,
    /// never saved. A published snapshot is already upgraded.
    pub fn scratch(snapshot: &Snapshot) -> DocumentStore {
        DocumentStore::assemble(
            ProjectFile::untitled(),
            snapshot.document.clone(),
            History::new(),
            true,
            false,
        )
    }

    fn assemble(
        mut file: ProjectFile,
        current: Arc<Document>,
        history: History,
        history_owned: bool,
        upgraded_on_open: bool,
    ) -> DocumentStore {
        if let Some(root) = current.root() {
            crate::media_versions::hydrate_embedded(root, &mut file.media);
        }
        let queries = QueryContext::new(&current);
        let token = history.state_token();
        DocumentStore {
            runtime_state: None,
            text_host: None,
            doc_id: new_doc_id(),
            current,
            file,
            history,
            history_owned,
            archive_judged: Default::default(),
            selection: Selection::default(),
            queries,
            gesture: None,
            state_token: token,
            saved_token: token,
            outside_writes: 0,
            saved_outside_writes: 0,
            metadata_token: 0,
            saved_metadata_token: 0,
            save_metadata_token: 0,
            log: VecDeque::new(),
            listeners: Vec::new(),
            next_listener: 1,
            save_ticket: Weak::new(),
            tap: None,
            upgraded_on_open,
        }
    }

    /// Opening upgraded the document or its history (old 3D objects), so it
    /// no longer matches the file or slot it came from.
    pub fn upgraded_on_open(&self) -> bool {
        self.upgraded_on_open
    }

    // ── reading ────────────────────────────────────────────────────────────

    /// Contribution facts for worker-side planning. Never authored or persisted.
    pub fn set_runtime_state(
        &mut self,
        state: crate::state::StateSnapshot,
        text: Option<crate::text_host::SharedTextHost>,
    ) {
        self.runtime_state = Some(state);
        self.text_host = text;
    }

    pub fn doc_id(&self) -> u64 {
        self.doc_id
    }

    /// The current revision (monotonic, starts at 1).
    pub fn revision(&self) -> u64 {
        self.current.revision()
    }

    /// The current immutable snapshot.
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            doc_id: self.doc_id,
            revision: self.revision(),
            document: self.current.clone(),
        }
    }

    /// The current tree (borrowed).
    pub fn root(&self) -> Option<&Node> {
        self.current.root()
    }

    /// The open composition id.
    pub fn open_comp(&self) -> Option<String> {
        self.root().and_then(model::open_comp_id)
    }

    /// Layers panel rows of the open composition, at the current revision.
    pub fn layers(&self) -> Vec<LayerInfo> {
        self.root()
            .map(|r| model::list_layers(r, None))
            .unwrap_or_default()
    }

    /// The synchronous query context, at the current revision.
    pub fn queries(&mut self) -> &mut QueryContext {
        &mut self.queries
    }

    /// Changes after `revision`, oldest first, or `None` when the log no
    /// longer reaches back that far (take a [`Snapshot`] instead).
    pub fn changes_since(&self, revision: u64) -> Option<Vec<Arc<Change>>> {
        if revision == self.revision() {
            return Some(Vec::new());
        }
        let first = self.log.front()?;
        if revision + 1 < first.revision || revision > self.revision() {
            return None;
        }
        Some(
            self.log
                .iter()
                .filter(|c| c.revision > revision)
                .cloned()
                .collect(),
        )
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    pub fn can_undo(&self) -> bool {
        self.gesture.is_none() && self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.gesture.is_none() && self.history.can_redo()
    }

    pub fn file(&self) -> &ProjectFile {
        &self.file
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.file.path.as_deref()
    }

    /// The embedded media catalog; imports add to it.
    pub fn media(&self) -> &MediaTable {
        &self.file.media
    }

    pub fn media_mut(&mut self) -> &mut MediaTable {
        &mut self.file.media
    }

    /// Replaces the saved workspace snapshot (`ws`). Not an edit: it does not
    /// make the project dirty (as in the old app).
    pub fn set_workspace(&mut self, workspace: Option<Value>) {
        self.file.envelope.workspace = workspace;
    }

    /// Unsaved changes: the document is not in the state last saved (or
    /// opened). Undoing back to that state makes it clean again.
    ///
    /// Difference: the old app compared content fingerprints, so retyping a
    /// value back by hand also became clean; here only reaching the same
    /// history state does (see `docs/gpui/core-api.md`), and only with no
    /// write outside history since that state was saved.
    pub fn is_dirty(&self) -> bool {
        self.state_token != self.saved_token || self.outside_writes != self.saved_outside_writes
    }

    /// Anything a save would write that its destination lacks: unsaved
    /// edits, or workspace and metadata changes since the last save.
    pub fn needs_save(&self) -> bool {
        self.is_dirty() || self.metadata_token != self.saved_metadata_token
    }

    /// Whether a save of this document is running.
    pub fn is_saving(&self) -> bool {
        self.save_ticket.strong_count() > 0
    }

    // ── subscriptions ──────────────────────────────────────────────────────

    /// Calls `listener` synchronously after every event, on the thread that
    /// caused it. A listener must not call back into the store (it cannot:
    /// the store is mutably borrowed); GPUI wrappers just `cx.notify()`.
    pub fn subscribe(
        &mut self,
        listener: impl FnMut(&StoreEvent) + Send + 'static,
    ) -> Subscription {
        let id = self.next_listener;
        self.next_listener += 1;
        self.listeners.push((id, Box::new(listener)));
        Subscription(id)
    }

    pub fn unsubscribe(&mut self, subscription: Subscription) {
        self.listeners.retain(|(id, _)| *id != subscription.0);
    }

    fn emit(&mut self, event: StoreEvent) {
        for (_, listener) in self.listeners.iter_mut() {
            listener(&event);
        }
    }

    // ── selection ──────────────────────────────────────────────────────────

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// `PM.selectLayers(ids, add)`. Ids not in the open composition are
    /// ignored.
    pub fn select_layers<I, S>(&mut self, ids: I, add: bool)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let live = self
            .root()
            .map(|r| model::layer_ids(r, None))
            .unwrap_or_default();
        let before = self.selection.clone();
        self.selection.select_layers(
            ids.into_iter()
                .map(Into::into)
                .filter(|id| live.contains(id)),
            add,
        );
        if self.selection != before {
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    /// Adds or removes one layer (Shift/Cmd-click).
    pub fn toggle_layer(&mut self, id: &str) {
        let live = self
            .root()
            .is_some_and(|r| model::layer_ids(r, None).contains(id));
        if live || self.selection.contains(id) {
            self.selection.toggle_layer(id);
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    /// Selects keyframes by id.
    pub fn select_keys<I, S>(&mut self, ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selection.set_keys(ids.into_iter().map(Into::into));
        self.emit(StoreEvent::SelectionChanged);
    }

    /// Selects keyframes as picked on their layers, `(layer, key)`.
    pub fn select_layer_keys(&mut self, pairs: Vec<(String, String)>) {
        self.selection.set_layer_keys(pairs);
        self.emit(StoreEvent::SelectionChanged);
    }

    /// The focused channel (`PM.sel.chan`).
    pub fn set_selected_channel(&mut self, channel: Option<String>) {
        if self.selection.channel() != channel.as_deref() {
            self.selection.set_channel(channel);
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    pub fn clear_selection(&mut self) {
        if !self.selection.is_empty() {
            self.selection.clear();
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    fn prune_selection(&mut self) -> bool {
        let Some(root) = self.current.root() else {
            let had = !self.selection.is_empty();
            self.selection.clear();
            return had;
        };
        let layers = model::layer_ids(root, None);
        let keys = (!self.selection.keys().is_empty()).then(|| {
            let mut keys = HashSet::new();
            if let Some(comp) = model::comp_node(root, model::open_comp_id(root).as_deref()) {
                model::keyframe_ids(comp, &mut keys);
            }
            keys
        });
        self.selection.prune(&layers, keys.as_ref())
    }

    // ── transactions ───────────────────────────────────────────────────────

    /// Runs one command as one undo step. `Ok(None)`: nothing to change.
    pub fn execute(
        &mut self,
        command: Command,
        origin: Origin,
    ) -> Result<Option<Committed>, TransactionError> {
        self.transact_with(
            std::slice::from_ref(&command),
            TransactOptions {
                label: None,
                origin,
                expected_revision: None,
            },
        )
    }

    /// Runs several commands atomically as one undo step labelled `label`.
    /// Each command sees the effect of the ones before it.
    pub fn transact(
        &mut self,
        label: &str,
        origin: Origin,
        commands: &[Command],
    ) -> Result<Option<Committed>, TransactionError> {
        self.transact_with(
            commands,
            TransactOptions {
                label: Some(label.to_owned()),
                origin,
                expected_revision: None,
            },
        )
    }

    pub fn transact_with(
        &mut self,
        commands: &[Command],
        options: TransactOptions,
    ) -> Result<Option<Committed>, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        if let Some(expected) = options.expected_revision
            && expected != self.revision()
        {
            return Err(TransactionError::Stale {
                expected,
                current: self.revision(),
            });
        }
        let staged = self.stage_commands(commands, options.origin)?;
        let label = options
            .label
            .or_else(|| commands.first().map(Command::label))
            .unwrap_or_else(|| "Edit".to_owned());
        Ok(self.commit_staged(
            staged,
            label.clone(),
            options.origin,
            ChangeCause::Edit {
                label,
                origin: options.origin,
            },
        ))
    }

    /// Replaces the whole project with `project` (stored or canonical form)
    /// as one undo step (`PM.hist.restoreSnapshot`): only what differs is
    /// written, as id-keyed operations.
    pub fn restore_project(
        &mut self,
        project: Value,
        label: &str,
        origin: Origin,
    ) -> Result<Option<Committed>, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        let mut canonical = model::to_canonical(project);
        if let Some(current) = self.root() {
            crate::memory_metadata::preserve_value(&mut canonical, current);
        }
        project_io::validate_stored_project(&pm_model::canonical::realize_project(
            canonical.clone(),
        ))
        .map_err(|e| TransactionError::Invalid(e.to_string()))?;
        let after = Node::from_value(canonical, self.revision() + 1);
        let ops = diff_node_ops(self.current.root(), Some(&after));
        let staged = self.stage_ops(&ops, true)?;
        Ok(self.commit_staged(
            staged,
            label.to_owned(),
            origin,
            ChangeCause::Restore {
                label: label.to_owned(),
                origin,
            },
        ))
    }

    fn commit_staged(
        &mut self,
        staged: Staged,
        label: String,
        origin: Origin,
        cause: ChangeCause,
    ) -> Option<Committed> {
        if !staged.changed {
            for item in staged.media {
                self.file.media.insert(item);
            }
            return None;
        }
        let structural = staged
            .forward
            .iter()
            .chain(staged.inverse.iter())
            .any(|op| !matches!(op, Op::Leaf(_)));
        let saved = structural
            .then(|| {
                saved_form(
                    self.current.root(),
                    staged.document.root(),
                    staged.scopes.clone(),
                )
            })
            .flatten();
        let revision = staged.document.revision();
        let forward: Arc<[Op]> = staged.forward.clone().into();
        let record = self.history.push(
            label,
            origin,
            staged.forward,
            staged.inverse,
            saved,
            revision,
        );
        self.history_owned = true;
        self.tap_push();
        for item in staged.media {
            self.file.media.insert(item);
        }
        self.publish(staged.document, forward, cause);
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Some(Committed { revision, record })
    }

    // ── undo / redo ────────────────────────────────────────────────────────

    /// `PM.hist.undo()`: applies the newest record's inverse as a new
    /// revision. Returns that revision.
    pub fn undo(&mut self) -> Result<u64, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        let record = self
            .history
            .peek_undo()
            .ok_or(TransactionError::NothingToUndo)?;
        if record.selection.is_none() && !record.is_document() {
            return Err(TransactionError::ExternalEntry);
        }
        self.kernel_undo()?;
        Ok(self.revision())
    }

    /// `PM.hist.redo()`.
    pub fn redo(&mut self) -> Result<u64, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        let record = self
            .history
            .peek_redo()
            .ok_or(TransactionError::NothingToRedo)?;
        if record.selection.is_none() && !record.is_document() {
            return Err(TransactionError::ExternalEntry);
        }
        self.kernel_redo()?;
        Ok(self.revision())
    }

    /// A position to squash back to ([`DocumentStore::squash_since`]).
    pub fn history_mark(&self) -> HistoryMark {
        self.history.mark()
    }

    /// Combines every step since `mark` into one (`PM.hist.squash`: a
    /// bounded agent run becomes a single undo step). `None` if the mark is
    /// gone or steps were undone since.
    pub fn squash_since(&mut self, mark: &HistoryMark, label: &str) -> Option<u64> {
        if self.gesture.is_some() {
            return None;
        }
        let before_len = self.history.len();
        let id = self.history.squash_since(mark, label)?;
        self.history_owned = true;
        let count = before_len + 1 - self.history.len();
        self.tap(|h| Tap::Squash {
            count,
            record: h.records().last().expect("pushed").clone(),
        });
        // The state is unchanged; only its name moved to the new record.
        let dirty_before = self.is_dirty();
        if self.saved_token == self.state_token {
            self.saved_token = id;
        }
        self.state_token = id;
        debug_assert_eq!(dirty_before, self.is_dirty());
        self.emit(StoreEvent::HistoryChanged);
        Some(id)
    }

    // ── gestures ───────────────────────────────────────────────────────────

    /// Starts a gesture (a scrub-drag, a slider drag). Steps publish live
    /// revisions; [`commit_gesture`](Self::commit_gesture) makes the whole
    /// gesture one undo step, [`cancel_gesture`](Self::cancel_gesture)
    /// restores the starting values. While it is open, other edits, undo and
    /// redo are refused.
    pub fn begin_gesture(
        &mut self,
        label: &str,
        origin: Origin,
    ) -> Result<GestureId, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        let id = GestureId(next_token());
        self.gesture = Some(Gesture {
            id,
            label: label.to_owned(),
            origin,
            base: self.current.clone(),
            base_token: self.state_token,
            scopes: Vec::new(),
        });
        Ok(id)
    }

    fn gesture_mut(&mut self, id: GestureId) -> Result<&mut Gesture, TransactionError> {
        match &mut self.gesture {
            Some(g) if g.id == id => Ok(g),
            _ => Err(TransactionError::NoSuchGesture),
        }
    }

    pub fn gesture_active(&self) -> bool {
        self.gesture.is_some()
    }

    /// One step: stages `commands` on the current state and publishes it.
    /// Returns the new revision, or `None` when nothing changed.
    pub fn update_gesture(
        &mut self,
        id: GestureId,
        commands: &[Command],
    ) -> Result<Option<u64>, TransactionError> {
        let origin = self.gesture_mut(id)?.origin;
        let staged = self.stage_commands(commands, origin)?;
        if !staged.changed {
            for item in staged.media {
                self.file.media.insert(item);
            }
            return Ok(None);
        }
        self.gesture_mut(id)?.scopes.extend(staged.scopes);
        let revision = staged.document.revision();
        self.publish(
            staged.document,
            staged.forward.into(),
            ChangeCause::GestureStep { gesture: id },
        );
        self.set_state_token(next_token());
        Ok(Some(revision))
    }

    /// One step computed from the gesture's *starting* state: `commands`
    /// are staged on the document as it was at [`begin_gesture`](Self::begin_gesture)
    /// and the result replaces the previous step. Use it for relative
    /// commands in drags (`KeyframeCommand::Move { delta }`,
    /// `TimingCommand::Move { delta }` with the total pointer delta): a key
    /// removed by a temporary collision comes back when the pointer moves on,
    /// as the old timeline's `applyKeyframeGesture` did. Returns the new
    /// revision, or `None` when the document already looks like that.
    pub fn update_gesture_from_start(
        &mut self,
        id: GestureId,
        commands: &[Command],
    ) -> Result<Option<u64>, TransactionError> {
        let (origin, base) = {
            let g = self.gesture_mut(id)?;
            (g.origin, g.base.clone())
        };
        let staged = self.stage_commands_on(base.root(), commands, origin)?;
        let ops = diff_node_ops(self.current.root(), staged.document.root());
        if ops.is_empty() {
            self.gesture_mut(id)?.scopes.extend(staged.scopes);
            return Ok(None);
        }
        let step = self.stage_ops(&ops, false)?;
        self.gesture_mut(id)?.scopes.extend(staged.scopes);
        let revision = step.document.revision();
        self.publish(
            step.document,
            ops.into(),
            ChangeCause::GestureStep { gesture: id },
        );
        self.set_state_token(next_token());
        Ok(Some(revision))
    }

    /// Ends the gesture as one undo step from its starting values to its
    /// final ones. `None` when it changed nothing overall.
    pub fn commit_gesture(&mut self, id: GestureId) -> Result<Option<u64>, TransactionError> {
        self.gesture_mut(id)?;
        let gesture = self.gesture.take().expect("checked");
        let (forward, inverse) =
            match saved_form(gesture.base.root(), self.current.root(), gesture.scopes) {
                Some((forward, backward)) => (
                    forward.into_iter().map(Op::Leaf).collect::<Vec<_>>(),
                    backward.into_iter().map(Op::Leaf).collect::<Vec<_>>(),
                ),
                None => (
                    diff_node_ops(gesture.base.root(), self.current.root()),
                    diff_node_ops(self.current.root(), gesture.base.root()),
                ),
            };
        if forward == inverse || forward.is_empty() {
            self.set_state_token(gesture.base_token);
            return Ok(None);
        }
        let record = self.history.push(
            gesture.label,
            gesture.origin,
            forward,
            inverse,
            None,
            self.revision(),
        );
        self.history_owned = true;
        self.tap_push();
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Ok(Some(record))
    }

    /// Ends the gesture, restoring every value it touched (a new revision).
    /// On `Err` (not expected: the patches come from this very tree) the
    /// gesture stays open so the caller can commit it instead.
    pub fn cancel_gesture(&mut self, id: GestureId) -> Result<(), TransactionError> {
        let gesture = match &self.gesture {
            Some(g) if g.id == id => g,
            _ => return Err(TransactionError::NoSuchGesture),
        };
        let ops = diff_node_ops(self.current.root(), gesture.base.root());
        let base_token = gesture.base_token;
        if !ops.is_empty() {
            let staged = self.stage_ops(&ops, false)?;
            self.publish(
                staged.document,
                ops.into(),
                ChangeCause::GestureCancel { gesture: id },
            );
        }
        self.gesture = None;
        self.set_state_token(base_token);
        Ok(())
    }

    // ── staging ────────────────────────────────────────────────────────────

    fn stage_commands(
        &self,
        commands: &[Command],
        origin: Origin,
    ) -> Result<Staged, TransactionError> {
        let staged = self.stage_commands_on(self.current.root(), commands, origin)?;
        #[cfg(test)]
        assert_inverse_restores(self.current.root(), staged.document.root(), &staged.inverse);
        Ok(staged)
    }

    /// [`stage_commands`](Self::stage_commands) on another tree (a
    /// gesture's start).
    fn stage_commands_on(
        &self,
        start: Option<&Node>,
        commands: &[Command],
        origin: Origin,
    ) -> Result<Staged, TransactionError> {
        let revision = self.revision() + 1;
        let mut root = start.cloned();
        let mut forward = Vec::new();
        let mut inverse = Vec::new();
        let mut scopes = Vec::new();
        let mut changed = false;
        let mut presets = crate::commands::properties::Presets::default();
        // Flush before any other command can read or override the animation,
        // and once more at the end, matching runCommands/finishPresets.
        for command in commands
            .iter()
            .map(Some)
            .chain((!commands.is_empty()).then_some(None))
        {
            let tree = root.as_ref().ok_or(TransactionError::NoDocument)?;
            let property = match command {
                Some(Command::Property(crate::PropertyCommand::Set(c))) => Some(c),
                _ => None,
            };
            let mut ops = if property.is_none() {
                presets.finish(tree, origin)?
            } else {
                Vec::new()
            };
            // A flush can change the scratch tree. Apply it before planning
            // the following command, using the same inverse/history path.
            for op in ops.drain(..) {
                let (inv, scope) = capture_inverse(root.as_ref(), &op);
                let (next, did) = apply_one(root, &op, revision)?;
                root = next;
                changed |= did;
                inverse.push(inv);
                scopes.push(scope);
                forward.push(op);
            }
            let tree = root.as_ref().ok_or(TransactionError::NoDocument)?;
            let ops = match (command, property) {
                (_, Some(c)) => {
                    crate::commands::properties::plan_set(tree, origin, c, &mut presets)?
                }
                (Some(c), None) => c.plan_with_state(
                    tree,
                    origin,
                    self.runtime_state.as_ref(),
                    self.text_host.as_ref(),
                )?,
                (None, None) => Vec::new(),
            };
            for op in ops {
                let (inv, scope) = capture_inverse(root.as_ref(), &op);
                let (next, did) = apply_one(root, &op, revision)?;
                root = next;
                changed |= did;
                inverse.push(inv);
                scopes.push(scope);
                forward.push(op);
            }
        }
        for op in crate::asset_order::reconcile(start, root.as_ref(), &forward) {
            let (inv, scope) = capture_inverse(root.as_ref(), &op);
            let (next, did) = apply_one(root, &op, revision)?;
            root = next;
            changed |= did;
            inverse.push(inv);
            scopes.push(scope);
            forward.push(op);
        }
        inverse.reverse();
        let document = Document::from_root(root, revision);
        validate(&document, &forward)?;
        let media = commands
            .iter()
            .flat_map(crate::commands::scene3d::import_media)
            .collect::<Vec<_>>();
        for item in &media {
            if self
                .file
                .media
                .get(item.id.as_str().unwrap_or(""))
                .is_some_and(|old| old.revision != item.revision || old.length() != item.length())
            {
                return Err(TransactionError::Invalid(
                    "Retained media already owns this asset ID".into(),
                ));
            }
        }
        Ok(Staged {
            media,
            document,
            forward,
            inverse,
            scopes,
            changed,
        })
    }

    /// Undo and Redo: applies what still matches. A write outside history can
    /// remove what an id-keyed operation names; that operation is skipped, as
    /// a leaf patch without its parent is, and the entry stays usable. The
    /// result's `forward` is what applied, the only thing that may be
    /// published: journal replay refuses an operation that does not apply.
    /// `inverse` is filled only when something was skipped.
    fn stage_history_ops(&self, ops: &[Op]) -> Result<Staged, TransactionError> {
        match self.stage_ops(ops, false) {
            Err(TransactionError::Patch(_)) => {}
            staged => return staged,
        }
        let revision = self.revision() + 1;
        let mut root = self.current.root().cloned();
        let mut applied = Vec::with_capacity(ops.len());
        let mut inverse = Vec::with_capacity(ops.len());
        let mut changed = false;
        for op in ops {
            let (undo, _) = capture_inverse(root.as_ref(), op);
            if let Ok((next, did)) = apply_one(root.clone(), op, revision) {
                root = next;
                changed |= did;
                applied.push(op.clone());
                inverse.push(undo);
            }
        }
        inverse.reverse();
        let document = Document::from_root(root, revision);
        validate(&document, &applied)?;
        Ok(Staged {
            media: vec![],
            document,
            forward: applied,
            inverse,
            scopes: vec![],
            changed,
        })
    }

    /// One Undo or Redo of a document entry: publishes what applied and
    /// returns it with the operations reversing it (`reverse` when all did).
    fn history_step(
        &mut self,
        ops: &Arc<[Op]>,
        reverse: &Arc<[Op]>,
        cause: ChangeCause,
    ) -> Result<HistoryStep, TransactionError> {
        let staged = self.stage_history_ops(ops)?;
        let skipped = staged.forward.len() != ops.len();
        let applied: Arc<[Op]> = staged.forward.into();
        self.publish(staged.document, applied.clone(), cause);
        if !skipped {
            return Ok((applied, reverse.clone()));
        }
        // This position no longer names the document it recorded.
        self.note_outside_write();
        Ok((applied, staged.inverse.into()))
    }

    /// Stages raw operations (undo, redo, cancel, restore). `capture`
    /// records inverses (only `restore_project` needs them).
    fn stage_ops(&self, ops: &[Op], capture: bool) -> Result<Staged, TransactionError> {
        self.stage_ops_from(self.current.root().cloned(), ops, capture)
    }

    /// [`stage_ops`](Self::stage_ops) on another tree.
    fn stage_ops_from(
        &self,
        root: Option<Node>,
        ops: &[Op],
        capture: bool,
    ) -> Result<Staged, TransactionError> {
        let admitted = if capture
            && ops.iter().any(|op| {
                op.path().is_empty()
                    || op.path().first().is_some_and(|part| {
                        crate::memory_metadata::ALL_FIELDS
                            .contains(&part.property_key().as_str().unwrap_or(""))
                    })
            }) {
            let mut admitted = ops.to_vec();
            for op in &mut admitted {
                if op.path().first().is_some_and(|part| {
                    crate::memory_metadata::ALL_FIELDS
                        .contains(&part.property_key().as_str().unwrap_or(""))
                }) {
                    return Err(TransactionError::Invalid(
                        "Project notes require metadata admission".into(),
                    ));
                }
                if let Op::Leaf(patch) = op
                    && patch.path.is_empty()
                    && let (Some(value), Some(current)) = (&mut patch.value, self.root())
                {
                    crate::memory_metadata::preserve_value(value, current);
                }
            }
            Some(admitted)
        } else {
            None
        };
        let mut forward = admitted.unwrap_or_else(|| ops.to_vec());
        let revision = self.revision() + 1;
        let start = root.clone();
        let mut root = root;
        let mut inverse = Vec::new();
        let mut scopes = Vec::new();
        let mut changed = false;
        if capture {
            for op in &forward {
                let (inv, scope) = capture_inverse(root.as_ref(), op);
                let (next, did) = apply_one(root, op, revision)?;
                root = next;
                changed |= did;
                inverse.push(inv);
                scopes.push(scope);
            }
            let order = crate::asset_order::reconcile(start.as_ref(), root.as_ref(), &forward);
            for op in order {
                let (inv, scope) = capture_inverse(root.as_ref(), &op);
                let (next, did) = apply_one(root, &op, revision)?;
                root = next;
                changed |= did;
                inverse.push(inv);
                scopes.push(scope);
                forward.push(op);
            }
            inverse.reverse();
            // An entry holds what the edit changed, not the values it replaced:
            // a member written whole for one new layer costs that layer.
            if forward.iter().any(writes_large_value) {
                forward = diff_node_ops(start.as_ref(), root.as_ref());
                inverse = diff_node_ops(root.as_ref(), start.as_ref());
                scopes = forward
                    .iter()
                    .chain(&inverse)
                    .map(|op| op.path().to_vec())
                    .collect();
            }
        } else {
            let mut scratch = Document::from_root(root, revision - 1);
            changed = scratch
                .apply_ops_at(&forward, revision)
                .map_err(|e| TransactionError::Patch(e.to_string()))?
                .changed;
            root = scratch.root().cloned();
        }
        let document = Document::from_root(root, revision);
        validate(&document, &forward)?;
        Ok(Staged {
            media: vec![],
            document,
            forward,
            inverse,
            scopes,
            changed,
        })
    }

    fn publish(&mut self, mut document: Document, mut ops: Arc<[Op]>, cause: ChangeCause) {
        if matches!(
            cause,
            ChangeCause::Undo { .. }
                | ChangeCause::Redo { .. }
                | ChangeCause::Revert { .. }
                | ChangeCause::GestureCancel { .. }
                | ChangeCause::GestureStep { .. }
        ) && let Some(current) = self.root()
            && let Some(preserved) = crate::memory_metadata::preserve(&document, current)
                .expect("valid metadata patches")
        {
            document = preserved;
            ops = diff_node_ops(self.root(), document.root()).into();
        }
        let base_revision = self.revision();
        document.set_saving_app_version(self.current.saving_app_version().map(str::to_owned));
        let document = Arc::new(document);
        let revision = document.revision();
        debug_assert!(revision > base_revision);
        // The mirror follows before anyone hears about the revision.
        self.queries.apply(&ops, base_revision, revision, &document);
        self.current = document;
        if let Some(root) = self.current.root() {
            crate::media_versions::retarget(root, &mut self.file.media);
        }
        let change = Arc::new(Change {
            doc_id: self.doc_id,
            base_revision,
            revision,
            ops,
            cause,
        });
        if let Some(tap) = &mut self.tap {
            tap.push(Tap::Rev {
                base: base_revision,
                revision,
                ops: change.ops.clone(),
            });
        }
        self.log.push_back(change.clone());
        while self.log.len() > CHANGE_LOG {
            self.log.pop_front();
        }
        let pruned = self.prune_selection();
        self.emit(StoreEvent::Changed(change));
        if pruned {
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    fn set_state_token(&mut self, token: u64) {
        let was = self.is_dirty();
        self.state_token = token;
        let now = self.is_dirty();
        if was != now {
            self.emit(StoreEvent::DirtyChanged(now));
        }
    }

    fn note_outside_write(&mut self) {
        let was = self.is_dirty();
        self.outside_writes += 1;
        if !was {
            self.emit(StoreEvent::DirtyChanged(true));
        }
    }

    // ── saving ─────────────────────────────────────────────────────────────

    /// Freezes the current revision for saving. Cheap (no serialization);
    /// run the plan with [`SavePlan::run`] on a worker, then pass the
    /// result's receipt to [`finish_save`](Self::finish_save). Edits may
    /// continue meanwhile; they stay dirty.
    pub fn prepare_save(
        &mut self,
        target: SaveTarget,
        options: SaveOptions,
    ) -> Result<SavePlan, ProjectIoError> {
        if self.is_saving() {
            return Err(ProjectIoError::SaveInProgress);
        }
        let plan = self.freeze_save(target, options)?;
        self.save_ticket = Arc::downgrade(&plan.ticket);
        self.save_metadata_token = self.metadata_token;
        Ok(plan)
    }

    /// A frozen portable snapshot independent of the live Save ticket/path.
    /// Run it on a worker and never adopt its receipt with finish_save.
    pub fn prepare_portable_export(
        &self,
        path: std::path::PathBuf,
    ) -> Result<SavePlan, ProjectIoError> {
        self.freeze_save(
            SaveTarget::SaveAs {
                path,
                replace: false,
            },
            SaveOptions {
                portable: true,
                ..Default::default()
            },
        )
    }

    fn freeze_save(
        &self,
        target: SaveTarget,
        options: SaveOptions,
    ) -> Result<SavePlan, ProjectIoError> {
        let (destination, publish, replace_existing, reuse_previous) = match target {
            SaveTarget::InPlace => {
                let path = self
                    .file
                    .path
                    .clone()
                    .ok_or(ProjectIoError::NoDestination)?;
                let version = self
                    .file
                    .version
                    .clone()
                    .ok_or(ProjectIoError::NoDestination)?;
                (
                    path,
                    Some(Publish::Replace((*version).clone())),
                    false,
                    true,
                )
            }
            SaveTarget::SaveAs { path, replace } => (path, None, replace, false),
        };
        // Retain owned media even when no known consumer currently reaches it.
        // Future/extension records and refused histories may still own it (N4).
        let media = self
            .file
            .media
            .iter()
            .filter(|m| {
                // A file holds an asset's bytes as its own: proxy-only bytes
                // there would reopen as the original. They stay in the library.
                if self
                    .file
                    .media
                    .is_proxy_only(m.id.as_str().unwrap_or_default())
                {
                    return false;
                }
                if self.root().is_some_and(|root| {
                    crate::media_versions::redundant_active_original(root, &self.file.media, m)
                }) {
                    return false;
                }
                // Omit linked bytes only when the saved document retains a
                // reference to that source. An incomplete admission must never
                // silently lose its sole remaining payload.
                let reference = self
                    .root()
                    .and_then(|r| r.get("assets"))
                    .and_then(|a| m.id.as_str().and_then(|id| a.get(id)))
                    .and_then(|a| a.get("sourcePath"))
                    .and_then(model::str_of);
                let retained = match (&m.source, reference) {
                    (project_io::MediaSource::File { path, .. }, Some(reference)) => {
                        path == std::path::Path::new(reference)
                    }
                    _ => false,
                };
                options.portable || !self.file.media.is_linked(&m.id) || !retained
            })
            .cloned()
            .collect();
        let live_media = self.reachable_assets();
        let source_links = self
            .file
            .media
            .iter()
            .filter(|m| self.file.media.is_linked(&m.id))
            .map(|m| m.id.clone())
            .collect();
        let file_history = self.file.envelope.history.as_ref();
        let history = match (file_history, self.history_owned) {
            (Some(raw), false) => HistoryToWrite::Raw(raw.value()),
            _ => HistoryToWrite::Session(self.history.snapshot()),
        };
        let mut extra = self.file.envelope.extra.clone();
        if pm_fx_defs::catalog::saving_generation(self.current.saving_app_version())
            == pm_fx_defs::catalog::ReferenceGeneration::Main
        {
            extra.insert(
                "effectDefinitionSavingVersion",
                self.current
                    .saving_app_version()
                    .map_or(Value::Null, Value::from),
            );
        }
        if self.history_owned
            && let Some(stored) = file_history
            && self.file_history_needs_archive(stored)
        {
            let raw = &stored.value();
            // Preserve the complete input after editing a history whose newer
            // metadata this kernel cannot represent. Never overwrite a future
            // archive with the same key: retain both as values.
            let key = "nativeHistoryArchive";
            let previous = extra.remove(key);
            let archived = match previous {
                Some(v) if v == *raw => v,
                Some(Value::Array(mut items)) => {
                    if !items.contains(raw) {
                        items.push(raw.clone());
                    }
                    Value::Array(items)
                }
                Some(v) => Value::Array(vec![v, raw.clone()]),
                None => raw.clone(),
            };
            extra.insert(key, archived);
        }
        let ticket = Arc::new(());
        Ok(SavePlan {
            container_index: self.file.envelope.container_index.clone(),
            revision: self.revision(),
            doc_id: self.doc_id,
            state_token: self.state_token,
            outside_writes: self.outside_writes,
            document: self.current.clone(),
            had_comps: self.file.envelope.had_comps,
            history,
            workspace: self.file.envelope.workspace.clone(),
            extra,
            media,
            live_media,
            source_links,
            destination,
            publish,
            replace_existing,
            reuse_previous,
            options,
            ticket,
        })
    }

    /// Adopts a finished save: the file's new identity and media ranges,
    /// and *exactly* the saved state as clean (edits made while it ran stay
    /// dirty).
    pub fn finish_save(&mut self, receipt: SaveReceipt) -> Result<(), ProjectIoError> {
        if receipt.portable {
            return Err(ProjectIoError::Invalid(
                "A portable export is not a document save.".into(),
            ));
        }
        if receipt.doc_id != self.doc_id {
            return Err(ProjectIoError::WrongDocument);
        }
        self.file.path = Some(receipt.path.clone());
        self.file.format = pm_model::container::ContainerFormat::Pmv4;
        self.file.version = Some(receipt.version.clone());
        self.file.envelope.wrapped = true;
        // Keep the raw history until it is changed, including across repeated
        // saves. Changed stacks archive their input in prepare_save.
        for entry in receipt.media.iter() {
            let current = self.file.media.get(entry.id.as_str().unwrap_or_default());
            if current.is_some_and(|c| c.revision == entry.revision)
                && !self.file.media.is_linked(&entry.id)
            {
                self.file.media.insert(entry.clone());
            }
        }
        if let Some(root) = self.root().cloned() {
            crate::media_versions::hydrate_embedded(&root, &mut self.file.media);
        }
        let was = self.is_dirty();
        self.saved_token = receipt.state_token;
        self.saved_outside_writes = receipt.outside_writes;
        self.saved_metadata_token = self.save_metadata_token;
        let revision = receipt.revision;
        drop(receipt);
        self.emit(StoreEvent::Saved { revision });
        let now = self.is_dirty();
        if was != now {
            self.emit(StoreEvent::DirtyChanged(now));
        }
        Ok(())
    }

    /// The file this store saves to was renamed or moved while open and is
    /// now at `version.path` ([`project_io::FileVersion::moved`]): saves and
    /// embedded media follow it there. Nothing about its content changes.
    pub fn file_moved(&mut self, version: project_io::FileVersion) {
        let Some(old) = self.file.version.clone() else {
            return;
        };
        let new = Arc::new(version);
        let moved: Vec<_> = self
            .file
            .media
            .iter()
            .filter_map(|entry| match &entry.source {
                project_io::MediaSource::Embedded {
                    file,
                    offset,
                    length,
                    sha256,
                } if Arc::ptr_eq(file, &old) => Some(project_io::MediaEntry {
                    source: project_io::MediaSource::Embedded {
                        file: new.clone(),
                        offset: *offset,
                        length: *length,
                        sha256: sha256.clone(),
                    },
                    ..entry.clone()
                }),
                _ => None,
            })
            .collect();
        for entry in moved {
            self.file.media.insert(entry);
        }
        self.file.path = Some(new.path.clone());
        self.file.version = Some(new);
    }

    /// What this project's first save would work out about its undo history
    /// on the kernel thread, as a job for another thread to run after the
    /// open: what each step reaches and its digest, and whether the file's
    /// history member needs an archive. Each answer is the one the save
    /// would compute, kept where the save looks for it; a save that comes
    /// first, or a job never run, changes nothing but who computes it.
    pub fn save_warmup(&self) -> impl FnOnce() + Send + 'static {
        let records = self.history.records().to_vec();
        let judged = self.archive_judged.clone();
        let stored = self.file.envelope.history.clone();
        let open = self.root().and_then(model::open_comp_id);
        move || {
            for record in &records {
                record.warm();
            }
            if let Some(stored) = stored {
                // Judged once per member and open composition: a save that
                // runs this again (every save does, on its worker) finds
                // the answer and inflates nothing.
                let known = |judged: &Option<ArchiveJudgment>| {
                    judged
                        .as_ref()
                        .is_some_and(|(member, comp, _)| *member == stored && *comp == open)
                };
                if known(&judged.lock().unwrap()) {
                    return;
                }
                let needs = project_io::history_needs_archive(&stored.value(), open.as_deref());
                let mut judged = judged.lock().unwrap();
                if !known(&judged) {
                    *judged = Some((stored, open, needs));
                }
            }
        }
    }

    /// [`project_io::history_needs_archive`] for the file's `history` member.
    /// The member is what the file held when it was opened and never changes
    /// while it is open, so it is judged once per open composition: judging
    /// inflates and imports the whole of it, which every save of a project
    /// with a long history paid again.
    fn file_history_needs_archive(&self, stored: &crate::history::CompactJson) -> bool {
        let open = model::open_comp_id(self.root().expect("project root"));
        let mut judged = self.archive_judged.lock().unwrap();
        if let Some((member, comp, needs)) = &*judged
            && member == stored
            && *comp == open
        {
            return *needs;
        }
        let needs = project_io::history_needs_archive(&stored.value(), open.as_deref());
        *judged = Some((stored.clone(), open, needs));
        needs
    }

    /// Asset ids the project or its undo history can still reach.
    pub fn reachable_assets(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        for record in self.history.records() {
            ids.extend(record.referenced_assets().iter().cloned());
        }
        if let Some(NodeKind::Object(assets)) =
            self.root().and_then(|r| r.get("assets")).map(Node::kind)
        {
            ids.extend(assets.keys().filter_map(|k| k.as_str().map(str::to_owned)));
        }
        if let Some(root) = self.root() {
            crate::commands::scene3d::collect_original_assets(&root.to_value(), &mut ids);
        }
        ids
    }
}

/// What one kernel write applied ([`DocumentStore::kernel_apply`]).
#[derive(Clone, Debug)]
pub(crate) struct Applied {
    pub forward: Vec<Op>,
    /// Inverses in undo order (last operation first).
    pub inverse: Vec<Op>,
    /// Where each operation's inverse was captured (forward order).
    pub scopes: Vec<Vec<PathPart>>,
}

/// The operations a history step applied, and those reversing them.
type HistoryStep = (Arc<[Op]>, Arc<[Op]>);

/// What a kernel undo/redo step did ([`DocumentStore::kernel_undo`]).
#[derive(Clone, Debug)]
pub(crate) enum Step {
    Selection,
    /// A document entry: its operations were applied as `revision`.
    Document {
        ops: Arc<[Op]>,
        /// The operations that reverse this step (the other side of the record).
        reverse: Arc<[Op]>,
        label: String,
    },
    /// An external entry: the cursor moved; its realm must run the closure.
    External {
        external: crate::history::ExternalRef,
        label: String,
    },
}

/// What a kernel write carries.
#[derive(Clone, Debug)]
pub enum WriteBody {
    /// Typed commands, planned on the scratch tree in order.
    Commands(Vec<Command>),
    /// Raw operations (direct writes, `edit.mutate` results, journal replay).
    Ops(Vec<Op>),
}

impl WriteBody {
    pub fn is_empty(&self) -> bool {
        match self {
            WriteBody::Commands(c) => c.is_empty(),
            WriteBody::Ops(o) => o.is_empty(),
        }
    }
}

impl From<Command> for WriteBody {
    fn from(c: Command) -> Self {
        WriteBody::Commands(vec![c])
    }
}

impl From<Vec<Command>> for WriteBody {
    fn from(c: Vec<Command>) -> Self {
        WriteBody::Commands(c)
    }
}

/// Kernel primitives: apply now, record later. The transaction layer
/// (`crate::tx`) composes these into today's open-entry semantics; the
/// stand-alone [`DocumentStore::execute`]/gesture API above stays for
/// single-owner callers and tests.
impl DocumentStore {
    /// Recovery/save work independent of the composition Edited indicator.
    pub fn has_pending_metadata(&self) -> bool {
        self.metadata_token != self.saved_metadata_token
    }

    pub fn metadata_revision(&self) -> u64 {
        self.metadata_token
    }

    pub(crate) fn publish_memory(&mut self, next: Value) -> Result<(), TransactionError> {
        self.publish_metadata(next, crate::memory_metadata::FIELDS)
    }
    fn publish_metadata(&mut self, next: Value, fields: &[&str]) -> Result<(), TransactionError> {
        let root = self.root().ok_or(TransactionError::NoDocument)?;
        let ops = crate::memory_metadata::patches_for(root, &next, fields);
        if ops.is_empty() {
            return Ok(());
        }
        let document = crate::memory_metadata::apply(&self.current, &ops)
            .map_err(TransactionError::Invalid)?;
        self.current = Arc::new(document);
        self.queries = QueryContext::new(&self.current);
        self.metadata_token = next_token();
        if let Some(tap) = &mut self.tap {
            tap.push(Tap::Metadata { ops: ops.into() });
        }
        self.emit(StoreEvent::MetadataChanged {
            sequence: self.metadata_token,
        });
        Ok(())
    }

    /// Bytes the library's media store holds for assets the file lacks.
    pub(crate) fn hydrate_library_media(&mut self, store: &std::path::Path) {
        if let Some(root) = self.root().cloned() {
            crate::library_media::hydrate(&root, &self.history, &mut self.file.media, store);
        }
    }
    pub(crate) fn hydrate_originals(&mut self, archive: &crate::media_archive::Archive) {
        if let Some(root) = self.root().cloned() {
            crate::media_versions::hydrate_archive(&root, &mut self.file.media, archive);
        }
    }

    pub(crate) fn admit_media(
        &mut self,
        admission: crate::MediaAdmission,
        origin: Origin,
    ) -> Result<Option<Committed>, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        if self.revision() != admission.expected_revision {
            return Err(TransactionError::Stale {
                expected: admission.expected_revision,
                current: self.revision(),
            });
        }
        let mut canonical = model::to_canonical(admission.project);
        let root = self.root().ok_or(TransactionError::NoDocument)?;
        crate::memory_metadata::preserve_value(&mut canonical, root);
        project_io::validate_stored_project(&pm_model::canonical::realize_project(
            canonical.clone(),
        ))
        .map_err(|e| TransactionError::Invalid(e.to_string()))?;
        let after = Node::from_value(canonical, self.revision() + 1);
        let plan = crate::media_versions::plan(root, &after, &self.file.media, &admission.bindings)
            .map_err(TransactionError::Invalid)?;
        let ops = diff_node_ops(self.current.root(), Some(&after));
        let mut staged = self.stage_ops(&ops, true)?;
        if !admission.place.is_empty() {
            let placed =
                self.stage_commands_on(staged.document.root(), &admission.place, origin)?;
            staged.forward.extend(placed.forward);
            staged.inverse.splice(0..0, placed.inverse);
            staged.scopes.extend(placed.scopes);
            staged.changed |= placed.changed;
            staged.document = placed.document;
        }
        if !staged.changed {
            return Ok(None);
        }
        // Validate everything before publishing ownership. The metadata tap is
        // first: a torn journal can leave unused originals, never an asset
        // admitted without the immutable ownership record for its bytes.
        let mut fields = Map::new();
        fields.insert(crate::media_versions::FIELD, plan.manifest);
        self.publish_metadata(Value::Object(fields), &[crate::media_versions::FIELD])?;
        if let Some(document) =
            crate::memory_metadata::preserve(&staged.document, self.root().unwrap())
                .map_err(TransactionError::Invalid)?
        {
            staged.document = document;
        }
        staged.forward = diff_node_ops(self.current.root(), staged.document.root());
        staged.media.extend(plan.entries);
        let label = admission.label;
        Ok(self.commit_staged(
            staged,
            label.clone(),
            origin,
            ChangeCause::Edit { label, origin },
        ))
    }

    /// Starts recording durable changes for the journal.
    pub(crate) fn enable_tap(&mut self) {
        self.tap.get_or_insert_with(Vec::new);
    }

    /// The durable changes since the last call.
    pub(crate) fn take_tap(&mut self) -> Vec<Tap> {
        self.tap.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn tap(&mut self, event: impl FnOnce(&History) -> Tap) {
        if let Some(tap) = &mut self.tap {
            tap.push(event(&self.history));
        }
    }

    fn tap_push(&mut self) {
        self.tap(|h| Tap::Push(h.records().last().expect("pushed").clone()));
    }

    /// Reverts a cursor move (an undo barrier).
    pub(crate) fn kernel_unstep(&mut self, undone: bool) {
        if undone {
            self.history.mark_redone();
            self.history_owned = true;
            self.tap(|_| Tap::Redo { external: true });
        } else {
            self.history.mark_undone();
            self.history_owned = true;
            self.tap(|_| Tap::Undo { external: true });
        }
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
    }

    /// `PM.hist.clear()`.
    pub(crate) fn kernel_clear(&mut self) {
        self.history.clear();
        self.history_owned = true;
        self.tap(|_| Tap::Clear);
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
    }

    /// External entries with a cleanup that retired.
    pub(crate) fn take_retired(&mut self) -> Vec<crate::history::ExternalRef> {
        self.history.take_retired()
    }

    /// A write no history entry covers.
    pub(crate) fn kernel_outside_write(&mut self) {
        self.note_outside_write();
    }

    pub(crate) fn kernel_state_token(&self) -> u64 {
        self.state_token
    }
    pub(crate) fn kernel_restore_state_token(&mut self, token: u64) {
        self.set_state_token(token);
    }

    pub(crate) fn current(&self) -> &Arc<Document> {
        &self.current
    }

    /// Stages, validates and publishes `body` without recording history.
    /// `Ok(None)`: nothing changed (nothing published).
    pub(crate) fn kernel_apply(
        &mut self,
        body: &WriteBody,
        origin: Origin,
        cause: ChangeCause,
    ) -> Result<Option<Applied>, TransactionError> {
        let staged = match body {
            WriteBody::Commands(commands) => self.stage_commands(commands, origin)?,
            WriteBody::Ops(ops) => self.stage_ops(ops, true)?,
        };
        if !staged.changed {
            for item in staged.media {
                self.file.media.insert(item);
            }
            return Ok(None);
        }
        let applied = Applied {
            forward: staged.forward.clone(),
            inverse: staged.inverse,
            scopes: staged.scopes,
        };
        for item in staged.media {
            self.file.media.insert(item);
        }
        self.publish(staged.document, staged.forward.into(), cause);
        self.set_state_token(next_token());
        Ok(Some(applied))
    }

    /// A gesture step that supersedes the previous one: `prev_inverse`
    /// (that step's inverses, already validated by the caller) and `body`
    /// are staged together and published as one revision. The returned
    /// forward/inverse describe `body` alone, relative to the reverted state.
    pub(crate) fn kernel_replace(
        &mut self,
        prev_inverse: &[Op],
        body: &WriteBody,
        origin: Origin,
        cause: ChangeCause,
    ) -> Result<Option<Applied>, TransactionError> {
        let revision = self.revision() + 1;
        let mut root = self.current.root().cloned();
        for op in prev_inverse {
            root = apply_one(root, op, revision)?.0;
        }
        let mut staged = match body {
            WriteBody::Commands(commands) => {
                self.stage_commands_on(root.as_ref(), commands, origin)?
            }
            WriteBody::Ops(ops) => self.stage_ops_from(root, ops, true)?,
        };
        let mut ops = prev_inverse.to_vec();
        ops.extend(staged.forward.iter().cloned());
        if let Some(current) = self.root()
            && let Some(preserved) = crate::memory_metadata::preserve(&staged.document, current)
                .map_err(TransactionError::Invalid)?
        {
            staged.document = preserved;
            ops = diff_node_ops(self.root(), staged.document.root());
        }
        validate(&staged.document, &ops)?;
        if ops.is_empty() {
            return Ok(None);
        }
        let applied = Applied {
            forward: staged.forward,
            inverse: staged.inverse,
            scopes: staged.scopes,
        };
        for item in staged.media {
            self.file.media.insert(item);
        }
        self.publish(staged.document, ops.into(), cause);
        self.set_state_token(next_token());
        Ok(Some(applied))
    }

    /// Publishes an already staged document (a validated cancel).
    pub(crate) fn kernel_publish(&mut self, document: Document, ops: Vec<Op>, cause: ChangeCause) {
        self.publish(document, ops.into(), cause);
        self.set_state_token(next_token());
    }

    /// Pushes a document entry covering writes that are already applied.
    /// `before` is the document right before the first of them (for the
    /// file's leaf-patch form when any operation is structural).
    pub(crate) fn kernel_record(
        &mut self,
        label: String,
        origin: Origin,
        forward: Vec<Op>,
        inverse: Vec<Op>,
        before: &Document,
        scopes: Vec<Vec<PathPart>>,
    ) -> u64 {
        let structural = forward
            .iter()
            .chain(inverse.iter())
            .any(|op| !matches!(op, Op::Leaf(_)));
        let saved = structural
            .then(|| saved_form(before.root(), self.current.root(), scopes))
            .flatten();
        let record = self
            .history
            .push(label, origin, forward, inverse, saved, self.revision());
        self.history_owned = true;
        self.tap_push();
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        record
    }

    pub(crate) fn kernel_selection(&mut self, before: Selection, after: Selection) -> Option<u64> {
        let id = self.history.push_selection(before, after);
        if id.is_some() {
            self.emit(StoreEvent::HistoryChanged);
        }
        id
    }

    pub(crate) fn trim_history_one(&mut self) -> bool {
        let trimmed = self.history.trim_one_shared();
        if trimmed {
            self.emit(StoreEvent::HistoryChanged);
        }
        trimmed
    }

    /// `history.external`: pushes a closure entry.
    pub(crate) fn kernel_push_external(
        &mut self,
        label: String,
        origin: Origin,
        external: crate::history::ExternalRef,
        bytes: usize,
    ) -> u64 {
        let id = self
            .history
            .push_external(label, origin, external, bytes, self.revision());
        self.tap(|h| Tap::PushExternal(h.records().last().expect("pushed").clone()));
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        id
    }

    /// Undo of the newest applied entry, document or external. An external
    /// entry moves the cursor first (as `idx--` did before `entry.undo()`),
    /// then its realm runs the closure.
    pub(crate) fn kernel_undo(&mut self) -> Result<Step, TransactionError> {
        let record = self
            .history
            .peek_undo()
            .ok_or(TransactionError::NothingToUndo)?
            .clone();
        if let Some(selection) = &record.selection {
            self.set_selection(selection.0.clone());
            self.history.mark_undone();
            self.history_owned = true;
            self.emit(StoreEvent::HistoryChanged);
            return Ok(Step::Selection);
        }
        if let Some(external) = record.external {
            self.history.mark_undone();
            self.history_owned = true;
            self.tap(|_| Tap::Undo { external: true });
            self.set_state_token(self.history.state_token());
            self.emit(StoreEvent::HistoryChanged);
            return Ok(Step::External {
                external,
                label: record.label,
            });
        }
        let (ops, reverse) = self.history_step(
            &record.inverse.to_vec().into(),
            &record.forward.to_vec().into(),
            ChangeCause::Undo {
                label: record.label.clone(),
            },
        )?;
        self.history.mark_undone();
        self.history_owned = true;
        self.tap(|_| Tap::Undo { external: false });
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Ok(Step::Document {
            ops,
            reverse,
            label: record.label,
        })
    }

    /// Redo, as [`kernel_undo`](Self::kernel_undo).
    pub(crate) fn kernel_redo(&mut self) -> Result<Step, TransactionError> {
        let record = self
            .history
            .peek_redo()
            .ok_or(TransactionError::NothingToRedo)?
            .clone();
        if let Some(selection) = &record.selection {
            self.set_selection(selection.1.clone());
            self.history.mark_redone();
            self.history_owned = true;
            self.emit(StoreEvent::HistoryChanged);
            return Ok(Step::Selection);
        }
        if let Some(external) = record.external {
            self.history.mark_redone();
            self.history_owned = true;
            self.tap(|_| Tap::Redo { external: true });
            self.set_state_token(self.history.state_token());
            self.emit(StoreEvent::HistoryChanged);
            return Ok(Step::External {
                external,
                label: record.label,
            });
        }
        let (ops, reverse) = self.history_step(
            &record.forward.to_vec().into(),
            &record.inverse.to_vec().into(),
            ChangeCause::Redo {
                label: record.label.clone(),
            },
        )?;
        self.history.mark_redone();
        self.history_owned = true;
        self.tap(|_| Tap::Redo { external: false });
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Ok(Step::Document {
            ops,
            reverse,
            label: record.label,
        })
    }

    /// Replaces the selection (a cancelled edit restores the one it began
    /// with).
    pub(crate) fn set_selection(&mut self, selection: Selection) {
        if self.selection != selection {
            self.selection = selection;
            self.prune_selection();
            self.emit(StoreEvent::SelectionChanged);
        }
    }

    /// A store over a library project (clean: it matches its slot).
    pub(crate) fn adopt(document: Document, history: History) -> DocumentStore {
        DocumentStore::from_parts(ProjectFile::untitled(), document, history, true)
    }

    pub(crate) fn inherit_file(&mut self, file: ProjectFile) {
        self.file = file;
    }
    pub(crate) fn mark_library_saved(&mut self) {
        self.saved_token = self.state_token;
        self.saved_outside_writes = self.outside_writes;
        self.saved_metadata_token = self.metadata_token;
        self.emit(StoreEvent::Saved {
            revision: self.revision(),
        });
        self.emit(StoreEvent::DirtyChanged(false));
    }
    pub(crate) fn mark_recovered(&mut self) {
        self.saved_token = next_token();
    }

    /// Every change still in the log, oldest first (published to mirrors).
    pub(crate) fn recent_changes(&self) -> Vec<Arc<Change>> {
        self.log.iter().cloned().collect()
    }

    /// A store adopting recovered state (journal recovery): dirty, since
    /// the recovered work is not in any saved file.
    pub(crate) fn recovered(document: Document, history: History) -> DocumentStore {
        let mut store = DocumentStore::from_parts(ProjectFile::untitled(), document, history, true);
        store.saved_token = next_token();
        store
    }
}

// ── staging helpers ─────────────────────────────────────────────────────────

/// Applies one operation to a tree as `revision`.
pub(crate) fn apply_one(
    root: Option<Node>,
    op: &Op,
    revision: u64,
) -> Result<(Option<Node>, bool), TransactionError> {
    let mut step = Document::from_root(root, revision - 1);
    let outcome = step
        .apply_ops_at(std::slice::from_ref(op), revision)
        .map_err(|e| TransactionError::Patch(e.to_string()))?;
    Ok((step.root().cloned(), outcome.changed))
}

fn value_at(root: Option<&Node>, path: &[PathPart]) -> Patch {
    match root.and_then(|r| r.lookup(path)) {
        Some(node) => Patch::set(path.to_vec(), node.to_value()),
        None => Patch::delete(path.to_vec()),
    }
}

/// The operation that undoes `op` on `root` (the tree right before it), and
/// the path its leaf-patch form must cover.
fn capture_inverse(root: Option<&Node>, op: &Op) -> (Op, Vec<PathPart>) {
    match op {
        Op::Leaf(patch) => {
            let path = &patch.path;
            let scope = match path.split_last() {
                None => Vec::new(),
                Some((_, parent_path)) => {
                    let parent = root.and_then(|r| r.lookup(parent_path));
                    match parent.map(Node::kind) {
                        // Objects: the member itself.
                        Some(NodeKind::Object(_)) | None => path.clone(),
                        // Arrays: an element only when it exists and is being
                        // replaced; deletes splice and writes past the end
                        // resize, so the whole array is restored instead.
                        Some(NodeKind::Array(_)) => {
                            let element = root.and_then(|r| r.lookup(path));
                            if patch.exists && element.is_some() {
                                path.clone()
                            } else {
                                parent_path.to_vec()
                            }
                        }
                        // Strings and other primitives: restore the parent.
                        Some(_) => parent_path.to_vec(),
                    }
                }
            };
            (Op::Leaf(value_at(root, &scope)), scope)
        }
        Op::Order { path, .. } => {
            let ids = root
                .and_then(|r| r.lookup(path))
                .map(|array| match array.kind() {
                    NodeKind::Array(items) => items
                        .iter()
                        .filter_map(|i| i.get("id").and_then(model::string_of).cloned())
                        .collect(),
                    _ => Vec::new(),
                })
                .unwrap_or_default();
            (
                Op::Order {
                    path: path.clone(),
                    ids,
                },
                path.clone(),
            )
        }
        Op::Insert { path, value, .. } => {
            let id = value
                .get("id")
                .and_then(Value::as_js_string)
                .cloned()
                .unwrap_or_default();
            (
                Op::Delete {
                    path: path.clone(),
                    id,
                },
                path.clone(),
            )
        }
        Op::Delete { path, id } => {
            let found = root
                .and_then(|r| r.lookup(path))
                .and_then(|array| match array.kind() {
                    NodeKind::Array(items) => items
                        .iter()
                        .position(|i| i.get("id").and_then(model::string_of) == Some(id))
                        .map(|index| (index, items[index].to_value())),
                    _ => None,
                });
            match found {
                Some((index, value)) => (
                    Op::Insert {
                        path: path.clone(),
                        index: index as u32,
                        value,
                    },
                    path.clone(),
                ),
                // The delete will fail and abort the transaction anyway.
                None => (value_at_op(root, path), path.clone()),
            }
        }
    }
}

fn value_at_op(root: Option<&Node>, path: &[PathPart]) -> Op {
    Op::Leaf(value_at(root, path))
}

fn same_part(a: &PathPart, b: &PathPart) -> bool {
    a.property_key() == b.property_key()
}

/// The most an entry's whole-value form may hold. Past it the entry keeps its
/// operations only: on a 180 MB project, adding a layer must cost that layer.
pub(crate) const SAVED_FORM_BYTES: usize = 1 << 20;

/// Whether `node`'s serialized size passes `budget`, which it spends. Stops as
/// soon as it does, so the check is bounded however large the value is.
fn spends(node: &Node, budget: &mut usize) -> bool {
    let cost = match node.kind() {
        NodeKind::Null | NodeKind::Bool(_) | NodeKind::Number(_) => 8,
        NodeKind::String(s) => s.as_bytes().len() + 2,
        NodeKind::Array(items) => {
            *budget = budget.saturating_sub(items.len());
            return *budget == 0 || items.iter().any(|item| spends(item, budget));
        }
        NodeKind::Object(map) => {
            for (key, value) in map.iter() {
                *budget = budget.saturating_sub(key.as_bytes().len() + 3);
                if *budget == 0 || spends(value, budget) {
                    return true;
                }
            }
            return *budget == 0;
        }
    };
    *budget = budget.saturating_sub(cost);
    *budget == 0
}

/// A raw write that replaces a member with a value past the same limit.
fn writes_large_value(op: &Op) -> bool {
    let Op::Leaf(patch) = op else { return false };
    if patch.path.is_empty() {
        return false;
    }
    fn size(value: &Value, budget: &mut usize) -> bool {
        match value {
            Value::Array(items) => {
                *budget = budget.saturating_sub(items.len());
                *budget == 0 || items.iter().any(|item| size(item, budget))
            }
            Value::Object(map) => map.iter().any(|(key, value)| {
                *budget = budget.saturating_sub(key.as_bytes().len() + 3);
                *budget == 0 || size(value, budget)
            }),
            Value::String(s) => {
                *budget = budget.saturating_sub(s.as_bytes().len() + 2);
                *budget == 0
            }
            _ => {
                *budget = budget.saturating_sub(8);
                *budget == 0
            }
        }
    }
    let mut budget = SAVED_FORM_BYTES;
    patch.value.as_ref().is_some_and(|v| size(v, &mut budget))
}

/// [`scoped_patches`] for what a file stores, or `None` when the values are
/// too large to keep whole; the entry then saves as operations.
pub(crate) fn saved_form(
    before: Option<&Node>,
    after: Option<&Node>,
    scopes: Vec<Vec<PathPart>>,
) -> Option<(Vec<Patch>, Vec<Patch>)> {
    let kept = kept_scopes(scopes);
    let mut budget = SAVED_FORM_BYTES;
    for root in [before, after].into_iter().flatten() {
        for path in &kept {
            if root.lookup(path).is_some_and(|n| spends(n, &mut budget)) {
                return None;
            }
        }
    }
    Some(scoped_patches(before, after, kept))
}

/// Whether anything changed under `scopes`, without copying what they cover.
pub(crate) fn scopes_changed(
    before: Option<&Node>,
    after: Option<&Node>,
    scopes: Vec<Vec<PathPart>>,
) -> bool {
    kept_scopes(scopes).iter().any(|path| {
        let at = |root: Option<&Node>| root.and_then(|r| r.lookup(path)).cloned();
        !diff_node_ops(at(before).as_ref(), at(after).as_ref()).is_empty()
    })
}

fn kept_scopes(mut scopes: Vec<Vec<PathPart>>) -> Vec<Vec<PathPart>> {
    scopes.sort_by_key(Vec::len);
    let mut kept: Vec<Vec<PathPart>> = Vec::new();
    for path in scopes {
        let covered = kept
            .iter()
            .any(|k| k.len() <= path.len() && k.iter().zip(&path).all(|(a, b)| same_part(a, b)));
        if !covered {
            kept.push(path);
        }
    }
    kept
}

/// The old app's scoped history entries (`scopedPatches`): for each touched
/// path not already covered by a shorter one, its value before and after.
pub(crate) fn scoped_patches(
    before: Option<&Node>,
    after: Option<&Node>,
    scopes: Vec<Vec<PathPart>>,
) -> (Vec<Patch>, Vec<Patch>) {
    let kept = kept_scopes(scopes);
    let mut forward = Vec::new();
    let mut backward = Vec::new();
    for path in kept {
        let next = value_at(after, &path);
        let previous = value_at(before, &path);
        if next != previous {
            forward.push(next);
            backward.push(previous);
        }
    }
    (forward, backward)
}

/// Whole-transaction validation on the staged result.
pub(crate) fn validate(document: &Document, ops: &[Op]) -> Result<(), TransactionError> {
    let invalid = |m: String| Err(TransactionError::Invalid(m));
    let Some(root) = document.root() else {
        return invalid("the project was removed".into());
    };
    if !matches!(root.kind(), NodeKind::Object(_)) {
        return invalid("the project is not an object".into());
    }
    let open = model::open_comp_id(root);
    if let Some(open) = &open
        && model::comp_node(root, Some(open))
            .is_none_or(|c| !matches!(c.kind(), NodeKind::Object(_)))
    {
        return invalid(format!("the open composition {open} is missing"));
    }
    // Compositions the transaction touched.
    let mut all = false;
    let mut touched: Vec<Option<String>> = Vec::new();
    for op in ops {
        let path = op.path();
        let first = path.first().map(PathPart::property_key);
        match (first.as_ref().and_then(|k| k.as_str()), path.get(1)) {
            (None, _) => all = true,
            (Some("comps"), None) => all = true,
            (Some("comps"), Some(id)) => {
                touched.push(id.property_key().as_str().map(str::to_owned))
            }
            (Some(k), _) if pm_model::canonical::COMP_FIELDS.contains(&k) || k == "compId" => {
                touched.push(None);
                if k == "compId" {
                    all = true;
                }
            }
            _ => {}
        }
    }
    let comps: Vec<Option<String>> = if all {
        let mut list = vec![None];
        if let Some(NodeKind::Object(map)) = root.get("comps").map(Node::kind) {
            list.extend(
                map.keys()
                    .filter_map(|k| k.as_str().map(|s| Some(s.to_owned()))),
            );
        }
        list
    } else {
        touched.sort();
        touched.dedup();
        touched
    };
    for comp_id in comps {
        let Some(comp) = model::comp_node(root, comp_id.as_deref()) else {
            continue;
        };
        if comp_id.is_none() && open.is_some() {
            continue; // the root of a canonical project holds no layers
        }
        let Some(layers) = comp.get("layers") else {
            if comp_id.as_deref() == open.as_deref() {
                return invalid("the open composition has no layers".into());
            }
            continue;
        };
        let NodeKind::Array(items) = layers.kind() else {
            return invalid("layers is not an array".into());
        };
        let mut ids = HashSet::with_capacity(items.len());
        for layer in items.iter() {
            if crate::commands::scene3d::is_scene3d_layer(layer) {
                crate::commands::scene3d::validate_layer(&layer.to_value())
                    .map_err(|e| TransactionError::Invalid(e.to_string()))?;
            }
            let id = layer.get("id").and_then(model::str_of);
            match id {
                Some(id) if !id.is_empty() => {
                    if !ids.insert(id) {
                        return invalid(format!("two layers have the id {id}"));
                    }
                }
                _ => return invalid("a layer has no id".into()),
            }
        }
    }
    Ok(())
}

/// Test builds check every captured inverse against the real previous tree.
#[cfg(test)]
pub(crate) fn assert_inverse_restores(before: Option<&Node>, after: Option<&Node>, inverse: &[Op]) {
    let mut doc = Document::from_root(after.cloned(), 1);
    doc.apply_ops_at(inverse, 2).expect("inverse applies");
    assert_eq!(
        doc.to_value(),
        before.map(Node::to_value),
        "inverse restores the state"
    );
}
