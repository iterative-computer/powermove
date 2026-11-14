//! Reading a file in a folder macOS guards (Downloads, Desktop, Documents,
//! a removable volume). The first read there makes the system ask the
//! person; until they answer, and after a No, reads fail with "Operation not
//! permitted" (EPERM), never with anything that says the file is damaged.
//! An open waits for the answer here before the kernel reads the project,
//! so a Yes opens it the first time and a No is said in plain words.
use std::io::Read;
use std::path::Path;
use std::time::{Duration, Instant};

/// What reading `path` came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Access {
    /// Readable (or failing for a reason the open itself reports).
    Readable,
    /// macOS refused: the person said No, or did not answer in time.
    Refused,
}

/// How long an open waits for the person to answer the system's question.
pub const ANSWER: Duration = Duration::from_secs(60);
/// How often it reads again meanwhile.
const POLL: Duration = Duration::from_millis(250);
/// A read the system held this long waited for the person's answer: a
/// refusal after it is that answer, not a question still on screen.
const HELD: Duration = Duration::from_millis(500);

/// EPERM: the system's policy refused (EACCES is the file's own permissions).
pub fn refused(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(1)
}

/// The first bytes of a project file, or the first entry of a package folder.
pub fn read_start(path: &Path) -> std::io::Result<()> {
    if std::fs::metadata(path)?.is_dir() {
        std::fs::read_dir(path)?.next().transpose()?;
    } else {
        std::fs::File::open(path)?.read(&mut [0u8; 1])?;
    }
    Ok(())
}

/// Reads `path` with `read` until it succeeds, fails for another reason, or
/// macOS's refusal stands: answered (a read held for the question), or
/// unanswered for `answer`. Runs on a worker: a read can wait on the person.
pub fn wait_for_read(
    path: &Path,
    answer: Duration,
    read: impl Fn(&Path) -> std::io::Result<()>,
) -> Access {
    let start = Instant::now();
    loop {
        let began = Instant::now();
        match read(path) {
            Err(error) if refused(&error) => {
                if began.elapsed() >= HELD || start.elapsed() >= answer {
                    return Access::Refused;
                }
                std::thread::sleep(POLL);
            }
            // Read, missing, or the file's own permissions: the open says so.
            _ => return Access::Readable,
        }
    }
}

/// Whether a failure's text is the system refusing access (the kernel's
/// storage errors carry the OS message).
pub fn refused_text(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("operation not permitted") || t.contains("os error 1)")
}

/// Whether a failure's text came from the operating system (an errno), so
/// it can never mean the file's contents are damaged.
pub fn os_text(text: &str) -> bool {
    text.to_lowercase().contains("os error")
}

/// The sentence for a refused read of `name`, for an alert's title.
pub fn refused_title(name: &str) -> String {
    format!("Powermove isn’t allowed to open “{name}”.")
}
/// What the person can do about it.
pub const REFUSED_DETAIL: &str = "macOS refused access to the folder it’s in. To allow it, choose Apple menu ▸ System Settings ▸ Privacy & Security ▸ Files & Folders, turn on that folder for Powermove, then try again.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::io::ErrorKind;

    fn eperm() -> std::io::Error {
        std::io::Error::from_raw_os_error(1)
    }

    /// The system asks, the person says Yes a second later: the open goes on.
    #[test]
    fn an_open_waits_for_the_person_to_allow_access() {
        let reads = Cell::new(0);
        let access = wait_for_read(Path::new("p.pmv"), ANSWER, |_| {
            reads.set(reads.get() + 1);
            if reads.get() < 5 {
                Err(eperm())
            } else {
                Ok(())
            }
        });
        assert_eq!(access, Access::Readable);
        assert_eq!(reads.get(), 5);
    }

    /// No answer in time, or a read the question held that came back
    /// refused (the person's No): refused, never read further.
    #[test]
    fn a_refusal_is_refused() {
        let access = wait_for_read(Path::new("p.pmv"), Duration::from_millis(600), |_| {
            Err(eperm())
        });
        assert_eq!(access, Access::Refused);
        let reads = Cell::new(0);
        let access = wait_for_read(Path::new("p.pmv"), ANSWER, |_| {
            reads.set(reads.get() + 1);
            std::thread::sleep(HELD);
            Err(eperm())
        });
        assert_eq!((access, reads.get()), (Access::Refused, 1));
    }

    /// The file's own permissions, a missing file: the open reports those.
    #[test]
    fn other_failures_go_to_the_open() {
        for error in [ErrorKind::PermissionDenied, ErrorKind::NotFound] {
            let reads = Cell::new(0);
            let access = wait_for_read(Path::new("p.pmv"), ANSWER, |_| {
                reads.set(reads.get() + 1);
                Err(error.into())
            });
            assert_eq!((access, reads.get()), (Access::Readable, 1));
        }
    }

    #[test]
    fn os_refusals_are_told_apart_from_damage() {
        assert!(refused_text(
            "Cannot read /x/p.pmv: Operation not permitted (os error 1)"
        ));
        assert!(!refused_text("Permission denied (os error 13)"));
        assert!(os_text("Permission denied (os error 13)"));
        assert!(!os_text("Invalid project container: bad magic"));
    }
}
