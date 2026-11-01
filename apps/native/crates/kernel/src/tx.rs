//! Transactions (plan v6 §3.2.1): one state machine with today's semantics.
//!
//! A [`Project`] is one open project inside the kernel actor: its
//! [`DocumentStore`], its single **open entry** (today's `pending` history
//! entry) and the transaction bookkeeping around it. Every write API of the
//! v1–3 contract is modelled as a [`Call`] and answered in **its own form**
//! ([`Reply`]), as C-MATRIX's write-admission table records it:
//!
//! | Today | Here |
//! |---|---|
//! | `history.do(label, fn)` | [`HistoryCall::Do`] (native body) or `Begin` + writes + `Commit`/`Rollback` (realm-run callback) |
//! | `history.begin/commit/cancel` | [`HistoryCall::Begin`]/[`Commit`](HistoryCall::Commit)/[`Cancel`](HistoryCall::Cancel) |
//! | `history.external` | [`HistoryCall::External`]; undo runs the closure through a [`Continuation`] |
//! | `history.undo/redo` | [`HistoryCall::Undo`]/[`Redo`](HistoryCall::Redo) |
//! | `edit.begin/dispatch/apply/mutate/commit/cancel` | [`EditCall`] (returns [`EditResult`], `bool`, or a throw, as today) |
//! | `EditGestureAPI` command mode, native drags | [`GestureCall`] (joinable gesture lease streaming provisional steps) |
//! | `workspace.mutate` | [`WorkspaceCall::Mutate`] |
//! | MCP / UI / agent edits | [`Call::Transact`] (optional expected revision) |
//! | `PM.proj` raw writes, touches (write barrier flush) | [`Call::DirectWrite`] |
//!
//! # The rules (and the rule that picked each)
//!
//! * **Short transactions never wait.** The actor runs one call at a time,
//!   so a short transaction *is* the writer lease: it stages, validates,
//!   publishes and replies before the next call runs. If an entry is open
//!   (any owner, any realm), the write applies immediately and **folds** into
//!   it; the ack says `folded: Some(entry)` (§3.2.1 "today's nesting is
//!   kept"; D10 queueing is not implemented, as decided).
//! * **One open entry per project, nesting by depth**, exactly as
//!   `history.begin` nests today. `edit.begin` and gestures open a **lease**
//!   inside it; `edit.begin` throws while another live source edit exists.
//! * **Cancel never removes an acknowledged write.** `edit.cancel`, a
//!   gesture cancel and `history.do`'s throw path revert only the lease
//!   owner's operations, newest first, each only if its after-image still
//!   holds (a [`Probe`] per operation, checked against the staged tree).
//!   Foreign writes folded in are split into their own history entry. If any
//!   owner inverse conflicts (overlapping values, a create/delete the foreign
//!   writer depends on, or an index-addressed write inside an array the
//!   other side restructured), nothing is reverted: the entry is kept whole
//!   and the reply carries [`Notice::CommitWithNotice`]. Rule: S5's choice,
//!   "the simplest implementation that satisfies the invariant", favouring
//!   keeping work.
//! * **`history.cancel` keeps the edits and drops the entry**, as today.
//! * **Owner failure seals.** When a realm generation fails
//!   ([`Call::RealmFailed`]), its open entry is recorded whole (owner and
//!   foreign operations; every applied write was acknowledged when it
//!   returned), later calls from that generation are refused
//!   ([`KernelError::StaleGeneration`], they were never acknowledged), and
//!   its external entries become undo barriers.
//! * **Txids.** Calls may carry `txid`; with the caller's realm and
//!   generation it forms a [`TxId`]. A retry returns the stored [`Ack`]
//!   without running again; [`Call::QueryTx`] answers a lost acknowledgement.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use pm_model::patch::{Patch, PathPart};
use pm_model::{Document, JsString, Node, NodeKind, Op, Value};

use crate::commands::Origin;
use crate::document::{Applied, ChangeCause, DocumentStore, Snapshot, Step, WriteBody};
use crate::error::TransactionError;
use crate::history::ExternalRef;
use crate::selection::Selection;

/// The native realm (shell, native UI, MCP and agents running in Rust).
pub const NATIVE_REALM: u32 = 0;

/// A writer: a realm and its generation (a restarted realm gets a new one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Writer {
    pub realm: u32,
    pub generation: u32,
}

/// Who is calling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Caller {
    pub writer: Writer,
    /// The origin edits from this caller carry (lock policy, provenance).
    pub origin: Origin,
}

impl Caller {
    /// The native UI.
    pub fn native() -> Caller {
        Caller {
            writer: Writer {
                realm: NATIVE_REALM,
                generation: 1,
            },
            origin: Origin::Interface,
        }
    }

    pub fn realm(realm: u32, generation: u32, origin: Origin) -> Caller {
        Caller {
            writer: Writer { realm, generation },
            origin,
        }
    }
}

/// `(realm, generation, txid)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TxId {
    pub realm: u32,
    pub generation: u32,
    pub txid: u64,
}

/// One call into a project.
#[derive(Clone, Debug)]
pub struct Request {
    pub caller: Caller,
    /// Optional transaction id for retry dedupe and lost-ack recovery.
    pub txid: Option<u64>,
    pub call: Call,
}

impl Request {
    pub fn new(caller: Caller, call: impl Into<Call>) -> Request {
        Request {
            caller,
            txid: None,
            call: call.into(),
        }
    }

    pub fn with_txid(mut self, txid: u64) -> Request {
        self.txid = Some(txid);
        self
    }
}

