//! The random-access regression harness (S6 as a suite): for every file and
//! every backend that decodes it, sequential decode, references, and random
//! access to every composition frame selected by section 5 rule 7, with a new
//! session per probe and within one session.
#![allow(dead_code)]

use pm_media::decode::arbiter::LocalArbiter;
use pm_media::decode::service::MediaSource;
use pm_media::decode::session::Session;
use pm_media::decode::{Backend, StreamSpec, backends, open};
use pm_media::ff::{Input, PacketReader};
use pm_media::frame::{DecodedFrame, Pixels};
use pm_media::pool::FramePool;
use pm_media::time::{expected_frame, frame_time, grid};
use pm_media::vt::DecodePurpose;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

pub fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixtures_dir() -> PathBuf {
    manifest().join("../../../apps/desktop/e2e/fixtures")
}

/// The generated corpus (tests/corpus/gen.sh): `PM_MEDIA_CORPUS` or the
/// default cache directory. Panics with instructions when missing.
pub fn corpus_dir() -> PathBuf {
    let dir = std::env::var_os("PM_MEDIA_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Caches/powermove-engine/media-corpus"));
    assert!(
        dir.join("h264-bpyramid.mp4").is_file(),
        "the media corpus is missing at {}; run engine/crates/pm-media/tests/corpus/gen.sh",
        dir.display()
    );
    dir
}

pub fn ffmpeg_bin() -> PathBuf {
    let dir = std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join("Library/Caches/powermove-engine/ffmpeg/out"));
    dir.join("bin/ffmpeg")
}

pub fn videos(dir: &Path, filter: &dyn Fn(&str) -> bool) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            ["mp4", "mov", "mkv", "webm", "ts", "mxf", "avi", "dv", "vob"].contains(&ext) && filter(&name)
        })
        .collect();
    v.sort();
    v
}

#[derive(Default, Debug)]
pub struct RandomReport {
    pub probes: usize,
    pub matched: usize,
    pub mismatched: Vec<usize>,
    pub errors: Vec<String>,
    pub selection_mismatches: usize,
    pub mean_ms: f64,
    pub max_ms: f64,
}

#[derive(Debug)]
pub struct BackendRun {
    pub backend: Backend,
    pub name: String,
    pub hardware: bool,
    pub output: String,
    pub frames: usize,
    pub copied_frames: usize,
    pub seq_md5: Vec<String>,
    pub norm: Vec<String>,
    pub reference: Option<(String, usize, usize, isize)>,
    pub new_session: RandomReport,
    pub same_session: RandomReport,
    pub seq_ms: f64,
}

pub struct FileRun {
    pub name: String,
    pub source: Arc<MediaSource>,
    pub runs: Vec<BackendRun>,
    pub errors: Vec<String>,
}

