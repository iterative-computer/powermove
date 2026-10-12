//! The S6 matrix runner: index, rule 7 selection, sequential and random-access
//! decode per backend, framemd5 references (FFmpeg and AVFoundation), zero-copy
//! GPU import and normalization checks, cross-backend agreement, timings.

use crate::ff::{self, FfDecoder, Input, Packet, PacketReader, StreamInfo};
use crate::frame::{Decoded, FrameDesc, Pixels};
use crate::gpu::{Compare, Gpu};
use crate::index::{FrameIndex, IndexSource, IndexStats};
use crate::time::{self, NominalRate, Rational, SelectionCheck};
use crate::pool::MetalPool;
use crate::sw::SwDecoder;
use crate::sw::dav1d::Dav1dDecoder;
use crate::sw::ffalpha::FfAlphaDecoder;
use crate::sw::vpx::{Codec as VpxCodec, VpxDecoder};
use crate::vt::{self, BufferFacts, VtDecoder, VtPlan};
use anyhow::{Context, Result, bail, ensure};
use md5::{Digest, Md5};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Options {
    /// Cap on random-access probes per backend (every composition frame when None).
    pub random_limit: Option<usize>,
    pub gpu: bool,
    pub references: bool,
    pub threads: i32,
    pub ffmpeg: PathBuf,
    /// The AVFoundation framemd5 tool (`scripts/avf-framemd5.swift`, compiled).
    pub avf: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            random_limit: None,
            gpu: true,
            references: true,
            threads: 0,
            ffmpeg: PathBuf::from("/opt/homebrew/bin/ffmpeg"),
            avf: None,
        }
    }
}

#[derive(Serialize)]
pub struct FileReport {
    pub file: String,
    pub bytes: u64,
    pub info: Option<StreamInfo>,
    pub index: Option<IndexReport>,
    pub nominal_rate: Option<NominalRate>,
    pub selection: Option<SelectionCheck>,
    pub backends: Vec<BackendReport>,
    /// Section 13 normalized values (Y′, A, Cb, Cr as f32) of VideoToolbox versus
    /// FFmpeg, frame by frame: both backends must produce the same picture.
    pub cross_backend: Vec<RefCompare>,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct IndexReport {
    pub source: IndexSource,
    pub timebase: Rational,
    pub stats: IndexStats,
    pub duplicate_pts: usize,
    /// Presented PTS and keyframe flags versus FFmpeg's demuxer.
    pub demux_crosscheck: String,
    pub demux_crosscheck_ok: bool,
}

#[derive(Serialize, Default)]
pub struct RefCompare {
    pub reference: String,
    pub frames: usize,
    pub matched: usize,
    /// Index of our frame 0 in the reference's list (references that drop a
    /// leading frame are aligned rather than reported as all-mismatching).
    pub offset: isize,
    pub first_mismatch: Option<usize>,
    pub error: Option<String>,
    pub note: Option<String>,
}

impl RefCompare {
    pub fn ok(&self, frames: usize) -> bool {
        self.error.is_none() && self.matched == frames && self.frames == frames && self.offset == 0
    }
}

#[derive(Serialize, Default)]
pub struct RandomReport {
    /// Composition frames probed (rule 7 selects the source frame for each).
    pub probes: usize,
    pub matched: usize,
    pub mismatched: Vec<usize>,
    pub missing: Vec<usize>,
    /// Probes whose rule 7 selection differs from the expected source frame.
    pub selection_mismatches: Vec<usize>,
    /// Non-target pictures that failed to decode during random access (leading
    /// pictures whose references precede the random-access point).
    pub leading_failures: usize,
    /// RASL pictures skipped because decoding started at their CRA.
    pub rasl_skipped: usize,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub mean_packets: f64,
    pub max_packets: usize,
}

#[derive(Serialize, Default)]
pub struct GpuReport {
    pub frames: usize,
    /// Frames whose every plane texture was verified to alias decoder memory.
    pub aliased: usize,
    /// CPU copies of decoded pixels (FFmpeg decoders without direct rendering).
    pub cpu_copies: usize,
    pub compare: Compare,
    pub import_us_p50: f64,
    pub import_us_p95: f64,
    pub normalize_us_p50: f64,
    pub plane_formats: String,
}

#[derive(Serialize, Default)]
pub struct BackendReport {
    pub backend: String,
    pub detail: String,
    pub hardware: Option<bool>,
    /// Sequential decode complete, every random-access probe equal to the
    /// sequential frame, rule 7 selections as expected, AVFoundation agreeing
    /// (MOV/MP4). Fixture content checks are reported separately (`checks_ok`):
    /// they also depend on the file's color tags.
    pub ok: bool,
    /// Every decoded frame reached wgpu as textures aliasing the decoder's own
    /// output memory (no CPU copy, no upload).
    pub zero_copy: bool,
    pub error: Option<String>,
    pub frames: usize,
    pub unique_hashes: usize,
    pub decoded_not_presented: usize,
    /// Wall time of the verifying sequential pass (decode, framemd5, zero-copy
    /// import, normalization, readback, CPU reference): not a decode speed.
    pub sequential_pass_ms: f64,
    /// Decode-only throughput: VideoToolbox with asynchronous decompression
    /// and one wait at the end; software decoders with their own threading,
    /// frames dropped as soon as they are output. No hashing, import, or
    /// readback.
    pub throughput_fps: f64,
    pub reference: Option<RefCompare>,
    /// VideoToolbox versus FFmpeg's software decoder repacked to the same layout.
    pub software_crosscheck: Option<RefCompare>,
    /// VideoToolbox versus AVFoundation (`AVAssetReader`, edit lists applied the
    /// QuickTime way), aligned by presentation time. MOV/MP4 only.
    pub avfoundation: Option<RefCompare>,
    /// Random access with a fresh decoder (new session) per probe.
    pub random: RandomReport,
    /// Random access within one session: flush (software) or idle session
    /// (VideoToolbox, asynchronous submission), as playback seeks would.
    pub random_warm: RandomReport,
    pub gpu: GpuReport,
    pub buffer: Option<BufferFacts>,
    pub checks: Vec<String>,
    pub checks_ok: bool,
    #[serde(skip)]
    pub norm_hashes: Vec<String>,
}

/// What a decoder reports for one submitted picture.
enum Event {
    Frame(Decoded),
    Failed { pts: i64, status: i32 },
}

/// One decoder behind a common interface.
enum Dec {
    Vt { plan: VtPlan, ts: i64, dec: Option<VtDecoder>, hw: bool },
    /// FFmpeg, dav1d, or libvpx; `make` opens a fresh decoder.
    Sw { make: Box<dyn Fn() -> Result<Box<dyn SwDecoder>>>, dec: Option<Box<dyn SwDecoder>>, copies: usize },
}

impl Dec {
    fn reset(&mut self) -> Result<()> {
        match self {
            Dec::Vt { plan, ts, dec, hw } => {
                *dec = None;
                let d = VtDecoder::new(plan, *ts, false)?;
                *hw = d.hardware;
                *dec = Some(d);
            }
            Dec::Sw { make, dec, copies } => {
                if let Some(d) = dec.take() {
                    *copies += d.copies();
                }
                *dec = Some(make()?);
            }
        }
        Ok(())
    }