#[derive(Clone, Debug)]
pub enum Call {
    History(HistoryCall),
    Edit(EditCall),
    Gesture(GestureCall),
    Workspace(WorkspaceCall),
    /// A short native edit (UI, MCP, agents): one step, or folded.
    Transact {
        label: Option<String>,
        body: WriteBody,
        /// Refuse with `Stale` unless the document is at this revision.
        expected_revision: Option<u64>,
    },
    /// The write barrier's flush of direct `PM.proj` writes (per-path last
    /// writer wins plus validation). Never an undo step of its own; folds
    /// into an open entry, as today's snapshot diff picks it up.
    DirectWrite { ops: Vec<Op> },
    /// Lost acknowledgement: the stored ack of `txid`, if any.
    QueryTx { txid: TxId },
    /// The caller's realm generation crashed, hung or was disposed.
    RealmFailed,
    /// A realm finished running an external closure the kernel asked for.
    ExternalDone {
        ticket: u64,
        ok: bool,
        message: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub enum HistoryCall {
    /// `history.do` with a native body: atomic, nests into an open entry.
    Do { label: String, body: WriteBody },
    Begin { label: String },
    Commit { label: Option<String> },
    /// Drops the pending entry, keeps the edits.
    Cancel,
    /// `history.do`'s throw path (the realm-run callback threw): reverts the
    /// entry owner's operations (selectively) and closes the entry.
    Rollback,
    External {
        label: String,
        handle: u64,
        bytes: usize,
        cleanup: bool,
    },
    Undo,
    Redo,
    /// `PM.hist.undoIfTop(id)`.
    UndoIfTop { entry: u64 },
    /// `PM.hist.clear()`.
    Clear,
}

#[derive(Clone, Debug)]
pub enum EditCall {
    Begin { label: String },
    Dispatch { body: WriteBody },
    Apply {
        body: WriteBody,
        base_revision: Option<u64>,
        label: Option<String>,
    },
    /// `edit.mutate`: the operations the realm's structural action produced.
    Mutate { label: String, ops: Vec<Op> },
    Commit { label: Option<String> },
    Cancel,
}

#[derive(Clone, Debug)]
pub enum GestureCall {
    Begin { label: String },
    /// A provisional step. `replace`: supersede the previous step (from-start
    /// semantics for relative drags); otherwise accumulate.
    Step {
        gesture: u64,
        body: WriteBody,
        replace: bool,
    },
    Commit { gesture: u64 },
    Cancel { gesture: u64 },
}

#[derive(Clone, Debug)]
pub enum WorkspaceCall {
    Get,
    /// `workspace.mutate`: the workspace after the realm's callback ran.
    /// `threw`: the callback threw after writing (partial writes are kept,
    /// and the throw is returned, as today).
    Mutate { next: Value, threw: Option<String> },
}

macro_rules! into_call {
    ($t:ty, $v:ident) => {
        impl From<$t> for Call {
            fn from(c: $t) -> Call {
                Call::$v(c)
            }
        }
    };
}
into_call!(HistoryCall, History);
into_call!(EditCall, Edit);
into_call!(GestureCall, Gesture);
into_call!(WorkspaceCall, Workspace);

/// `EditResult`.
#[derive(Clone, Debug, PartialEq)]
pub struct EditResult {
    pub ok: bool,
    pub message: Option<String>,
    pub revision: u64,
}

/// Run an external closure in its realm, then send [`Call::ExternalDone`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Continuation {
    pub ticket: u64,
    pub handle: u64,
    pub direction: Direction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Undo,
    Redo,
}

/// A call's answer, in the form the API returns today.
#[derive(Clone, Debug)]
pub enum Reply {
    /// `undefined`.
    Unit,
    Bool(bool),
    /// `history.begin`: the before snapshot (`null` inside a scoped entry).
    Before(Option<Snapshot>),
    /// `history.external`: the entry id (`null` when refused).
    Entry(Option<u64>),
    Edit(EditResult),
    /// A short native edit or `history.do`: the revision, and whether the
    /// document changed.
    Applied { revision: u64, changed: bool },
    /// The caller's realm runs the closure inline, then reports.
    Continuation(Continuation),
    Workspace(Value),
    /// A gesture opened.
    Gesture(u64),
    /// `QueryTx`.
    Ack(Option<Box<Ack>>),
    /// A throw where today throws.
    Throw(KernelError),
}

/// Something the user should hear about.
#[derive(Clone, Debug, PartialEq)]
pub enum Notice {
    /// A cancel could not revert without removing acknowledged work; the
    /// whole entry was kept (undoable).
    CommitWithNotice { label: String, reason: String },
    /// A cancel reverted the owner; folded foreign writes became their own
    /// entry.
    ForeignSplit { label: String, writes: usize },
    /// A failed realm's open entry was sealed into history.
    Sealed { label: String, writes: usize },
    /// Undo stopped at an entry whose realm is gone.
    Barrier { label: String },
    /// The edit is applied but its journal write failed (disk full, I/O
    /// error): it is not crash-safe until the next successful write.
    NotDurable { message: String },
}

/// Errors returned as throws (or refusals) in the caller's API form.
#[derive(Clone, Debug, PartialEq)]
pub enum KernelError {
    Transaction(TransactionError),
    /// `edit.begin` while a source edit is live.
    SourceEditActive,
    /// The calling realm generation failed; nothing from it is admitted.
    StaleGeneration,
    /// An external closure threw, or its realm stopped.
    ExternalFailed(String),
    /// `workspace.mutate`'s callback threw (its partial writes are kept).
    Callback(String),
    /// A direct write was refused; `resync` holds the authority's value at
    /// each refused path (`None`: it no longer exists).
    DirectWrite {
        message: String,
        resync: Vec<(Vec<PathPart>, Option<Value>)>,
    },
    UnknownTicket,
    NoSuchGesture,
}

impl std::fmt::Display for KernelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KernelError::Transaction(e) => e.fmt(f),
            KernelError::SourceEditActive => f.write_str("A source edit is already active"),
            KernelError::StaleGeneration => {
                f.write_str("This extension was restarted; the call was not applied")
            }
            KernelError::ExternalFailed(m) => write!(f, "{m}"),
            KernelError::Callback(m) => write!(f, "{m}"),
            KernelError::DirectWrite { message, .. } => f.write_str(message),
            KernelError::UnknownTicket => f.write_str("Unknown continuation"),
            KernelError::NoSuchGesture => f.write_str("That drag is not in progress"),
        }
    }
}

impl From<TransactionError> for KernelError {
    fn from(e: TransactionError) -> Self {
        KernelError::Transaction(e)
    }
}

/// The acknowledgement of one call.
#[derive(Clone, Debug)]
pub struct Ack {
    pub txid: Option<TxId>,
    /// The document revision after the call.
    pub revision: u64,
    /// The open entry the call's write folded into.
    pub folded: Option<u64>,
    pub notice: Option<Notice>,
    pub reply: Reply,
}

/// Messages the kernel sends to realms (delivered by the actor).
#[derive(Clone, Debug, PartialEq)]
pub enum RealmMessage {
    /// Run an external undo/redo closure, then send [`Call::ExternalDone`].
    RunExternal {
        writer: Writer,
        continuation: Continuation,
    },
    /// An external entry retired; run its cleanup closure.
    Cleanup { writer: Writer, handle: u64 },
}

/// What [`Project::handle`] produced.
#[derive(Debug)]
pub struct Handled {
    /// `None`: the reply is deferred until an external closure finishes in
    /// another realm (`deferred` names the ticket).
    pub reply: Option<Ack>,
    pub deferred: Option<u64>,
    /// Deferred replies this call resolved: `(ticket, ack)`.
    pub resolved: Vec<(u64, Ack)>,
}

impl Handled {
    fn reply(ack: Ack) -> Handled {
        Handled {
            reply: Some(ack),
            deferred: None,
            resolved: Vec::new(),
        }
    }
}

/// An operation's after-image, checked before its inverse may apply.
#[derive(Clone, Debug)]
enum Probe {
    Path {
        path: Vec<PathPart>,
        node: Option<Node>,
    },
    Element {
        array: Vec<PathPart>,
        id: JsString,
        node: Option<Node>,
    },
    Order {
        array: Vec<PathPart>,
        ids: Vec<JsString>,
    },
}

