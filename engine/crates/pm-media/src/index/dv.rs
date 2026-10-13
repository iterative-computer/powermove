//! Raw DV (`.dv`, `.dif`): every frame has the same size (DV, DVCAM,
//! DVCPRO, DVCPRO50, DVCPRO HD), so the index is computed from the first
//! frame's size and the file length. Nothing past the first frame is read.

use super::{CodecConfig, Frame, FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::error::{MediaError, Result};
use std::path::Path;

pub fn build(path: &Path, input: &crate::ff::Input, stream: usize) -> Result<FrameIndex> {
    let len = std::fs::metadata(path)?.len();
    let (pos, size) = input.first_packet(stream)?;
    if pos != 0 || size == 0 {
        return Err(MediaError::unsupported("DV stream does not start at byte 0"));
    }
    let size = size as u64;
    // DIF sequences are 12,000 bytes; a frame is 10 (525/60) or 12 (625/50)
    // of them per channel.
    if size % 12_000 != 0 {
        return Err(MediaError::unsupported(format!("DV frame size {size} is not whole DIF sequences")));
    }
    let n = len / size;
    if n == 0 {
        return Err(MediaError::invalid("DV file shorter than one frame"));
    }
    let timebase = input.timebase(stream)?;
    let samples: Vec<Sample> = (0..n)
        .map(|k| Sample {
            dts: Some(k as i64),
            cts: k as i64,
            duration: 1,
            key: true,
            loc: Loc::File { offset: k * size, size: size as u32, additional: None },
            nal_type: None,
        })
        .collect();
    // The demuxer's timebase is one frame for raw DV; check that.
    let frame_duration = input.packet_duration(stream).unwrap_or(1);
    if frame_duration != 1 {
        return Err(MediaError::unsupported(format!("DV packet duration {frame_duration} in the stream timebase")));
    }
    let frames = (0..samples.len()).map(|i| Frame { sample: i, pts: i as i64 }).collect();
    let mut idx = FrameIndex {
        source: IndexSource::DvConstantFrames,
        track: 0,
        timebase,
        samples,
        frames,
        config: CodecConfig::default(),
        nal_codec: None,
        stats: IndexStats {
            detail: Some(format!(
                "{size}-byte frames{}",
                if len % size != 0 { format!(", {} trailing bytes ignored", len % size) } else { String::new() }
            )),
            ..Default::default()
        },
    };
    idx.finish();
    Ok(idx)
}
