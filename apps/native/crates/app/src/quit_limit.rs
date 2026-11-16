//! Every quit ends within a bound, whatever the main thread is doing.
//!
//! A quit signal (SIGTERM: a logout, a shutdown, a script) is bounded from
//! the moment it arrives, by the thread that hears it: a quit stuck on the
//! main thread (a window's close waiting on a busy engine or kernel) can no
//! longer keep it from ending. A chosen quit is bounded stage by stage once
//! nobody is being asked anything: from the moment its last window settled
//! (every question answered, every chosen save done), then while it writes
//! the project library, then while the app tears down. A stage that has not
//! finished in its time ends the process the way Quit Now does: every edit
//! the kernel acknowledged is made durable in its journal, the engine jobs
//! this app started are stopped, and the next open offers the unsaved work
//! back. Nothing is saved that the person did not choose to save.
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

/// From the last window settling to the library write starting.
pub const SETTLED: Duration = Duration::from_secs(5);
/// The library write and release (a quit compacts library projects).
pub const LIBRARY: Duration = Duration::from_secs(20);
/// From the library released to the process gone (the app's teardown).
pub const TEARDOWN: Duration = Duration::from_secs(3);
/// A quit signal, from its arrival.
pub const SIGNAL: Duration = Duration::from_secs(3);

/// The chosen quit's current stage; a new stage or a cancel supersedes it.
static STAGE: AtomicU64 = AtomicU64::new(0);
/// A quit signal's limit is armed once and never cancelled.
static SIGNALLED: AtomicBool = AtomicBool::new(false);
/// Whose journals the limit makes durable before ending.
static KERNEL: OnceLock<Mutex<Option<pm_kernel::KernelHandle>>> = OnceLock::new();

/// The kernel whose journals an ended quit flushes (the app's one kernel).
pub fn set_kernel(kernel: pm_kernel::KernelHandle) {
    *KERNEL.get_or_init(Default::default).lock().unwrap() = Some(kernel);
}

/// Bounds the chosen quit's stage `name` to `limit` from now, replacing the
/// bound of the stage before it.
pub fn stage(name: &'static str, limit: Duration) {
    let mine = begin();
    // A unit test running a quit flow keeps its process.
    if cfg!(test) {
        return;
    }
    watch(limit, move || current(mine), move || end(name));
}
/// A new stage, superseding every stage before it.
fn begin() -> u64 {
    STAGE.fetch_add(1, Ordering::AcqRel) + 1
}
/// Whether `stage` is still the quit's current one.
fn current(stage: u64) -> bool {
    STAGE.load(Ordering::Acquire) == stage
}

/// The chosen quit was cancelled (a Cancel, a failed save): no bound stays.
pub fn cancel() {
    STAGE.fetch_add(1, Ordering::AcqRel);
}

/// A quit signal arrived (on the thread that heard it): the process ends
/// within [`SIGNAL`] whatever else happens, a quit in progress included.
pub fn signalled() {
    if !SIGNALLED.swap(true, Ordering::AcqRel) && !cfg!(test) {
        watch(SIGNAL, || true, || end("signal"));
    }
}

/// After `limit`, on a thread of its own, `then` runs if `still` holds.
fn watch(
    limit: Duration,
    still: impl FnOnce() -> bool + Send + 'static,
    then: impl FnOnce() + Send + 'static,
) {
    let _ = std::thread::Builder::new()
        .name("pm-quit-limit".into())
        .spawn(move || {
            std::thread::sleep(limit);
            if still() {
                then();
            }
        });
}

/// Ends the process as Quit Now does. Runs off the main thread, so it never
/// waits on whatever holds the quit.
fn end(stage: &'static str) -> ! {
    crate::diag::line(serde_json::json!({"quit":"limit","stage":stage}));
    eprintln!("Quit took too long ({stage}); ending. Unsaved work is kept for recovery.");
    let kernel = KERNEL.get().and_then(|k| k.lock().ok()?.clone());
    if let Some(kernel) = kernel {
        // Bounded: a disk that never answers does not hold the end either.
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("pm-quit-journals".into())
            .spawn(move || {
                let _ = tx.send(kernel.sync_journals());
            });
        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(Err(e)) => eprintln!("The journals could not be flushed to disk: {e}"),
            Err(_) => eprintln!("The journals did not flush in time."),
            Ok(Ok(())) => {}
        }
    }
    // `exit` runs no drops: stop the engine jobs this app started (its
    // process group, when it leads one, as a launched app does).
    // SAFETY: plain process calls; this process ignores the SIGTERM it sends
    // itself (blocked in every thread, waited for by one).
    unsafe {
        if libc::getpgrp() == libc::getpid() {
            libc::killpg(libc::getpgrp(), libc::SIGTERM);
        }
    }
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;
    /// A new stage or a cancel supersedes a stage's bound; only the current
    /// stage's watch ends the quit, and it does so without the main thread.
    #[test]
    fn only_the_current_stage_ends_the_quit() {
        let first = begin();
        assert!(current(first));
        let second = begin();
        assert!(!current(first) && current(second));
        cancel();
        assert!(!current(second));
        let (tx, rx) = std::sync::mpsc::channel();
        let (late, now) = (begin(), tx.clone());
        watch(
            Duration::from_millis(30),
            move || current(late),
            move || {
                let _ = now.send("late");
            },
        );
        let live = begin();
        watch(
            Duration::from_millis(30),
            move || current(live),
            move || {
                let _ = tx.send("live");
            },
        );
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok("live"));
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        cancel();
    }
}