/// One write applied while the entry was open.
#[derive(Clone, Debug)]
struct EntryOp {
    writer: Writer,
    label: String,
    origin: Origin,
    /// Undo/redo applied while the entry was open (today's snapshot diff
    /// includes them; a cancel neither reverts nor splits them).
    navigation: bool,
    forward: Vec<Op>,
    inverse: Vec<Op>,
    scopes: Vec<Vec<PathPart>>,
    probes: Vec<Probe>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseKind {
    Edit,
    Gesture,
}

#[derive(Clone, Debug)]
struct Lease {
    id: u64,
    kind: LeaseKind,
    owner: Writer,
    label: String,
    origin: Origin,
    /// Index into `ops` where the lease began.
    start: usize,
    selection: Selection,
    /// The previous provisional step (index into `ops`), for `replace`.
    last_step: Option<usize>,
}

#[derive(Clone, Debug)]
struct OpenEntry {
    id: u64,
    label: String,
    origin: Origin,
    owner: Writer,
    /// One element per open `begin`/lease (today's `depth`), so a failed
    /// realm's nesting can be released.
    holders: Vec<Writer>,
    /// Opened by `edit.begin` or a gesture (`history.begin` inside returns
    /// `null`, as a scoped pending entry had no snapshot).
    scoped: bool,
    before: Arc<Document>,
    ops: Vec<EntryOp>,
    leases: Vec<Lease>,
}

/// What a selective cancel reverts.
#[derive(Clone, Copy, Debug)]
enum Scope {
    /// A lease's owner operations since it began.
    Lease(u64),
    /// Everything this writer wrote into the entry (`history.do` threw).
    Writer(Writer),
}

/// Retained acknowledgements per project.
const MAX_ACKS: usize = 4096;

impl OpenEntry {
    /// Releases one level held by `writer` (or, if it holds none, the
    /// newest level: today's depth counter had no owner).
    fn release(&mut self, writer: Writer) {
        match self.holders.iter().rposition(|h| *h == writer) {
            Some(i) => {
                self.holders.remove(i);
            }
            None => {
                self.holders.pop();
            }
        }
    }
}

/// One open project inside the kernel.
pub struct Project {
    pub(crate) store: DocumentStore,
    entry: Option<OpenEntry>,
    acks: HashMap<TxId, Ack>,
    ack_order: VecDeque<TxId>,
    dead: HashSet<Writer>,
    workspace: Value,
    next_id: u64,
    /// Continuations waiting for a realm: ticket → (waiting caller, inline).
    tickets: HashMap<u64, Ticket>,
    outbox: Vec<RealmMessage>,
    notices: Vec<Notice>,
}

#[derive(Clone, Debug)]
struct Ticket {
    external: ExternalRef,
    label: String,
    /// The caller's realm runs the closure itself (the reply to
    /// `ExternalDone` is the result); otherwise the undo caller's reply is
    /// deferred until then.
    inline: bool,
    txid: Option<TxId>,
}

impl Project {
    pub fn new(store: DocumentStore) -> Project {
        Project {
            store,
            entry: None,
            acks: HashMap::new(),
            ack_order: VecDeque::new(),
            dead: HashSet::new(),
            workspace: Value::Null,
            next_id: 1,
            tickets: HashMap::new(),
            outbox: Vec::new(),
            notices: Vec::new(),
        }
    }

    pub fn store(&self) -> &DocumentStore {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut DocumentStore {
        &mut self.store
    }

    pub fn workspace(&self) -> &Value {
        &self.workspace
    }

    pub fn set_workspace(&mut self, workspace: Value) {
        self.workspace = workspace;
    }

    /// The open entry: `(id, label, depth)` (the "gesture open" indicator).
    pub fn open_entry(&self) -> Option<(u64, &str, u32)> {
        self.entry
            .as_ref()
            .map(|e| (e.id, e.label.as_str(), e.holders.len() as u32))
    }

    /// Whether a live source edit (`edit.begin`) is open.
    pub fn source_edit_live(&self) -> bool {
        self.entry
            .as_ref()
            .is_some_and(|e| e.leases.iter().any(|l| l.kind == LeaseKind::Edit))
    }

    /// Messages for realms produced since the last call.
    pub fn take_outbox(&mut self) -> Vec<RealmMessage> {
        std::mem::take(&mut self.outbox)
    }

    pub(crate) fn push_notice(&mut self, notice: Notice) {
        self.notices.push(notice);
    }

    /// Notices produced since the last call.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    fn next(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn revision(&self) -> u64 {
        self.store.revision()
    }

    /// Runs one call. The single entry point of the transaction layer.
    pub fn handle(&mut self, request: Request) -> Handled {
        let Request { caller, txid, call } = request;
        let txid = txid.map(|txid| TxId {
            realm: caller.writer.realm,
            generation: caller.writer.generation,
            txid,
        });
        if let Call::QueryTx { txid } = &call {
            let ack = self.acks.get(txid).cloned().map(Box::new);
            return Handled::reply(self.ack(None, None, None, Reply::Ack(ack)));
        }
        if let Some(key) = txid
            && let Some(ack) = self.acks.get(&key)
        {
            return Handled::reply(ack.clone());
        }
        if self.dead.contains(&caller.writer) {
            return Handled::reply(self.ack(
                txid,
                None,
                None,
                Reply::Throw(KernelError::StaleGeneration),
            ));
        }
        let handled = match call {
            Call::History(h) => self.history(caller, txid, h),
            Call::Edit(e) => Handled::reply(self.edit(caller, txid, e)),
            Call::Gesture(g) => Handled::reply(self.gesture(caller, txid, g)),
            Call::Workspace(w) => Handled::reply(self.workspace_call(txid, w)),
            Call::Transact {
                label,
                body,
                expected_revision,
            } => Handled::reply(self.transact(caller, txid, label, body, expected_revision)),
            Call::DirectWrite { ops } => Handled::reply(self.direct_write(caller, txid, ops)),
            Call::RealmFailed => self.realm_failed(caller, txid),
            Call::ExternalDone {
                ticket,
                ok,
                message,
            } => self.external_done(txid, ticket, ok, message),
            Call::QueryTx { .. } => unreachable!("answered above"),
        };
        for retired in self.store.take_retired() {
            self.outbox.push(RealmMessage::Cleanup {
                writer: Writer {
                    realm: retired.realm,
                    generation: retired.generation,
                },
                handle: retired.handle,
            });
        }
        if let (Some(key), Some(ack)) = (txid, &handled.reply) {
            self.remember(key, ack.clone());
        }
        for (_, ack) in &handled.resolved {
            if let Some(key) = ack.txid {
                self.remember(key, ack.clone());
            }
        }
        if let Some(notice) = handled.reply.as_ref().and_then(|a| a.notice.clone()) {
            self.notices.push(notice);
        }
        handled
    }

