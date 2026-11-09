//! A realm's answer, waited for on a worker thread: woken when the realm
//! completes it, never by polling.
use std::{
    future::Future,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
    thread::{self, Thread},
    time::Instant,
};

struct Unpark(Thread);
impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
/// The ticket's result, or `None` if it isn't in by `deadline`.
pub fn answer<T>(
    ticket: pm_runtime_v8::Ticket<T>,
    deadline: Instant,
) -> Option<pm_runtime_v8::Result<T>> {
    let waker = Waker::from(Arc::new(Unpark(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut ticket = pin!(ticket);
    loop {
        if let Poll::Ready(result) = ticket.as_mut().poll(&mut cx) {
            return Some(result);
        }
        thread::park_timeout(deadline.checked_duration_since(Instant::now())?);
    }
}
