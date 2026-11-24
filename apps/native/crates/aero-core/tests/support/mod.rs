//! Test support: the vendored fake inference server
//! (`vendor/fx/test-support/fake-inference-server.zig`, built by build.rs)
//! and a minimal HTTP/1.1 client the tests use as the host's fetch edge. No
//! real model, no credentials.

#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};

/// The fake server, killed (by the pid this test started) when dropped.
pub struct FakeServer {
    child: Child,
    pub port: u16,
}

impl FakeServer {
    pub fn start() -> FakeServer {
        let path = env!("PM_AERO_FAKE_SERVER");
        assert!(
            !path.is_empty(),
            "built without the libfx core: no fake server"
        );
        let mut child = Command::new(path)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the fake inference server");
        let mut line = String::new();
        BufReader::new(child.stdout.take().expect("stdout"))
            .read_line(&mut line)
            .expect("read the server's port");
        // {"host":"127.0.0.1","port":N}
        let port = line
            .split("\"port\":")
            .nth(1)
            .and_then(|rest| rest.trim().trim_end_matches('}').parse().ok())
            .unwrap_or_else(|| panic!("unexpected server banner: {line}"));
        FakeServer { child, port }
    }
}

impl Drop for FakeServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Sends one request and streams the response body to `on_body` as it
/// arrives (chunked encoding decoded). Returns the status.
pub fn http(
    url: &str,
    method: &str,
    headers: &[(String, String)],
    body: &[u8],
    mut on_status: impl FnMut(u16),
    mut on_body: impl FnMut(&[u8]),
) -> std::io::Result<()> {
    let rest = url.strip_prefix("http://").expect("loopback http only");
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let path = if path.is_empty() { "/" } else { path };
    let mut stream = TcpStream::connect(authority)?;
    stream.set_nodelay(true)?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nhost: {authority}\r\nconnection: close\r\ncontent-length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if lower == "host" || lower == "content-length" || lower == "connection" {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    on_status(status);
    let mut chunked = false;
    let mut length: Option<usize> = None;
    loop {
        line.clear();
        reader.read_line(&mut line)?;
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let lower = header.to_ascii_lowercase();
        if lower.starts_with("transfer-encoding:") && lower.contains("chunked") {
            chunked = true;
        } else if let Some(value) = lower.strip_prefix("content-length:") {
            length = value.trim().parse().ok();
        }
    }
    if chunked {
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0"), 16)
                .unwrap_or(0);
            if size == 0 {
                break;
            }
            let mut chunk = vec![0u8; size];
            reader.read_exact(&mut chunk)?;
            on_body(&chunk);
            line.clear();
            reader.read_line(&mut line)?;
        }
    } else {
        let mut buf = [0u8; 16 * 1024];
        let mut remaining = length.unwrap_or(usize::MAX);
        while remaining > 0 {
            let want = remaining.min(buf.len());
            let read = reader.read(&mut buf[..want])?;
            if read == 0 {
                break;
            }
            on_body(&buf[..read]);
            remaining -= read;
        }
    }
    Ok(())
}

/// Waits until `fd` is readable or `timeout_ms` passes, then empties it.
pub fn wait_ready(fd: i32, timeout_ms: i32) {
    let mut poll = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut poll, 1, timeout_ms) };
    let mut sink = [0u8; 64];
    // SAFETY: non-blocking read into a local buffer; wake bytes carry no state.
    while unsafe { libc::read(fd, sink.as_mut_ptr().cast(), sink.len()) } > 0 {}
}
