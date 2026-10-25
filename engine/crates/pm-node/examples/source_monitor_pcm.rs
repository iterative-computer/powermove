//! Offline float PCM from the exact unity-gain graph used by SourceAudio.
//!
//! `source_monitor_pcm <source> <output.f32le> [sample-rate]`
//! Output is interleaved stereo IEEE-754 little-endian samples, without a
//! header. Capture Chromium's `decodeAudioData` at the same sample rate and
//! compare the aligned channels against this file.

use std::env;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pm_audio::offline;
use pm_audio::{MixFormat, MixGraph, Mixer, SourceCache};
use pm_eval::Evaluator;
use pm_node::present::source_audio::{SourceMonitorTiming, source_monitor_document};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("missing source path")?);
    let output = PathBuf::from(args.next().ok_or("missing output path")?);
    let rate = args
        .next()
        .map(|s| s.to_string_lossy().parse::<u32>())
        .transpose()?
        .unwrap_or(48_000);
    if args.next().is_some() || !(8_000..=192_000).contains(&rate) {
        return Err("usage: source_monitor_pcm <source> <output.f32le> [rate 8000..192000]".into());
    }
    let timing = SourceMonitorTiming::open(&source)?;
    let duration = timing.duration;
    let doc = source_monitor_document("source", &timing)?;
    let cache = SourceCache::new(128 << 20, 0);
    let media = source.clone();
    let resolver = move |asset: &str| (asset == "source").then(|| media.clone());
    let graph = MixGraph::build(&doc, "source", &mut Evaluator::new(), &cache, &resolver)?;
    if !graph.problems.is_empty() {
        return Err(format!("source graph could not open media: {:?}", graph.problems).into());
    }
    let mut mixer = Mixer::new(
        MixFormat::stereo(rate),
        Arc::new(doc),
        Arc::new(graph),
        Evaluator::new(),
        cache,
    );
    let frames = (duration * rate as f64).round() as i64;
    let temporary = tempfile::NamedTempFile::new_in(output.parent().unwrap_or(Path::new(".")))?;
    let mut writer = BufWriter::new(temporary);
    let summary = offline::render(&mut mixer, 0, frames, 8192, |_, samples| {
        for &sample in samples {
            writer.write_all(&sample.to_le_bytes())?;
        }
        Ok(())
    })?;
    if !summary.errors.is_empty() {
        return Err(format!("source render errors: {:?}", summary.errors).into());
    }
    writer.flush()?;
    writer.into_inner()?.persist(&output)?;
    eprintln!(
        "source={} output={} rate={} channels=2 frames={} format=f32le-interleaved",
        Path::new(&source).display(),
        output.display(),
        rate,
        summary.frames
    );
    Ok(())
}