    fn remember(&mut self, key: TxId, ack: Ack) {
        if self.acks.insert(key, ack).is_none() {
            self.ack_order.push_back(key);
            while self.ack_order.len() > MAX_ACKS {
                if let Some(old) = self.ack_order.pop_front() {
                    self.acks.remove(&old);
                }
            }
        }
    }

    fn ack(
        &self,
        txid: Option<TxId>,
        folded: Option<u64>,
        notice: Option<Notice>,
        reply: Reply,
    ) -> Ack {
        Ack {
            txid,
            revision: self.revision(),
            folded,
            notice,
            reply,
        }
    }

    // ── writes ──────────────────────────────────────────────────────────────

    /// Applies one short write: folds into the open entry, or records it as
    /// its own step. Returns `(changed, folded)`.
    fn write(
        &mut self,
        writer: Writer,
        origin: Origin,
        label: &str,
        body: &WriteBody,
        record: bool,
    ) -> Result<(bool, Option<u64>), TransactionError> {
        let entry_id = self.entry.as_ref().map(|e| e.id);
        let before = self.store.current().clone();
        let applied = self.store.kernel_apply(
            body,
            origin,
            ChangeCause::Write {
                label: label.to_owned(),
                origin,
                entry: entry_id,
            },
        )?;
        let Some(applied) = applied else {
            return Ok((false, entry_id));
        };
        if self.entry.is_some() {
            self.fold(writer, origin, label, applied, false);
            return Ok((true, entry_id));
        }
        if record {
            self.store.kernel_record(
                label.to_owned(),
                origin,
                applied.forward,
                applied.inverse,
                &before,
                applied.scopes,
            );
        }
        Ok((true, None))
    }

    fn fold(
        &mut self,
        writer: Writer,
        origin: Origin,
        label: &str,
        applied: Applied,
        navigation: bool,
    ) -> usize {
        let root = self.store.current().root().cloned();
        let probes = probes(root.as_ref(), &applied.forward, &applied.scopes);
        let entry = self.entry.as_mut().expect("open entry");
        entry.ops.push(EntryOp {
            writer,
            label: label.to_owned(),
            origin,
            navigation,
            forward: applied.forward,
            inverse: applied.inverse,
            scopes: applied.scopes,
            probes,
        });
        entry.ops.len() - 1
    }

    fn open(&mut self, owner: Writer, label: &str, origin: Origin, scoped: bool) -> u64 {
        let id = self.next();
        self.entry = Some(OpenEntry {
            id,
            label: label.to_owned(),
            origin,
            owner,
            holders: vec![owner],
            scoped,
            before: self.store.current().clone(),
            ops: Vec::new(),
            leases: Vec::new(),
        });
        id
    }

    /// Records an entry's operations as one step (if they changed
    /// anything overall). Returns whether a record was pushed.
    fn record_ops(&mut self, label: String, origin: Origin, before: &Document, ops: Vec<EntryOp>) -> bool {
        if ops.is_empty() {
            return false;
        }
        let scopes: Vec<Vec<PathPart>> = ops.iter().flat_map(|o| o.scopes.clone()).collect();
        let (forward_patches, _) = crate::document::scoped_patches(
            before.root(),
            self.store.current().root(),
            scopes.clone(),
        );
        if forward_patches.is_empty() {
            return false; // net unchanged: today's commit compares snapshots
        }
        let forward: Vec<Op> = ops.iter().flat_map(|o| o.forward.iter().cloned()).collect();
        let inverse: Vec<Op> = ops
            .iter()
            .rev()
            .flat_map(|o| o.inverse.iter().cloned())
            .collect();
        self.store
            .kernel_record(label, origin, forward, inverse, before, scopes);
        true
    }

    /// Closes the open entry as one step. `true` when a record was pushed.
    fn close_entry(&mut self, label: Option<String>) -> bool {
        let Some(entry) = self.entry.take() else {
            return false;
        };
        let label = label.unwrap_or(entry.label);
        self.record_ops(label, entry.origin, &entry.before, entry.ops)
    }

    /// Reverts `owner`'s operations at `ops[start..]` if every inverse still
    /// validates. `Ok(n)`: n writes reverted (and removed from the entry);
    /// `Err(reason)`: conflict, nothing changed.
    fn revert_owner(&mut self, owner: Writer, start: usize, label: &str) -> Result<usize, String> {
        let entry = self.entry.as_ref().expect("open entry");
        let owned: Vec<usize> = (start..entry.ops.len())
            .filter(|&i| entry.ops[i].writer == owner && !entry.ops[i].navigation)
            .collect();
        if owned.is_empty() {
            return Ok(0);
        }
        // Index hazards: an index-addressed write inside an array the other
        // side restructured after it (or before it, for the owner's
        // restructure) would undo the wrong element.
        for &i in &owned {
            let o = &entry.ops[i];
            for f in entry.ops.iter().skip(i + 1) {
                if f.writer == owner || f.navigation {
                    continue;
                }
                // The owner restructured a list the foreign write then
                // addressed by index, or the foreign write restructured a
                // list the owner had addressed by index.
                if hazard(o, f) || hazard(f, o) {
                    return Err(format!(
                        "{} changed the same list as this edit",
                        describe(f.origin)
                    ));
                }
            }
        }
        let revision = self.revision() + 1;
        let mut root = self.store.current().root().cloned();
        let mut ops = Vec::new();
        for &i in owned.iter().rev() {
            let op = &entry.ops[i];
            if !op.probes.iter().all(|p| probe_holds(root.as_ref(), p)) {
                return Err("another writer changed what this edit changed".into());
            }
            for inv in &op.inverse {
                root = crate::document::apply_one(root, inv, revision)
                    .map_err(|e| e.to_string())?
                    .0;
                ops.push(inv.clone());
            }
        }
        let document = Document::from_root(root, revision);
        crate::document::validate(&document, &ops).map_err(|e| e.to_string())?;
        self.store.kernel_publish(
            document,
            ops,
            ChangeCause::Revert {
                label: label.to_owned(),
            },
        );
        let entry = self.entry.as_mut().expect("open entry");
        let n = owned.len();
        for &i in owned.iter().rev() {
            entry.ops.remove(i);
        }
        Ok(n)
    }

