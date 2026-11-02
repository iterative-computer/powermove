//! Bounded, nonblocking UI -> worker publication. The receiving worker binds
//! the real service states listed on AuthorityServices; no UI-side second writer.
//! Drop/disconnection fails closed. A full mailbox never loses typed values.
use super::values::{Save, SheetView, ValuesService};
use super::{AuthorityServices, Command, PrivateValue, RequestId, ServiceCode, Target};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

#[derive(Debug)]
pub enum Work {
    Authority(Command),
    Save(Save),
    Remove {
        target: Target,
        request: RequestId,
        env_key: String,
        key: String,
    },
    Reveal {
        target: Target,
        request: RequestId,
        env_key: String,
        key: String,
        label: String,
    },
}
#[derive(Debug)]
pub enum Completion {
    Authority {
        target: Target,
        request: RequestId,
        result: Result<(), ServiceCode>,
    },
    Saved {
        target: Target,
        request: RequestId,
        result: Result<(), ServiceCode>,
    },
    Removed {
        target: Target,
        request: RequestId,
        result: Result<(), ServiceCode>,
    },
    Revealed {
        target: Target,
        request: RequestId,
        label: String,
        result: Result<PrivateValue, ServiceCode>,
    },
}
#[derive(Clone)]
pub struct Services {
    sender: SyncSender<Work>,
}
pub fn channel(capacity: usize) -> (Services, Receiver<Work>) {
    let (sender, receiver) = sync_channel(capacity);
    (Services { sender }, receiver)
}
impl AuthorityServices for Services {
    fn enqueue(&mut self, command: Command) -> Result<(), ServiceCode> {
        self.sender
            .try_send(Work::Authority(command))
            .map_err(|_| ServiceCode::Unavailable)
    }
}
impl ValuesService for Services {
    fn enqueue_save(&mut self, save: Save) -> Result<(), (ServiceCode, Save)> {
        match self.sender.try_send(Work::Save(save)) {
            Ok(()) => Ok(()),
            Err(
                TrySendError::Full(Work::Save(save)) | TrySendError::Disconnected(Work::Save(save)),
            ) => Err((ServiceCode::Unavailable, save)),
            Err(_) => unreachable!("try_send returns the same variant"),
        }
    }
    fn enqueue_remove(
        &mut self,
        target: Target,
        request: RequestId,
        env_key: String,
        key: String,
    ) -> Result<(), ServiceCode> {
        self.sender
            .try_send(Work::Remove {
                target,
                request,
                env_key,
                key,
            })
            .map_err(|_| ServiceCode::Unavailable)
    }
}
impl Services {
    /// Use Setup::reveal_key's validated output. The worker must revalidate
    /// generation/declaration before reading Keychain, and return only through
    /// Completion::Revealed -> explicit Prompt::Reveal. Never log the result.
    pub fn reveal(
        &mut self,
        target: Target,
        request: RequestId,
        env_key: String,
        key: String,
        label: String,
    ) -> Result<(), ServiceCode> {
        self.sender
            .try_send(Work::Reveal {
                target,
                request,
                env_key,
                key,
                label,
            })
            .map_err(|_| ServiceCode::Unavailable)
    }
}

/// Native setup adapter contract. Concrete rendering is blocked by the missing
/// Opus sheet metrics recorded in values::DESIGN_TODOS. The host must instantiate
/// NSTextField/NSSecureTextField, never render secrets through a v4/DOM tree.
/// `SheetView` is safe to inspect; field contents are fetched separately through
/// Setup::field_value and the platform secure text driver. Esc/Return and Tab
/// follow SheetView's grammar; no JS callbacks run on the main thread.
pub trait SetupHost {
    fn present_sheet(&mut self, view: SheetView);
    fn update_sheet(&mut self, view: SheetView);
    fn close_sheet(&mut self, target: &Target);
}
