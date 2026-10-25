//! Pixel hand-off through POSIX shared memory (the Phase 3c render
//! transport, `docs/engine/rfcs/16-render-transport.md`).
//!
//! The interactive engine writes a snapshot's pixels into a segment it
//! creates exclusively (mode 0600, a random 96-bit name, `/pm.` and 24 hex
//! digits) and names it in its answer to main. Producers that can write in
//! place fill the mapping directly ([`publish_with`]). Main maps the segment
//! once, copies the pixels into a `Buffer` it hands to the window, and
//! unlinks it ([`take`]). A segment nobody takes is unlinked by the engine
//! after [`ORPHAN_AFTER`] ([`Segments::reap`]) and at shutdown.
//!
//! The reader runs in main, so it trusts nothing about the segment: the
//! name must have the engine's exact shape, the segment must be a regular
//! shared-memory object owned by this user, and it must hold at least the
//! length the answer claims (never more than [`MAX_SEGMENT`]).

use std::ffi::CString;
use std::io;
use std::time::{Duration, Instant};

/// The largest segment either side makes or reads: a 16384² RGBA8 SVG
/// material map. Snapshot requests retain their separate 512 MiB cap.
pub const MAX_SEGMENT: usize = 1024 * 1024 * 1024;
/// A segment main has not taken by then is unlinked by the engine.
pub const ORPHAN_AFTER: Duration = Duration::from_secs(60);

/// Whether `name` has the shape the engine gives its segments.
pub fn name_ok(name: &str) -> bool {
    name.len() == 28
        && name.starts_with("/pm.")
        && name[4..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 12];
    // SAFETY: arc4random_buf fills the buffer and cannot fail.
    unsafe { libc::arc4random_buf(bytes.as_mut_ptr().cast(), bytes.len()) };
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn c_name(name: &str) -> io::Result<CString> {
    if !name_ok(name) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a pixel segment name",
        ));
    }
    CString::new(name).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL in name"))
}

/// Creates a segment holding `bytes` and returns its name. The caller
/// records it in a [`Segments`] so it is unlinked if nobody takes it.
pub fn publish(bytes: &[u8]) -> io::Result<String> {
    publish_with(bytes.len(), |segment| {
        segment.copy_from_slice(bytes);
        Ok(())
    })
}

/// Creates a `length`-byte segment, lets `fill` write it in place (it starts
/// zeroed), and returns its name; a failed fill unlinks the segment.
pub fn publish_with(
    length: usize,
    fill: impl FnOnce(&mut [u8]) -> io::Result<()>,
) -> io::Result<String> {
    if length == 0 || length > MAX_SEGMENT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a {length}-byte pixel segment"),
        ));
    }
    let mut fill = Some(fill);
    for _ in 0..4 {
        let name = format!("/pm.{}", random_hex()?);
        let c = c_name(&name)?;
        // SAFETY: a valid C string; O_EXCL never opens someone else's segment.
        let fd = unsafe {
            libc::shm_open(
                c.as_ptr(),
                libc::O_CREAT | libc::O_EXCL | libc::O_RDWR,
                0o600 as libc::c_uint,
            )
        };
        if fd < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EEXIST) {
                continue;
            }
            return Err(error);
        }
        let written = write_into(fd, length, fill.take().expect("filled once"));
        // SAFETY: fd is ours.
        unsafe { libc::close(fd) };
        if let Err(error) = written {
            // SAFETY: a valid C string naming the segment just made.
            unsafe { libc::shm_unlink(c.as_ptr()) };
            return Err(error);
        }
        return Ok(name);
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free pixel segment name",
    ))
}

fn write_into(
    fd: libc::c_int,
    length: usize,
    fill: impl FnOnce(&mut [u8]) -> io::Result<()>,
) -> io::Result<()> {
    // SAFETY: fd is a fresh shared-memory object; the size fits off_t.
    if unsafe { libc::ftruncate(fd, length as libc::off_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: mapping the whole object we just sized, shared and writable.
    let map = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if map == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the fresh mapping is `length` zeroed bytes, ours alone until
    // the name is returned; it is unmapped below on every path.
    let result = fill(unsafe { std::slice::from_raw_parts_mut(map.cast::<u8>(), length) });
    // SAFETY: the mapping made above.
    unsafe { libc::munmap(map, length) };
    result
}

/// Maps the segment `name`, copies its first `length` bytes out, and
/// unlinks it. Main's half: refuses names of another shape, objects of
/// another owner, and segments shorter than `length`.
pub fn take(name: &str, length: usize) -> io::Result<Vec<u8>> {
    let c = c_name(name)?;
    if length == 0 || length > MAX_SEGMENT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("a {length}-byte pixel segment"),
        ));
    }
    // SAFETY: a valid C string; read-only.
    let fd = unsafe { libc::shm_open(c.as_ptr(), libc::O_RDONLY, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // Unlink at once: the mapping keeps the memory until it is unmapped,
    // and no one else can open it again.
    // SAFETY: a valid C string.
    unsafe { libc::shm_unlink(c.as_ptr()) };
    let result = read_from(fd, length);
    // SAFETY: fd is ours.
    unsafe { libc::close(fd) };
    result
}

fn read_from(fd: libc::c_int, length: usize) -> io::Result<Vec<u8>> {
    // SAFETY: zeroed stat is a valid out-parameter.
    let mut stat: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: fd is open; stat is writable.
    if unsafe { libc::fstat(fd, &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: getuid has no preconditions.
    if stat.st_uid != unsafe { libc::getuid() } {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pixel segment belongs to another user",
        ));
    }
    if (stat.st_size as u64) < length as u64 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!("the pixel segment holds {} of {length} bytes", stat.st_size),
        ));
    }
    // SAFETY: mapping `length` bytes the object holds, read-only.
    let map = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            length,
            libc::PROT_READ,
            libc::MAP_SHARED,
            fd,
            0,
        )
    };
    if map == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    let mut out = vec![0u8; length];
    // SAFETY: the mapping is `length` bytes long.
    unsafe {
        std::ptr::copy_nonoverlapping(map.cast::<u8>(), out.as_mut_ptr(), length);
        libc::munmap(map, length);
    }
    Ok(out)
}