    /// The selective cancel of a lease (or, `lease == None`, of the entry
    /// owner for `history.do`'s throw path). Closes the lease and lowers the
    /// depth; when the entry closes, folded foreign writes become their own
    /// entry. Returns the notice, if any.
    fn cancel_scope(&mut self, scope: Scope) -> Option<Notice> {
        let entry = self.entry.as_ref()?;
        let (owner, start, label, selection, lease) = match scope {
            Scope::Lease(id) => {
                let l = entry.leases.iter().find(|l| l.id == id)?;
                (l.owner, l.start, l.label.clone(), Some(l.selection.clone()), Some(id))
            }
            Scope::Writer(writer) => (writer, 0, entry.label.clone(), None, None),
        };
        let outcome = self.revert_owner(owner, start, &label);
        let entry = self.entry.as_mut().expect("open entry");
        match lease {
            Some(id) => {
                entry.leases.retain(|l| l.id != id);
                entry.release(owner);
            }
            None => {
                // `rollback` resets the depth, as today.
                entry.holders.clear();
                entry.leases.clear();
            }
        }
        let closing = entry.holders.is_empty();
        match outcome {
            Err(reason) => {
                if closing {
                    let entry = self.entry.take().expect("open entry");
                    let label = entry.label.clone();
                    self.record_ops(label.clone(), entry.origin, &entry.before, entry.ops);
                    return Some(Notice::CommitWithNotice { label, reason });
                }
                Some(Notice::CommitWithNotice { label, reason })
            }
            Ok(_) => {
                if let Some(selection) = selection {
                    self.store.set_selection(selection);
                }
                if !closing {
                    return None;
                }
                let entry = self.entry.take().expect("open entry");
                let kept: Vec<EntryOp> = entry.ops.into_iter().filter(|o| !o.navigation).collect();
                let writes = kept.len();
                if writes == 0 {
                    return None;
                }
                let split_label = kept
                    .iter()
                    .find(|o| o.writer != owner)
                    .map_or_else(|| entry.label.clone(), |o| o.label.clone());
                let recorded = self.record_ops(split_label.clone(), kept[0].origin, &entry.before, kept);
                recorded.then_some(Notice::ForeignSplit {
                    label: split_label,
                    writes,
                })
            }
        }
    }

    // ── history.* ───────────────────────────────────────────────────────────

    fn history(&mut self, caller: Caller, txid: Option<TxId>, call: HistoryCall) -> Handled {
        let writer = caller.writer;
        let reply = match call {
            HistoryCall::Do { label, body } => {
                match self.write(writer, caller.origin, &label, &body, true) {
                    Ok((changed, folded)) => {
                        return Handled::reply(self.ack(
                            txid,
                            folded,
                            None,
                            Reply::Applied {
                                revision: self.revision(),
                                changed,
                            },
                        ));
                    }
                    Err(e) => Reply::Throw(e.into()),
                }
            }
            HistoryCall::Begin { label } => match &mut self.entry {
                Some(entry) => {
                    entry.holders.push(writer);
                    let before = (!entry.scoped).then(|| Snapshot {
                        doc_id: self.store.doc_id(),
                        revision: entry.before.revision(),
                        document: entry.before.clone(),
                    });
                    Reply::Before(before)
                }
                None => {
                    self.open(writer, &label, caller.origin, false);
                    Reply::Before(Some(self.store.snapshot()))
                }
            },
            HistoryCall::Commit { label } => Reply::Bool(self.commit(writer, label)),
            HistoryCall::Cancel => {
                // Keeps the edits; drops the entry (no rollback).
                self.entry = None;
                Reply::Unit
            }
            HistoryCall::Rollback => {
                let notice = self.cancel_scope(Scope::Writer(writer));
                self.entry = None;
                return Handled::reply(self.ack(txid, None, notice, Reply::Bool(true)));
            }
            HistoryCall::External {
                label,
                handle,
                bytes,
                cleanup,
            } => {
                let external = ExternalRef {
                    realm: writer.realm,
                    generation: writer.generation,
                    handle,
                    cleanup,
                };
                let label = if label.is_empty() {
                    "Interface change".to_owned()
                } else {
                    label
                };
                Reply::Entry(Some(self.store.kernel_push_external(
                    label,
                    caller.origin,
                    external,
                    bytes,
                )))
            }
            HistoryCall::Undo => return self.step(caller, txid, Direction::Undo),
            HistoryCall::Redo => return self.step(caller, txid, Direction::Redo),
            HistoryCall::UndoIfTop { entry } => {
                if self.store.history().peek_undo().map(|r| r.id) != Some(entry) {
                    Reply::Bool(false)
                } else {
                    return self.step(caller, txid, Direction::Undo);
                }
            }
            HistoryCall::Clear => {
                self.store.kernel_clear();
                Reply::Unit
            }
        };
        Handled::reply(self.ack(txid, None, None, reply))
    }

    /// `history.commit`: `false` unless the outermost commit recorded a step.
    fn commit(&mut self, writer: Writer, label: Option<String>) -> bool {
        let Some(entry) = &mut self.entry else {
            return false;
        };
        entry.release(writer);
        if !entry.holders.is_empty() {
            return false;
        }
        self.close_entry(label)
    }

    fn step(&mut self, caller: Caller, txid: Option<TxId>, direction: Direction) -> Handled {
        let result = match direction {
            Direction::Undo => self.store.kernel_undo(),
            Direction::Redo => self.store.kernel_redo(),
        };
        match result {
            Err(TransactionError::NothingToUndo | TransactionError::NothingToRedo) => {
                Handled::reply(self.ack(txid, None, None, Reply::Bool(false)))
            }
            Err(e) => Handled::reply(self.ack(txid, None, None, Reply::Throw(e.into()))),
            Ok(Step::Document {
                ops,
                reverse,
                label,
            }) => {
                let folded = self.entry.as_ref().map(|e| e.id);
                if folded.is_some() {
                    // Today's pending snapshot diff includes the undo.
                    let applied = Applied {
                        scopes: ops.iter().map(|o| o.path().to_vec()).collect(),
                        forward: ops.to_vec(),
                        inverse: reverse.to_vec(),
                    };
                    self.fold(caller.writer, caller.origin, &label, applied, true);
                }
                Handled::reply(self.ack(txid, folded, None, Reply::Bool(true)))
            }
            Ok(Step::External { external, label }) => {
                let owner = Writer {
                    realm: external.realm,
                    generation: external.generation,
                };
                if self.dead.contains(&owner) {
                    // Barrier: the closure is gone; the cursor stays.
                    self.store.kernel_unstep(direction == Direction::Undo);
                    return Handled::reply(self.ack(
                        txid,
                        None,
                        Some(Notice::Barrier { label }),
                        Reply::Bool(false),
                    ));
                }
                let ticket = self.next();
                let continuation = Continuation {
                    ticket,
                    handle: external.handle,
                    direction,
                };
                let inline = caller.writer == owner;
                self.tickets.insert(
                    ticket,
                    Ticket {
                        external,
                        label,
                        inline,
                        txid,
                    },
                );
                if inline {
                    // The realm is blocked in its own synchronous undo: run
                    // the handle inline, then report (continuation).
                    return Handled::reply(self.ack(
                        txid,
                        None,
                        None,
                        Reply::Continuation(continuation),
                    ));
                }
                self.outbox.push(RealmMessage::RunExternal {
                    writer: owner,
                    continuation,
                });
                Handled {
                    reply: None,
                    deferred: Some(ticket),
                    resolved: Vec::new(),
                }
            }
        }
    }

