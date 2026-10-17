//! Random-seek and drag timing on one file through the decode service, as the
//! viewer asks for frames (docs/engine/scrub.md). The seek sequence is the
//! one `e2e/native-engine-tracer-measure.spec.ts` uses (seed 7, the same
//! generator), so the numbers compare with the app's.
//!
//!   cargo run --release -p pm-media --example seek_bench -- FILE [--seeks 60] [--fresh] [--gop]
//!
//! `--fresh` opens a new session per seek; `--gop` prints the stream's
//! picture structure (reference and non-reference pictures per GOP).

use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::vt::DecodePurpose;
use std::time::Instant;

fn dist(xs: &mut [f64]) -> String {
    if xs.is_empty() {
        return "n 0".into();
    }
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |p: f64| xs[((xs.len() - 1) as f64 * p + 1e-9).floor() as usize];
    format!(
        "n {} p50 {:.1} p95 {:.1} max {:.1} ms",
        xs.len(),
        at(0.5),
        at(0.95),
        at(1.0)
    )
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = std::path::PathBuf::from(args.first().expect("FILE"));
    let mut seeks = 60usize;
    let mut fresh = false;
    let mut gop = false;
    let mut skip = true;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--seeks" => seeks = it.next().unwrap().parse().unwrap(),
            "--fresh" => fresh = true,
            "--gop" => gop = true,
            "--no-skip" => skip = false,
            other => panic!("unknown argument {other}"),
        }
    }
    let t = Instant::now();
    let src = MediaSource::open(&path).unwrap_or_else(|e| panic!("{e}"));
    println!(
        "open {:.1} ms: {} samples, {} sync",
        t.elapsed().as_secs_f64() * 1e3,
        src.index.samples.len(),
        src.index.samples.iter().filter(|s| s.key).count()
    );
    if gop {
        structure(&src, &path);
    }
    let svc = DecodeService::new(ServiceConfig {
        cache_bytes: 0,
        ..Default::default()
    });
    // The spec's generator: frames inside the clip and a 30 s composition.
    let frames = ((src.pts.len() as f64).min(30.0 * 30000.0 / 1001.0)).floor() as usize - 2;
    let mut seed = 7.0f64;
    let mut random = || {
        seed = (seed * 1103515245.0 + 12345.0) % 2147483648.0;
        seed / 2147483648.0
    };
    let mut lat = vec![];
    let mut first = None;
    {
        let mut guard = Some(svc.session(&src, 1, DecodePurpose::Display).unwrap());
        for i in 0..seeks {
            let k = 1 + (random() * frames as f64).floor() as usize;
            if fresh {
                guard = None;
                svc.release(Some(1), None);
            }
            let t = Instant::now();
            let s =
                guard.get_or_insert_with(|| svc.session(&src, 1, DecodePurpose::Display).unwrap());
            s.set_skip_disposable(skip);
            let before = s.stats();
            let f = s.frame(k).unwrap_or_else(|e| panic!("frame {k}: {e}"));
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let st = s.stats();
            drop(f);
            if i == 0 {
                first = Some(ms);
            }
            lat.push(ms);
            println!(
                "seek {i:2} frame {k:4}: {ms:6.1} ms fed {:3} skipped {:3} read {:5.1} send {:6.1} wait {:6.1} ms out {:3}",
                st.samples_fed - before.samples_fed,
                st.disposable_skipped - before.disposable_skipped,
                (st.read_us - before.read_us) as f64 / 1e3,
                (st.send_us - before.send_us) as f64 / 1e3,
                (st.wait_us - before.wait_us) as f64 / 1e3,
                st.frames_out - before.frames_out,
            );
        }
    }
    println!("first seek {:.1} ms", first.unwrap_or(0.0));
    println!("random seek: {}", dist(&mut lat));
    // A forward drag one frame at a time (the spec's drag), from frame 1.
    let mut s = svc.session(&src, 2, DecodePurpose::Display).unwrap();
    s.set_skip_disposable(skip);
    let mut step = vec![];
    let t0 = Instant::now();
    for k in 1..=360usize.min(frames) {
        let t = Instant::now();
        let f = s.frame(k).unwrap();
        drop(f);
        step.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "forward steps: {} total {:.0} ms",
        dist(&mut step),
        t0.elapsed().as_secs_f64() * 1e3
    );
    drop(s);
    drag(&src, frames, 1, "forward drag, 1 frame per refresh");
    drag(&src, frames, 7, "forward drag, 7 frames per refresh");
    drag(&src, frames, -1, "reverse drag, 1 frame per refresh");
    drag(&src, frames, -7, "reverse drag, 7 frames per refresh");
}

