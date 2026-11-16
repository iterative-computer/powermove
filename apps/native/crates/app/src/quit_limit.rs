//! How long a quit may go without progress, and what happens then.
//!
//! A quit the person chose is never ended by a timer. Once nobody is being
//! asked anything, each stage of it (closing the windows, writing the
//! library) has a bound; a stage that makes no progress in its
//! time is reported to the app ([`stalls`]), which asks the person whether to
//! keep waiting or quit now. Only their Quit Now ends the process, and only
//! once every acknowledged edit is durable in its journal ([`secure`]): a
//! flush that fails or does not finish is said, and the process stays.
//!
//! A quit signal (SIGTERM: a logout, a shutdown, a script) has nobody to ask
//! and the system ends the process soon anyway: the thread that heard it
//! bounds it from that moment ([`signalled`]), a quit already under way and
//! stuck on the main thread included. That path never blocks (no stderr),
//! tries the journal flush with a bound, stops the engine jobs this app
//! started, and exits.
use std::sync::{
    Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::Duration;

/// Closing the windows: a window's close that takes longer has stalled.
pub const WINDOWS: Duration = Duration::from_secs(5);
/// Writing and releasing the library (a quit compacts library projects).
pub const LIBRARY: Duration = Duration::from_secs(20);
/// A quit signal, from its arrival.
pub const SIGNAL: Duration = Duration::from_secs(3);
/// How long making the journals durable may take before Quit Now says it
/// could not.
pub const SECURE: Duration = Duration::from_secs(5);
/// The journal flush a signal's end attempts, within its limit.
const SIGNAL_FLUSH: Duration = Duration::from_secs(1);

/// The chosen quit's current stage; a new stage or a cancel supersedes it.
static STAGE: AtomicU64 = AtomicU64::new(0);
/// A quit signal's limit is armed once and never cancelled.
static SIGNALLED: AtomicBool = AtomicBool::new(false);
/// Whose journals are made durable before the process ends.
static KERNEL: OnceLock<Mutex<Option<pm_kernel::KernelHandle>>> = OnceLock::new();
/// Where stalled stages are reported (the app's quit coordinator).
static STALLS: OnceLock<async_channel::Sender<Stall>> = OnceLock::new();

/// A chosen quit's stage that made no progress in its bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stall {
    pub stage: &'static str,
    id: u64,
}
impl Stall {
    /// Whether it is still the quit's current stage (nothing moved since).
    pub fn current(&self) -> bool {
        current(self.id)
    }
}

/// The kernel whose journals are made durable (the app's one kernel).
pub fn set_kernel(kernel: pm_kernel::KernelHandle) {
    *KERNEL.get_or_init(Default::default).lock().unwrap() = Some(kernel);
}

/// The stalls of chosen quits, for the app to ask about.
pub fn stalls() -> async_channel::Receiver<Stall> {
    let (tx, rx) = async_channel::unbounded();
    let _ = STALLS.set(tx);
    rx
}

/// Bounds the chosen quit's stage `name` to `limit` from now, replacing the
/// bound of the stage before it. Expiry only reports the stall.
pub fn stage(name: &'static str, limit: Duration) {
    let id = begin();
    let stall = Stall { stage: name, id };
    watch(limit, move || current(id), move || report(stall));
}

/// The chosen quit was cancelled, or someone is being asked something: no
/// bound runs until the next stage.
pub fn cancel() {
    STAGE.fetch_add(1, Ordering::AcqRel);
}

/// A new stage, superseding every stage before it.
fn begin() -> u64 {
    STAGE.fetch_add(1, Ordering::AcqRel) + 1
}
/// Whether `stage` is still the quit's current one.
fn current(stage: u64) -> bool {
    STAGE.load(Ordering::Acquire) == stage
}

fn report(stall: Stall) {
    crate::diag::line(serde_json::json!({"quit":"stalled","stage":stall.stage}));
    if let Some(tx) = STALLS.get() {
        let _ = tx.try_send(stall);
    }
}

/// A quit signal arrived (on the thread that heard it): the process ends
/// within [`SIGNAL`] whatever else happens, a quit in progress included.
pub fn signalled() {
    if !SIGNALLED.swap(true, Ordering::AcqRel) {
        let kernel = kernel();
        arm_signal(SIGNAL, move || kernel.map_or(Ok(()), |k| k.sync_journals()));
    }
}
/// After `limit`, the signal's end: nobody to ask, and nothing on this path
/// may block (a stalled stderr pipe included). The journals get a bounded
/// flush, the engine jobs this app started are stopped, and the process
/// exits.
fn arm_signal(limit: Duration, flush: impl FnOnce() -> std::io::Result<()> + Send + 'static) {
    watch(
        limit,
        || true,
        move || {
            crate::diag::line(serde_json::json!({"quit":"limit","stage":"signal"}));
            let _ = flush_within(flush, SIGNAL_FLUSH);
            stop_jobs_and_exit()
        },
    );
}

