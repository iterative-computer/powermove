//! The S6 gate as tests: every e2e fixture video (and, with `--ignored`, the
//! generated seek corpus) decodes on every applicable backend, random access to
//! every composition frame (rule 7) equals the sequential decode, references
//! agree, and every frame reaches wgpu without a copy except through FFmpeg's
//! own libdav1d and libvpx wrappers (which the direct sessions replace).
//!
//! Needs a Metal GPU, Homebrew FFmpeg 8.1 (`/opt/homebrew/bin/ffmpeg`), dav1d,
//! libvpx, and swiftc for the AVFoundation reference.

use s6_decode::gpu::Gpu;
use s6_decode::run::{self, FileReport, Options};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn avf_tool() -> Option<PathBuf> {
    let src = root().join("scripts/avf-framemd5.swift");
    let out = root().join("target/avf/avf-framemd5-test");
    std::fs::create_dir_all(out.parent()?).ok()?;
    let ok = std::process::Command::new("swiftc")
        .args(["-O", "-suppress-warnings"])
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .status()
        .ok()?
        .success();
    ok.then_some(out)
}

fn videos(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ["mp4", "mov", "mkv", "webm", "ts"].contains(&e))
        })
        .collect();
    v.sort();
    v
}

/// Backends whose frames are allowed to be copied: FFmpeg's libdav1d and
/// libvpx wrappers (they decode into their own memory). The direct dav1d and
/// libvpx-vp9 sessions and the paired native VP8/VP9 decoders cover those
/// streams without a copy; `zero_copy_path` checks that one exists.
fn copy_allowed(detail: &str) -> bool {
    detail.starts_with("libdav1d") || detail.starts_with("libvpx ") || detail.starts_with("libvpx-vp9 (own")
}

/// Fixture content checks known to fail for a reason outside the decoder.
/// hevc-alpha.mov was encoded by VideoToolbox from BGRA with BT.709
/// coefficients but carries no matrix tag; section 9 decodes untagged video at
/// heights up to 576 as BT.601, so its green reads (0.08, 1.18, 0.03). Its
/// decode is exact (VideoToolbox equals AVFoundation and FFmpeg).
const KNOWN_CONTENT: [(&str, &str); 1] = [("hevc-alpha.mov", "the file is untagged")];

