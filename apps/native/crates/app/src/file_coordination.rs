//! Coordinated writes of project files through the system's file
//! coordination (`NSFileCoordinator`), for the kernel's saves and the moves
//! of a recovery ([`pm_kernel::project_io::WriteCoordinator`] says what
//! that covers and what it does not).
//!
//! The system's call blocks until every other coordinated reader and writer
//! of the file is done and every program presenting it has answered. That
//! can be never (a program that hangs). So the call is made on a thread of
//! its own and the save waits for its turn only so long; then it cancels
//! the request and goes ahead without one.

use std::path::Path;
use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2_foundation::{
    NSError, NSFileCoordinator, NSFileCoordinatorWritingOptions, NSString, NSURL,
};
use pm_kernel::project_io::{SharedWriteCoordinator, WriteCoordinator};

/// How long a save waits for its turn before it goes ahead without one.
const WAIT: Duration = Duration::from_secs(5);

/// The system's file coordination, for the kernel.
pub fn system() -> SharedWriteCoordinator {
    SharedWriteCoordinator(Arc::new(System { wait: WAIT }))
}

struct System {
    wait: Duration,
}

#[derive(Clone, Copy, PartialEq)]
enum Turn {
    Asking,
    Granted,
    GaveUp,
    Done,
}

type Shared = Arc<(Mutex<Turn>, Condvar)>;

/// The coordinator object on its way to the thread that asks. The asking
/// thread is the only one that uses it, apart from `cancel`, which the
/// system allows from any thread.
struct Asking(Retained<NSFileCoordinator>);
// SAFETY: see above; the object is not used from two threads at once
// except for `cancel`.
unsafe impl Send for Asking {}

/// Lets the write go when the work is over, however it ends.
struct Release(Shared);
impl Drop for Release {
    fn drop(&mut self) {
        let (turn, changed) = &*self.0;
        *turn.lock().unwrap_or_else(|e| e.into_inner()) = Turn::Done;
        changed.notify_all();
    }
}

impl WriteCoordinator for System {
    fn coordinate(&self, path: &Path, work: &mut dyn FnMut(bool)) {
        let shared: Shared = Arc::new((Mutex::new(Turn::Asking), Condvar::new()));
        let coordinator = NSFileCoordinator::new();
        let asking = Asking(coordinator.clone());
        let (theirs, file) = (shared.clone(), path.to_owned());
        let asked = std::thread::Builder::new()
            .name("pm-file-coordination".into())
            .spawn(move || {
                let asking = asking;
                let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
                let held = theirs.clone();
                let accessor = RcBlock::new(move |_: NonNull<NSURL>| {
                    let (turn, changed) = &*held;
                    let mut turn = turn.lock().unwrap_or_else(|e| e.into_inner());
                    // Granted after the save stopped waiting: not taken.
                    if *turn != Turn::Asking {
                        return;
                    }
                    *turn = Turn::Granted;
                    changed.notify_all();
                    // The write is held until the work is over.
                    while *turn != Turn::Done {
                        turn = changed.wait(turn).unwrap_or_else(|e| e.into_inner());
                    }
                });
                let mut error: Option<Retained<NSError>> = None;
                // Merging: a program that presents the file and holds
                // unsaved changes to it is asked to write them first.
                asking
                    .0
                    .coordinateWritingItemAtURL_options_error_byAccessor(
                        &url,
                        NSFileCoordinatorWritingOptions::ForMerging,
                        Some(&mut error),
                        &accessor,
                    );
                // Back without the accessor having run: refused or cancelled.
                let (turn, changed) = &*theirs;
                let mut turn = turn.lock().unwrap_or_else(|e| e.into_inner());
                if *turn == Turn::Asking {
                    *turn = Turn::GaveUp;
                    changed.notify_all();
                }
            });
        let (turn, changed) = &*shared;
        let granted = asked.is_ok() && {
            let (mut now, _) = changed
                .wait_timeout_while(
                    turn.lock().unwrap_or_else(|e| e.into_inner()),
                    self.wait,
                    |turn| *turn == Turn::Asking,
                )
                .unwrap_or_else(|e| e.into_inner());
            if *now == Turn::Asking {
                *now = Turn::GaveUp;
            }
            *now == Turn::Granted
        };
        if !granted {
            coordinator.cancel();
        }
        {
            let _release = Release(shared.clone());
            work(granted);
        }
        if granted && let Ok(asked) = asked {
            let _ = asked.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Instant;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pm-file-coordination-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("cut.pmv");
        std::fs::write(&file, b"project").unwrap();
        file
    }

    /// Another writer in a coordinated write of `file` for `hold`. Returns
    /// once it has the write; the receiver says when it let go.
    fn another_writer(file: &Path, hold: Duration) -> mpsc::Receiver<Instant> {
        let (has_it, wait_for_it) = mpsc::channel();
        let (let_go, released) = mpsc::channel();
        let file = file.to_owned();
        std::thread::spawn(move || {
            let url = NSURL::fileURLWithPath(&NSString::from_str(&file.to_string_lossy()));
            let accessor = RcBlock::new(move |_: NonNull<NSURL>| {
                has_it.send(()).unwrap();
                std::thread::sleep(hold);
                let_go.send(Instant::now()).unwrap();
            });
            NSFileCoordinator::new().coordinateWritingItemAtURL_options_error_byAccessor(
                &url,
                NSFileCoordinatorWritingOptions::ForMerging,
                None,
                &accessor,
            );
        });
        wait_for_it.recv().unwrap();
        released
    }

    #[test]
    fn a_save_waits_for_another_coordinated_writer_and_then_has_its_turn() {
        let file = scratch("waits");
        let released = another_writer(&file, Duration::from_millis(400));
        let mut ran = None;
        System {
            wait: Duration::from_secs(10),
        }
        .coordinate(&file, &mut |granted| ran = Some((granted, Instant::now())));
        let (granted, at) = ran.expect("the work ran");
        assert!(granted, "the write was granted once the other let go");
        assert!(at >= released.recv().unwrap(), "and not before");
        // And the write is given up afterwards: the next one is granted.
        let mut again = None;
        System {
            wait: Duration::from_secs(10),
        }
        .coordinate(&file, &mut |granted| again = Some(granted));
        assert_eq!(again, Some(true));
    }

    #[test]
    fn a_writer_that_does_not_let_go_is_waited_for_only_so_long() {
        let file = scratch("bounded");
        let released = another_writer(&file, Duration::from_secs(3));
        let asked = Instant::now();
        let mut ran = None;
        System {
            wait: Duration::from_millis(300),
        }
        .coordinate(&file, &mut |granted| ran = Some((granted, Instant::now())));
        let (granted, at) = ran.expect("the work ran all the same");
        assert!(!granted, "without a coordinated write");
        assert!(at - asked < Duration::from_secs(2), "{:?}", at - asked);
        assert!(
            at < released.recv().unwrap(),
            "while the other still held it"
        );
    }

    #[test]
    fn a_file_nobody_else_touches_is_granted_at_once() {
        let file = scratch("free");
        let asked = Instant::now();
        let mut ran = None;
        system()
            .0
            .coordinate(&file, &mut |granted| ran = Some(granted));
        assert_eq!(ran, Some(true));
        assert!(asked.elapsed() < Duration::from_secs(2));
    }
}
