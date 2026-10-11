//! `s6 [--json out.json] [--random-limit N] [--no-gpu] [--no-refs] [--no-avf]
//!     [--threads N] FILE|DIR...`
//! `s6 --dump-index FILE`
//!
//! Runs the S6 decode matrix and prints a per-file, per-backend summary. Exits
//! non-zero when any backend fails its decode checks.

use anyhow::{Result, bail};
use s6_decode::gpu::Gpu;
use s6_decode::run::{self, Options, RefCompare};
use std::path::{Path, PathBuf};
use std::process::Command;

const VIDEO_EXT: [&str; 9] = ["mp4", "mov", "mkv", "webm", "ts", "m2ts", "mts", "m4v", "mxf"];

/// Compiles `scripts/avf-framemd5.swift` into the target directory when it is
/// missing or older than the script.
fn avf_tool() -> Result<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = root.join("scripts/avf-framemd5.swift");
    let out = root.join("target/avf/avf-framemd5");
    let stale = match (std::fs::metadata(&src), std::fs::metadata(&out)) {
        (Ok(s), Ok(o)) => s.modified()? > o.modified()?,
        (Ok(_), Err(_)) => true,
        (Err(e), _) => bail!("{}: {e}", src.display()),
    };
    if stale {
        std::fs::create_dir_all(out.parent().unwrap())?;
        let st = Command::new("swiftc").args(["-O", "-suppress-warnings"]).arg(&src).arg("-o").arg(&out).status()?;
        if !st.success() {
            bail!("swiftc failed for {}", src.display());
        }
    }
    Ok(out)
}

fn dump_index(path: &Path) -> Result<()> {
    let inp = s6_decode::ff::Input::open(path)?;
    let idx = run::build_index(path, &inp)?;
    println!("source {:?} timebase {} stats {:?}", idx.source, idx.timebase, idx.stats);
    for (i, s) in idx.samples.iter().enumerate() {
        println!(
            "{i:5} dts {:>10} pts {:>10} dur {:>6} key {} presented {} nal {:?} {:?}",
            s.dts.map(|d| d.to_string()).unwrap_or("-".into()),
            s.pts,
            s.duration,
            s.key as u8,
            s.presented as u8,
            s.nal_type,
            s.loc
        );
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut opt = Options::default();
    let mut json: Option<PathBuf> = None;
    let mut inputs: Vec<PathBuf> = vec![];
    let mut avf = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--json" => json = args.next().map(PathBuf::from),
            "--random-limit" => opt.random_limit = args.next().and_then(|v| v.parse().ok()),
            "--no-gpu" => opt.gpu = false,
            "--no-refs" => opt.references = false,
            "--no-avf" => avf = false,
            "--threads" => opt.threads = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--dump-index" => {
                let Some(f) = args.next() else { bail!("--dump-index FILE") };
                return dump_index(Path::new(&f));
            }
            _ => inputs.push(PathBuf::from(a)),
        }
    }
    if avf && opt.references {
        match avf_tool() {
            Ok(t) => opt.avf = Some(t),
            Err(e) => eprintln!("AVFoundation reference unavailable: {e:#}"),
        }
    }
    let mut files = vec![];
    for i in inputs {
        if i.is_dir() {
            let mut v: Vec<PathBuf> = std::fs::read_dir(&i)?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| VIDEO_EXT.contains(&e)))
                .collect();
            v.sort();
            files.extend(v);
        } else {
            files.push(i);
        }
    }
    // FFmpeg's own log is noise here (decode failures surface as return codes);
    // S6_FFMPEG_LOG=1 restores it.
    if std::env::var_os("S6_FFMPEG_LOG").is_none() {
        unsafe { ffmpeg_sys_the_third::av_log_set_level(ffmpeg_sys_the_third::AV_LOG_QUIET) };
    }
    let gpu = Gpu::new()?;
    eprintln!("GPU: {}", gpu.adapter);
    let mut reports = vec![];
    for f in &files {
        let t = std::time::Instant::now();
        let r = run::run_file(f, &gpu, &opt);
        print_summary(&r);
        println!("   ({:.1} s)", t.elapsed().as_secs_f64());
        reports.push(r);
    }
    let (pool_slots, pool_bytes) = gpu.pool.stats();
    println!("pool: {pool_slots} slots, {:.1} MiB", pool_bytes as f64 / 1048576.0);
    if let Some(j) = json {
        std::fs::write(
            &j,
            serde_json::to_string_pretty(&serde_json::json!({
                "adapter": gpu.adapter,
                "files": reports,
            }))?,
        )?;
    }
    let failed = reports
        .iter()
        .any(|r| !r.errors.is_empty() || r.backends.iter().any(|b| b.error.is_some() || (b.frames > 0 && !b.ok)));
    std::process::exit(if failed { 1 } else { 0 });
}

