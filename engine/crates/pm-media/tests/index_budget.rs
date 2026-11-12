//! A corrupt index is refused before anything of its declared size is
//! allocated: a sample table whose count disagrees with the timing table,
//! and an index block larger than any real index, cost no more memory than
//! a sound file's index.

mod common;

use common::*;
use std::alloc::{GlobalAlloc, Layout, System};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The largest single allocation since the last reset.
struct Largest;
static LARGEST: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Largest {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LARGEST.fetch_max(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        LARGEST.fetch_max(size, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Largest = Largest;
/// Tests measure one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

/// Far more than any index this suite's files need, far less than what
/// the corruptions declare.
const BUDGET: usize = 32 << 20;

fn clip(path: &Path) {
    let status = std::process::Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=64x64:rate=25:duration=0.4")
        .args(["-c:v", "mpeg4", "-pix_fmt", "yuv420p"])
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn find_last(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .expect("the box to corrupt")
}

/// The probe of `path`, and the largest allocation it made.
fn probe_largest(path: &Path) -> (bool, usize) {
    LARGEST.store(0, Ordering::Relaxed);
    let ok = pm_media::probe::probe(path).is_ok();
    (ok, LARGEST.load(Ordering::Relaxed))
}

#[test]
fn a_sample_count_the_timing_table_lacks_is_refused_before_allocating() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mp4");
    clip(&path);
    let mut bytes = std::fs::read(&path).unwrap();
    // `stsz`: version and flags, a fixed sample size, the sample count.
    let at = find_last(&bytes, b"stsz") + 8;
    bytes[at..at + 4].copy_from_slice(&1u32.to_be_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&(1u32 << 26).to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();
    let (ok, largest) = probe_largest(&path);
    assert!(!ok, "the corrupt table must not index");
    assert!(largest < BUDGET, "{largest} bytes in one allocation");
}

#[test]
fn an_index_block_larger_than_any_index_is_refused_before_allocating() {
    let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mp4");
    clip(&path);
    let mut bytes = std::fs::read(&path).unwrap();
    // The movie box closes this file; it now declares 300 MiB, which a
    // (sparse) file extended to that length can hold.
    let at = find_last(&bytes, b"moov") - 4;
    let size = 300u32 << 20;
    bytes[at..at + 4].copy_from_slice(&size.to_be_bytes());
    std::fs::write(&path, &bytes).unwrap();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(at as u64 + u64::from(size))
        .unwrap();
    let (_, largest) = probe_largest(&path);
    assert!(largest < BUDGET, "{largest} bytes in one allocation");
}