fn kernel() -> Option<pm_kernel::KernelHandle> {
    KERNEL.get().and_then(|k| k.lock().ok()?.clone())
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

/// Makes every acknowledged edit durable in its journal, waiting at most
/// `bound`: Quit Now ends the process only on `Ok`.
pub fn secure(bound: Duration) -> Result<(), String> {
    match kernel() {
        Some(kernel) => flush_within(move || kernel.sync_journals(), bound),
        None => Ok(()),
    }
}

/// Runs `flush` on a thread of its own and waits at most `bound` for it.
fn flush_within(
    flush: impl FnOnce() -> std::io::Result<()> + Send + 'static,
    bound: Duration,
) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("pm-quit-journals".into())
        .spawn(move || {
            let _ = tx.send(flush());
        })
        .map_err(|e| format!("the journals could not be flushed: {e}"))?;
    match rx.recv_timeout(bound) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("the journals could not be written to disk: {e}")),
        Err(_) => Err(format!(
            "the disk did not finish writing the journals within {} seconds",
            bound.as_secs()
        )),
    }
}

/// `exit` runs no drops: stops the engine jobs this app started (its
/// process group, when it leads one, as a launched app does), then exits.
pub fn stop_jobs_and_exit() -> ! {
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
    //! Each case runs in a child process (this test binary, run again with
    //! `PM_QUIT_CHILD` naming the case), so ending or not ending is real.
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::Instant;

    fn child(case: &str, stderr_blocked: bool) -> std::process::Child {
        Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "quit_limit::tests::child_case",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("PM_QUIT_CHILD", case)
            // Diagnostics on: the end's own line must not block either.
            .env("PM_NATIVE_MAIN_TRACE", "1")
            .stdout(Stdio::piped())
            .stderr(if stderr_blocked {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .spawn()
            .unwrap()
    }
    fn ended_within(child: &mut std::process::Child, within: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < within {
            if child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
    fn output_of(mut child: std::process::Child) -> String {
        use std::io::Read;
        let mut out = String::new();
        let _ = child.kill();
        let _ = child.stdout.take().unwrap().read_to_string(&mut out);
        let _ = child.wait();
        out
    }

    /// The children. Not a test on its own: it does nothing without
    /// `PM_QUIT_CHILD`.
    #[test]
    fn child_case() {
        let Ok(case) = std::env::var("PM_QUIT_CHILD") else {
            return;
        };
        match case.as_str() {
            // A chosen quit's stage stalls: it is reported, and nothing ends.
            "stalled" => {
                let rx = stalls();
                stage("windows", Duration::from_millis(100));
                let stall = rx.recv_blocking().unwrap();
                println!("asked {} current={}", stall.stage, stall.current());
                std::thread::sleep(Duration::from_secs(30));
            }
            // Quit Now with a flush that fails, or never finishes: it says
            // so, and the process stays.
            "quit-now" => {
                let failed = flush_within(|| Err(std::io::Error::other("disk gone")), SECURE);
                let hung = flush_within(
                    || {
                        std::thread::sleep(Duration::from_secs(60));
                        Ok(())
                    },
                    Duration::from_millis(200),
                );
                println!("failed={failed:?} hung={hung:?}");
                if failed.is_ok() || hung.is_ok() {
                    stop_jobs_and_exit();
                }
                std::thread::sleep(Duration::from_secs(30));
            }
            // A signal quit while stderr is held and its pipe is full.
            "signal" => {
                let _held = std::thread::spawn(|| {
                    let _lock = std::io::stderr().lock();
                    std::thread::sleep(Duration::from_secs(60));
                });
                std::thread::sleep(Duration::from_millis(50));
                arm_signal(Duration::from_millis(100), || {
                    std::thread::sleep(Duration::from_secs(60));
                    Ok(())
                });
                std::thread::sleep(Duration::from_secs(30));
            }
            other => panic!("unknown case {other}"),
        }
    }

    /// A chosen quit that stops making progress asks; it never exits.
    #[test]
    fn a_stalled_chosen_quit_asks_and_does_not_exit() {
        let mut c = child("stalled", false);
        assert!(
            !ended_within(&mut c, Duration::from_secs(3)),
            "a stalled chosen quit ended"
        );
        let out = output_of(c);
        assert!(out.contains("asked windows current=true"), "{out}");
    }

    /// Quit Now whose journal flush fails or does not finish says so and
    /// keeps the process.
    #[test]
    fn quit_now_with_a_failing_flush_does_not_exit() {
        let mut c = child("quit-now", false);
        assert!(
            !ended_within(&mut c, Duration::from_secs(3)),
            "Quit Now ended without a durable flush"
        );
        let out = output_of(c);
        assert!(
            out.contains("disk gone") && out.contains("did not finish"),
            "{out}"
        );
    }

    /// A signal quit ends within its limit even with stderr blocked and a
    /// journal flush that never finishes.
    #[test]
    fn a_signal_quit_exits_within_its_limit_with_stderr_blocked() {
        let mut c = child("signal", true);
        assert!(
            ended_within(&mut c, Duration::from_secs(3)),
            "the signal quit did not end"
        );
    }

    /// A new stage or a cancel supersedes a stage's bound.
    #[test]
    fn only_the_current_stage_is_reported() {
        let first = begin();
        assert!(current(first));
        let second = begin();
        assert!(!current(first) && current(second));
        cancel();
        assert!(!current(second));
    }
}
