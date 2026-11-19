//! Submit interaction work before yielding the UI thread. Its continuation
//! still applies results through GPUI after the current event has returned.
use gpui::{BackgroundExecutor, Task};
use std::{
    future::Future,
    pin::Pin,
    sync::OnceLock,
    task::{Context as TaskContext, Poll, Waker},
};

enum Work<F: Future> {
    Started(Task<F::Output>),
    // Keep large timeline futures out of the UI continuation.
    Deferred(Pin<Box<F>>),
    Awaiting(Pin<Box<F>>),
    Ready(F::Output),
}

fn eager() -> bool {
    // Same-binary measurement control: 0 restores the old foreground turn
    // before background work is even submitted.
    static EAGER: OnceLock<bool> = OnceLock::new();
    *EAGER.get_or_init(|| std::env::var_os("PM_DRAG_EAGER_WORK").is_none_or(|value| value != "0"))
}

async fn finish<F>(executor: BackgroundExecutor, work: Work<F>) -> F::Output
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    match work {
        Work::Started(task) => task.await,
        Work::Deferred(work) => executor.spawn(work).await,
        Work::Awaiting(work) => work.await,
        Work::Ready(result) => result,
    }
}

pub(crate) fn start<F>(
    executor: &BackgroundExecutor,
    work: F,
) -> impl Future<Output = F::Output> + 'static + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    start_when(executor, true, work)
}

pub(crate) fn start_when<F>(
    executor: &BackgroundExecutor,
    interactive: bool,
    work: F,
) -> impl Future<Output = F::Output> + 'static + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let executor = executor.clone();
    let work = if interactive && eager() {
        Work::Started(executor.spawn(work))
    } else {
        Work::Deferred(Box::pin(work))
    };
    finish(executor, work)
}

pub(crate) fn defer<F>(
    executor: &BackgroundExecutor,
    work: F,
) -> impl Future<Output = F::Output> + 'static + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    start_when(executor, false, work)
}

/// Only for nonblocking request submission, like Kernel::call. Compute and
/// media work must use start, so it stays entirely on the background executor.
pub(crate) fn start_request<F>(
    executor: &BackgroundExecutor,
    work: F,
) -> impl Future<Output = F::Output> + 'static + use<F>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let executor = executor.clone();
    let mut work = Box::pin(work);
    let work = if eager() {
        // Poll only the request future, never the UI continuation. This sends
        // the kernel message now, as Controller::call_at does. The UI task
        // awaits the nonblocking answer directly, with no background relay.
        // Its initial poll registers the real waker (or reads an early reply).
        match work
            .as_mut()
            .poll(&mut TaskContext::from_waker(Waker::noop()))
        {
            Poll::Ready(result) => Work::Ready(result),
            Poll::Pending => Work::Awaiting(work),
        }
    } else {
        Work::Deferred(work)
    };
    finish(executor, work)
}

// Hoist just the initial submission. Keeping completion in Context::spawn
// preserves the weak entity, App borrowing and existing error handling.
macro_rules! spawn {
    ($cx:ident, $start:ident [$($arg:expr),*], async move |$this:ident, $async_cx:ident| {
        let $result:pat = $worker_cx:ident.background_executor().spawn($work:expr $(,)?).await;
        $($rest:tt)*
    } $(,)?) => {{
        let work = $crate::interaction_work::$start($cx.background_executor(), $($arg,)* $work);
        $cx.spawn(async move |$this, $async_cx| {
            let $result = work.await;
            $($rest)*
        })
    }};
    ($cx:ident, async $($body:tt)*) => {
        $crate::interaction_work::spawn!($cx, start [], async $($body)*)
    };
}
pub(crate) use spawn;

macro_rules! submit {
    ($cx:ident, $($body:tt)*) => {
        $crate::interaction_work::spawn!($cx, start_request [], $($body)*)
    };
}
pub(crate) use submit;

// Script events retain their UI-turn ordering and the realm's backpressure.
// Starting them earlier is unrelated to native gesture work and can overfill
// its bounded command queue during a burst of native layout changes.
macro_rules! ordered {
    ($cx:ident, $($body:tt)*) => {
        $crate::interaction_work::spawn!($cx, defer [], $($body)*)
    };
}
pub(crate) use ordered;

// Sync and persistence are maintenance jobs. Preserve their original turn;
// only native pointer gestures need the earlier worker submission. The caller
// supplies its job type so the enum can remain private to the timeline.
macro_rules! timeline {
    ($cx:ident, $job:ident, $kind:ident, $($body:tt)*) => {
        $crate::interaction_work::spawn!($cx, start_when [matches!(
            &$job,
            $kind::SelectLayer(..) | $kind::SelectKey { .. } | $kind::Begin(..)
                | $kind::Sample(..) | $kind::Finish(..)
        )], $($body)*)
    };
}
pub(crate) use timeline;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    #[gpui::test]
    fn background_work_starts_before_foreground_poll(cx: &mut gpui::TestAppContext) {
        let started = Arc::new(AtomicBool::new(false));
        let observed = started.clone();
        let completion = start(&cx.background_executor, async move {
            observed.store(true, Ordering::SeqCst);
        });
        cx.background_executor.run_until_parked();
        assert!(
            started.load(Ordering::SeqCst),
            "interaction work waited for a foreground poll"
        );
        drop(completion);
    }

    #[gpui::test]
    fn early_reply_needs_no_background_relay(cx: &mut gpui::TestAppContext) {
        let ready = Arc::new(AtomicBool::new(false));
        let observed = ready.clone();
        let completion = start_request(
            &cx.background_executor,
            std::future::poll_fn(move |_| {
                if observed.load(Ordering::SeqCst) {
                    Poll::Ready(7)
                } else {
                    Poll::Pending
                }
            }),
        );
        // A reply can arrive after submission but before the UI task first
        // polls its completion. It must not be stranded behind another worker.
        ready.store(true, Ordering::SeqCst);
        let mut completion = Box::pin(completion);
        assert!(
            matches!(
                completion
                    .as_mut()
                    .poll(&mut TaskContext::from_waker(Waker::noop())),
                Poll::Ready(7)
            ),
            "reply waited for a background executor"
        );
    }

    #[gpui::test]
    fn request_is_submitted_before_either_executor_polls(cx: &mut gpui::TestAppContext) {
        let submitted = Arc::new(AtomicBool::new(false));
        let observed = submitted.clone();
        let completion = start_request(&cx.background_executor, async move {
            observed.store(true, Ordering::SeqCst);
            std::future::pending::<()>().await;
        });
        assert!(
            submitted.load(Ordering::SeqCst),
            "request waited for an executor poll"
        );
        drop(completion);
        cx.background_executor.run_until_parked();
    }

    #[gpui::test]
    fn non_interaction_work_keeps_foreground_turn(cx: &mut gpui::TestAppContext) {
        let started = Arc::new(AtomicBool::new(false));
        let observed = started.clone();
        let completion = start_when(&cx.background_executor, false, async move {
            observed.store(true, Ordering::SeqCst);
        });
        cx.background_executor.run_until_parked();
        assert!(
            !started.load(Ordering::SeqCst),
            "maintenance ran before its foreground turn"
        );
        let mut completion = Box::pin(completion);
        assert!(matches!(
            completion
                .as_mut()
                .poll(&mut TaskContext::from_waker(Waker::noop())),
            Poll::Pending
        ));
        cx.background_executor.run_until_parked();
        assert!(started.load(Ordering::SeqCst));
        assert!(matches!(
            completion
                .as_mut()
                .poll(&mut TaskContext::from_waker(Waker::noop())),
            Poll::Ready(())
        ));
    }
}
