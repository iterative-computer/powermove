//! ffmpeg child processes: started without a console window, killable from
//! any task, and reporting the tail of stderr when they fail.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Result, anyhow};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStdin, ChildStdout, Command};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// `CREATE_NO_WINDOW`: a GUI app's children otherwise flash a console.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STDERR_TAIL_BYTES: usize = 1024 * 1024;

pub struct Spawn<'a> {
    pub binary: &'a Path,
    pub args: Vec<String>,
    /// Names the process in launch errors ("image sequence encoder").
    pub operation: &'a str,
    pub stdin: bool,
    pub stdout: bool,
    /// Killed once this elapses, reporting `timeout_message`.
    pub timeout: Option<(Duration, &'static str)>,
    /// Killed when this is cancelled (window closed, export cancelled).
    pub cancel: CancellationToken,
}

/// A running child. Dropping it does not stop the process; call [`Process::kill`].
pub struct Process {
    kill: CancellationToken,
    done: watch::Receiver<Option<Result<(), String>>>,
}

impl Process {
    pub fn kill(&self) {
        self.kill.cancel();
    }

    pub fn exited(&self) -> bool {
        self.done.borrow().is_some()
    }

    /// Wait for exit; a non-zero status becomes an error carrying stderr.
    pub async fn wait(&self) -> Result<()> {
        let mut done = self.done.clone();
        let outcome = done.wait_for(Option::is_some).await.map_err(|_| anyhow!("ffmpeg monitor stopped"))?;
        outcome.clone().unwrap().map_err(|message| anyhow!(message))
    }
}

pub fn spawn(options: Spawn<'_>) -> Result<(Process, Option<ChildStdin>, Option<ChildStdout>)> {
    let mut command = Command::new(options.binary);
    command
        .args(&options.args)
        .stdin(if options.stdin { Stdio::piped() } else { Stdio::null() })
        .stdout(if options.stdout { Stdio::piped() } else { Stdio::null() })
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().map_err(|error| {
        let name = options.binary.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
        anyhow!("Could not start {} ({name}): {error}", options.operation)
    })?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = tokio::spawn(read_tail(child.stderr.take().expect("stderr is piped")));

    let kill = options.cancel.child_token();
    let (sender, done) = watch::channel(None);
    let killer = kill.clone();
    let timeout = options.timeout;
    tokio::spawn(async move {
        let deadline = async {
            match timeout {
                Some((duration, _)) => tokio::time::sleep(duration).await,
                None => std::future::pending().await,
            }
        };
        let outcome = tokio::select! {
            status = child.wait() => match status {
                Ok(status) if status.success() => Ok(()),
                Ok(status) => {
                    let stderr = stderr.await.unwrap_or_default();
                    let stderr = stderr.trim();
                    Err(if stderr.is_empty() {
                        format!("ffmpeg stopped with code {}", status.code().map_or("unknown".into(), |code| code.to_string()))
                    } else {
                        stderr.to_owned()
                    })
                }
                Err(error) => Err(error.to_string()),
            },
            _ = killer.cancelled() => {
                let _ = child.kill().await;
                Err("ffmpeg was stopped".to_owned())
            }
            _ = deadline => {
                let _ = child.kill().await;
                Err(timeout.map_or("", |(_, message)| message).to_owned())
            }
        };
        let _ = sender.send(Some(outcome));
    });
    Ok((Process { kill, done }, stdin, stdout))
}

/// Run to completion, discarding output.
pub async fn run(options: Spawn<'_>) -> Result<()> {
    let (process, _, _) = spawn(options)?;
    process.wait().await
}

async fn read_tail(mut stream: impl AsyncRead + Unpin) -> String {
    let mut tail = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(count) = stream.read(&mut buffer).await {
        if count == 0 {
            break;
        }
        tail.extend_from_slice(&buffer[..count]);
        if tail.len() > STDERR_TAIL_BYTES {
            tail.drain(..tail.len() - STDERR_TAIL_BYTES);
        }
    }
    String::from_utf8_lossy(&tail).into_owned()
}