fn check(r: &FileReport, failures: &mut Vec<String>) {
    let name = Path::new(&r.file).file_name().unwrap().to_string_lossy().into_owned();
    for e in &r.errors {
        failures.push(format!("{name}: {e}"));
    }
    if let Some(ix) = &r.index
        && !ix.demux_crosscheck_ok && ix.stats.edit_list.is_none() {
            failures.push(format!("{name}: index differs from FFmpeg's demuxer: {}", ix.demux_crosscheck));
        }
    if let Some(s) = &r.selection
        && s.rule7_mismatches != 0 {
            failures.push(format!("{name}: rule 7 selected the wrong frame {} times", s.rule7_mismatches));
        }
    let mut decoded = 0;
    for b in &r.backends {
        if b.frames == 0 && b.error.is_none() {
            continue;
        }
        decoded += 1;
        let tag = format!("{name} [{}]", b.backend);
        if let Some(e) = &b.error {
            failures.push(format!("{tag}: {e}"));
            continue;
        }
        if !b.ok {
            failures.push(format!(
                "{tag}: decode checks failed (random {}/{}, mismatched {:?}, missing {:?}, selection {:?}; same session {}/{}, mismatched {:?}, missing {:?})",
                b.random.matched,
                b.random.probes,
                b.random.mismatched,
                b.random.missing,
                b.random.selection_mismatches,
                b.random_warm.matched,
                b.random_warm.probes,
                b.random_warm.mismatched,
                b.random_warm.missing
            ));
        }
        let comp = r.selection.as_ref().map(|s| s.composition_frames).unwrap_or(0);
        if b.random.probes != comp || b.random_warm.probes != comp {
            failures.push(format!("{tag}: probed {} and {} of {comp} composition frames", b.random.probes, b.random_warm.probes));
        }
        let known = KNOWN_CONTENT.iter().find(|k| k.0 == name);
        for c in b.checks.iter().filter(|c| c.contains("FAIL")) {
            match known {
                Some((_, why)) if c.contains(why) => {}
                _ => failures.push(format!("{tag}: content check failed: {c}")),
            }
        }
        if !b.zero_copy && !copy_allowed(&b.detail) {
            failures.push(format!("{tag}: not zero-copy ({} of {} frames aliased)", b.gpu.aliased, b.gpu.frames));
        }
        if b.gpu.compare.max_ulps > 1 {
            failures.push(format!("{tag}: GPU normalization differs from the CPU reference by {} ulps", b.gpu.compare.max_ulps));
        }
        // The FFmpeg reference may drop the first frame of an edit (it does not
        // show a frame straddling the edit start); AVFoundation is the authority
        // for MOV/MP4 and must match exactly.
        for rc in [&b.reference, &b.software_crosscheck].into_iter().flatten() {
            let tolerated = rc.offset == 1 && r.index.as_ref().is_some_and(|i| i.stats.edit_list.is_some());
            if rc.error.is_none() && !rc.ok(b.frames) && !tolerated {
                failures.push(format!("{tag}: {} matched {}/{} (offset {})", rc.reference, rc.matched, b.frames, rc.offset));
            }
        }
        if let Some(a) = &b.avfoundation
            && !a.ok(b.frames) {
                failures.push(format!("{tag}: {} matched {}/{} {:?}", a.reference, a.matched, b.frames, a.error));
            }
    }
    if decoded == 0 {
        failures.push(format!("{name}: no backend decoded it"));
    }
    // Every file needs at least one passing backend that reached wgpu without
    // a copy.
    if !r.backends.iter().any(|b| b.ok && b.zero_copy) {
        failures.push(format!("{name}: no zero-copy backend passed"));
    }
    for c in &r.cross_backend {
        if c.matched != c.frames {
            failures.push(format!("{name}: {} matched {}/{}", c.reference, c.matched, c.frames));
        }
    }
}

fn run_dir(dir: &Path) {
    let gpu = Gpu::new().expect("Metal GPU");
    let opt = Options { avf: avf_tool(), ..Default::default() };
    assert!(opt.avf.is_some(), "swiftc could not build the AVFoundation reference");
    let files = videos(dir);
    assert!(!files.is_empty(), "no videos in {}", dir.display());
    let mut failures = vec![];
    for f in &files {
        check(&run::run_file(f, &gpu, &opt), &mut failures);
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn e2e_fixture_matrix() {
    run_dir(&root().join("../../../apps/desktop/e2e/fixtures"));
}

/// `scripts/gen-corpus.sh` first (Homebrew FFmpeg with libx264, libx265,
/// libvpx, SVT-AV1); about two minutes.
#[test]
#[ignore]
fn seek_corpus() {
    let dir = root().join("corpus");
    assert!(dir.is_dir(), "run scripts/gen-corpus.sh first");
    run_dir(&dir);
}

/// Regression: FFmpeg 8.1's HEVC alpha path releases the alpha-layer
/// picture's luma buffer reference mid-decode (`replace_alpha_plane`). With a
/// single buffer reference per frame that freed the slot under a live decode
/// and corrupted random-access frames in about 40% of runs at 4 threads;
/// `get_buffer2` now gives every plane its own reference. Ten runs with frame
/// threading must all be exact.
#[test]
fn hevc_alpha_ffmpeg_random_access_is_stable() {
    let gpu = Gpu::new().expect("Metal GPU");
    let opt = Options { gpu: false, references: false, threads: 4, ..Default::default() };
    let f = root().join("../../../apps/desktop/e2e/fixtures/hevc-alpha.mov");
    for run in 0..10 {
        let r = run::run_file(&f, &gpu, &opt);
        let b = r.backends.iter().find(|b| b.backend == "FFmpeg").expect("FFmpeg backend");
        assert!(
            b.ok && b.random.matched == b.random.probes && b.random_warm.matched == b.random_warm.probes,
            "run {run}: random {}/{} {:?}, same session {}/{} {:?}",
            b.random.matched,
            b.random.probes,
            b.random.mismatched,
            b.random_warm.matched,
            b.random_warm.probes,
            b.random_warm.mismatched
        );
    }
}
