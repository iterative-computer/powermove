//! Real native Claude Code registration adapter. Run on the Settings worker.
//! Uses argv directly, bounded nonblocking pipes, no shell or credential copying.
use crate::{Error, Result, registration::ClaudeCli};
use std::{
    collections::BTreeMap,
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const OUTPUT_LIMIT: usize = 1024 * 1024;
pub struct NativeClaudeCli {
    executable: PathBuf,
    cwd: PathBuf,
    environment: BTreeMap<String, String>,
    timeout: Duration,
}
impl NativeClaudeCli {
    /// The application supplies its discovered native CLI and explicit user
    /// configuration environment. This constructor performs no registration.
    pub fn new(executable: PathBuf, cwd: PathBuf, environment: BTreeMap<String, String>) -> Self {
        Self {
            executable,
            cwd,
            environment,
            timeout: Duration::from_secs(60),
        }
    }
    /// Hosts/tests may shorten the deadline, never extend the shipped ceiling.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout.min(Duration::from_secs(60));
        self
    }
}
fn nonblocking(pipe: &impl AsRawFd) -> io::Result<()> {
    // SAFETY: the borrowed fd is live; F_GETFL/F_SETFL change only its status flags.
    let flags = unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_GETFL) };
    if flags == -1
        || unsafe { libc::fcntl(pipe.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
fn drain(pipe: &mut impl Read, output: &mut Vec<u8>) -> io::Result<bool> {
    let mut buffer = [0; 8192];
    // Bound each poll even if a broken CLI writes continuously.
    for _ in 0..32 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if output.len() + n > OUTPUT_LIMIT {
                    return Err(io::Error::other(
                        "Claude Code registration output exceeded 1 MiB",
                    ));
                }
                output.extend_from_slice(&buffer[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(false)
}
impl ClaudeCli for NativeClaudeCli {
    fn run_mcp(&self, args: &[String]) -> Result<()> {
        let mut child = Command::new(&self.executable)
            .arg("mcp")
            .args(args)
            .current_dir(&self.cwd)
            .env_clear()
            .envs(&self.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()?;
        let mut reaped = false;
        let outcome = (|| -> Result<()> {
            let mut stdout = child.stdout.take().expect("piped stdout");
            let mut stderr = child.stderr.take().expect("piped stderr");
            nonblocking(&stdout)?;
            nonblocking(&stderr)?;
            let mut out = Vec::new();
            let mut err = Vec::new();
            let deadline = Instant::now() + self.timeout;
            loop {
                let out_done = drain(&mut stdout, &mut out)?;
                let err_done = drain(&mut stderr, &mut err)?;
                // Do not reap while inherited pipes are still open: a timed-out
                // process group must keep its leader pid reserved until kill.
                if out_done
                    && err_done
                    && let Some(status) = child.try_wait()?
                {
                    reaped = true;
                    if status.success() {
                        return Ok(());
                    }
                    let message = format!(
                        "{}\n{}",
                        String::from_utf8_lossy(&out),
                        String::from_utf8_lossy(&err)
                    );
                    let last = pm_model::json::js_trim(&message)
                        .split('\n')
                        .next_back()
                        .unwrap_or("");
                    return Err(Error::invalid(if last.is_empty() {
                        "Claude Code could not change its settings.".into()
                    } else {
                        let mut units = 0;
                        last.chars()
                            .take_while(|c| {
                                units += c.len_utf16();
                                units <= 240
                            })
                            .collect::<String>()
                    }));
                }
                if Instant::now() >= deadline {
                    return Err(Error::invalid("Claude Code registration timed out."));
                }
                thread::sleep(Duration::from_millis(2));
            }
        })();
        if outcome.is_err() && !reaped {
            // The worker owns this process group. Stop descendants before reaping;
            // cleanup failure never replaces the original CLI/storage diagnostic.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
        }
        let _ = child.wait();
        outcome
    }
}
