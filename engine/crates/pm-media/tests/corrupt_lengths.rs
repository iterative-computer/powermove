//! A corrupt file fails the probe cleanly. Containers declare the length of
//! their index blocks themselves (an 8-byte BER length in MXF, 32-bit chunk
//! sizes in AVI, box sizes in MP4); a length the file cannot hold must be
//! an error, never an allocation of that size.

mod common;

use common::*;
use std::path::Path;
use std::process::Command;

fn encode(path: &Path, args: &[&str]) {
    let status = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=720x576:rate=25:duration=0.4")
        .args(args)
        .arg(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn find_last(bytes: &[u8], needle: &[u8]) -> usize {
    bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .expect("the block to corrupt")
}

/// The file must be refused or read another way; reaching the end of this
/// function at all is the point (an unchecked length aborts the process).
fn probes_without_dying(path: &Path) {
    match pm_media::probe::probe(path) {
        Ok(probed) => assert!(probed.index.is_some_and(|index| !index.frames.is_empty())),
        Err(error) => assert!(!error.to_string().is_empty()),
    }
}

#[test]
fn an_mxf_random_index_pack_declaring_exabytes_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mxf");
    encode(
        &path,
        &[
            "-c:v",
            "mpeg2video",
            "-b:v",
            "4M",
            "-pix_fmt",
            "yuv420p",
            "-f",
            "mxf",
        ],
    );
    let mut bytes = std::fs::read(&path).unwrap();
    // The random index pack closes the file: key, BER length, entries, and
    // its own total length in the last four bytes.
    let key = [
        0x06, 0x0E, 0x2B, 0x34, 0x02, 0x05, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x11, 0x01,
    ];
    let at = find_last(&bytes, &key) + 16;
    let old = if bytes[at] < 0x80 {
        1
    } else {
        1 + (bytes[at] & 0x7f) as usize
    };
    // An 8-byte length of 2^62 bytes.
    let huge = [0x88, 0x40, 0, 0, 0, 0, 0, 0, 0];
    bytes.splice(at..at + old, huge);
    let grown = (huge.len() - old) as u32;
    let end = bytes.len();
    let total = u32::from_be_bytes(bytes[end - 4..].try_into().unwrap()) + grown;
    bytes[end - 4..].copy_from_slice(&total.to_be_bytes());
    std::fs::write(&path, bytes).unwrap();
    probes_without_dying(&path);
}

#[test]
fn avi_and_mp4_index_blocks_longer_than_the_file_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let avi = dir.path().join("clip.avi");
    encode(&avi, &["-c:v", "mpeg4", "-q:v", "5", "-pix_fmt", "yuv420p"]);
    let mut bytes = std::fs::read(&avi).unwrap();
    let at = find_last(&bytes, b"idx1") + 4;
    bytes[at..at + 4].copy_from_slice(&0xFFFF_FFF0u32.to_le_bytes());
    std::fs::write(&avi, bytes).unwrap();
    probes_without_dying(&avi);

    let mp4 = dir.path().join("clip.mp4");
    let mut bytes = std::fs::read(fixtures_dir().join("h264-aac.mp4")).unwrap();
    let at = find_last(&bytes, b"moov") - 4;
    bytes[at..at + 4].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
    std::fs::write(&mp4, bytes).unwrap();
    probes_without_dying(&mp4);
}
