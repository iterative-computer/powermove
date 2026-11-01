//! Dedicated Rust worker. Submit/poll never wait for I/O or JavaScript.
use anyhow::{Result, anyhow};
use std::{sync::mpsc, thread};

type Task<T> = Box<dyn FnOnce(&mut T) + Send>;
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
                // A failed init closes the channel, making queued receivers report failure.
                if let Ok(state) = &mut state {
                    while let Ok(task) = receiver.recv() {
                        task(state);
                    }
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
                let _ = sender.send(task(state));
            }))
            .map_err(|_| anyhow!("service worker unavailable or queue full"))?;
        Ok(Pending { receiver })
    }
}

impl<T> Pending<T> {
    pub fn try_take(&self) -> Result<Option<T>> {
        match self.receiver.try_recv() {
            Ok(value) => value.map(Some),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(anyhow!("service worker stopped before replying"))
            }
        }
    }
}