fn print_ref(label: &str, rc: &RefCompare, frames: usize) {
    match &rc.error {
        Some(e) => println!("      {label}: {} -> error {e}", rc.reference),
        None => println!(
            "      {label}: {} -> {}/{} match (ref frames {}{}), first mismatch {:?}{}",
            rc.reference,
            rc.matched,
            frames,
            rc.frames,
            if rc.offset != 0 { format!(", aligned at our frame {}", rc.offset) } else { String::new() },
            rc.first_mismatch,
            rc.note.as_ref().map(|n| format!(" [{n}]")).unwrap_or_default()
        ),
    }
}

fn print_summary(r: &run::FileReport) {
    let name = Path::new(&r.file).file_name().unwrap().to_string_lossy();
    let info = r
        .info
        .as_ref()
        .map(|i| format!("{} {} {}x{} {} {} matrix {}", i.container, i.codec, i.width, i.height, i.pix_fmt, i.color_range, i.color_space))
        .unwrap_or_default();
    println!("== {name}: {info}");
    if let Some(ix) = &r.index {
        println!(
            "   index {:?} tb {} samples {} presented {} sync {} build {:.2} ms; demux check: {}{}",
            ix.source,
            ix.timebase,
            ix.stats.samples,
            ix.stats.presented,
            ix.stats.sync_samples,
            ix.stats.build_ms,
            ix.demux_crosscheck,
            if ix.demux_crosscheck_ok { "" } else { "  <-- differs" }
        );
        if let Some(e) = &ix.stats.edit_list {
            println!("   edit list: {e}");
        }
        if let (Some(c), Some(m)) = (ix.stats.cue_points, ix.stats.cue_points_matched) {
            println!("   cues: {m}/{c} matched scanned keyframes; header reads {}", ix.stats.scan_reads);
        }
        if let Some(o) = &ix.stats.open_gop {
            println!("   open GOP: {o}");
        }
    }
    if let (Some(n), Some(s)) = (&r.nominal_rate, &r.selection) {
        println!(
            "   nominal rate {} ({}, candidates {:?}); {} composition frames, holes {}, doubled {}; rule 7 mismatches {}, naive rule {}",
            n.rate,
            n.method,
            n.candidates.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
            s.composition_frames,
            s.holes,
            s.doubled,
            s.rule7_mismatches,
            s.naive_mismatches
        );
    }
    for e in &r.errors {
        println!("   ERROR {e}");
    }
    for b in &r.backends {
        if b.frames == 0 && b.error.is_none() {
            println!("   [{}] not used: {}", b.backend, b.detail);
            continue;
        }
        println!(
            "   [{}] {} hw={:?} decode {} zero-copy {}{}",
            b.backend,
            b.detail,
            b.hardware,
            if b.ok { "PASS" } else { "FAIL" },
            if b.zero_copy { "yes" } else { "NO" },
            if b.checks.is_empty() { "" } else if b.checks_ok { " content ok" } else { " content FAIL" }
        );
        if let Some(e) = &b.error {
            println!("      error: {e}");
        }
        let rnd = &b.random;
        println!(
            "      frames {} unique {} | seq {:.1} ms ({:.0} fps) | random {}/{} (mismatch {:?}, missing {:?}, selection {:?}; leading failures {}, RASL skipped {}) mean {:.2} p95 {:.2} max {:.2} ms, {:.1} pkts avg",
            b.frames,
            b.unique_hashes,
            b.sequential_decode_ms,
            b.sequential_fps,
            rnd.matched,
            rnd.probes,
            rnd.mismatched,
            rnd.missing,
            rnd.selection_mismatches,
            rnd.leading_failures,
            rnd.rasl_skipped,
            rnd.mean_ms,
            rnd.p95_ms,
            rnd.max_ms,
            rnd.mean_packets
        );
        for (label, rc) in [("ref", &b.reference), ("sw", &b.software_crosscheck), ("avf", &b.avfoundation)] {
            if let Some(rc) = rc {
                print_ref(label, rc, b.frames);
            }
        }
        let g = &b.gpu;
        println!(
            "      gpu: {} | frames {} aliased {} cpu copies {} | GPU vs CPU exact {}/{} max |d| {:e} max ulps {} | import p50 {:.0} us p95 {:.0} us, normalize+readback p50 {:.0} us",
            g.plane_formats,
            g.frames,
            g.aliased,
            g.cpu_copies,
            g.compare.exact,
            g.compare.values,
            g.compare.max_abs,
            g.compare.max_ulps,
            g.import_us_p50,
            g.import_us_p95,
            g.normalize_us_p50
        );
        if let Some(bf) = &b.buffer {
            println!("      buffer: {} (request match {}) iosurface {} planes {}", bf.pixel_format, bf.matches_request, bf.iosurface, bf.planes);
        }
        for c in &b.checks {
            println!("      check: {c}");
        }
    }
    if let Some(c) = &r.cross_backend {
        print_ref("cross-backend", c, c.frames);
    }
}
