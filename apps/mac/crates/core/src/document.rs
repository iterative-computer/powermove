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
use pm_model::{Document, Node, NodeKind, Op, Value};

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
}

/// One published revision: what readers need to follow incrementally.
#[derive(Clone, Debug)]
pub struct Change {
    pub doc_id: u64,
    /// The revision these operations apply on top of.
    pub base_revision: u64,
    pub revision: u64,
    /// The operations, in order (`pm_engine::document::DocumentUpdate::Ops`).
    pub ops: Arc<[Op]>,
    pub cause: ChangeCause,
}

/// What subscribers hear, synchronously, after the store is fully updated.
#[derive(Clone, Debug)]
pub enum StoreEvent {
    /// A new revision is published (the query mirror already follows it).
    Changed(Arc<Change>),
    SelectionChanged,
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
    document: Document,
    forward: Vec<Op>,
    inverse: Vec<Op>,
    /// Where each inverse was captured (for the leaf-patch file form).
    scopes: Vec<Vec<PathPart>>,
    changed: bool,
}

/// Most changes kept for [`DocumentStore::changes_since`].
const CHANGE_LOG: usize = 256;

type Listener = Box<dyn FnMut(&StoreEvent) + Send + 'static>;

/// The single document authority. See the crate documentation for the
/// threading model.
pub struct DocumentStore {
    doc_id: u64,
    current: Arc<Document>,
    file: ProjectFile,
    history: History,
    /// Whether the history now differs from the file's `history` member.
    history_owned: bool,
    selection: Selection,
    queries: QueryContext,
    gesture: Option<Gesture>,
    state_token: u64,
    saved_token: u64,
    log: VecDeque<Arc<Change>>,
    listeners: Vec<(u64, Listener)>,
    next_listener: u64,
    save_ticket: Weak<()>,
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
        let history_owned = history.is_some();
        DocumentStore::from_parts(file, document, history.unwrap_or_default(), history_owned)
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
        let queries = QueryContext::new(&document);
        let token = history.state_token();
        DocumentStore {
            doc_id: new_doc_id(),
            current: Arc::new(document),
            file,
            history,
            history_owned,
            selection: Selection::default(),
            queries,
            gesture: None,
            state_token: token,
            saved_token: token,
            log: VecDeque::new(),
            listeners: Vec::new(),
            next_listener: 1,
            save_ticket: Weak::new(),
        }
    }

    // ── reading ────────────────────────────────────────────────────────────

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

    /// The embedded media catalog (the media lane adds imports here).
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
    /// history state does (see `docs/gpui/core-api.md`).
    pub fn is_dirty(&self) -> bool {
        self.state_token != self.saved_token
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

    /// Selects keyframes by id (the timeline lane owns key identity).
    pub fn select_keys<I, S>(&mut self, ids: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selection.set_keys(ids.into_iter().map(Into::into));
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
        let canonical = model::to_canonical(project);
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
            return None;
        }
        let structural = staged
            .forward
            .iter()
            .chain(staged.inverse.iter())
            .any(|op| !matches!(op, Op::Leaf(_)));
        let saved = structural.then(|| {
            scoped_patches(
                self.current.root(),
                staged.document.root(),
                staged.scopes.clone(),
            )
        });
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
            .ok_or(TransactionError::NothingToUndo)?
            .clone();
        let staged = self.stage_ops(&record.inverse, false)?;
        let revision = staged.document.revision();
        self.publish(
            staged.document,
            record.inverse.clone(),
            ChangeCause::Undo {
                label: record.label.clone(),
            },
        );
        self.history.mark_undone();
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Ok(revision)
    }

    /// `PM.hist.redo()`.
    pub fn redo(&mut self) -> Result<u64, TransactionError> {
        if self.gesture.is_some() {
            return Err(TransactionError::GestureInProgress);
        }
        let record = self
            .history
            .peek_redo()
            .ok_or(TransactionError::NothingToRedo)?
            .clone();
        let staged = self.stage_ops(&record.forward, false)?;
        let revision = staged.document.revision();
        self.publish(
            staged.document,
            record.forward.clone(),
            ChangeCause::Redo {
                label: record.label.clone(),
            },
        );
        self.history.mark_redone();
        self.set_state_token(self.history.state_token());
        self.emit(StoreEvent::HistoryChanged);
        Ok(revision)
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
        let id = self.history.squash_since(mark, label)?;
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

    /// Ends the gesture as one undo step from its starting values to its
    /// final ones. `None` when it changed nothing overall.
    pub fn commit_gesture(&mut self, id: GestureId) -> Result<Option<u64>, TransactionError> {
        self.gesture_mut(id)?;
        let gesture = self.gesture.take().expect("checked");
        let (forward, backward) =
            scoped_patches(gesture.base.root(), self.current.root(), gesture.scopes);
        if forward == backward {
            self.set_state_token(gesture.base_token);
            return Ok(None);
        }
        let forward: Vec<Op> = forward.into_iter().map(Op::Leaf).collect();
        let inverse: Vec<Op> = backward.into_iter().map(Op::Leaf).collect();
        let record = self.history.push(
            gesture.label,
            gesture.origin,
            forward,
            inverse,
            None,
            self.revision(),
        );
        self.history_owned = true;
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
        let (forward, backward) = scoped_patches(
            gesture.base.root(),
            self.current.root(),
            gesture.scopes.clone(),
        );
        let base_token = gesture.base_token;
        if forward != backward {
            let ops: Vec<Op> = backward.into_iter().map(Op::Leaf).collect();
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
        let revision = self.revision() + 1;
        let mut root = self.current.root().cloned();
        let mut forward = Vec::new();
        let mut inverse = Vec::new();
        let mut scopes = Vec::new();
        let mut changed = false;
        for command in commands {
            let tree = root.as_ref().ok_or(TransactionError::NoDocument)?;
            let ops = command.plan(tree, origin)?;
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
        inverse.reverse();
        let document = Document::from_root(root, revision);
        validate(&document, &forward)?;
        #[cfg(test)]
        assert_inverse_restores(self.current.root(), document.root(), &inverse);
        Ok(Staged {
            document,
            forward,
            inverse,
            scopes,
            changed,
        })
    }

    /// Stages raw operations (undo, redo, cancel, restore). `capture`
    /// records inverses (only `restore_project` needs them).
    fn stage_ops(&self, ops: &[Op], capture: bool) -> Result<Staged, TransactionError> {
        let revision = self.revision() + 1;
        let mut root = self.current.root().cloned();
        let mut inverse = Vec::new();
        let mut scopes = Vec::new();
        let mut changed = false;
        if capture {
            for op in ops {
                let (inv, scope) = capture_inverse(root.as_ref(), op);
                let (next, did) = apply_one(root, op, revision)?;
                root = next;
                changed |= did;
                inverse.push(inv);
                scopes.push(scope);
            }
            inverse.reverse();
        } else {
            let mut scratch = Document::from_root(root, revision - 1);
            changed = scratch
                .apply_ops_at(ops, revision)
                .map_err(|e| TransactionError::Patch(e.to_string()))?
                .changed;
            root = scratch.root().cloned();
        }
        let document = Document::from_root(root, revision);
        validate(&document, ops)?;
        Ok(Staged {
            document,
            forward: ops.to_vec(),
            inverse,
            scopes,
            changed,
        })
    }

    fn publish(&mut self, document: Document, ops: Arc<[Op]>, cause: ChangeCause) {
        let base_revision = self.revision();
        let document = Arc::new(document);
        let revision = document.revision();
        debug_assert!(revision > base_revision);
        // The mirror follows before anyone hears about the revision.
        self.queries.apply(&ops, base_revision, revision, &document);
        self.current = document;
        let change = Arc::new(Change {
            doc_id: self.doc_id,
            base_revision,
            revision,
            ops,
            cause,
        });
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
        // Media the project or its undo history can still reach (`fileAssets`).
        let reachable = self.reachable_assets();
        let media = self
            .file
            .media
            .iter()
            .filter(|m| m.id.as_str().is_some_and(|id| reachable.contains(id)))
            .cloned()
            .collect();
        let history = match (&self.file.envelope.history, self.history_owned) {
            (Some(raw), false) => HistoryToWrite::Raw(raw.clone()),
            _ => HistoryToWrite::Session(self.history.snapshot()),
        };
        let ticket = Arc::new(());
        self.save_ticket = Arc::downgrade(&ticket);
        Ok(SavePlan {
            revision: self.revision(),
            doc_id: self.doc_id,
            state_token: self.state_token,
            document: self.current.clone(),
            had_comps: self.file.envelope.had_comps,
            history,
            workspace: self.file.envelope.workspace.clone(),
            extra: self.file.envelope.extra.clone(),
            media,
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
        if receipt.doc_id != self.doc_id {
            return Err(ProjectIoError::WrongDocument);
        }
        self.file.path = Some(receipt.path.clone());
        self.file.format = pm_model::container::ContainerFormat::Pmv4;
        self.file.version = Some(receipt.version.clone());
        self.file.envelope.wrapped = true;
        // The file now holds this session's history.
        self.file.envelope.history = None;
        self.history_owned = true;
        for entry in receipt.media.iter() {
            let current = self.file.media.get(entry.id.as_str().unwrap_or_default());
            if current.is_some_and(|c| c.revision == entry.revision) {
                self.file.media.insert(entry.clone());
            }
        }
        let was = self.is_dirty();
        self.saved_token = receipt.state_token;
        let revision = receipt.revision;
        drop(receipt);
        self.emit(StoreEvent::Saved { revision });
        let now = self.is_dirty();
        if was != now {
            self.emit(StoreEvent::DirtyChanged(now));
        }
        Ok(())
    }

    /// Asset ids the project or its undo history can still reach.
    pub fn reachable_assets(&self) -> HashSet<String> {
        let mut ids = HashSet::new();
        let mut add_keys = |v: Option<&Value>, ids: &mut HashSet<String>| {
            if let Some(Value::Object(map)) = v {
                ids.extend(map.keys().filter_map(|k| k.as_str().map(str::to_owned)));
            }
        };
        for record in self.history.records() {
            if record.saved.is_some() {
                let (f, b) = record.saved_patches();
                for p in f.iter().chain(b.iter()) {
                    project_io::collect_patch_assets(p, &mut ids, &mut add_keys);
                }
            } else {
                for op in record.forward.iter().chain(record.inverse.iter()) {
                    if let Op::Leaf(p) = op {
                        project_io::collect_patch_assets(p, &mut ids, &mut add_keys);
                    }
                }
            }
        }
        if let Some(NodeKind::Object(assets)) =
            self.root().and_then(|r| r.get("assets")).map(Node::kind)
        {
            ids.extend(assets.keys().filter_map(|k| k.as_str().map(str::to_owned)));
        }
        ids
    }
}

// ── staging helpers ─────────────────────────────────────────────────────────

/// Applies one operation to a tree as `revision`.
fn apply_one(
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

/// The old app's scoped history entries (`scopedPatches`): for each touched
/// path not already covered by a shorter one, its value before and after.
fn scoped_patches(
    before: Option<&Node>,
    after: Option<&Node>,
    mut scopes: Vec<Vec<PathPart>>,
) -> (Vec<Patch>, Vec<Patch>) {
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
fn validate(document: &Document, ops: &[Op]) -> Result<(), TransactionError> {
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
