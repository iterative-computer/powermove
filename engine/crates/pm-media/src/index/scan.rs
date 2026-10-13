//! Index for containers without frame tables (MPEG-TS, M2TS, MPEG-PS, VOB,
//! FLV, ASF, Ogg, elementary streams): FFmpeg's demuxer reads every packet;
//! nothing is decoded. Section 13: "scanning only where none exist".

use super::{CodecConfig, FrameIndex, IndexSource, IndexStats, Loc, NalCodec, Sample};
use crate::error::{MediaError, Result};
use crate::sys::ffmpeg as ff;
use std::path::Path;

/// H.264 or HEVC, for NAL-type marking.
pub fn nal_codec(id: ff::AVCodecID) -> Option<NalCodec> {
    match id {
        ff::AV_CODEC_ID_H264 => Some(NalCodec::H264),
        ff::AV_CODEC_ID_HEVC => Some(NalCodec::Hevc),
        _ => None,
    }
}

pub fn build(path: &Path, stream: usize) -> Result<FrameIndex> {
    let (pkts, tb, codec) = crate::ff::demux_scan(path, stream)?;
    if pkts.is_empty() {
        return Err(MediaError::invalid("the stream has no packets"));
    }
    let mut seen = std::collections::HashSet::new();
    for p in &pkts {
        if p.pos < 0 || !seen.insert(p.pos) {
            return Err(MediaError::unsupported(
                "packet byte positions are missing or repeat; random access cannot address packets",
            ));
        }
    }
    // Containers that carry only DTS (or neither) for some packets: a
    // missing PTS takes the DTS, and a stream without either is not indexable.
    let samples: Vec<Sample> = pkts
        .iter()
        .map(|p| {
            let cts = p.pts.or(p.dts).ok_or_else(|| MediaError::unsupported("packets without timestamps"))?;
            Ok(Sample {
                dts: p.dts,
                cts,
                duration: p.duration,
                key: p.key,
                loc: Loc::Demux { pos: p.pos },
                nal_type: p.nal_type,
            })
        })
        .collect::<Result<_>>()?;
    let nal = if pkts.iter().any(|p| p.nal_type.is_some()) { nal_codec(codec) } else { None };
    let mut idx = FrameIndex {
        source: IndexSource::DemuxScan,
        track: stream as u64,
        timebase: tb,
        samples,
        frames: vec![],
        config: CodecConfig::default(),
        nal_codec: nal,
        stats: IndexStats { scan_reads: pkts.len(), ..Default::default() },
    };
    // Packets the demuxer marks discardable (edit lists in MP4 read through
    // FFmpeg) are decoded but not presented.
    idx.frames = idx
        .samples
        .iter()
        .enumerate()
        .filter(|(i, _)| !pkts[*i].discard)
        .map(|(i, s)| super::Frame { sample: i, pts: s.cts })
        .collect();
    idx.finish();
    idx.summarize_open_gop();
    Ok(idx)
}