/// Whether a published segment is still waiting for its reader. `take`
/// unlinks before copying, so a missing name releases the publisher's
/// outstanding-result budget. Other errors must not release that budget.
pub fn exists(name: &str) -> io::Result<bool> {
    let c = c_name(name)?;
    // SAFETY: a validated segment name; no creation or mutation.
    let fd = unsafe { libc::shm_open(c.as_ptr(), libc::O_RDONLY, 0) };
    if fd < 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err(error)
        };
    }
    // SAFETY: this call owns the descriptor.
    unsafe { libc::close(fd) };
    Ok(true)
}

/// Unlinks `name` if it still exists (the engine's orphan cleanup).
pub fn unlink(name: &str) {
    if let Ok(c) = c_name(name) {
        // SAFETY: a valid C string; a missing segment is fine.
        unsafe { libc::shm_unlink(c.as_ptr()) };
    }
}

/// The segments one engine published and nobody may have taken yet.
#[derive(Debug, Default)]
pub struct Segments {
    live: Vec<(String, Instant)>,
}

impl Segments {
    /// Records a published segment.
    pub fn add(&mut self, name: String) {
        self.live.push((name, Instant::now()));
    }

    /// Forgets segments older than `after`, unlinking any main has not
    /// taken (taking unlinks, so an unlink of a taken one is a no-op).
    /// Returns how many records it dropped.
    pub fn reap(&mut self, after: Duration) -> usize {
        let now = Instant::now();
        let before = self.live.len();
        self.live.retain(|(name, at)| {
            if now.duration_since(*at) >= after {
                unlink(name);
                false
            } else {
                true
            }
        });
        before - self.live.len()
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }
}

impl Drop for Segments {
    fn drop(&mut self) {
        for (name, _) in self.live.drain(..) {
            unlink(&name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_segment_is_taken_once() {
        let bytes: Vec<u8> = (0..4096u32).map(|i| (i * 7) as u8).collect();
        let name = publish(&bytes).unwrap();
        assert!(name_ok(&name), "{name}");
        assert!(exists(&name).unwrap());
        assert_eq!(take(&name, bytes.len()).unwrap(), bytes);
        // Taken means unlinked.
        assert!(!exists(&name).unwrap());
        assert!(exists("/etc/passwd").is_err());
        assert!(take(&name, bytes.len()).is_err());
    }

    #[test]
    fn segments_are_written_in_place() {
        let name = publish_with(5, |segment| {
            assert_eq!(segment, [0; 5], "a fresh segment starts zeroed");
            segment[1..4].copy_from_slice(&[7, 8, 9]);
            Ok(())
        })
        .unwrap();
        assert_eq!(take(&name, 5).unwrap(), [0, 7, 8, 9, 0]);
        assert!(!exists(&name).unwrap(), "taking unlinks");
        // A failed fill reports its own error (publish unlinks the segment).
        let mut made = None;
        let error = publish_with(5, |segment| {
            made = Some(segment.len());
            Err(io::Error::other("packing failed"))
        })
        .unwrap_err();
        assert_eq!(
            (error.to_string().as_str(), made),
            ("packing failed", Some(5))
        );
    }

    #[test]
    fn a_shorter_segment_than_claimed_is_refused() {
        // macOS rounds a segment up to whole pages, so claim past them.
        let name = publish(&[1, 2, 3, 4]).unwrap();
        let error = take(&name, 1 << 20).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
        // The refusal still unlinked it.
        assert!(take(&name, 4).is_err());
    }

    #[test]
    fn foreign_names_are_refused_before_any_open() {
        for bad in [
            "/etc/passwd",
            "/pm.",
            "/pm.0123456789ABCDEF01234567",
            "pm.0123456789abcdef012345678",
            "/pm.0123456789abcdef0123456/",
        ] {
            assert_eq!(
                take(bad, 4).unwrap_err().kind(),
                io::ErrorKind::InvalidInput,
                "{bad}"
            );
        }
        assert!(publish(&[]).is_err());
    }

    #[test]
    fn orphans_are_unlinked_by_the_reaper_and_on_drop() {
        let mut segments = Segments::default();
        let a = publish(&[9; 64]).unwrap();
        segments.add(a.clone());
        assert_eq!(segments.reap(Duration::from_secs(60)), 0);
        assert_eq!(segments.reap(Duration::ZERO), 1);
        assert!(take(&a, 64).is_err(), "reaped");
        let b = publish(&[9; 64]).unwrap();
        segments.add(b.clone());
        drop(segments);
        assert!(take(&b, 64).is_err(), "unlinked on drop");
    }
}