    fn external_done(
        &mut self,
        txid: Option<TxId>,
        ticket: u64,
        ok: bool,
        message: Option<String>,
    ) -> Handled {
        let Some(t) = self.tickets.remove(&ticket) else {
            return Handled::reply(self.ack(txid, None, None, Reply::Throw(KernelError::UnknownTicket)));
        };
        // The cursor already moved (as `idx--` ran before `entry.undo()`).
        let result = if ok {
            Reply::Bool(true)
        } else {
            Reply::Throw(KernelError::ExternalFailed(
                message.unwrap_or_else(|| format!("Undo of “{}” failed", t.label)),
            ))
        };
        if t.inline {
            return Handled::reply(self.ack(txid, None, None, result));
        }
        let resolved = self.ack(t.txid, None, None, result);
        Handled {
            reply: Some(self.ack(txid, None, None, Reply::Unit)),
            deferred: None,
            resolved: vec![(ticket, resolved)],
        }
    }

    // ── edit.* ──────────────────────────────────────────────────────────────

    fn edit_ok(&self, txid: Option<TxId>, folded: Option<u64>, message: &str) -> Ack {
        self.ack(
            txid,
            folded,
            None,
            Reply::Edit(EditResult {
                ok: true,
                message: Some(message.to_owned()),
                revision: self.revision(),
            }),
        )
    }

    fn edit_fail(&self, txid: Option<TxId>, message: impl Into<String>) -> Ack {
        self.ack(
            txid,
            None,
            None,
            Reply::Edit(EditResult {
                ok: false,
                message: Some(message.into()),
                revision: self.revision(),
            }),
        )
    }

    fn live_lease(&self) -> Option<&Lease> {
        self.entry
            .as_ref()?
            .leases
            .iter()
            .rev()
            .find(|l| l.kind == LeaseKind::Edit)
    }

    fn open_lease(&mut self, caller: Caller, kind: LeaseKind, label: &str) -> u64 {
        if self.entry.is_none() {
            self.open(caller.writer, label, caller.origin, true);
        } else {
            self.entry.as_mut().expect("open").holders.push(caller.writer);
        }
        let id = self.next();
        let selection = self.store.selection().clone();
        let entry = self.entry.as_mut().expect("open");
        entry.leases.push(Lease {
            id,
            kind,
            owner: caller.writer,
            label: label.to_owned(),
            origin: caller.origin,
            start: entry.ops.len(),
            selection,
            last_step: None,
        });
        id
    }

    /// Ends a lease successfully; closes the entry at depth 0. `true` when a
    /// record was pushed.
    fn end_lease(&mut self, id: u64, label: Option<String>) -> bool {
        let Some(entry) = &mut self.entry else {
            return false;
        };
        let owner = entry.leases.iter().find(|l| l.id == id).map(|l| l.owner);
        entry.leases.retain(|l| l.id != id);
        if let Some(owner) = owner {
            entry.release(owner);
        }
        if !entry.holders.is_empty() {
            return false;
        }
        self.close_entry(label)
    }

    fn edit(&mut self, caller: Caller, txid: Option<TxId>, call: EditCall) -> Ack {
        match call {
            EditCall::Begin { label } => {
                if self.live_lease().is_some() {
                    return self.ack(txid, None, None, Reply::Throw(KernelError::SourceEditActive));
                }
                let label = if label.is_empty() { "Edit source".to_owned() } else { label };
                self.open_lease(caller, LeaseKind::Edit, &label);
                self.ack(txid, self.entry.as_ref().map(|e| e.id), None, Reply::Unit)
            }
            EditCall::Dispatch { body } => {
                let (label, origin) = match self.live_lease() {
                    Some(l) => (l.label.clone(), l.origin),
                    None => ("Edit source".to_owned(), caller.origin),
                };
                self.edit_apply(Caller { origin, ..caller }, txid, body, None, Some(label))
            }
            EditCall::Apply {
                body,
                base_revision,
                label,
            } => self.edit_apply(caller, txid, body, base_revision, label),
            EditCall::Mutate { label, ops } => {
                if self.live_lease().is_some() {
                    return self.edit_fail(txid, "Finish the active source edit before changing structure");
                }
                if ops.iter().any(|op| !structural_path(op.path())) {
                    return self.edit_fail(txid, "Structural edits may only change the project layer tree");
                }
                let label = if label.is_empty() { "Edit source".to_owned() } else { label };
                match self.write(caller.writer, caller.origin, &label, &WriteBody::Ops(ops), true) {
                    Ok((false, folded)) => self.edit_ok(txid, folded, "No source changes"),
                    Ok((true, folded)) => self.edit_ok(txid, folded, &label),
                    Err(e) => self.edit_fail(txid, e.to_string()),
                }
            }
            EditCall::Commit { label } => {
                let Some(lease) = self.live_lease().cloned() else {
                    return self.edit_fail(txid, "No source edit is active");
                };
                let entry_id = self.entry.as_ref().map(|e| e.id);
                let changed = self
                    .entry
                    .as_ref()
                    .is_some_and(|e| e.ops[lease.start.min(e.ops.len())..].iter().any(|o| !o.navigation));
                let label = label.unwrap_or_else(|| lease.label.clone());
                let recorded = self.end_lease(lease.id, Some(label.clone()));
                let message = if changed || recorded { label.as_str() } else { "No source changes" };
                self.edit_ok(txid, entry_id, message)
            }
            EditCall::Cancel => {
                let Some(lease) = self.live_lease().map(|l| l.id) else {
                    return self.ack(txid, None, None, Reply::Bool(false));
                };
                let notice = self.cancel_scope(Scope::Lease(lease));
                self.ack(txid, None, notice, Reply::Bool(true))
            }
        }
    }

    fn edit_apply(
        &mut self,
        caller: Caller,
        txid: Option<TxId>,
        body: WriteBody,
        base_revision: Option<u64>,
        label: Option<String>,
    ) -> Ack {
        if body.is_empty() {
            return self.edit_fail(txid, "No source edits supplied");
        }
        if let Some(base) = base_revision
            && base != self.revision()
        {
            return self.edit_fail(
                txid,
                format!("Project changed: expected revision {base}, found {}", self.revision()),
            );
        }
        let label = label.unwrap_or_else(|| "Edit source".to_owned());
        match self.write(caller.writer, caller.origin, &label, &body, true) {
            Ok((_, folded)) => self.edit_ok(txid, folded, &label),
            // Atomic: a failed batch applied nothing, so there is nothing to
            // roll back (today's rollback of the whole pending entry would
            // remove acknowledged writes; the kernel never does).
            Err(e) => self.edit_fail(txid, e.to_string()),
        }
    }

    // ── gestures ────────────────────────────────────────────────────────────