fn norm_hash(f: &DecodedFrame) -> String {
    use md5::{Digest, Md5};
    let (l, c) = f.reference().unwrap();
    let mut h = Md5::new();
    for v in l.iter().chain(&c) {
        h.update(v[0].to_bits().to_le_bytes());
        h.update(v[1].to_bits().to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

fn shuffled(n: usize, seed: u64) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    let mut s = seed | 1;
    for i in (1..n).rev() {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        let r = s.wrapping_mul(0x2545_F491_4F6C_DD1D);
        v.swap(i, (r % (i as u64 + 1)) as usize);
    }
    v
}

fn session(src: &MediaSource, b: Backend, pool: &FramePool, purpose: DecodePurpose) -> pm_media::Result<Session> {
    let input = Input::open(src.path())?;
    let spec = StreamSpec { input: &input, stream: src.stream, info: &src.info, index: &src.index, extradata: &src.extradata };
    let (dec, _) = open(&spec, purpose, pool, &LocalArbiter::unlimited(), 4, Some(b))?;
    let reader = PacketReader::new(src.path(), src.stream)?;
    let mut s = Session::new(src.index.clone(), reader, dec, Some(src.color));
    s.set_window(0);
    Ok(s)
}

/// framemd5 lines of FFmpeg's decode (our LGPL build).
fn framemd5(args: &[&str]) -> Result<Vec<String>, String> {
    let out = Command::new(ffmpeg_bin())
        .args(["-v", "error", "-nostdin", "-threads", "4"])
        .args(args)
        .args(["-f", "framemd5", "-"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("ffmpeg failed").to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string()))
        .collect())
}

/// (reference, matched, frames, offset): aligned on the first reference frame.
fn compare(name: String, r: Result<Vec<String>, String>, seq: &[String]) -> (String, usize, usize, isize) {
    match r {
        Err(e) => (format!("{name}: error {e}"), 0, 0, 0),
        Ok(h) => {
            let offset = match h.first() {
                Some(f0) if seq.first() != Some(f0) => seq.iter().take(8).position(|s| s == f0).map(|p| p as isize).unwrap_or(0),
                _ => 0,
            };
            let ours = &seq[offset as usize..];
            let matched = h.iter().zip(ours).filter(|(a, b)| a == b).count();
            (name, matched, h.len(), offset)
        }
    }
}

fn reference_for(src: &MediaSource, run: &BackendRun) -> Option<(String, usize, usize, isize)> {
    let file = src.path().to_str().unwrap();
    let mut args: Vec<&str> = vec![];
    let fmt;
    match run.backend {
        Backend::VideoToolbox => {
            fmt = match run.output.as_str() {
                "420v" | "420f" => "nv12",
                "x420" | "xf20" => "p010le",
                "x422" | "xf22" => "p210le",
                "y416" => "ayuv64le",
                _ => return None,
            };
            let vf = format!("hwdownload,format={fmt}");
            let r = framemd5(&["-hwaccel", "videotoolbox", "-hwaccel_output_format", "videotoolbox_vld", "-i", file, "-map", "0:v:0", "-vf", &vf, "-fps_mode", "passthrough"]);
            return Some(compare(format!("ffmpeg -hwaccel videotoolbox ({fmt})"), r, &run.seq_md5));
        }
        Backend::Dav1d => args.extend(["-c:v", "libdav1d"]),
        Backend::FfmpegAlphaPair => args.extend(["-c:v", if src.info.codec == "vp9" { "libvpx-vp9" } else { "libvpx" }]),
        Backend::Ffmpeg => {}
    }
    if run.copied_frames > 0 && run.backend == Backend::Ffmpeg && run.name.contains("converted") {
        return None;
    }
    args.extend(["-i", file, "-map", "0:v:0", "-fps_mode", "passthrough"]);
    Some(compare("ffmpeg software framemd5".into(), framemd5(&args), &run.seq_md5))
}

pub fn run_file(path: &Path, only: Option<&[Backend]>, purpose: DecodePurpose) -> FileRun {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let source = MediaSource::open(path).unwrap_or_else(|e| panic!("{name}: open: {e}"));
    let pool = FramePool::new(64 << 20);
    let mut runs = vec![];
    let mut errors = vec![];
    let idx = &source.index;
    let n = idx.frames.len();
    for b in backends(&source.info, idx) {
        if only.is_some_and(|o| !o.contains(&b)) {
            continue;
        }
        let mut s = match session(&source, b, &pool, purpose) {
            Ok(s) => s,
            Err(e) => {
                errors.push(format!("{name} [{b:?}]: open: {e}"));
                continue;
            }
        };
        let info = s.decoder_info();
        // Sequential: every sample in decode order through a fresh decoder.
        let t0 = Instant::now();
        let input = Input::open(path).unwrap();
        let spec = StreamSpec { input: &input, stream: source.stream, info: &source.info, index: idx, extradata: &source.extradata };
        let (mut dec, _) = open(&spec, purpose, &pool, &LocalArbiter::unlimited(), 4, Some(b)).unwrap();
        let mut reader = PacketReader::new(path, source.stream).unwrap();
        let all: Vec<usize> = (0..idx.samples.len()).collect();
        let pkts = reader.read(idx, &all).unwrap();
        let mut out = vec![];
        for p in &pkts {
            dec.send(p, &mut out).unwrap_or_else(|e| panic!("{name} [{b:?}]: sequential send: {e}"));
        }
        dec.drain(&mut out).unwrap();
        let by_cts: HashMap<i64, usize> = idx.samples.iter().enumerate().map(|(i, s)| (s.cts, i)).collect();
        let mut by_sample: HashMap<usize, DecodedFrame> = HashMap::new();
        for f in out {
            let s = by_cts[&f.pts];
            assert!(by_sample.insert(s, f).is_none(), "{name} [{b:?}]: sample {s} decoded twice");
        }
        let mut seq_md5 = vec![];
        let mut norm = vec![];
        let mut copied = 0;
        let mut output = info.output.clone().unwrap_or_default();
        for k in 0..n {
            let s = idx.frames[k].sample;
            let f = by_sample.get(&s).unwrap_or_else(|| panic!("{name} [{b:?}]: sequential decode missed frame {k}"));
            if output.is_empty() {
                output = f.format.label.clone();
            }
            if f.copied() {
                copied += 1;
            }
            match &f.pixels {
                Pixels::CoreVideo(_) => {}
                Pixels::Pool(p) => assert!(p.planes.iter().all(|pl| pl.ptr() as usize == pl.decoder_ptr)),
            }
            seq_md5.push(f.md5_hex().unwrap());
            norm.push(norm_hash(f));
        }
        drop(by_sample);
        drop(dec);
        let seq_ms = t0.elapsed().as_secs_f64() * 1e3;
        let mut run = BackendRun {
            backend: b,
            name: info.name.clone(),
            hardware: info.hardware,
            output,
            frames: n,
            copied_frames: copied,
            seq_md5,
            norm,
            reference: None,
            new_session: RandomReport::default(),
            same_session: RandomReport::default(),
            seq_ms,
        };
        run.reference = reference_for(&source, &run);

        // Random access over every composition frame (rule 7), shuffled.
        let pts = &source.pts;
        let g = grid(pts, idx.timebase, source.rate());
        let comp = (*g.last().unwrap() + 1) as usize;
        let order = shuffled(comp, 0x5eed_0006 ^ comp as u64);
        for warm in [false, true] {
            let mut r = RandomReport::default();
            let mut times = vec![];
            for &cn in &order {
                r.probes += 1;
                let t = frame_time(pts[0], idx.timebase, source.rate(), cn as i64);
                let Some(target) = source.select(t) else {
                    r.errors.push(format!("composition frame {cn} selects nothing"));
                    continue;
                };
                if Some(target) != expected_frame(&g, cn as i64) {
                    r.selection_mismatches += 1;
                }
                let t0 = Instant::now();
                let got = if warm {
                    s.presented(target)
                } else {
                    session(&source, b, &pool, purpose).and_then(|mut fresh| fresh.presented(target))
                };
                times.push(t0.elapsed().as_secs_f64() * 1e3);
                match got.and_then(|f| f.md5_hex()) {
                    Ok(h) if h == run.seq_md5[target] => r.matched += 1,
                    Ok(_) => r.mismatched.push(cn),
                    Err(e) => r.errors.push(format!("frame {target}: {e}")),
                }
            }
            r.mean_ms = times.iter().sum::<f64>() / times.len().max(1) as f64;
            r.max_ms = times.iter().copied().fold(0.0, f64::max);
            if warm {
                run.same_session = r;
            } else {
                run.new_session = r;
            }
        }
        runs.push(run);
    }
    FileRun { name, source, runs, errors }
}

/// Every check the suite makes; returns failures.
pub fn check(r: &FileRun) -> Vec<String> {
    let mut f = r.errors.clone();
    let name = &r.name;
    if r.runs.is_empty() {
        f.push(format!("{name}: no backend decoded it"));
    }
    let edit_list = r.source.index.stats.edit_list.is_some();
    for run in &r.runs {
        let tag = format!("{name} [{:?} {}]", run.backend, run.name);
        for (label, rr) in [("new session", &run.new_session), ("same session", &run.same_session)] {
            if rr.matched != rr.probes || !rr.errors.is_empty() || rr.selection_mismatches != 0 {
                f.push(format!(
                    "{tag}: random access ({label}) {}/{} exact, mismatched {:?}, errors {:?}, selection {}",
                    rr.matched,
                    rr.probes,
                    &rr.mismatched[..rr.mismatched.len().min(10)],
                    &rr.errors[..rr.errors.len().min(3)],
                    rr.selection_mismatches
                ));
            }
        }
        if let Some((refname, matched, frames, offset)) = &run.reference {
            // FFmpeg drops the frame straddling an edit list's start; the other
            // frames must match, offset by one.
            let tolerated = edit_list && *offset == 1 && *matched == *frames;
            if !(tolerated || (*matched == *frames && *frames == run.frames && *offset == 0)) {
                f.push(format!("{tag}: {refname} matched {matched}/{frames} (ours {}, offset {offset})", run.frames));
            }
        }
    }
    // Every backend decodes the same picture (section 13 normalization, f32 bits).
    if let [a, rest @ ..] = &r.runs[..] {
        for b in rest {
            let same = a.norm.iter().zip(&b.norm).filter(|(x, y)| x == y).count();
            if same != a.norm.len() {
                f.push(format!(
                    "{name}: {:?} and {:?} normalized output agree on {same}/{} frames",
                    a.backend,
                    b.backend,
                    a.norm.len()
                ));
            }
        }
    }
    if !r.runs.iter().any(|run| run.copied_frames == 0) {
        f.push(format!("{name}: no backend reached GPU-shareable memory without a copy"));
    }
    f
}

pub fn summary(r: &FileRun) -> String {
    let idx = &r.source.index;
    let mut s = format!(
        "{}: {:?} {} {} frames, rate {}/{} ({:?}), index {:?} {:.2} ms",
        r.name,
        r.source.info.codec,
        r.source.info.pix_fmt,
        idx.frames.len(),
        r.source.rate().num(),
        r.source.rate().den(),
        r.source.nominal.method,
        idx.source,
        idx.stats.build_ms
    );
    for run in &r.runs {
        s.push_str(&format!(
            "\n  {:?} {} hw={} out={} copied={} new {}/{} ({:.2} ms mean) same {}/{} ({:.2} ms) ref {:?}",
            run.backend,
            run.name,
            run.hardware,
            run.output,
            run.copied_frames,
            run.new_session.matched,
            run.new_session.probes,
            run.new_session.mean_ms,
            run.same_session.matched,
            run.same_session.probes,
            run.same_session.mean_ms,
            run.reference.as_ref().map(|r| (r.1, r.2, r.3))
        ));
    }
    s
}

pub fn run_all(files: &[PathBuf], purpose: DecodePurpose) {
    assert!(!files.is_empty(), "no files");
    let mut failures = vec![];
    for p in files {
        let r = run_file(p, None, purpose);
        eprintln!("{}", summary(&r));
        failures.extend(check(&r));
    }
    assert!(failures.is_empty(), "{} failures:\n{}", failures.len(), failures.join("\n"));
}
