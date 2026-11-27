//! Asynchronous kernel-backed project context. A lifecycle lease supplied by
//! the shell/session owner prevents close/reopen between preflight and ack.
//! No lease implementation is fabricated here: pm-kernel::Call::Transact only
//! checks revisions. See the integration requirement in agentpanel.md.
use crate::{Error, Result};
use pm_kernel::{
    Ack, Call, Caller, Command, HistoryCall, KernelHandle, Published, QueryContext, QueryMirror,
    Reply, Request, SelectionCall, Snapshot, WriteBody,
};
use std::sync::Arc;

/// The host MUST prevent close/reopen/rebinding this project until this value
/// is dropped. Acquisition is asynchronous on the host; never block main.
/// A lease does not exclude other writers or gestures (kernel folding applies).
pub trait DocumentLease {
    fn document_id(&self) -> u64;
}

pub struct ProjectModel {
    kernel: KernelHandle,
    project: String,
    document: u64,
    caller: Caller,
    mirror: QueryMirror,
}
impl ProjectModel {
    pub fn new(kernel: KernelHandle, project: String, caller: Caller) -> Result<Self> {
        let p = kernel
            .published(&project)
            .ok_or_else(|| Error::UnknownId(project.clone()))?;
        Ok(Self {
            mirror: QueryMirror::new(&p),
            document: p.snapshot.doc_id,
            kernel,
            project,
            caller,
        })
    }
    pub fn document_id(&self) -> u64 {
        self.document
    }
    pub fn published(&self) -> Result<Arc<Published>> {
        let p = self
            .kernel
            .published(&self.project)
            .ok_or_else(|| Error::UnknownId(self.project.clone()))?;
        if p.snapshot.doc_id != self.document {
            return Err(Error::Stale);
        }
        Ok(p)
    }
    /// The mirror is synchronized on the caller's thread. No JS or actor wait.
    pub fn queries(&mut self) -> Result<&mut QueryContext> {
        let p = self.published()?;
        self.mirror.sync(&p);
        Ok(self.mirror.queries())
    }
    async fn call(&self, lease: impl DocumentLease, request: Request) -> Result<Ack> {
        if lease.document_id() != self.document {
            return Err(Error::Stale);
        }
        self.published()?;
        let result = self.kernel.call(&self.project, request).await;
        // Keep lifecycle fencing through publication/ack, not merely enqueue.
        drop(lease);
        let ack = result
            .ok_or_else(|| Error::Host("kernel_stopped".into()))?
            .map_err(|e| Error::Host(e.to_string()))?;
        if let Reply::Throw(e) = &ack.reply {
            return Err(Error::Host(format!("{e:?}")));
        }
        Ok(ack)
    }
    /// One atomic undoable native batch. txid is the caller's stable retry id;
    /// the kernel deduplicates it and reports folding into an active gesture.
    pub async fn transact(
        &self,
        lease: impl DocumentLease,
        commands: Vec<Command>,
        label: String,
        expected_revision: u64,
        txid: u64,
    ) -> Result<Ack> {
        self.call(
            lease,
            Request::new(
                self.caller,
                Call::Transact {
                    label: Some(label),
                    body: WriteBody::Commands(commands),
                    expected_revision: Some(expected_revision),
                },
            )
            .with_txid(txid),
        )
        .await
    }
    pub async fn select(&self, lease: impl DocumentLease, selection: SelectionCall) -> Result<Ack> {
        self.call(lease, Request::new(self.caller, selection)).await
    }
    /// The user's ordinary Undo/Redo command, including external continuations.
    pub async fn history(&self, lease: impl DocumentLease, redo: bool) -> Result<Ack> {
        self.call(
            lease,
            Request::new(
                self.caller,
                if redo {
                    HistoryCall::Redo
                } else {
                    HistoryCall::Undo
                },
            ),
        )
        .await
    }
    /// Guarded result-specific undo. Refuses any interleaving or new document
    /// atomically inside the actor; it never blindly undoes someone else's top
    /// history entry. App-extension/workspace changes need their owning adapter.
    pub async fn undo_result(
        &self,
        lease: impl DocumentLease,
        before: &Snapshot,
        after_revision: u64,
        label: String,
        txid: u64,
    ) -> Result<Ack> {
        if before.doc_id != self.document {
            return Err(Error::Stale);
        }
        let project = before
            .document
            .to_value()
            .ok_or(Error::Invalid("checkpoint document"))?;
        self.call(
            lease,
            Request::new(
                self.caller,
                Call::Restore {
                    project,
                    label,
                    expected_revision: after_revision,
                    expected_document: self.document,
                },
            )
            .with_txid(txid),
        )
        .await
    }
}
