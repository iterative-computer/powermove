//! Deadline signals from the interactive process to the job process (contract 13).
//!
//! In the product, main relays these between the two utility processes. The spike
//! uses a FIFO created by the orchestrator: A writes fixed 16-byte records (atomic,
//! below `PIPE_BUF`), B drains them without blocking between command buffers.
//! Each record carries the sender's host time so B can measure relay latency.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    /// Playback started: job engine enters its playback (sliced) policy.
    PlaybackStart = b'P',
    /// Playback stopped: job engine may return to full speed.
    PlaybackStop = b'S',
    /// A frame completed within its deadline (sent every frame; lets B align to A).
    FrameOk = b'F',
    /// A frame completed late enough to be at risk (latency over the risk threshold)
    /// but still before its deadline.
    AtRisk = b'R',
    /// A frame missed its deadline.
    Miss = b'M',
}

impl Kind {
    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            b'P' => Self::PlaybackStart,
            b'S' => Self::PlaybackStop,
            b'F' => Self::FrameOk,
            b'R' => Self::AtRisk,
            b'M' => Self::Miss,
            _ => return None,
        })
    }
}

pub const RECORD: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Message {
    pub kind: Kind,
    /// Sender host time (ns, mach timebase).
    pub sent_ns: u64,
}

impl Message {
    pub fn encode(&self) -> [u8; RECORD] {
        let mut b = [0u8; RECORD];
        b[0] = self.kind as u8;
        b[8..16].copy_from_slice(&self.sent_ns.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Self> {
        if b.len() < RECORD {
            return None;
        }
        Some(Self {
            kind: Kind::from_u8(b[0])?,
            sent_ns: u64::from_le_bytes(b[8..16].try_into().unwrap()),
        })
    }
}

pub fn make_fifo(path: &str) -> io::Result<()> {
    let c = CString::new(path).unwrap();
    let _ = std::fs::remove_file(path);
    // SAFETY: valid C string.
    let rc = unsafe { libc::mkfifo(c.as_ptr(), 0o600) };
    if rc == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

fn open(path: &str, flags: i32) -> io::Result<RawFd> {
    let c = CString::new(path).unwrap();
    // SAFETY: valid C string.
    let fd = unsafe { libc::open(c.as_ptr(), flags) };
    if fd < 0 { Err(io::Error::last_os_error()) } else { Ok(fd) }
}

pub struct Sender {
    fd: RawFd,
    pub sent: u64,
    pub dropped: u64,
}

impl Sender {
    /// Open the FIFO for writing without blocking (O_RDWR never waits for a reader).
    pub fn open(path: &str) -> io::Result<Self> {
        Ok(Self { fd: open(path, libc::O_RDWR | libc::O_NONBLOCK)?, sent: 0, dropped: 0 })
    }

    pub fn send(&mut self, kind: Kind) {
        let rec = Message { kind, sent_ns: crate::clock::now_ns() }.encode();
        // SAFETY: fd is open; RECORD < PIPE_BUF so the write is atomic or fails whole.
        let n = unsafe { libc::write(self.fd, rec.as_ptr() as *const _, RECORD) };
        if n == RECORD as isize {
            self.sent += 1;
        } else {
            self.dropped += 1;
        }
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        // SAFETY: we own the fd.
        unsafe { libc::close(self.fd) };
    }
}

pub struct Receiver {
    fd: RawFd,
    carry: Vec<u8>,
}

impl Receiver {
    pub fn open(path: &str) -> io::Result<Self> {
        Ok(Self { fd: open(path, libc::O_RDONLY | libc::O_NONBLOCK)?, carry: Vec::new() })
    }

    /// Drain every complete record currently in the pipe.
    pub fn drain(&mut self) -> Vec<Message> {
        let mut buf = [0u8; 4096];
        loop {
            // SAFETY: fd is open, buf is valid for its length.
            let n = unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n <= 0 {
                break;
            }
            self.carry.extend_from_slice(&buf[..n as usize]);
        }
        let whole = self.carry.len() / RECORD * RECORD;
        let out = self.carry[..whole].chunks(RECORD).filter_map(Message::decode).collect();
        self.carry.drain(..whole);
        out
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        // SAFETY: we own the fd.
        unsafe { libc::close(self.fd) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_round_trip() {
        for kind in [Kind::PlaybackStart, Kind::PlaybackStop, Kind::FrameOk, Kind::AtRisk, Kind::Miss] {
            let m = Message { kind, sent_ns: 0x0123_4567_89ab_cdef };
            assert_eq!(Message::decode(&m.encode()), Some(m));
        }
        assert_eq!(Message::decode(&[b'?'; RECORD]), None);
    }

    #[test]
    fn fifo_delivers_in_order() {
        let path = std::env::temp_dir().join(format!("s8-test-{}.fifo", std::process::id()));
        let path = path.to_str().unwrap().to_string();
        make_fifo(&path).unwrap();
        let mut rx = Receiver::open(&path).unwrap();
        let mut tx = Sender::open(&path).unwrap();
        assert!(rx.drain().is_empty());
        tx.send(Kind::PlaybackStart);
        tx.send(Kind::Miss);
        tx.send(Kind::FrameOk);
        let got: Vec<Kind> = rx.drain().iter().map(|m| m.kind).collect();
        assert_eq!(got, vec![Kind::PlaybackStart, Kind::Miss, Kind::FrameOk]);
        assert_eq!(tx.sent, 3);
        std::fs::remove_file(&path).unwrap();
    }
}
