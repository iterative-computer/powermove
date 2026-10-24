//! Bounded, serial document ingestion away from the engine's event loop.
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;

use pm_protocol::{
    common::ContentHash,
    error::{EngineProcess, ErrorCode},
    host::{CapabilityKind, HandleAccess, HandleResolved},
    patch::{DocumentEncoding, Resync, ResyncSource},
};

pub(crate) type Failure = (ErrorCode, String);
pub(crate) enum Source {
    Inline(Vec<u8>),
    File(HandleResolved, u64),
}
pub(crate) struct Work {
    pub token: u64,
    pub resync: Resync,
    pub base: Option<pm_model::Document>,
    pub source: Source,
}
pub(crate) struct Completed {
    pub token: u64,
    pub result: Result<pm_model::Document, Failure>,
}
pub(crate) struct Reader {
    input: Option<SyncSender<Work>>,
    output: Receiver<Completed>,
    thread: Option<JoinHandle<()>>,
}

impl Reader {
    pub fn new(process: EngineProcess) -> std::io::Result<Self> {
        // No unbounded document copies and no thread per session.
        let (input, jobs) = mpsc::sync_channel::<Work>(2);
        let (answers, output) = mpsc::channel();
        let thread = crate::qos::spawn_engine_thread(process, "pm-document-reader", move || {
            while let Ok(work) = jobs.recv() {
                let token = work.token;
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read(work)))
                    .unwrap_or_else(|_| {
                        Err((ErrorCode::Internal, "The document reader failed.".into()))
                    });
                if answers.send(Completed { token, result }).is_err() {
                    break;
                }
            }
        })?;
        Ok(Self {
            input: Some(input),
            output,
            thread: Some(thread),
        })
    }
    pub fn submit(&self, work: Work) -> Result<(), Failure> {
        self.input
            .as_ref()
            .expect("reader lives")
            .try_send(work)
            .map_err(|error| match error {
                TrySendError::Full(_) => (
                    ErrorCode::BudgetExceeded,
                    "The document reader is busy; retry the resync.".into(),
                ),
                TrySendError::Disconnected(_) => {
                    (ErrorCode::Internal, "The document reader stopped.".into())
                }
            })
    }
    pub fn poll(&self) -> Option<Completed> {
        self.output.try_recv().ok()
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.input.take();
        // Explicit shutdown: do not leave an ingestion thread behind.
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn parse(
    resync: &Resync,
    base: Option<&pm_model::Document>,
    body: &[u8],
) -> Result<pm_model::Document, Failure> {
    let root = match resync.encoding {
        DocumentEncoding::CanonicalJsonV1 => {
            pm_model::Node::parse_bytes(body, resync.revision.get()).map_err(|e| {
                (
                    ErrorCode::InvalidMessage,
                    format!("The resync body is not JSON ({e})."),
                )
            })?
        }
        DocumentEncoding::CanonicalBinaryV1 => return Err((
            ErrorCode::Unsupported,
            "The canonical binary document encoding is not readable yet; resync as canonical JSON."
                .into(),
        )),
    };
    let mirror = match base {
        Some(base) => pm_model::Document::reconciled_node(base, Some(root), resync.revision.get()),
        None => pm_model::Document::from_root(Some(root), resync.revision.get()),
    };
    if mirror.canonical_digest().map(ContentHash) != Some(resync.hash) {
        return Err((
            ErrorCode::InvalidMessage,
            "The resync body does not match its canonical hash; send it again.".into(),
        ));
    }
    Ok(mirror)
}
fn read(work: Work) -> Result<pm_model::Document, Failure> {
    let bytes = match work.source {
        Source::Inline(body) => body,
        Source::File(resolved, length) => {
            let ResyncSource::CacheFile { handle, .. } = &work.resync.source else {
                return Err((ErrorCode::Internal, "not a file resync".into()));
            };
            if &resolved.handle != handle
                || !matches!(
                    resolved.access,
                    HandleAccess::Read | HandleAccess::ReadWrite
                )
                || !matches!(resolved.kind, CapabilityKind::Cache { .. })
            {
                return Err((
                    ErrorCode::CapabilityRejected,
                    "The resync handle does not name a readable cache file.".into(),
                ));
            }
            // Bound the allocation even if a staged file changes after validation.
            use std::io::Read;
            let file = std::fs::File::open(&resolved.path).map_err(|e| {
                (
                    ErrorCode::CapabilityRejected,
                    format!("The resync file could not be read ({}).", e.kind()),
                )
            })?;
            let mut bytes = Vec::new();
            file.take(length.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| {
                    (
                        ErrorCode::CapabilityRejected,
                        format!("The resync file could not be read ({}).", e.kind()),
                    )
                })?;
            if bytes.len() as u64 != length {
                return Err((
                    ErrorCode::InvalidMessage,
                    format!(
                        "The resync file holds {} bytes, not the {length} announced.",
                        bytes.len()
                    ),
                ));
            }
            bytes
        }
    };
    parse(&work.resync, work.base.as_ref(), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_protocol::common::{Bytes, Revision};

    #[test]
    fn a_full_reader_queue_reports_backpressure_without_accepting_more_bytes() {
        let (input, jobs) = mpsc::sync_channel(1);
        let (_answers, output) = mpsc::channel();
        let reader = Reader {
            input: Some(input),
            output,
            thread: None,
        };
        let work = || Work {
            token: 1,
            resync: Resync {
                document: "doc".into(),
                revision: Revision::new(1).unwrap(),
                encoding: DocumentEncoding::CanonicalJsonV1,
                source: ResyncSource::Inline {
                    body: Bytes(vec![]),
                },
                hash: ContentHash([0; 32]),
            },
            base: None,
            source: Source::Inline(vec![]),
        };
        reader.submit(work()).unwrap();
        let (code, _) = reader.submit(work()).unwrap_err();
        assert_eq!(code, ErrorCode::BudgetExceeded);
        assert_eq!(jobs.try_iter().count(), 1);
    }
}
