//! Prints a file's probe as JSON, or decodes presented frames and prints
//! their MD5 (framemd5 layout) and the session's counters.
//!
//!   cargo run -p pm-media --example inspect -- FILE                 # probe JSON
//!   cargo run -p pm-media --example inspect -- FILE 12 3 40 [--backend ffmpeg|vt|dav1d|alpha]

use pm_media::decode::Backend;
use pm_media::decode::service::{DecodeService, MediaSource, ServiceConfig};
use pm_media::vt::DecodePurpose;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: inspect FILE [FRAME...] [--backend ffmpeg|vt|dav1d|alpha] [--export]");
        std::process::exit(2);
    };
    let path = std::path::Path::new(path);
    let mut frames = vec![];
    let mut backend = None;
    let mut purpose = DecodePurpose::Display;
    let mut it = args[1..].iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--backend" => {
                backend = Some(match it.next().map(String::as_str) {
                    Some("ffmpeg") => Backend::Ffmpeg,
                    Some("vt") => Backend::VideoToolbox,
                    Some("dav1d") => Backend::Dav1d,
                    Some("alpha") => Backend::FfmpegAlphaPair,
                    other => panic!("unknown backend {other:?}"),
                })
            }
            "--export" => purpose = DecodePurpose::Export,
            n => frames.push(n.parse::<usize>().expect("frame number")),
        }
    }
    if frames.is_empty() {
        let p = pm_media::probe::probe(path).unwrap_or_else(|e| panic!("{e}"));
        println!("{}", p.probe.to_json().unwrap());
        return;
    }
    let src = MediaSource::open(path).unwrap_or_else(|e| panic!("{e}"));
    let svc = DecodeService::new(ServiceConfig { cache_bytes: 0, ..Default::default() });
    let mut s = svc.session_with(&src, 1, purpose, backend).unwrap_or_else(|e| panic!("{e}"));
    println!("{:?}", s.info());
    for n in frames {
        let sample = src.index.frames[n].sample;
        match s.frame(n) {
            Ok(f) => println!("frame {n} sample {sample} pts {} {} {}", f.pts, f.format.label, f.md5_hex().unwrap()),
            Err(e) => println!("frame {n} sample {sample}: {e}"),
        }
    }
    println!("{:?}", s.stats());
}
