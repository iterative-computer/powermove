//! One agent CLI attempt: spawn, stream stdout lines, keep bounded
//! diagnostics, and stop the whole process tree on cancel or timeout.

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::runtime;

const MAX_DIAGNOSTIC_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Writes to the CLI's stdin; `close` ends the input so the CLI can exit.
#[derive(Clone)]
pub struct Stdin(mpsc::UnboundedSender<Option<String>>);

impl Stdin {
    pub fn write(&self, line: String) {
        let _ = self.0.send(Some(line));
    }

    pub fn close(&self) {
        let _ = self.0.send(None);
    }

    #[cfg(test)]
    pub fn from_sender(sender: mpsc::UnboundedSender<Option<String>>) -> Self {
        Self(sender)
    }
}

#[derive(Debug, Default)]
pub struct Exit {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub spawn_error: Option<String>,
    /// Stopped by the cancel token.
    pub cancelled: bool,
    pub timed_out: bool,
}

impl Exit {
    pub fn success(&self) -> bool {
        self.code == Some(0) && !self.cancelled && !self.timed_out
    }
}

fn append_bounded(buffer: &mut String, text: &str) {
    if buffer.len() >= MAX_DIAGNOSTIC_BYTES {
        return;
    }
    let room = MAX_DIAGNOSTIC_BYTES - buffer.len();
    let mut cut = text.len().min(room);
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    buffer.push_str(&text[..cut]);
}

/// Run `command` to completion. `on_start` receives the stdin handle once the
/// process exists; `on_line` sees each stdout line (oversized lines are dropped).
pub async fn run(mut command: Command, cancel: &CancellationToken, timeout: Duration, on_start: impl FnOnce(Stdin), mut on_line: impl FnMut(&str)) -> Exit {
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Exit { spawn_error: Some(error.to_string()), ..Exit::default() },
    };
    let (sender, mut inbox) = mpsc::unbounded_channel::<Option<String>>();
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            while let Some(Some(line)) = inbox.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
    }
    on_start(Stdin(sender));
    let stderr_task = child.stderr.take().map(|mut stderr| {
        tokio::spawn(async move {
            let mut text = String::new();
            let mut buffer = vec![0u8; 8192];
            while let Ok(read) = stderr.read(&mut buffer).await {
                if read == 0 {
                    break;
                }
                append_bounded(&mut text, &String::from_utf8_lossy(&buffer[..read]));
            }
            text
        })
    });
    let mut exit = Exit::default();
    let mut reader = child.stdout.take().map(BufReader::new);
    let mut line = Vec::new();
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    loop {
        let read = async {
            match reader.as_mut() {
                Some(reader) => reader.read_until(b'\n', &mut line).await.unwrap_or(0),
                None => 0,
            }
        };
        tokio::select! {
            read = read => {
                if read == 0 {
                    break;
                }
                if line.len() <= MAX_LINE_BYTES {
                    let text = String::from_utf8_lossy(&line);
                    append_bounded(&mut exit.stdout, &text);
                    on_line(text.trim_end_matches(['\r', '\n']));
                }
                line.clear();
            }
            () = cancel.cancelled() => {
                exit.cancelled = true;
                break;
            }
            () = &mut deadline => {
                exit.timed_out = true;
                break;
            }
        }
    }
    if exit.cancelled || exit.timed_out {
        runtime::kill_tree(&mut child).await;
    }
    exit.code = child.wait().await.ok().and_then(|status| status.code());
    if let Some(task) = stderr_task {
        exit.stderr = task.await.unwrap_or_default();
    }
    exit
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn shell(script: &str) -> Command {
        let mut command = Command::new("powershell");
        command.args(["-NoProfile", "-Command", script]);
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        command
    }

    #[tokio::test]
    async fn streams_lines_and_stops_on_cancel() {
        let cancel = CancellationToken::new();
        let mut lines = Vec::new();
        let exit = run(shell("Write-Output one; Write-Output two; [Console]::Error.WriteLine('oops')"), &cancel, Duration::from_secs(30), |_| {}, |line| lines.push(line.to_owned())).await;
        assert_eq!(lines, ["one", "two"]);
        assert!(exit.success());
        assert!(exit.stderr.contains("oops"));

        let stopper = cancel.clone();
        let exit = run(shell("Write-Output ready; Start-Sleep -Seconds 30"), &cancel, Duration::from_secs(60), |_| {}, |_| stopper.cancel()).await;
        assert!(exit.cancelled && !exit.success());
    }
}