    /// Feeds packets and drains the decoder at the end. `busy` accumulates time
    /// spent inside the decoder (not in `on`).
    fn feed(&mut self, pkts: &[Packet], on: &mut dyn FnMut(Event) -> Result<()>, busy: &mut f64) -> Result<()> {
        match self {
            Dec::Vt { dec, .. } => {
                let d = dec.as_mut().context("no session")?;
                for p in pkts {
                    let t = Instant::now();
                    let st = d.decode(p)?;
                    let frames = d.finish()?;
                    *busy += t.elapsed().as_secs_f64();
                    let mut reported = false;
                    for f in frames {
                        reported |= f.pts == p.pts;
                        match f.buffer {
                            Some(b) if f.status == 0 => on(Event::Frame(Decoded { pts: f.pts, px: Pixels::Vt(b) }))?,
                            _ => on(Event::Failed { pts: f.pts, status: f.status })?,
                        }
                    }
                    if st != 0 && !reported {
                        on(Event::Failed { pts: p.pts, status: st })?;
                    }
                }
            }
            Dec::Sw { dec, .. } => {
                let d = dec.as_mut().context("no decoder")?;
                let mut out = Vec::new();
                for p in pkts.iter().map(Some).chain([None]) {
                    let t = Instant::now();
                    d.send(p, &mut out)?;
                    *busy += t.elapsed().as_secs_f64();
                    for (pts, f) in out.drain(..) {
                        on(Event::Frame(Decoded { pts, px: Pixels::Pool(f) }))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn copies(&self) -> usize {
        match self {
            Dec::Sw { dec, copies, .. } => *copies + dec.as_ref().map(|d| d.copies()).unwrap_or(0),
            _ => 0,
        }
    }

    /// Prepares the open session for decoding from another random-access
    /// point (a seek). VideoToolbox sessions are idle after `finish`; an
    /// end-of-sequence sample drops H.264 and HEVC references (see
    /// `VtDecoder::start_sequence`). Software decoders flush.
    fn seek(&mut self) -> Result<()> {
        match self {
            Dec::Vt { dec, .. } => {
                // `S6_NO_EOS` reproduces the stale-reference failure.
                if std::env::var_os("S6_NO_EOS").is_none() {
                    let st = dec.as_mut().context("no session")?.start_sequence()?;
                    ensure!(st == 0, "end-of-sequence sample: OSStatus {st}");
                }
            }
            Dec::Sw { dec, .. } => dec.as_mut().context("no decoder")?.flush(),
        }
        Ok(())
    }

    /// Like `feed`, but VideoToolbox gets every packet asynchronously with one
    /// wait at the end (how a seek submits a GOP).
    fn feed_batch(&mut self, pkts: &[Packet], on: &mut dyn FnMut(Event) -> Result<()>, busy: &mut f64) -> Result<()> {
        match self {
            Dec::Vt { dec, .. } => {
                let d = dec.as_mut().context("no session")?;
                let t = Instant::now();
                let mut rejected = vec![];
                for p in pkts {
                    let st = d.decode_async(p)?;
                    if st != 0 {
                        rejected.push((p.pts, st));
                    }
                }
                let frames = d.finish()?;
                *busy += t.elapsed().as_secs_f64();
                for f in frames {
                    rejected.retain(|r| r.0 != f.pts);
                    match f.buffer {
                        Some(b) if f.status == 0 => on(Event::Frame(Decoded { pts: f.pts, px: Pixels::Vt(b) }))?,
                        _ => on(Event::Failed { pts: f.pts, status: f.status })?,
                    }
                }
                for (pts, status) in rejected {
                    on(Event::Failed { pts, status })?;
                }
                Ok(())
            }
            Dec::Sw { .. } => self.feed(pkts, on, busy),
        }
    }
}

pub fn build_index(path: &Path, inp: &Input) -> Result<FrameIndex> {
    let name = inp.format_name();
    let mut idx = if name.contains("mov") || name.contains("mp4") {
        crate::index::mp4::build(path)?
    } else if name.contains("matroska") || name.contains("webm") {
        crate::index::mkv::build(path)?
    } else {
        ff::scan_index(path)?
    };
    // Open GOP: mark NAL types from the sample headers (H.264 avcC, HEVC hvcC;
    // length-prefixed samples). MPEG-TS packets are marked during the scan.
    use crate::index::NalCodec;
    let config = match (&idx.sample_entry, &idx.codec_id) {
        (Some(se), _) if matches!(&se.fourcc, b"hvc1" | b"hev1") => se.atom(b"hvcC").map(|a| (NalCodec::Hevc, a.to_vec())),
        (Some(se), _) if matches!(&se.fourcc, b"avc1" | b"avc3") => se.atom(b"avcC").map(|a| (NalCodec::H264, a.to_vec())),
        (None, Some(id)) if id == "V_MPEGH/ISO/HEVC" => idx.codec_private.clone().map(|c| (NalCodec::Hevc, c)),
        (None, Some(id)) if id == "V_MPEG4/ISO/AVC" => idx.codec_private.clone().map(|c| (NalCodec::H264, c)),
        _ => None,
    };
    let length_size = |codec: NalCodec, c: &[u8]| match codec {
        NalCodec::Hevc if c.len() > 21 => Some((c[21] & 3) as usize + 1),
        NalCodec::H264 if c.len() > 4 => Some((c[4] & 3) as usize + 1),
        _ => None,
    };
    if let Some((codec, c)) = config
        && let Some(ls) = length_size(codec, &c)
    {
        let t0 = Instant::now();
        idx.mark_nal_types(path, codec, ls)?;
        idx.stats.build_ms += t0.elapsed().as_secs_f64() * 1e3;
    }
    Ok(idx)
}

fn crosscheck(path: &Path, idx: &FrameIndex) -> Result<(String, bool)> {
    let (pkts, tb) = ff::demux_scan(path)?;
    if tb != idx.timebase {
        return Ok((format!("demuxer timebase {tb} vs index {}", idx.timebase), false));
    }
    // FFmpeg's mov demuxer rebases PTS for edit lists; compare presentation order
    // differences, keyframe count, and packet count.
    let mut dp: Vec<i64> = pkts.iter().filter(|p| !p.discard).map(|p| p.pts).collect();
    dp.sort_unstable();
    let ip = idx.presented_pts();
    let off = match (dp.first(), ip.first()) {
        (Some(a), Some(b)) => a - b,
        _ => 0,
    };
    let pts_ok = dp.len() == ip.len() && dp.iter().zip(&ip).all(|(a, b)| a - off == *b);
    let dk = pkts.iter().filter(|p| p.key).count();
    let ik = idx.samples.iter().filter(|s| s.key).count();
    let ok = pts_ok && pkts.len() == idx.samples.len() && dk == ik;
    Ok((
        format!(
            "packets {} vs {}, presented {} vs {}, keyframes {} vs {}, PTS {} (offset {off})",
            pkts.len(),
            idx.samples.len(),
            dp.len(),
            ip.len(),
            dk,
            ik,
            if pts_ok { "equal" } else { "differ" }
        ),
        ok,
    ))
}

fn framemd5(ffmpeg: &Path, args: &[&str]) -> Result<Vec<String>> {
    let out = Command::new(ffmpeg).args(["-v", "error", "-nostdin"]).args(args).args(["-f", "framemd5", "-"]).output()?;
    if !out.status.success() {
        bail!("ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or(""));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| l.rsplit(',').next().map(|h| h.trim().to_string()))
        .collect())
}

/// Positional comparison, aligned on the first reference frame (a reference
/// that drops our first frames gets a positive offset).
fn compare_refs(name: String, r: Result<Vec<String>>, seq: &[String]) -> RefCompare {
    match r {
        Err(e) => RefCompare { reference: name, error: Some(e.to_string()), ..Default::default() },
        Ok(h) => {
            let offset = match h.first() {
                Some(f0) if seq.first() != Some(f0) => {
                    seq.iter().take(8).position(|s| s == f0).map(|p| p as isize).unwrap_or(0)
                }
                _ => 0,
            };
            let ours = &seq[offset as usize..];
            let n = h.len().min(ours.len());
            let matched = (0..n).filter(|&i| h[i] == ours[i]).count();
            let first = (0..h.len().max(ours.len())).find(|&i| h.get(i) != ours.get(i)).map(|i| i + offset as usize);
            RefCompare { reference: name, frames: h.len(), matched, offset, first_mismatch: first, ..Default::default() }
        }
    }
}

/// AVFoundation reference: every AVAssetReader frame is matched to the
/// presented frame showing at its presentation time.
fn compare_avf(tool: &Path, file: &Path, fourcc: &str, idx: &FrameIndex, seq: &[String]) -> RefCompare {
    let name = format!("AVFoundation AVAssetReader, {fourcc}");
    let out = match Command::new(tool).arg(file).arg(fourcc).output() {
        Ok(o) => o,
        Err(e) => return RefCompare { reference: name, error: Some(e.to_string()), ..Default::default() },
    };
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("failed").to_string();
        return RefCompare { reference: name, error: Some(e), ..Default::default() };
    }
    let pts = idx.presented_pts();
    let tb = idx.timebase;
    let mut rc = RefCompare { reference: name, ..Default::default() };
    let mut shown = vec![];
    for l in String::from_utf8_lossy(&out.stdout).lines() {
        let f: Vec<&str> = l.split_whitespace().collect();
        let (Ok(v), Ok(ts), Some(h)) = (f[0].parse::<i128>(), f[1].parse::<i128>(), f.get(2)) else { continue };
        rc.frames += 1;
        // Last presented frame with pts × tb ≤ v / ts.
        let k = pts.partition_point(|&p| p as i128 * tb.num as i128 * ts <= v * tb.den as i128).checked_sub(1);
        match k {
            Some(k) if seq[k] == *h => {
                rc.matched += 1;
                shown.push(k);
            }
            _ => {
                rc.first_mismatch.get_or_insert(rc.frames - 1);
                shown.extend(k);
            }
        }
    }
    // Each presented frame shown exactly once, in order.
    let bijective = shown.len() == pts.len() && shown.iter().enumerate().all(|(i, &k)| i == k);
    if !bijective {
        rc.note = Some(format!("AVFoundation frames map to {} of our {} frames out of order or with repeats", shown.len(), pts.len()));
        rc.first_mismatch.get_or_insert(0);
    }
    rc
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// Deterministic shuffle (xorshift64*).
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

#[derive(serde::Deserialize)]
struct FixtureEntry {
    file: String,
    #[serde(rename = "hasAlpha")]
    has_alpha: Option<bool>,
    #[serde(rename = "alphaLayout")]
    alpha_layout: Option<String>,
    #[serde(rename = "expectedColorAt")]
    c1: Option<ColorAt>,
    #[serde(rename = "expectedColorAt2")]
    c2: Option<ColorAt>,
}

#[derive(serde::Deserialize, Clone)]
struct ColorAt {
    t: f64,
    rgb: [f64; 3],
}

fn fixture_meta(path: &Path) -> Option<FixtureEntry> {
    let j = std::fs::read_to_string(path.parent()?.join("fixtures.json")).ok()?;
    let v: Vec<serde_json::Value> = serde_json::from_str(&j).ok()?;
    let name = path.file_name()?.to_str()?;
    v.into_iter().filter_map(|e| serde_json::from_value::<FixtureEntry>(e).ok()).find(|e| e.file == name)
}

/// Frames whose normalized output is kept for content checks.
enum ProbeKind {
    Rgb([f64; 3], f64),
    Alpha(String),
}

struct Probe {
    n: usize,
    kind: ProbeKind,
}

fn norm_hash(luma: &[[f32; 2]], chroma: &[[f32; 2]]) -> String {
    let mut h = Md5::new();
    for v in luma.iter().chain(chroma) {
        h.update(v[0].to_bits().to_le_bytes());
        h.update(v[1].to_bits().to_le_bytes());
    }
    format!("{:x}", h.finalize())
}

pub fn run_file(path: &Path, gpu: &Gpu, opt: &Options) -> FileReport {
    let mut rep = FileReport {
        file: path.display().to_string(),
        bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
        info: None,
        index: None,
        nominal_rate: None,
        selection: None,
        backends: vec![],
        cross_backend: vec![],
        errors: vec![],
    };
    if let Err(e) = run_file_inner(path, gpu, opt, &mut rep) {
        rep.errors.push(format!("{e:#}"));
    }
    rep
}

fn run_file_inner(path: &Path, gpu: &Gpu, opt: &Options, rep: &mut FileReport) -> Result<()> {
    let inp = Input::open(path)?;
    let info = ff::stream_info(&inp);
    rep.info = Some(info.clone());
    let extradata = ff::extradata(&inp);
    let idx = build_index(path, &inp)?;
    let (cc, cc_ok) = crosscheck(path, &idx).unwrap_or_else(|e| (format!("error: {e}"), false));
    rep.index = Some(IndexReport {
        source: idx.source,
        timebase: idx.timebase,
        stats: idx.stats.clone(),
        duplicate_pts: idx.duplicate_pts(),
        demux_crosscheck: cc,
        demux_crosscheck_ok: cc_ok,
    });
    let pts = idx.presented_pts();
    if pts.len() < 2 {
        bail!("fewer than two presented frames");
    }
    let nominal = time::nominal_rate(&pts, idx.timebase);
    rep.selection = Some(time::check_selection(&pts, idx.timebase, nominal.rate));
    rep.nominal_rate = Some(nominal.clone());

    // Content probes from fixtures.json, located by rule 7.
    let mut probes: Vec<Probe> = vec![];
    if let Some(m) = fixture_meta(path) {
        for c in [&m.c1, &m.c2].into_iter().flatten() {
            let t_den = 1_000_000i128;
            let t_num = (c.t * 1e6).round() as i128
                + pts[0] as i128 * idx.timebase.num as i128 * t_den / idx.timebase.den as i128;
            if let Some(n) = time::select_frame(&pts, idx.timebase, nominal.rate, t_num, t_den) {
                probes.push(Probe { n, kind: ProbeKind::Rgb(c.rgb, c.t) });
            }
        }
        if m.has_alpha == Some(true) {
            probes.push(Probe { n: pts.len() / 2, kind: ProbeKind::Alpha(m.alpha_layout.clone().unwrap_or_default()) });
        }
    }

    let mut backends: Vec<(String, Result<Dec, String>)> = vec![];
    match vt::plan(&info, &idx, &extradata) {
        Ok(p) => backends.push((
            "VideoToolbox".into(),
            Ok(Dec::Vt { plan: p, ts: idx.timebase.den, dec: None, hw: false }),
        )),
        Err(why) => backends.push(("VideoToolbox".into(), Err(why))),
    }
    // FFmpeg: VP8/VP9 with alpha need libvpx (section 13); AV1 uses libdav1d.
    let ff_name = match (info.codec.as_str(), info.alpha_mode) {
        ("vp9", true) => Some("libvpx-vp9".to_string()),
        ("vp8", true) => Some("libvpx".to_string()),
        ("av1", _) => Some("libdav1d".to_string()),
        _ => None,
    };
    if info.codec == "prores" {
        backends.push(("FFmpeg".into(), Err("ProRes decodes only through VideoToolbox (decision 12)".into())));
    } else {
        let ff_inp = Input::open(path)?;
        let (name, pool, threads) = (ff_name.clone(), gpu.pool.clone(), opt.threads);
        let make = move || -> Result<Box<dyn SwDecoder>> {
            Ok(Box::new(FfDecoder::new(&ff_inp, name.as_deref(), pool.clone(), threads)?))
        };
        backends.push(("FFmpeg".into(), Ok(Dec::Sw { make: Box::new(make), dec: None, copies: 0 })));
    }
    // FFmpeg's libdav1d and libvpx wrappers ignore get_buffer2; the direct
    // sessions decode into the pool without a copy.
    if info.codec == "av1" {
        let (pool, threads) = (gpu.pool.clone(), opt.threads);
        let make = move || -> Result<Box<dyn SwDecoder>> { Ok(Box::new(Dav1dDecoder::new(pool.clone(), threads)?)) };
        backends.push(("dav1d".into(), Ok(Dec::Sw { make: Box::new(make), dec: None, copies: 0 })));
    }
    if matches!(info.codec.as_str(), "vp8" | "vp9") && info.alpha_mode {
        let codec = if info.codec == "vp9" { VpxCodec::Vp9 } else { VpxCodec::Vp8 };
        let pool = MetalPool::new_zeroed(gpu.pool.device());
        let threads = opt.threads.max(0) as u32;
        // Probe once: libvpx supports external frame buffers for VP9 only.
        match VpxDecoder::new(codec, pool.clone(), true, threads) {
            Ok(_) => {
                let make = move || -> Result<Box<dyn SwDecoder>> {
                    Ok(Box::new(VpxDecoder::new(codec, pool.clone(), true, threads)?))
                };
                backends.push(("libvpx".into(), Ok(Dec::Sw { make: Box::new(make), dec: None, copies: 0 })));
            }
            Err(e) => backends.push(("libvpx".into(), Err(format!("{e:#}; two FFmpeg native decoders are the zero-copy path")))),
        }
        // Color and alpha through two FFmpeg native decoders (direct rendering).
        let alpha_inp = Input::open(path)?;
        let (pool, threads) = (gpu.pool.clone(), opt.threads);
        let make = move || -> Result<Box<dyn SwDecoder>> { Ok(Box::new(FfAlphaDecoder::new(&alpha_inp, pool.clone(), threads)?)) };
        backends.push(("FFmpeg native + alpha".into(), Ok(Dec::Sw { make: Box::new(make), dec: None, copies: 0 })));
    }

    for (name, dec) in backends {
        let mut br = BackendReport { backend: name.clone(), ..Default::default() };
        match dec {
            Err(why) => br.detail = why,
            Ok(mut dec) => {
                if let Err(e) = run_backend(path, gpu, opt, &idx, &info, &nominal, &mut dec, &probes, ff_name.as_deref(), &mut br) {
                    br.error = Some(format!("{e:#}"));
                }
                br.ok = br.error.is_none()
                    && br.random.matched == br.random.probes
                    && br.random.selection_mismatches.is_empty()
                    && br.random_warm.matched == br.random_warm.probes
                    && br.random_warm.probes == br.random.probes
                    && br.frames == idx.order.len()
                    && br.avfoundation.as_ref().is_none_or(|a| a.ok(br.frames));
                br.zero_copy = opt.gpu && br.error.is_none() && br.gpu.frames == br.frames && br.gpu.aliased == br.gpu.frames;
            }
        }
        rep.backends.push(br);
    }

    let hashed: Vec<&BackendReport> =
        rep.backends.iter().filter(|b| !b.norm_hashes.is_empty() && b.norm_hashes.len() == idx.order.len()).collect();
    if let [a, rest @ ..] = &hashed[..] {
        for b in rest {
            let matched = a.norm_hashes.iter().zip(&b.norm_hashes).filter(|(x, y)| x == y).count();
            let first = a.norm_hashes.iter().zip(&b.norm_hashes).position(|(x, y)| x != y);
            rep.cross_backend.push(RefCompare {
                reference: format!("{} vs {} normalized Y'CbCrA (f32 bits)", a.backend, b.backend),
                frames: idx.order.len(),
                matched,
                first_mismatch: first,
                ..Default::default()
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_backend(
    path: &Path,
    gpu: &Gpu,
    opt: &Options,
    idx: &FrameIndex,
    info: &StreamInfo,
    nominal: &NominalRate,
    dec: &mut Dec,
    probes: &[Probe],
    ff_name: Option<&str>,
    br: &mut BackendReport,
) -> Result<()> {
    let n = idx.order.len();
    let pos: HashMap<i64, usize> = idx.order.iter().enumerate().map(|(i, &s)| (idx.samples[s].pts, i)).collect();
    let mut reader = PacketReader::new(path)?;

    // Sequential pass: every sample in decode order, hashing and importing each
    // presented frame.
    dec.reset()?;
    let mut hashes: Vec<Option<String>> = vec![None; n];
    let mut norms: Vec<Option<String>> = vec![None; n];
    let mut not_presented = 0;
    let mut compare = Compare::default();
    let mut import_us = vec![];
    let mut norm_us = vec![];
    let mut aliased = 0;
    let mut gpu_frames = 0;
    let mut formats = String::new();
    let mut buffer = None;
    let mut kept: HashMap<usize, (FrameDesc, Vec<[f32; 2]>, Vec<[f32; 2]>)> = HashMap::new();
    let mut busy = 0.0;
    let all_idx: Vec<usize> = (0..idx.samples.len()).collect();
    let all = reader.read(idx, &all_idx)?;
    {
        let mut on = |ev: Event| -> Result<()> {
            let d = match ev {
                Event::Frame(d) => d,
                Event::Failed { pts, status } => bail!("sequential decode failed at pts {pts}: status {status}"),
            };
            let Some(&i) = pos.get(&d.pts) else {
                not_presented += 1;
                return Ok(());
            };
            if hashes[i].is_some() {
                bail!("frame {i} (pts {}) decoded twice", d.pts);
            }
            hashes[i] = Some(d.md5()?);
            if let (Pixels::Vt(pb), None) = (&d.px, &buffer) {
                buffer = Some(vt::buffer_facts(pb, 0));
            }
            if opt.gpu {
                let (imp, out) = gpu.import_and_normalize(&d)?;
                let (rl, rc) = d.reference()?;
                compare.add(&out.luma, &rl);
                compare.add(&out.chroma, &rc);
                norms[i] = Some(norm_hash(&rl, &rc));
                import_us.push(out.timing.import_us);
                norm_us.push(out.timing.normalize_us);
                gpu_frames += 1;
                if imp.aliased {
                    aliased += 1;
                }
                if formats.is_empty() {
                    formats = format!(
                        "{}: {}",
                        imp.desc.label,
                        imp.desc.planes.iter().map(|p| format!("{:?} {}x{}", p.format, p.width, p.height)).collect::<Vec<_>>().join(", ")
                    );
                }
                if probes.iter().any(|p| p.n == i) {
                    kept.insert(i, (imp.desc.clone(), out.luma, out.chroma));
                }
            }
            Ok(())
        };
        let t0 = Instant::now();
        dec.feed(&all, &mut on, &mut busy)?;
        br.sequential_pass_ms = t0.elapsed().as_secs_f64() * 1e3;
    }
    drop(all);
    let missing: Vec<usize> = (0..n).filter(|&i| hashes[i].is_none()).collect();
    if !missing.is_empty() {
        bail!("sequential decode missed {} of {n} frames (first {})", missing.len(), missing[0]);
    }
    let seq: Vec<String> = hashes.into_iter().map(|h| h.unwrap()).collect();
    if opt.gpu {
        br.norm_hashes = norms.into_iter().map(|h| h.unwrap()).collect();
    }
    br.frames = n;
    br.unique_hashes = seq.iter().collect::<std::collections::HashSet<_>>().len();
    br.decoded_not_presented = not_presented;
    if let (Some(b), Dec::Vt { plan, .. }) = (&mut buffer, &*dec) {
        b.matches_request = b.pixel_format == plan.pixel_format;
    }
    br.buffer = buffer;

    match &*dec {
        Dec::Vt { plan, hw, .. } => {
            br.hardware = Some(*hw);
            br.detail = format!(
                "{} -> {}{}{}; format from {}: {}",
                plan.codec_type,
                plan.pixel_format,
                if plan.alpha_layer { " (HEVC alpha layer)" } else { "" },
                if plan.annexb { ", Annex B rewritten" } else { "" },
                plan.format_source,
                plan.format_extensions
            );
        }
        Dec::Sw { dec: Some(d), .. } => {
            br.detail = format!("{} ({})", d.name(), if d.direct() { "renders into the pool" } else { "own buffers, copied" });
        }
        _ => {}
    }

    // Decode-only throughput.
    {
        let pkts = reader.read(idx, &all_idx)?;
        dec.reset()?;
        let (frames, secs) = match dec {
            Dec::Vt { dec: Some(d), .. } => {
                let (n, failed, secs) = d.throughput(&pkts)?;
                if failed > 0 {
                    bail!("{failed} frames failed in the throughput pass");
                }
                (n, secs)
            }
            _ => {
                let mut n = 0;
                let mut busy = 0.0;
                let t0 = Instant::now();
                dec.feed(&pkts, &mut |_| { n += 1; Ok(()) }, &mut busy)?;
                (n, t0.elapsed().as_secs_f64())
            }
        };
        if frames < n {
            bail!("throughput pass output {frames} of {n} frames");
        }
        br.throughput_fps = frames as f64 / secs;
    }

    // References.
    if opt.references {
        let file = path.to_str().unwrap();
        match &*dec {
            Dec::Vt { plan, .. } => {
                let fmt = match plan.format_fcc {
                    vt::F_420V | vt::F_420F => Some("nv12"),
                    vt::F_X420 | vt::F_XF20 => Some("p010le"),
                    vt::F_X422 | vt::F_XF22 => Some("p210le"),
                    vt::F_Y416 => Some("ayuv64le"),
                    _ => None,
                };
                if let Some(fmt) = fmt {
                    let vf = format!("hwdownload,format={fmt}");
                    br.reference = Some(compare_refs(
                        format!("ffmpeg -hwaccel videotoolbox, {fmt}"),
                        framemd5(
                            &opt.ffmpeg,
                            &["-hwaccel", "videotoolbox", "-hwaccel_output_format", "videotoolbox_vld", "-i", file, "-map", "0:v:0", "-vf", &vf, "-fps_mode", "passthrough"],
                        ),
                        &seq,
                    ));
                    if info.codec != "prores" {
                        let mut args: Vec<&str> = vec![];
                        if let Some(n) = ff_name {
                            args.extend(["-c:v", n]);
                        }
                        args.extend(["-i", file, "-map", "0:v:0", "-pix_fmt", fmt, "-fps_mode", "passthrough"]);
                        br.software_crosscheck =
                            Some(compare_refs(format!("ffmpeg software decode as {fmt}"), framemd5(&opt.ffmpeg, &args), &seq));
                    }
                } else {
                    br.reference = Some(RefCompare {
                        reference: "ffmpeg".into(),
                        error: Some(format!("FFmpeg cannot produce {} (alpha layer)", plan.pixel_format)),
                        ..Default::default()
                    });
                }
                if let (Some(tool), Some(_)) = (&opt.avf, &idx.sample_entry) {
                    br.avfoundation = Some(compare_avf(tool, path, &plan.pixel_format, idx, &seq));
                }
            }
            Dec::Sw { .. } => {
                let mut args: Vec<&str> = vec![];
                if let Some(n) = ff_name {
                    args.extend(["-c:v", n]);
                }
                args.extend(["-i", file, "-map", "0:v:0", "-fps_mode", "passthrough"]);
                br.reference = Some(compare_refs("ffmpeg software framemd5 (native format)".into(), framemd5(&opt.ffmpeg, &args), &seq));
            }
        }
    }

    // Random access: composition frames in shuffled order. Rule 7 picks the
    // source frame for each, which is decoded from its random-access point and
    // compared with the sequential decode of that frame: first with a fresh
    // decoder per probe, then within one session.
    br.random = random_pass(idx, nominal, dec, &mut reader, &seq, opt.random_limit, false)?;
    br.random_warm = random_pass(idx, nominal, dec, &mut reader, &seq, opt.random_limit, true)?;

    br.gpu = GpuReport {
        frames: gpu_frames,
        aliased,
        cpu_copies: dec.copies(),
        compare,
        import_us_p50: pct(&mut import_us.clone(), 0.5),
        import_us_p95: pct(&mut import_us, 0.95),
        normalize_us_p50: pct(&mut norm_us, 0.5),
        plane_formats: formats,
    };

    content_checks(info, probes, &kept, br);
    Ok(())
}

/// One random-access pass over the composition frames (shuffled, the same
/// order for both passes). `warm` keeps one decoder open and seeks within it.
#[allow(clippy::too_many_arguments)]
fn random_pass(
    idx: &FrameIndex,
    nominal: &NominalRate,
    dec: &mut Dec,
    reader: &mut PacketReader,
    seq: &[String],
    limit: Option<usize>,
    warm: bool,
) -> Result<RandomReport> {
    let mut r = RandomReport::default();
    let tb = idx.timebase;
    let pts = idx.presented_pts();
    let grid = time::grid(&pts, tb, nominal.rate);
    let comp = (*grid.last().unwrap() + 1) as usize;
    let order = shuffled(comp, 0x5eed_0006 ^ comp as u64);
    let probes_n = limit.map(|l| l.min(comp)).unwrap_or(comp);
    let trace = std::env::var_os("S6_TRACE").is_some();
    let mut times = vec![];
    let mut fed = vec![];
    if warm {
        dec.reset()?;
    }
    for &cn in order.iter().take(probes_n) {
        r.probes += 1;
        let (tn, td) = time::frame_time(pts[0], tb, nominal.rate, cn as i64);
        let Some(target) = time::select_frame(&pts, tb, nominal.rate, tn, td) else {
            r.missing.push(cn);
            continue;
        };
        if Some(target) != time::expected_frame(&grid, cn as i64) {
            r.selection_mismatches.push(cn);
        }
        let plan = idx.decode_plan(target);
        r.rasl_skipped += idx.decode_range(target).count() - plan.len();
        let want = idx.samples[idx.order[target]].pts;
        let t0 = Instant::now();
        let pkts = reader.read(idx, &plan)?;
        if warm {
            dec.seek()?;
        } else {
            dec.reset()?;
        }
        let mut got: Option<String> = None;
        let mut leading = 0;
        let mut busy = 0.0;
        let mut hit_ms = 0.0;
        if trace {
            eprintln!("probe{} comp {cn} -> frame {target} (pts {want}), samples {plan:?}", if warm { " (warm)" } else { "" });
        }
        let mut on = |ev: Event| -> Result<()> {
            if trace {
                match &ev {
                    Event::Frame(d) => eprintln!("  frame pts {}", d.pts),
                    Event::Failed { pts, status } => eprintln!("  failed pts {pts} status {status}"),
                }
            }
            match ev {
                Event::Frame(d) if d.pts == want && got.is_none() => {
                    hit_ms = t0.elapsed().as_secs_f64() * 1e3;
                    got = Some(d.md5()?);
                }
                Event::Frame(_) => {}
                Event::Failed { pts, .. } if pts == want => {}
                Event::Failed { .. } => leading += 1,
            }
            Ok(())
        };
        if warm {
            dec.feed_batch(&pkts, &mut on, &mut busy)?;
        } else {
            dec.feed(&pkts, &mut on, &mut busy)?;
        }
        r.leading_failures += leading;
        times.push(hit_ms);
        fed.push(pkts.len());
        match got {
            Some(h) if h == seq[target] => r.matched += 1,
            Some(h) => {
                if trace {
                    eprintln!(
                        "mismatch{}: comp {cn} frame {target} pts {want} plan {plan:?}; hash equals sequential frame {:?}",
                        if warm { " (warm)" } else { "" },
                        seq.iter().position(|s| *s == h)
                    );
                }
                r.mismatched.push(cn)
            }
            None => r.missing.push(cn),
        }
    }
    r.mean_packets = fed.iter().sum::<usize>() as f64 / fed.len().max(1) as f64;
    r.max_packets = fed.iter().copied().max().unwrap_or(0);
    r.mean_ms = times.iter().sum::<f64>() / times.len().max(1) as f64;
    r.p50_ms = pct(&mut times, 0.5);
    r.p95_ms = pct(&mut times, 0.95);
    r.max_ms = pct(&mut times, 1.0);
    r.mismatched.truncate(20);
    r.missing.truncate(20);
    r.selection_mismatches.truncate(20);
    Ok(r)
}

/// Content checks on the normalized GPU output of the fixture probes: the
/// expected color in the left half, and alpha in both halves.
fn content_checks(
    info: &StreamInfo,
    probes: &[Probe],
    kept: &HashMap<usize, (FrameDesc, Vec<[f32; 2]>, Vec<[f32; 2]>)>,
    br: &mut BackendReport,
) {
    br.checks_ok = true;
    for p in probes {
        let Some((d, luma, chroma)) = kept.get(&p.n) else { continue };
        let (w, h) = (d.width, d.height);
        let at = |fx: f64, fy: f64| {
            let x = ((w as f64 * fx) as usize).min(w - 1);
            let y = ((h as f64 * fy) as usize).min(h - 1);
            let cw = (w + (1 << d.log2_cw) - 1) >> d.log2_cw;
            let c = chroma[(y >> d.log2_ch) * cw + (x >> d.log2_cw)];
            (luma[y * w + x], c)
        };
        match &p.kind {
            ProbeKind::Rgb(rgb, t) => {
                let exp = rgb.map(|v| v / 255.0);
                let ([y, _], [cb, cr]) = at(0.25, 0.5);
                let (y, cb, cr) = (y as f64, cb as f64, cr as f64);
                // The tagged matrix, else section 9's default for untagged video
                // (BT.601 at heights up to 576, BT.709 above).
                let (matrix, kr, kb) = match info.color_space.as_str() {
                    "bt709" => ("BT.709 (tagged)", 0.2126, 0.0722),
                    "bt470bg" | "smpte170m" => ("BT.601 (tagged)", 0.299, 0.114),
                    _ if h <= 576 => ("BT.601 (untagged, section 9 default)", 0.299, 0.114),
                    _ => ("BT.709 (untagged, section 9 default)", 0.2126, 0.0722),
                };
                let to_rgb = |kr: f64, kb: f64| {
                    let kg = 1.0 - kr - kb;
                    let r = y + 2.0 * (1.0 - kr) * cr;
                    let b = y + 2.0 * (1.0 - kb) * cb;
                    let g = (y - kr * r - kb * b) / kg;
                    [r, g, b]
                };
                let err = |c: [f64; 3]| (0..3).map(|i| (c[i] - exp[i]).abs()).fold(0.0, f64::max);
                let c = to_rgb(kr, kb);
                let e = err(c);
                let ok = e < 0.06;
                br.checks_ok &= ok;
                let mut note = String::new();
                if !ok && info.color_space == "unknown" {
                    let e709 = err(to_rgb(0.2126, 0.0722));
                    note = format!(" (decoded as BT.709 the error would be {e709:.3}: the file is untagged)");
                }
                br.checks.push(format!(
                    "frame {}: expected rgb {:?} at t={t} -> {matrix} rgb ({:.3}, {:.3}, {:.3}), max error {e:.3} {}{note}",
                    p.n,
                    rgb,
                    c[0],
                    c[1],
                    c[2],
                    if ok { "ok" } else { "FAIL" }
                ));
            }
            ProbeKind::Alpha(layout) => {
                let (l, _) = at(0.25, 0.5);
                let (r, _) = at(0.75, 0.5);
                let has_alpha = d.alpha != crate::frame::AlphaSrc::None;
                let ok = has_alpha && l[1] > 0.95 && r[1] < 0.05;
                br.checks_ok &= ok;
                br.checks.push(format!(
                    "frame {}: alpha {layout} -> left {:.4}, right {:.4} {}",
                    p.n,
                    l[1],
                    r[1],
                    if ok { "ok" } else if has_alpha { "FAIL" } else { "FAIL (no alpha decoded)" }
                ));
            }
        }
    }
}