/// A playhead drag through the viewer's decode worker: a scrub request at
/// every 120 Hz refresh for 3 s, `step` frames apart, with 3 ms of
/// simulated drawing per refresh. Counts the distinct exact frames drawn.
fn drag(src: &std::sync::Arc<MediaSource>, frames: usize, step: i64, what: &str) {
    use pm_media::decode::scrub::{ViewerDecode, Want};
    let svc = std::sync::Arc::new(DecodeService::new(ServiceConfig {
        cache_bytes: 512 << 20,
        ..Default::default()
    }));
    let viewer = ViewerDecode::new(svc);
    let start = if step > 0 { 1 } else { frames as i64 - 1 };
    // The first frame, as a click before the drag.
    viewer.fetch(src, 1, start as usize, Want::Exact).unwrap();
    let period = std::time::Duration::from_micros(8_333);
    let t0 = Instant::now();
    let mut k = start;
    let mut asked = 0;
    let mut exact = std::collections::BTreeSet::new();
    let mut stand_ins = 0;
    let mut tick = 0u32;
    let mut lag = vec![];
    let mut requested = std::collections::HashMap::new();
    while t0.elapsed() < std::time::Duration::from_secs(3) {
        tick += 1;
        k = (k + step).rem_euclid(frames as i64 - 1).max(1);
        asked += 1;
        requested.entry(k as usize).or_insert(t0.elapsed());
        let f = viewer.fetch(src, 1, k as usize, Want::Scrub).unwrap();
        if f.exact {
            if exact.insert(f.shown)
                && let Some(at) = requested.get(&f.shown)
            {
                lag.push((t0.elapsed() - *at).as_secs_f64() * 1e3);
            }
        } else {
            stand_ins += 1;
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        let next = period * tick;
        if let Some(wait) = next.checked_sub(t0.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    println!(
        "{what}: asked {asked}, exact frames drawn {}, stand-ins {stand_ins}, request to exact {}; {:?}",
        exact.len(),
        dist(&mut lag),
        viewer.stats()
    );
}

/// Pictures per GOP: sync, unreferenced (disposable), other references.
fn structure(src: &MediaSource, path: &std::path::Path) {
    let idx = &src.index;
    let Some(d) = pm_media::decode::skip::Disposable::for_index(idx) else {
        println!("no NAL classifier for this stream");
        return;
    };
    let mut reader = pm_media::ff::PacketReader::new(path, src.stream).unwrap();
    let all: Vec<usize> = (0..idx.samples.len()).collect();
    let mut gops: Vec<(usize, usize, usize)> = vec![];
    for chunk in all.chunks(64) {
        for (p, &k) in reader.read(idx, chunk).unwrap().iter().zip(chunk) {
            if idx.samples[k].key {
                gops.push((0, 0, 0));
            }
            let g = gops.last_mut().unwrap();
            g.0 += 1;
            if d.disposable(&p.data) {
                g.1 += 1;
            } else {
                g.2 += 1;
            }
        }
    }
    for (i, (n, disp, refs)) in gops.iter().enumerate() {
        println!("gop {i}: {n} samples, {disp} unreferenced, {refs} referenced");
    }
}
