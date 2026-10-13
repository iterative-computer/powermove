//! Every container-table index agrees with FFmpeg's demuxer: packet count,
//! presented timestamps (up to the constant FFmpeg adds when it rebases a
//! stream), and keyframes. Edit lists and MXF temporal offsets are where
//! FFmpeg and the QuickTime rules differ; those files are compared on what
//! both define.

mod common;

use common::*;
use pm_media::ff::{Input, demux_scan};
use pm_media::index::{self, IndexSource};
use std::path::Path;

fn crosscheck(path: &Path) -> Result<String, String> {
    let input = Input::open(path).map_err(|e| e.to_string())?;
    let v = input.best_video().ok_or("no video")?;
    let idx = index::build(path, &input, v).map_err(|e| e.to_string())?;
    let (pkts, tb, _) = demux_scan(path, v).map_err(|e| e.to_string())?;
    let name = path.file_name().unwrap().to_string_lossy();
    let line = format!("{name}: {:?}, {} samples, {} presented, {} sync", idx.source, idx.samples.len(), idx.frames.len(), idx.stats.sync_samples);
    if matches!(idx.source, IndexSource::DemuxScan | IndexSource::DemuxScanDecodeOrder) {
        return Ok(line);
    }
    if pkts.len() != idx.samples.len() {
        return Err(format!("{line}: FFmpeg demuxes {} packets", pkts.len()));
    }
    let fk = pkts.iter().filter(|p| p.key).count();
    if fk != idx.stats.sync_samples {
        return Err(format!("{line}: FFmpeg flags {fk} keyframes"));
    }
    // Presented timestamps in FFmpeg's units, sorted, against ours.
    let mut theirs: Vec<i128> = pkts
        .iter()
        .filter(|p| !p.discard)
        .filter_map(|p| p.pts.map(|t| t as i128 * tb.num() as i128 * idx.timebase.den() as i128))
        .collect();
    theirs.sort_unstable();
    let ours: Vec<i128> =
        idx.presented_pts().iter().map(|&t| t as i128 * idx.timebase.num() as i128 * tb.den() as i128).collect();
    if idx.stats.edit_list.is_some() && theirs.len() + 1 == ours.len() {
        // FFmpeg drops a frame that straddles the edit start; AVFoundation and
        // the index show it (S6).
        let off = theirs[0] - ours[1];
        if ours[1..].iter().zip(&theirs).all(|(a, b)| b - a == off) {
            return Ok(format!("{line} (FFmpeg drops the straddling first frame)"));
        }
    }
    if theirs.len() != ours.len() {
        return Err(format!("{line}: FFmpeg presents {} frames", theirs.len()));
    }
    let off = theirs[0] - ours[0];
    if !ours.iter().zip(&theirs).all(|(a, b)| b - a == off) {
        return Err(format!("{line}: presented timestamps differ from FFmpeg's beyond a constant offset"));
    }
    Ok(line)
}

fn run(files: &[std::path::PathBuf]) {
    let mut failures = vec![];
    for f in files {
        match crosscheck(f) {
            Ok(l) => eprintln!("{l}"),
            Err(e) => failures.push(e),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn e2e_fixtures() {
    run(&videos(&fixtures_dir(), &|_| true));
}

#[test]
#[ignore]
fn corpus() {
    run(&videos(&corpus_dir(), &|_| true));
}
