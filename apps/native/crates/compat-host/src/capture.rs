//! Worker-polled completions for real page inspection and WebKit snapshots.
//! Creating a ticket never establishes paint or a successful capture.
use crate::{Error, Result};
#[cfg(any(target_os = "macos", test))]
use std::sync::mpsc::{self, SyncSender};
use std::sync::mpsc::{Receiver, TryRecvError};

pub struct CaptureTicket<T>(Receiver<Result<T>>);
impl<T> CaptureTicket<T> {
    pub fn try_take(&self) -> Option<Result<T>> {
        match self.0.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                Some(Err(Error::Unsupported("capture completion disconnected")))
            }
        }
    }
}
#[cfg(any(target_os = "macos", test))]
pub(crate) fn ticket<T>() -> (SyncSender<Result<T>>, CaptureTicket<T>) {
    let (sender, receiver) = mpsc::sync_channel(1);
    (sender, CaptureTicket(receiver))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn page_completion_stays_pending_until_the_platform_returns_and_failure_is_visible() {
        let (sender, ticket) = ticket::<Vec<u8>>();
        assert_eq!(ticket.try_take(), None);
        sender.try_send(Err(Error::StaleGeneration)).unwrap();
        assert_eq!(ticket.try_take(), Some(Err(Error::StaleGeneration)));
        drop(sender);
        assert!(ticket.try_take().unwrap().is_err());
    }
}