    fn gesture(&mut self, caller: Caller, txid: Option<TxId>, call: GestureCall) -> Ack {
        match call {
            GestureCall::Begin { label } => {
                let id = self.open_lease(caller, LeaseKind::Gesture, &label);
                self.ack(txid, self.entry.as_ref().map(|e| e.id), None, Reply::Gesture(id))
            }
            GestureCall::Step {
                gesture,
                body,
                replace,
            } => {
                let Some(lease) = self.gesture_lease(gesture, caller.writer) else {
                    return self.ack(txid, None, None, Reply::Throw(KernelError::NoSuchGesture));
                };
                match self.gesture_step(caller, &lease, &body, replace) {
                    Ok(()) => self.ack(
                        txid,
                        self.entry.as_ref().map(|e| e.id),
                        None,
                        Reply::Applied {
                            revision: self.revision(),
                            changed: true,
                        },
                    ),
                    Err(e) => self.ack(txid, None, None, Reply::Throw(e.into())),
                }
            }
            GestureCall::Commit { gesture } => {
                let Some(lease) = self.gesture_lease(gesture, caller.writer) else {
                    return self.ack(txid, None, None, Reply::Throw(KernelError::NoSuchGesture));
                };
                let recorded = self.end_lease(lease.id, None);
                self.ack(txid, None, None, Reply::Bool(recorded))
            }
            GestureCall::Cancel { gesture } => {
                let Some(lease) = self.gesture_lease(gesture, caller.writer) else {
                    return self.ack(txid, None, None, Reply::Throw(KernelError::NoSuchGesture));
                };
                let notice = self.cancel_scope(Scope::Lease(lease.id));
                self.ack(txid, None, notice, Reply::Bool(true))
            }
        }
    }

    fn gesture_lease(&self, id: u64, owner: Writer) -> Option<Lease> {
        self.entry
            .as_ref()?
            .leases
            .iter()
            .find(|l| l.id == id && l.kind == LeaseKind::Gesture && l.owner == owner)
            .cloned()
    }

    fn gesture_step(
        &mut self,
        caller: Caller,
        lease: &Lease,
        body: &WriteBody,
        replace: bool,
    ) -> Result<(), TransactionError> {
        let entry_id = self.entry.as_ref().map(|e| e.id);
        let cause = ChangeCause::Write {
            label: lease.label.clone(),
            origin: lease.origin,
            entry: entry_id,
        };
        // Supersede the previous step only while it is still the newest
        // write and its after-images hold; otherwise accumulate (a foreign
        // write landed in between and must not be clobbered).
        let previous = replace
            .then_some(lease.last_step)
            .flatten()
            .filter(|&i| {
                let entry = self.entry.as_ref().expect("open");
                i + 1 == entry.ops.len() && {
                    let root = self.store.current().root().cloned();
                    entry.ops[i]
                        .probes
                        .iter()
                        .all(|p| probe_holds(root.as_ref(), p))
                }
            });
        let applied = match previous {
            Some(i) => {
                let prev_inverse = self.entry.as_ref().expect("open").ops[i].inverse.clone();
                let applied = self
                    .store
                    .kernel_replace(&prev_inverse, body, lease.origin, cause)?;
                self.entry.as_mut().expect("open").ops.remove(i);
                applied
            }
            None => self.store.kernel_apply(body, lease.origin, cause)?,
        };
        let index = match applied {
            Some(applied) => Some(self.fold(
                caller.writer,
                lease.origin,
                &lease.label,
                applied,
                false,
            )),
            None => None,
        };
        let entry = self.entry.as_mut().expect("open");
        if let Some(l) = entry.leases.iter_mut().find(|l| l.id == lease.id) {
            l.last_step = index;
        }
        Ok(())
    }

    // ── others ──────────────────────────────────────────────────────────────

    fn workspace_call(&mut self, txid: Option<TxId>, call: WorkspaceCall) -> Ack {
        match call {
            WorkspaceCall::Get => self.ack(txid, None, None, Reply::Workspace(self.workspace.clone())),
            WorkspaceCall::Mutate { next, threw } => {
                // No document history and no source-edit admission, as
                // today; a throwing callback keeps its partial writes.
                self.workspace = next;
                match threw {
                    Some(message) => self.ack(txid, None, None, Reply::Throw(KernelError::Callback(message))),
                    None => self.ack(txid, None, None, Reply::Workspace(self.workspace.clone())),
                }
            }
        }
    }

    fn transact(
        &mut self,
        caller: Caller,
        txid: Option<TxId>,
        label: Option<String>,
        body: WriteBody,
        expected_revision: Option<u64>,
    ) -> Ack {
        if let Some(expected) = expected_revision
            && expected != self.revision()
        {
            return self.ack(
                txid,
                None,
                None,
                Reply::Throw(KernelError::Transaction(TransactionError::Stale {
                    expected,
                    current: self.revision(),
                })),
            );
        }
        let label = label
            .or_else(|| match &body {
                WriteBody::Commands(c) => c.first().map(crate::Command::label),
                WriteBody::Ops(_) => None,
            })
            .unwrap_or_else(|| "Edit".to_owned());
        match self.write(caller.writer, caller.origin, &label, &body, true) {
            Ok((changed, folded)) => self.ack(
                txid,
                folded,
                None,
                Reply::Applied {
                    revision: self.revision(),
                    changed,
                },
            ),
            Err(e) => self.ack(txid, None, None, Reply::Throw(e.into())),
        }
    }

    fn direct_write(&mut self, caller: Caller, txid: Option<TxId>, ops: Vec<Op>) -> Ack {
        // Per-path validation: a write whose parent no longer exists (the
        // target was deleted natively) is refused with the authority's value
        // so the page can resync that path.
        let root = self.store.current().root();
        let mut resync = Vec::new();
        for op in &ops {
            let path = op.path();
            let parent_ok = match path.split_last() {
                None => true,
                Some((_, parent)) => root.and_then(|r| r.lookup(parent)).is_some(),
            };
            if !parent_ok {
                let value = root.and_then(|r| r.lookup(path)).map(Node::to_value);
                resync.push((path.to_vec(), value));
            }
        }
        if !resync.is_empty() {
            return self.ack(
                txid,
                None,
                None,
                Reply::Throw(KernelError::DirectWrite {
                    message: "The target of a direct write no longer exists".into(),
                    resync,
                }),
            );
        }
        match self.write(caller.writer, caller.origin, "Direct edit", &WriteBody::Ops(ops.clone()), false) {
            Ok((changed, folded)) => self.ack(
                txid,
                folded,
                None,
                Reply::Applied {
                    revision: self.revision(),
                    changed,
                },
            ),
            Err(e) => {
                let root = self.store.current().root();
                let resync = ops
                    .iter()
                    .map(|op| {
                        let path = op.path().to_vec();
                        let value = root.and_then(|r| r.lookup(&path)).map(Node::to_value);
                        (path, value)
                    })
                    .collect();
                self.ack(
                    txid,
                    None,
                    None,
                    Reply::Throw(KernelError::DirectWrite {
                        message: e.to_string(),
                        resync,
                    }),
                )
            }
        }
    }

