//! Owned process groups. Every component the runner starts (the host, a
//! harness CLI and whatever they spawn) runs in its own process group, so a
//! deadline stops all of it at once and nothing is left holding a pipe.
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

/// Start `cmd` as the leader of a new process group.
pub fn spawn_group(cmd: &mut Command) -> std::io::Result<Child> {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0).spawn()
}

/// Whether any process of the group is still alive (zombies included until
/// they are reaped).
pub fn group_alive(pgid: u32) -> bool {
    // SAFETY: signal 0 only checks for existence; pgid is a plain integer.
    unsafe { libc::killpg(pgid as libc::pid_t, 0) == 0 }
}

fn kill_group(pgid: u32) {
    // SAFETY: SIGKILL to a process group the runner created.
    unsafe {
        libc::killpg(pgid as libc::pid_t, libc::SIGKILL);
    }
}

/// What stopping a group found.
#[derive(Debug)]
pub struct Stopped {
    pub status: Option<ExitStatus>,
    /// Processes of the group still alive after the grace period (they
    /// escaped the kill, e.g. by leaving the group).
    pub stragglers: bool,
}

/// Kill the whole group, reap the leader and wait until no member is left.
/// Members whose parent died are reparented to launchd, which reaps them.
pub fn stop_group(child: &mut Child, grace: Duration) -> Stopped {
    let pgid = child.id();
    kill_group(pgid);
    let status = child.wait().ok();
    let end = Instant::now() + grace;
    while group_alive(pgid) && Instant::now() < end {
        kill_group(pgid);
        std::thread::sleep(Duration::from_millis(10));
    }
    Stopped {
        status,
        stragglers: group_alive(pgid),
    }
}

/// Wait for the leader to exit by itself until `end`; None at the deadline.
pub fn wait_until(child: &mut Child, end: Instant) -> Option<ExitStatus> {
    loop {
        if let Ok(Some(s)) = child.try_wait() {
            return Some(s);
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
