//! Dedicated Rust worker. Submit/poll never wait for I/O or JavaScript.
use crate::error::Result;
use crate::error::{ServiceError, ServiceErrorCode};
use std::{sync::mpsc, thread};

type Task<T> = Box<dyn FnOnce(&mut Result<T>) + Send>;
pub struct ServiceWorker<T> {
    sender: mpsc::SyncSender<Task<T>>,
}
pub struct Pending<T> {
    receiver: mpsc::Receiver<Result<T>>,
}

impl<T: Send + 'static> ServiceWorker<T> {
    /// Initialization runs off-main too. Initialization errors reach every submitted task.
    pub fn start(init: impl FnOnce() -> Result<T> + Send + 'static) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Task<T>>(64);
        thread::Builder::new()
            .name("pm-services".into())
            .spawn(move || {
                let mut state = init();
                while let Ok(task) = receiver.recv() {
                    task(&mut state);
                }
            })?;
        Ok(Self { sender })
    }

    pub fn submit<R: Send + 'static>(
        &self,
        task: impl FnOnce(&mut T) -> Result<R> + Send + 'static,
    ) -> Result<Pending<R>> {
        let (sender, receiver) = mpsc::channel();
        self.sender
            .try_send(Box::new(move |state| {
                let result = match state {
                    Ok(state) => task(state),
                    Err(error) => Err(ServiceError::message(error.code, error.to_string())),
                };
                let _ = sender.send(result);
            }))
            .map_err(|_| {
                ServiceError::message(
                    ServiceErrorCode::Unavailable,
                    "service worker unavailable or queue full",
                )
            })?;
        Ok(Pending { receiver })
    }
}

impl<T> Pending<T> {
    pub fn try_take(&self) -> Result<Option<T>> {
        match self.receiver.try_recv() {
            Ok(value) => value.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => Err(ServiceError::message(
                ServiceErrorCode::Unavailable,
                "service worker stopped before replying",
            )),
        }
    }
}