    fn realm_failed(&mut self, caller: Caller, txid: Option<TxId>) -> Handled {
        let failed = caller.writer;
        self.dead.insert(failed);
        let mut notice = None;
        if let Some(entry) = &mut self.entry {
            entry.leases.retain(|l| l.owner != failed);
            entry.holders.retain(|h| *h != failed);
            if entry.owner == failed || entry.holders.is_empty() {
                // Seal: every applied write was acknowledged; keep them all.
                let entry = self.entry.take().expect("open");
                let writes = entry.ops.iter().filter(|o| !o.navigation).count();
                let label = entry.label.clone();
                self.record_ops(label.clone(), entry.origin, &entry.before, entry.ops);
                notice = Some(Notice::Sealed { label, writes });
            }
        }
        // Continuations waiting on the failed realm resolve as failures.
        let waiting: Vec<u64> = self
            .tickets
            .iter()
            .filter(|(_, t)| t.external.realm == failed.realm && t.external.generation == failed.generation)
            .map(|(k, _)| *k)
            .collect();
        let mut resolved = Vec::new();
        for ticket in waiting {
            let t = self.tickets.remove(&ticket).expect("listed");
            if !t.inline {
                resolved.push((
                    ticket,
                    self.ack(
                        t.txid,
                        None,
                        None,
                        Reply::Throw(KernelError::ExternalFailed(format!(
                            "The extension that owns “{}” stopped",
                            t.label
                        ))),
                    ),
                ));
            }
        }
        if let Some(n) = &notice {
            self.notices.push(n.clone());
        }
        Handled {
            reply: Some(self.ack(txid, None, notice, Reply::Unit)),
            deferred: None,
            resolved,
        }
    }
}

fn describe(origin: Origin) -> &'static str {
    match origin {
        Origin::Interface => "Another edit",
        Origin::Agent => "The agent",
        Origin::Mcp => "An MCP client",
        Origin::Extension => "An extension",
    }
}

/// `edit.mutate` may change only the layer tree and compositions (plus the
/// revision/provenance bookkeeping the kernel owns).
fn structural_path(path: &[PathPart]) -> bool {
    let first = path.first().map(PathPart::property_key);
    matches!(
        first.as_ref().and_then(|k| k.as_str()),
        Some("layers" | "comps" | "revision" | "edits")
    )
}

fn is_prefix(prefix: &[PathPart], path: &[PathPart]) -> bool {
    prefix.len() <= path.len()
        && prefix
            .iter()
            .zip(path)
            .all(|(a, b)| a.property_key() == b.property_key())
}

/// Arrays an operation restructures (inserts, deletes, reorders, splices).
fn restructured(op: &EntryOp) -> Vec<&[PathPart]> {
    let mut out = Vec::new();
    for (i, f) in op.forward.iter().enumerate() {
        match f {
            Op::Order { path, .. } | Op::Insert { path, .. } | Op::Delete { path, .. } => {
                out.push(path.as_slice())
            }
            Op::Leaf(p) => {
                if let Some(scope) = op.scopes.get(i)
                    && scope.len() < p.path.len()
                {
                    out.push(scope.as_slice());
                }
            }
        }
    }
    out
}

/// `later` writes by index inside an array `earlier` restructured.
fn hazard(earlier: &EntryOp, later: &EntryOp) -> bool {
    let arrays = restructured(earlier);
    later.forward.iter().any(|f| {
        let path = f.path();
        arrays
            .iter()
            .any(|a| path.len() > a.len() && is_prefix(a, path) && matches!(path[a.len()], PathPart::Index(_)))
    })
}

fn element<'a>(root: Option<&'a Node>, array: &[PathPart], id: &JsString) -> Option<&'a Node> {
    match root?.lookup(array)?.kind() {
        NodeKind::Array(items) => items
            .iter()
            .find(|i| i.get("id").and_then(crate::model::string_of) == Some(id)),
        _ => None,
    }
}

fn array_ids(root: Option<&Node>, array: &[PathPart]) -> Vec<JsString> {
    match root.and_then(|r| r.lookup(array)).map(Node::kind) {
        Some(NodeKind::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("id").and_then(crate::model::string_of).cloned())
            .collect(),
        _ => Vec::new(),
    }
}

/// After-images of a write's operations on the published tree.
fn probes(root: Option<&Node>, forward: &[Op], scopes: &[Vec<PathPart>]) -> Vec<Probe> {
    forward
        .iter()
        .enumerate()
        .map(|(i, op)| match op {
            Op::Leaf(p) => {
                let path = scopes.get(i).cloned().unwrap_or_else(|| p.path.clone());
                let node = root.and_then(|r| r.lookup(&path)).cloned();
                Probe::Path { path, node }
            }
            Op::Insert { path, value, .. } => {
                let id = value
                    .get("id")
                    .and_then(Value::as_js_string)
                    .cloned()
                    .unwrap_or_default();
                Probe::Element {
                    node: element(root, path, &id).cloned(),
                    array: path.clone(),
                    id,
                }
            }
            Op::Delete { path, id } => Probe::Element {
                array: path.clone(),
                id: id.clone(),
                node: None,
            },
            Op::Order { path, .. } => Probe::Order {
                array: path.clone(),
                ids: array_ids(root, path),
            },
        })
        .collect()
}

fn probe_holds(root: Option<&Node>, probe: &Probe) -> bool {
    match probe {
        Probe::Path { path, node } => same(root.and_then(|r| r.lookup(path)), node.as_ref()),
        Probe::Element { array, id, node } => same(element(root, array, id), node.as_ref()),
        Probe::Order { array, ids } => &array_ids(root, array) == ids,
    }
}

/// Content equality (node revisions ignored; shared subtrees short-cut).
fn same(a: Option<&Node>, b: Option<&Node>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_node(a, b),
        _ => false,
    }
}

fn same_node(a: &Node, b: &Node) -> bool {
    match (a.kind(), b.kind()) {
        (NodeKind::Array(x), NodeKind::Array(y)) => {
            Arc::ptr_eq(x, y) || x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same_node(p, q))
        }
        (NodeKind::Object(x), NodeKind::Object(y)) => {
            Arc::ptr_eq(x, y)
                || x.len() == y.len()
                    && x.iter()
                        .all(|(k, v)| y.get(k).is_some_and(|w| same_node(v, w)))
        }
        (x, y) => x == y,
    }
}

#[allow(dead_code)]
fn _assert_send() {
    fn send<T: Send>() {}
    send::<Project>();
    send::<Ack>();
    let _ = Patch::delete(Vec::<PathPart>::new());
}
